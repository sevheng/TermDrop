//! The MongoDB and Redis tabs: one tab per (host, kind), each filled by its
//! own folder's `build`. See `README.md` in this folder for the contract
//! each tab is built against.
//!
//! The frame lives here and in `tabs::CoreContent`: building a tab's view
//! once when it opens, routing backup/restore progress to it, and the
//! `SERVICE_TABS` lifecycle of `src/stores/connection.js` (opening a host's
//! tab again activates the one there, `AppModel::open_service`; closing the
//! last tab of a kind on a host releases the host's pooled connections with
//! `mongodb_disconnect` / `redis_disconnect`, [`release_on_close`]). A tab
//! implements only `build(ctx, window, cx) -> AnyView` in its own folder and
//! talks to the rest of the app through [`ServiceTabContext`].

pub mod common;
mod json;
pub mod mongo;
pub mod redis;

use std::future::Future;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::{
    AnyView, App, AsyncWindowContext, Context, Entity, SharedString, Task, WeakEntity, Window,
};
use termdrop_core::{Core, CoreEvent, Events};

use self::common::{MissingSecret, next_secret_prompt};
use crate::app::AppModel;
use crate::app::display::{mongo_display_uri, redis_display_uri};
use crate::app::tab_kind::TabKind;
use crate::backend::{HostRecord, HostSummary, SecretKind};
use crate::core_backend::Services;
use crate::panels::{EventKey, PanelLinks, PanelSubscription, subscribe_key};
use crate::ui::dialogs::{PromptOptions, prompt};
use crate::ui::file_dialog::{self, Filter};
use crate::ui::notify::notify_if_unfocused;
use crate::ui::toast::{ToastKind, toast};

/// Which datastore a tab browses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DatastoreKind {
    Mongo,
    Redis,
}

impl DatastoreKind {
    /// The datastore a tab of `kind` shows, if it is one.
    pub fn of(kind: TabKind) -> Option<Self> {
        match kind {
            TabKind::Mongodb => Some(Self::Mongo),
            TabKind::Redis => Some(Self::Redis),
            TabKind::Ssh | TabKind::Local | TabKind::Vnc => None,
        }
    }

    pub fn tab_kind(self) -> TabKind {
        match self {
            Self::Mongo => TabKind::Mongodb,
            Self::Redis => TabKind::Redis,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Mongo => "MongoDB",
            Self::Redis => "Redis",
        }
    }

    /// The keyring entry the datastore's own password lives in.
    pub fn secret_kind(self) -> SecretKind {
        match self {
            Self::Mongo => SecretKind::Mongo,
            Self::Redis => SecretKind::Redis,
        }
    }

    /// The Tauri command `closeServiceTab` calls when the last tab of this
    /// kind on a host closes (`SERVICE_TABS[kind].disconnect`).
    pub fn disconnect_command(self) -> &'static str {
        match self {
            Self::Mongo => "mongodb_disconnect",
            Self::Redis => "redis_disconnect",
        }
    }

    /// The prompt shown when the datastore's password is not stored: the
    /// Vue `withMongoSecret` / `withRedisSecret` wording.
    fn secret_prompt(self) -> (&'static str, &'static str) {
        match self {
            Self::Mongo => (
                "MongoDB password required",
                "No stored password for this MongoDB connection. Enter it to continue:",
            ),
            Self::Redis => (
                "Redis password required",
                "No stored password for this Redis connection. Enter it to continue:",
            ),
        }
    }
}

/// Whether closing `tab_kind` on a host must release the host's datastore
/// connections, and which. Only when it was the last tab **of that kind**
/// for the host: the caller passes `last_of_kind_for_host`, which compares
/// kind as well as host id (`Tabs::host_has_tab_of_kind`).
pub fn release_on_close(tab_kind: TabKind, last_of_kind_for_host: bool) -> Option<DatastoreKind> {
    DatastoreKind::of(tab_kind).filter(|_| last_of_kind_for_host)
}

/// Run the core's release for `kind` on `host_id`, in the background.
pub(crate) fn release_host(services: &Services, kind: DatastoreKind, host_id: i64) {
    let core = services.core.clone();
    services.rt.spawn(async move {
        // Both only drop pooled state and never fail; the Vue store ignored
        // the result as well.
        let _ = match kind {
            DatastoreKind::Mongo => core.mongodb_disconnect(host_id).await,
            DatastoreKind::Redis => core.redis_disconnect(host_id).await,
        };
    });
}

/// Builds a datastore tab's view. [`DefaultDatastores`] is the real one.
pub trait DatastoreFactory {
    fn build(
        &self,
        kind: DatastoreKind,
        ctx: ServiceTabContext,
        window: &mut Window,
        cx: &mut App,
    ) -> AnyView;
}

/// Each kind's own `build`.
pub struct DefaultDatastores;

impl DatastoreFactory for DefaultDatastores {
    fn build(
        &self,
        kind: DatastoreKind,
        ctx: ServiceTabContext,
        window: &mut Window,
        cx: &mut App,
    ) -> AnyView {
        match kind {
            DatastoreKind::Mongo => mongo::build(ctx, window, cx),
            DatastoreKind::Redis => redis::build(ctx, window, cx),
        }
    }
}

/// Keeps an op subscription alive; dropping it unsubscribes.
pub type OpSubscription = PanelSubscription;

/// Everything the frame gives a datastore tab. Cheap to clone; keep one in
/// the tab's view.
#[derive(Clone)]
pub struct ServiceTabContext {
    pub kind: DatastoreKind,
    /// The tab's id in the strip (`mongo-3`, `redis-4`).
    pub tab_id: String,
    /// The MongoDB- or Redis-only host the tab is on.
    pub host_id: i64,
    /// The core and the tokio runtime its async calls run on.
    pub services: Arc<Services>,
    /// The app model: hosts, tabs, theme, settings. Read it; change it only
    /// through its methods.
    pub model: Entity<AppModel>,
    links: Rc<PanelLinks>,
}

impl ServiceTabContext {
    pub(crate) fn new(
        kind: DatastoreKind,
        tab_id: String,
        host_id: i64,
        services: Arc<Services>,
        model: Entity<AppModel>,
        links: Rc<PanelLinks>,
    ) -> Self {
        Self {
            kind,
            tab_id,
            host_id,
            services,
            model,
            links,
        }
    }

    pub fn core(&self) -> &Arc<Core> {
        &self.services.core
    }

    /// The event sink for the calls that report progress (`mongodb_dump`,
    /// `mongodb_restore`, `mongodb_restore_archive`, `redis_export`,
    /// `redis_import`). What they emit comes back through
    /// [`Self::subscribe_op`].
    pub fn events(&self) -> Events {
        self.services.events()
    }

    /// Run a core future on the runtime. Prefer [`Self::run`], which also
    /// brings the result back to the view.
    pub fn spawn<F>(&self, future: F) -> tokio::task::JoinHandle<F::Output>
    where
        F: Future + Send + 'static,
        F::Output: Send + 'static,
    {
        self.services.rt.spawn(future)
    }

    /// Run a blocking closure (a local file read, `scan_restore_folder`,
    /// `redis::read_backup_header`) off the UI thread.
    pub fn spawn_blocking<T, F>(&self, f: F) -> tokio::task::JoinHandle<T>
    where
        F: FnOnce() -> T + Send + 'static,
        T: Send + 'static,
    {
        self.services.rt.spawn_blocking(f)
    }

    /// Run `future` on the runtime and hand its result to `done` on the view,
    /// on the UI thread. Keep the returned task in the view: dropping it
    /// cancels the UI half (the core call still runs to its end). A panicked
    /// or cancelled job arrives as `Err("task failed: ...")`.
    pub fn run<V, T, Fut>(
        &self,
        window: &mut Window,
        cx: &mut Context<V>,
        future: Fut,
        done: impl FnOnce(&mut V, Result<T, String>, &mut Window, &mut Context<V>) + 'static,
    ) -> Task<()>
    where
        V: 'static,
        T: Send + 'static,
        Fut: Future<Output = Result<T, String>> + Send + 'static,
    {
        let job = self.spawn(future);
        cx.spawn_in(window, async move |this, cx| {
            let result = job.await.unwrap_or_else(|e| Err(task_failed(e)));
            deliver(this, cx, result, done);
        })
    }

    /// [`Self::run`] with the Vue apps' prompt-and-retry for a password that
    /// is not stored (`withMongoSecret` / `withRedisSecret`).
    ///
    /// `call(core, ssh_password)` runs once; when it fails because a stored
    /// password is missing ([`common::missing_secret`]), the user is asked,
    /// and it runs again. Each password is asked for **at most once** per
    /// call ([`common::next_secret_prompt`]): a second miss of the same one
    /// would loop against a genuinely wrong password, so that result is
    /// final. A tunnelled Redis host missing both its own password and its
    /// bastion's is asked for one, then the other. Cancelling a prompt
    /// delivers the error it was asked for.
    ///
    /// - The datastore's own password is stored in the keyring before the
    ///   retry (so the next call and the other app find it).
    /// - A Redis host's **bastion** password (its tunnel's SSH host) is
    ///   stored under that SSH host, as the SSH tab stores a typed password,
    ///   and also passed as `ssh_password`, for `redis_connect`'s
    ///   `ssh_password`. Only `redis_connect` can use it, and only it can
    ///   need it: once the tunnel is up every other call reuses it.
    ///
    /// Use it for every call that is first to reach the server after the tab
    /// opens or after a refresh (`mongodb_list_databases`,
    /// `mongodb_list_collections`, `redis_connect`). The Vue tabs wrap
    /// exactly those.
    pub fn run_with_secret<V, T, F, Fut>(
        &self,
        window: &mut Window,
        cx: &mut Context<V>,
        call: F,
        done: impl FnOnce(&mut V, Result<T, String>, &mut Window, &mut Context<V>) + 'static,
    ) -> Task<()>
    where
        V: 'static,
        T: Send + 'static,
        F: Fn(Arc<Core>, Option<String>) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<T, String>> + Send + 'static,
    {
        let ctx = self.clone();
        let first = self.spawn(call(self.core().clone(), None));
        cx.spawn_in(window, async move |this, cx| {
            let mut result = first.await.unwrap_or_else(|e| Err(task_failed(e)));
            // At most one prompt per secret, so at most two retries.
            let mut asked = Vec::new();
            let mut ssh_password = None;
            while let Err(err) = &result {
                let Some(need) = next_secret_prompt(ctx.kind, err, &asked) else {
                    break;
                };
                asked.push(need);
                let Some(answer) = ctx.ask_secret(need, cx).await else {
                    break;
                };
                // A bastion password is passed on every later try; the
                // datastore's own is read back from the keyring.
                if answer.is_some() {
                    ssh_password = answer;
                }
                result = ctx
                    .spawn(call(ctx.core().clone(), ssh_password.clone()))
                    .await
                    .unwrap_or_else(|e| Err(task_failed(e)));
            }
            deliver(this, cx, result, done);
        })
    }

    /// Ask for the missing password and store it. `None` when the user
    /// cancelled or left it empty; `Some(ssh_password)` to retry with.
    async fn ask_secret(
        &self,
        need: MissingSecret,
        cx: &mut AsyncWindowContext,
    ) -> Option<Option<String>> {
        let bastion = match need {
            MissingSecret::Datastore => None,
            MissingSecret::Bastion => {
                let via = cx
                    .update(|_, cx| self.host_record(cx))
                    .ok()?
                    .ok()
                    .flatten()?
                    .redis_tunnel_host_id?;
                let name = cx
                    .update(|_, cx| self.model.read(cx).host(via).map(|h| h.name.clone()))
                    .ok()
                    .flatten()
                    .unwrap_or_else(|| "the SSH host".into());
                Some((via, name))
            }
        };
        let options = match &bastion {
            None => {
                let (title, message) = self.kind.secret_prompt();
                PromptOptions::new(title).message(message)
            }
            Some((_, name)) => PromptOptions::new("SSH password required").message(format!(
                "This Redis connection tunnels through {name}, and its password is not \
                 stored. Enter it to continue:"
            )),
        }
        .password()
        .confirm_label("Connect");

        let (tx, rx) = async_channel::bounded::<Option<String>>(1);
        cx.update(|window, cx| {
            prompt(window, cx, options, move |answer, _, _| {
                let _ = tx.try_send(answer);
            })
        })
        .ok()?;
        // A dialog torn down without an answer drops the sender: a cancel.
        let password = rx.recv().await.ok().flatten().filter(|p| !p.is_empty())?;

        let (kind, host_id) = match bastion {
            None => (self.kind.secret_kind(), self.host_id),
            Some((via, _)) => (SecretKind::SshPassword, via),
        };
        let stored = cx
            .update(|_, cx| self.model.read(cx).store_secret(kind, host_id, &password))
            .ok()?;
        if let Err(e) = stored {
            // The retry still gets the password; only the next call would
            // ask again. The SSH tab does the same.
            tracing::warn!("Could not store the password: {e}");
        }
        Some(bastion.map(|_| password))
    }

    /// Call `handler` on `view` for every progress and cancelled event of
    /// the backup or restore `op_id`, until the subscription is dropped or
    /// the view is released. Events arrive on the UI thread in the order the
    /// core emitted them; only this tab's kind is delivered (a Redis op id
    /// never sees MongoDB events).
    ///
    /// Subscribe **before** starting the call: the id is yours
    /// ([`Self::new_op_id`]), so nothing can be missed that way.
    pub fn subscribe_op<V: 'static>(
        &self,
        op_id: &str,
        view: WeakEntity<V>,
        handler: impl Fn(&mut V, &CoreEvent, &mut Context<V>) + 'static,
    ) -> OpSubscription {
        let key = match self.kind {
            DatastoreKind::Mongo => EventKey::MongoOp(op_id.to_string()),
            DatastoreKind::Redis => EventKey::RedisOp(op_id.to_string()),
        };
        subscribe_key(&self.links, key, view, handler)
    }

    /// A fresh op id for a backup or restore (`mongo-op-<uuid>`,
    /// `redis-op-<uuid>`).
    pub fn new_op_id(&self) -> String {
        let prefix = match self.kind {
            DatastoreKind::Mongo => "mongo-op",
            DatastoreKind::Redis => "redis-op",
        };
        format!("{prefix}-{}", uuid::Uuid::new_v4())
    }

    /// Ask the backup or restore `op_id` to stop (`mongodb_cancel` /
    /// `redis_cancel`). The call then ends with `Err("cancelled")` and a
    /// cancelled event. Quick: it only flips a flag.
    pub fn cancel_op(&self, op_id: &str) {
        match self.kind {
            DatastoreKind::Mongo => self.core().mongodb_cancel(op_id),
            DatastoreKind::Redis => self.core().redis_cancel(op_id),
        }
    }

    /// What `redis_export` writes into a backup's header after "TermDrop ".
    pub fn app_version(&self) -> &'static str {
        concat!(env!("CARGO_PKG_VERSION"), " (native)")
    }

    // ---- the host ------------------------------------------------------

    /// The host's row as the sidebar shows it (no URI, no secret).
    pub fn host(&self, cx: &App) -> Option<HostSummary> {
        self.model.read(cx).host(self.host_id).cloned()
    }

    pub fn host_name(&self, cx: &App) -> Option<String> {
        self.model
            .read(cx)
            .host(self.host_id)
            .map(|h| h.name.clone())
    }

    /// The host's full row, read from the database now (it may have been
    /// edited since the tab opened). Its URI carries no password the app put
    /// there, but a hand-written row may: never show it; use
    /// [`Self::display_uri`].
    pub fn host_record(&self, cx: &App) -> Result<Option<HostRecord>, String> {
        self.model.read(cx).host_record(self.host_id)
    }

    /// The connection URI with any credentials dropped, for a header: the
    /// sidebar's display form. Empty when the host has none. (After
    /// `redis_connect`, `RedisServerInfo::display_uri` is the core's own
    /// masked form of what was dialled.)
    pub fn display_uri(&self, cx: &App) -> String {
        let Ok(Some(record)) = self.host_record(cx) else {
            return String::new();
        };
        match self.kind {
            DatastoreKind::Mongo => record
                .mongo_uri
                .as_deref()
                .map(mongo_display_uri)
                .unwrap_or_default(),
            DatastoreKind::Redis => record
                .redis_uri
                .as_deref()
                .map(redis_display_uri)
                .unwrap_or_default(),
        }
    }

    /// Whether this tab is the active one, so on screen. Observe
    /// `self.model` to hear when that may have changed.
    pub fn is_shown(&self, cx: &App) -> bool {
        self.model.read(cx).active_tab_id() == Some(self.tab_id.as_str())
    }

    /// Close this tab (no confirm: the caller decides). The frame then runs
    /// the release if it was the host's last tab of this kind.
    pub fn close_tab(&self, cx: &mut App) {
        let id = self.tab_id.clone();
        self.model.update(cx, |m, cx| {
            m.close_tab(&id, cx);
        });
    }

    // ---- settings ------------------------------------------------------

    /// A setting from the shared `settings` table (the Tauri app's keys).
    pub fn setting(&self, cx: &App, key: &str) -> Option<String> {
        self.model.read(cx).setting(key)
    }

    /// Write a setting; other views hear `AppEvent::SettingChanged(key)`.
    pub fn set_setting(&self, key: &str, value: &str, cx: &mut App) {
        self.model.update(cx, |m, cx| m.set_setting(key, value, cx));
    }

    // ---- the window ----------------------------------------------------

    /// A toast. The kind is required: an error stays until dismissed.
    pub fn toast(
        &self,
        window: &mut Window,
        cx: &mut App,
        message: impl Into<String>,
        kind: ToastKind,
    ) {
        toast(window, cx, message.into(), kind);
    }

    /// A desktop notification, only when the window is not focused (the
    /// toast covers the case where it is). For a finished backup or restore:
    /// "Backup complete" / "Restore complete". Never fails.
    pub fn notify_if_unfocused(
        &self,
        window: &Window,
        cx: &App,
        title: impl Into<SharedString>,
        body: impl Into<SharedString>,
    ) {
        notify_if_unfocused(window, cx, title, body);
    }

    /// Native "open file" dialog (a backup to restore). `filters` in order,
    /// the first selected; end with `file_dialog::ALL_FILES` to offer
    /// everything.
    pub fn pick_file(
        &self,
        window: &Window,
        cx: &App,
        title: &str,
        filters: &[Filter<'_>],
        on_pick: impl FnOnce(PathBuf, &mut Window, &mut App) + 'static,
    ) {
        file_dialog::pick_file(window, cx, title, filters, on_pick);
    }

    /// Native "save file" dialog (a backup's destination).
    pub fn save_file(
        &self,
        window: &Window,
        cx: &App,
        title: &str,
        file_name: &str,
        filters: &[Filter<'_>],
        on_pick: impl FnOnce(PathBuf, &mut Window, &mut App) + 'static,
    ) {
        file_dialog::save_file(window, cx, title, file_name, filters, on_pick);
    }

    /// Native "choose folder" dialog (a MongoDB dump's folder).
    pub fn pick_folder(
        &self,
        window: &Window,
        cx: &App,
        title: &str,
        on_pick: impl FnOnce(PathBuf, &mut Window, &mut App) + 'static,
    ) {
        file_dialog::pick_folder(window, cx, title, on_pick);
    }
}

fn task_failed(e: tokio::task::JoinError) -> String {
    format!("task failed: {e}")
}

/// Hand `result` to `done` on the view, if it is still there.
fn deliver<V: 'static, T>(
    this: WeakEntity<V>,
    cx: &mut AsyncWindowContext,
    result: Result<T, String>,
    done: impl FnOnce(&mut V, Result<T, String>, &mut Window, &mut Context<V>) + 'static,
) {
    let _ = cx.update(|window, cx| this.update(cx, |view, cx| done(view, result, window, cx)));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_last_tab_of_a_datastore_kind_releases_the_host() {
        assert_eq!(
            release_on_close(TabKind::Mongodb, true),
            Some(DatastoreKind::Mongo)
        );
        assert_eq!(
            release_on_close(TabKind::Redis, true),
            Some(DatastoreKind::Redis)
        );
        assert_eq!(release_on_close(TabKind::Redis, false), None);
        // Shells and VNC have their own teardown.
        for kind in [TabKind::Ssh, TabKind::Local, TabKind::Vnc] {
            assert_eq!(release_on_close(kind, true), None);
        }
    }

    #[test]
    fn kinds_map_to_the_vue_service_tabs_table() {
        assert_eq!(
            DatastoreKind::Mongo.disconnect_command(),
            "mongodb_disconnect"
        );
        assert_eq!(
            DatastoreKind::Redis.disconnect_command(),
            "redis_disconnect"
        );
        for kind in [DatastoreKind::Mongo, DatastoreKind::Redis] {
            assert_eq!(DatastoreKind::of(kind.tab_kind()), Some(kind));
        }
        assert_eq!(DatastoreKind::of(TabKind::Vnc), None);
        assert_eq!(DatastoreKind::Mongo.secret_kind(), SecretKind::Mongo);
        assert_eq!(DatastoreKind::Redis.secret_kind(), SecretKind::Redis);
    }

    #[test]
    fn a_closed_tab_is_released_once_its_kind_is_gone_from_the_host() {
        use crate::app::tabs::{Tab, Tabs};
        let mut tabs = Tabs::default();
        tabs.open(Tab::new("s", Some(7), TabKind::Ssh, "s"));
        tabs.open(Tab::new("r", Some(7), TabKind::Redis, "r"));
        tabs.close("r");
        // The shell on the same host does not hold the Redis pool open.
        let last = !tabs.host_has_tab_of_kind(7, TabKind::Redis);
        assert_eq!(
            release_on_close(TabKind::Redis, last),
            Some(DatastoreKind::Redis)
        );
    }

    /// The bundled tools land next to the binary (`build.rs`), where
    /// `resolve_mongo_tool` looks: this test binary runs from
    /// `target/<profile>/deps`, whose parent is where the app binary is.
    #[test]
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    fn the_bundled_mongo_tools_are_next_to_the_binary() {
        let exe = std::env::current_exe().unwrap();
        let profile = exe.parent().and_then(|d| d.parent()).unwrap();
        for tool in ["mongodump", "mongorestore"] {
            assert!(
                profile.join(tool).exists(),
                "{tool} missing from {}",
                profile.display()
            );
        }
    }
}
