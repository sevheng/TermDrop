//! The right-hand panel beside an SSH tab: SFTP, Tunnels, Docker and
//! Security, as `MainWindow.vue` stacks them. See `README.md` in this folder
//! for the contract each panel is built against.
//!
//! The frame (the tab strip, the resizable width, which panel is showing,
//! when a panel is built and when it is dropped) lives in
//! `shell::main_window` and `tabs::CoreContent`. A panel only implements
//! `build(ctx, window, cx) -> AnyView` in its own folder and talks to the
//! rest of the app through [`PanelContext`].

pub mod common;
pub mod docker;
pub mod security;
pub mod sftp;
pub mod terminal_insert;
pub mod tunnels;

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::future::Future;
use std::rc::{Rc, Weak};
use std::sync::Arc;

use gpui_kit::{AnyView, App, Context, Entity, WeakEntity, Window};
use termdrop_core::{Core, CoreEvent, Events, PtyKind};

use crate::app::AppModel;
use crate::core_backend::Services;
use crate::tabs::{Hub, Terminals};
use crate::ui::dialogs::{ConfirmOptions, confirm};

/// One of the right panel's tabs, in strip order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum PanelKind {
    #[default]
    Sftp,
    Tunnels,
    Docker,
    Security,
}

/// How long a panel's view lives, which is also what it is keyed by.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PanelScope {
    /// One view per SSH tab, dropped when that tab closes. The SFTP panel:
    /// each tab has its own SFTP connection and its own current directory.
    PerTab,
    /// One view per host, shared by every SSH tab on it and dropped when the
    /// host's last SSH tab closes, which is also when the core drops the
    /// host's exec session and its Docker and security caches.
    PerHost,
}

impl PanelKind {
    pub const ALL: [PanelKind; 4] = [Self::Sftp, Self::Tunnels, Self::Docker, Self::Security];

    pub fn label(self) -> &'static str {
        match self {
            Self::Sftp => "SFTP",
            Self::Tunnels => "Tunnels",
            Self::Docker => "Docker",
            Self::Security => "Security",
        }
    }

    /// `MainWindow.vue` keys the SFTP panel by the tab's `sftpSessionId` and
    /// the other three by `hostId`.
    pub fn scope(self) -> PanelScope {
        match self {
            Self::Sftp => PanelScope::PerTab,
            Self::Tunnels | Self::Docker | Self::Security => PanelScope::PerHost,
        }
    }
}

/// Builds a panel's view. [`DefaultPanels`] is the real one; tests and the
/// demo can supply another.
pub trait PanelFactory {
    fn build(
        &self,
        kind: PanelKind,
        ctx: PanelContext,
        window: &mut Window,
        cx: &mut App,
    ) -> AnyView;
}

/// Each kind's own `build`.
pub struct DefaultPanels;

impl PanelFactory for DefaultPanels {
    fn build(
        &self,
        kind: PanelKind,
        ctx: PanelContext,
        window: &mut Window,
        cx: &mut App,
    ) -> AnyView {
        match kind {
            PanelKind::Sftp => sftp::build(ctx, window, cx),
            PanelKind::Tunnels => tunnels::build(ctx, window, cx),
            PanelKind::Docker => docker::build(ctx, window, cx),
            PanelKind::Security => security::build(ctx, window, cx),
        }
    }
}

// ---- events --------------------------------------------------------------

/// Which core events a panel can subscribe to.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum EventKey {
    /// `CoreEvent::SftpProgress` for transfers on this SFTP session id.
    SftpTransfer(String),
    /// `PtyConnected` / `PtyDisconnected` / `PtyError` of the docker exec PTY
    /// with this id (`PtyKind::ExecPty`). Its output does not come as events:
    /// it goes to the feed given to `Bridge::attach_exec_pty`.
    ExecPty(String),
    /// `CoreEvent::MongoProgress` / `MongoCancelled` of the MongoDB dump or
    /// restore with this op id (the MongoDB tab, `datastores::mongo`).
    MongoOp(String),
    /// `CoreEvent::RedisProgress` / `RedisCancelled` of the Redis backup or
    /// restore with this op id (the Redis tab, `datastores::redis`).
    RedisOp(String),
    /// `CoreEvent::VncBell` / `VncClipboard` / `VncResize` /
    /// `VncDisconnected` / `VncError` of the remote-desktop session with this
    /// id (`datastores::vnc`). Held until subscribed: the id is known only
    /// once `vnc_connect` returns, and the session may already have ended.
    VncSession(String),
}

impl EventKey {
    /// The panel subscription an event belongs to, if any. Shell events go to
    /// terminal tabs instead (`tabs::session_of`).
    pub fn of(event: &CoreEvent) -> Option<EventKey> {
        match event {
            CoreEvent::SftpProgress {
                sftp_session_id, ..
            } => Some(Self::SftpTransfer(sftp_session_id.clone())),
            CoreEvent::PtyConnected {
                kind: PtyKind::ExecPty,
                id,
            }
            | CoreEvent::PtyDisconnected {
                kind: PtyKind::ExecPty,
                id,
            }
            | CoreEvent::PtyError {
                kind: PtyKind::ExecPty,
                id,
                ..
            }
            | CoreEvent::PtyData {
                kind: PtyKind::ExecPty,
                id,
                ..
            } => Some(Self::ExecPty(id.clone())),
            CoreEvent::MongoProgress(p) => Some(Self::MongoOp(p.op_id.clone())),
            CoreEvent::MongoCancelled { op_id, .. } => Some(Self::MongoOp(op_id.clone())),
            CoreEvent::RedisProgress(p) => Some(Self::RedisOp(p.op_id.clone())),
            CoreEvent::RedisCancelled { op_id, .. } => Some(Self::RedisOp(op_id.clone())),
            CoreEvent::VncBell { session_id }
            | CoreEvent::VncClipboard { session_id, .. }
            | CoreEvent::VncResize { session_id, .. }
            | CoreEvent::VncDisconnected { session_id }
            | CoreEvent::VncError { session_id, .. } => Some(Self::VncSession(session_id.clone())),
            _ => None,
        }
    }
}

/// A subscriber's callback. Returns false once its view is gone, which
/// removes it.
type Callback = Rc<dyn Fn(&CoreEvent, &mut App) -> bool>;

/// Panel subscriptions, by key. Held in the tabs hub, which routes every core
/// event on the UI thread.
#[derive(Default)]
pub(crate) struct PanelRoutes {
    next: u64,
    subs: HashMap<EventKey, Vec<(u64, Callback)>>,
    /// Events for a key nobody has subscribed to yet, for the keys whose id
    /// is learned only after the core starts emitting ([`EventKey::VncSession`]).
    held: HashMap<EventKey, Vec<CoreEvent>>,
}

/// How many events to hold per key, and how many keys, before giving up on
/// a subscriber that never came.
const MAX_HELD_PER_KEY: usize = 16;
const MAX_HELD_KEYS: usize = 64;

impl PanelRoutes {
    fn add(&mut self, key: EventKey, callback: Callback) -> u64 {
        self.next += 1;
        self.subs
            .entry(key)
            .or_default()
            .push((self.next, callback));
        self.next
    }

    fn remove(&mut self, key: &EventKey, token: u64) {
        if let Some(list) = self.subs.get_mut(key) {
            list.retain(|(t, _)| *t != token);
            if list.is_empty() {
                self.subs.remove(key);
            }
        }
    }

    /// The callbacks for `key`, cloned so the hub's borrow can be released
    /// before any of them runs (a callback may subscribe or unsubscribe).
    pub(crate) fn callbacks(&self, key: &EventKey) -> Vec<(u64, Callback)> {
        self.subs.get(key).cloned().unwrap_or_default()
    }

    /// Keep `event` for the first subscriber to `key`, when the key is one
    /// whose events can arrive before anyone could subscribe.
    fn hold(&mut self, key: &EventKey, event: &CoreEvent) {
        if !matches!(key, EventKey::VncSession(_)) {
            return;
        }
        if self.held.len() >= MAX_HELD_KEYS && !self.held.contains_key(key) {
            self.held.clear();
        }
        let held = self.held.entry(key.clone()).or_default();
        if held.len() < MAX_HELD_PER_KEY {
            held.push(event.clone());
        }
    }

    fn take_held(&mut self, key: &EventKey) -> Vec<CoreEvent> {
        self.held.remove(key).unwrap_or_default()
    }

    pub(crate) fn remove_dead(&mut self, key: &EventKey, dead: &[u64]) {
        for token in dead {
            self.remove(key, *token);
        }
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.subs.values().map(Vec::len).sum()
    }
}

/// Deliver `event` to the panels subscribed to it. Called by the hub.
pub(crate) fn dispatch_to_panels(
    hub: &Rc<RefCell<Hub>>,
    key: EventKey,
    event: &CoreEvent,
    cx: &mut App,
) {
    let callbacks = hub.borrow().panels.callbacks(&key);
    if callbacks.is_empty() {
        hub.borrow_mut().panels.hold(&key, event);
        return;
    }
    let dead: Vec<u64> = callbacks
        .into_iter()
        .filter(|(_, f)| !f(event, cx))
        .map(|(t, _)| t)
        .collect();
    if !dead.is_empty() {
        hub.borrow_mut().panels.remove_dead(&key, &dead);
    }
}

/// Route the core events under `key` to `handler` on `view`. Shared by
/// [`PanelContext::subscribe`] and the datastore tabs' `subscribe_op`.
pub(crate) fn subscribe_key<V: 'static>(
    links: &PanelLinks,
    key: EventKey,
    view: WeakEntity<V>,
    handler: impl Fn(&mut V, &CoreEvent, &mut Context<V>) + 'static,
) -> PanelSubscription {
    let callback: Callback =
        Rc::new(move |event, cx| view.update(cx, |v, cx| handler(v, event, cx)).is_ok());
    let token = links.hub.borrow_mut().panels.add(key.clone(), callback);
    PanelSubscription {
        hub: Rc::downgrade(&links.hub),
        key,
        token,
    }
}

/// [`subscribe_key`], also returning what arrived for `key` before it was
/// subscribed, for the caller to apply in order (it is inside its own
/// update, so it cannot be called back here).
pub(crate) fn subscribe_key_with_held<V: 'static>(
    links: &PanelLinks,
    key: EventKey,
    view: WeakEntity<V>,
    handler: impl Fn(&mut V, &CoreEvent, &mut Context<V>) + 'static,
) -> (PanelSubscription, Vec<CoreEvent>) {
    let held = links.hub.borrow_mut().panels.take_held(&key);
    (subscribe_key(links, key, view, handler), held)
}

/// Keeps a subscription alive; dropping it unsubscribes. Store it in the
/// view that subscribed.
#[must_use = "dropping the subscription unsubscribes at once"]
pub struct PanelSubscription {
    hub: Weak<RefCell<Hub>>,
    key: EventKey,
    token: u64,
}

impl Drop for PanelSubscription {
    fn drop(&mut self) {
        if let Some(hub) = self.hub.upgrade()
            && let Ok(mut hub) = hub.try_borrow_mut()
        {
            hub.panels.remove(&self.key, self.token);
        }
        // Dropped mid-dispatch: the callback finds its view gone (or is
        // simply never called again) and is removed then.
    }
}

// ---- the context ---------------------------------------------------------

/// What the frame shares with every panel it builds.
pub(crate) struct PanelLinks {
    pub(crate) hub: Rc<RefCell<Hub>>,
    pub(crate) terminals: Terminals,
    /// The "typed, not run" explanation is shown before the first insert of
    /// the run only, as `MainWindow.vue`'s `terminalInsertConfirmed`.
    insert_confirmed: Cell<bool>,
    /// Views floating over the window ([`PanelContext::float`]), in the order
    /// they were added, each under its handle's token.
    floating: RefCell<Vec<(u64, AnyView)>>,
    next_float: Cell<u64>,
}

impl PanelLinks {
    pub(crate) fn new(hub: Rc<RefCell<Hub>>, terminals: Terminals) -> Rc<Self> {
        Rc::new(Self {
            hub,
            terminals,
            insert_confirmed: Cell::new(false),
            floating: RefCell::default(),
            next_float: Cell::new(0),
        })
    }

    /// The floating views, for the window to draw over its content.
    pub(crate) fn floating(&self) -> Vec<AnyView> {
        self.floating
            .borrow()
            .iter()
            .map(|(_, v)| v.clone())
            .collect()
    }
}

/// Keeps a view floating over the window; dropping it takes the view down.
/// Store it in the panel that floated the view.
#[must_use = "dropping the handle removes the floating view at once"]
pub struct FloatingHandle {
    links: Weak<PanelLinks>,
    token: u64,
}

impl Drop for FloatingHandle {
    fn drop(&mut self) {
        if let Some(links) = self.links.upgrade()
            && let Ok(mut floating) = links.floating.try_borrow_mut()
        {
            floating.retain(|(t, _)| *t != self.token);
        }
    }
}

/// Why [`PanelContext::show_terminal_pane`] showed nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PaneRefused {
    /// The active tab is not an SSH tab on this panel's host, or its
    /// terminal has gone.
    NoTerminal,
}

impl PaneRefused {
    /// What to tell the user, as a warning toast.
    pub fn message(&self) -> &'static str {
        match self {
            Self::NoTerminal => "Open a terminal on this host to show the pane",
        }
    }
}

/// Why [`PanelContext::insert_command`] typed nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InsertRefused {
    /// The active tab is not a connected shell on this panel's host.
    NoShell,
    /// Empty, too long, or holding a control character (a newline among
    /// them): see `terminal_insert`.
    NotInsertable,
}

/// Everything a panel gets from the frame. Cheap to clone; keep one in the
/// panel's view.
#[derive(Clone)]
pub struct PanelContext {
    pub kind: PanelKind,
    /// The host the panel is about. Always an SSH host.
    pub host_id: i64,
    /// The tab that owns a per-tab panel (SFTP); `None` for a per-host one,
    /// which is shared by every SSH tab on the host.
    pub tab_id: Option<String>,
    /// The tab's SFTP session, for the SFTP panel: always `Some` there (the
    /// frame builds it only once `sftp_connect` succeeded) and stable for
    /// the tab's life, because a reconnect replaces the session under the
    /// same id. `None` for the other panels.
    pub sftp_session_id: Option<String>,
    /// The core and the tokio runtime its async calls run on.
    pub services: Arc<Services>,
    /// The app model: hosts, tabs, theme, font size. Read it; change it only
    /// through its methods.
    pub model: Entity<AppModel>,
    links: Rc<PanelLinks>,
}

impl PanelContext {
    pub(crate) fn new(
        kind: PanelKind,
        host_id: i64,
        tab_id: Option<String>,
        sftp_session_id: Option<String>,
        services: Arc<Services>,
        model: Entity<AppModel>,
        links: Rc<PanelLinks>,
    ) -> Self {
        Self {
            kind,
            host_id,
            tab_id,
            sftp_session_id,
            services,
            model,
            links,
        }
    }

    pub fn core(&self) -> &Arc<Core> {
        &self.services.core
    }

    /// The event sink to hand to core calls that report progress
    /// (`sftp_upload`, `sftp_download`, `exec_pty_connect`). What it emits
    /// comes back through [`Self::subscribe`].
    pub fn events(&self) -> Events {
        self.services.events()
    }

    /// Run a core future on the runtime. Await the handle from
    /// `cx.spawn_in(window, async move |this, cx| ...)` on the UI side:
    ///
    /// ```ignore
    /// let core = ctx.core().clone();
    /// let job = ctx.spawn(async move { core.docker_ps(host_id, true).await });
    /// self.task = Some(cx.spawn_in(window, async move |this, cx| {
    ///     let result = job.await.unwrap_or_else(|e| Err(e.to_string()));
    ///     let _ = cx.update(|window, cx| {
    ///         this.update(cx, |this, cx| this.loaded(result, window, cx))
    ///     });
    /// }));
    /// ```
    pub fn spawn<F>(&self, future: F) -> tokio::task::JoinHandle<F::Output>
    where
        F: Future + Send + 'static,
        F::Output: Send + 'static,
    {
        self.services.rt.spawn(future)
    }

    /// Run a blocking closure (a sync core call, a local file read) off the
    /// UI thread.
    pub fn spawn_blocking<T, F>(&self, f: F) -> tokio::task::JoinHandle<T>
    where
        F: FnOnce() -> T + Send + 'static,
        T: Send + 'static,
    {
        self.services.rt.spawn_blocking(f)
    }

    /// Call `handler` on `view` for every core event under `key`, until the
    /// returned subscription is dropped or the view is released. Events
    /// arrive on the UI thread, in the order the core emitted them.
    ///
    /// Subscribe to an exec PTY's key **before** `exec_pty_connect`: the id
    /// is the caller's, so nothing can be missed that way.
    pub fn subscribe<V: 'static>(
        &self,
        key: EventKey,
        view: WeakEntity<V>,
        handler: impl Fn(&mut V, &CoreEvent, &mut Context<V>) + 'static,
    ) -> PanelSubscription {
        subscribe_key(&self.links, key, view, handler)
    }

    /// A fresh id for a docker exec PTY: the caller picks it
    /// (`Core::exec_pty_connect`), so subscribe to it before connecting.
    pub fn new_pty_session_id(&self) -> String {
        format!("exec-{}", uuid::Uuid::new_v4())
    }

    /// The terminal colours for the current theme, for a pane's own
    /// `TerminalView` (the Docker logs/exec pane).
    pub fn terminal_palette(&self, cx: &App) -> termdrop_terminal::Palette {
        crate::tabs::term_palette(self.model.read(cx).theme())
    }

    /// Whether this panel is on screen now (see `AppModel::panel_shown`).
    /// Observe `self.model` to hear when that may have changed:
    /// `cx.observe(&ctx.model, |this, _, cx| ...)`.
    pub fn is_shown(&self, cx: &App) -> bool {
        self.model
            .read(cx)
            .panel_shown(self.kind, self.host_id, self.tab_id.as_deref())
    }

    /// The host's display name.
    pub fn host_name(&self, cx: &App) -> Option<String> {
        self.model
            .read(cx)
            .host(self.host_id)
            .map(|h| h.name.clone())
    }

    /// The active tab, when it is an SSH tab on this panel's host (which it
    /// is whenever the panel is on screen) and its terminal is still open.
    fn active_terminal(
        &self,
        cx: &App,
    ) -> Option<(crate::app::tabs::Tab, Entity<crate::tabs::TerminalTab>)> {
        let tab = self.model.read(cx).active_tab()?.clone();
        if tab.host_id != Some(self.host_id) || !tab.kind.has_right_panel() {
            return None;
        }
        let view = self.links.terminals.borrow().get(&tab.id)?.upgrade()?;
        Some((tab, view))
    }

    /// The backend session id of the active SSH tab on this host, if it has
    /// one. For calls that address the shell itself; prefer
    /// [`Self::insert_command`] for typing into it.
    pub fn active_session_id(&self, cx: &App) -> Option<String> {
        let (_, view) = self.active_terminal(cx)?;
        view.read(cx).session_id()
    }

    /// Whether [`Self::insert_command`] would type into the active tab right
    /// now: it is a connected shell on this host. For deciding whether to
    /// offer "Send to terminal" at all; the insert checks again when called.
    pub fn can_insert(&self, cx: &App) -> bool {
        self.active_terminal(cx)
            .is_some_and(|(tab, _)| terminal_insert::can_receive_command(Some(&tab)))
    }

    /// Float `view` over the window: above every panel and terminal, below
    /// dialogs, notifications, menus and tooltips. Non-modal, for a tool
    /// window such as the SFTP editor; it positions itself (normally with
    /// `anchored()` in window coordinates) and should `occlude()` so clicks
    /// do not reach what is under it.
    ///
    /// The window draws it every frame whether or not this panel is shown,
    /// so the view decides: render nothing while [`Self::is_shown`] is false
    /// to hide with the panel. Keep the handle in the panel; dropping it (the
    /// panel going with its tab or host) removes the view.
    pub fn float(&self, view: AnyView) -> FloatingHandle {
        let token = self.links.next_float.get() + 1;
        self.links.next_float.set(token);
        self.links.floating.borrow_mut().push((token, view));
        FloatingHandle {
            links: Rc::downgrade(&self.links),
            token,
        }
    }

    /// Type `command` at the active terminal's prompt, for the user to
    /// review and submit: the port of `onSendToTerminal` plus the
    /// terminal's `onInsertCommand`.
    ///
    /// Refused unless the active tab is a connected shell on this host and
    /// the text passes `terminal_insert::is_insertable_command` (no control
    /// characters, at most 512 characters). It is written trimmed and with
    /// **no trailing newline**, then the terminal takes focus so Enter goes
    /// to the shell. The first insert of the run asks first, because a
    /// terminal inside `less` or `vim` takes the characters as commands.
    pub fn insert_command(
        &self,
        command: &str,
        window: &mut Window,
        cx: &mut App,
    ) -> Result<(), InsertRefused> {
        use terminal_insert::{can_receive_command, is_insertable_command, strip_submit};
        let Some((tab, view)) = self.active_terminal(cx) else {
            return Err(InsertRefused::NoShell);
        };
        if !can_receive_command(Some(&tab)) {
            return Err(InsertRefused::NoShell);
        }
        if !is_insertable_command(command) {
            return Err(InsertRefused::NotInsertable);
        }
        let text = strip_submit(command).to_string();
        let send = {
            let view = view.downgrade();
            move |window: &mut Window, cx: &mut App| {
                let _ = view.update(cx, |t, cx| t.insert_text(&text, window, cx));
            }
        };
        if self.links.insert_confirmed.get() {
            send(window, cx);
            return Ok(());
        }
        let links = self.links.clone();
        confirm(
            window,
            cx,
            ConfirmOptions::new(
                "Send to Terminal",
                "The command is typed at the prompt and not run \u{2014} you press Enter. \
                 If the terminal is inside an editor or pager instead of at a shell prompt, \
                 the characters go to that program. Send it?",
            ),
            move |yes, window, cx| {
                if yes {
                    links.insert_confirmed.set(true);
                    send(window, cx);
                }
            },
        );
        Ok(())
    }

    /// Show `pane` under the active terminal (the Docker panel's logs/exec
    /// pane: `docker-pane-open` in the Vue app), replacing any pane already
    /// there. Returns the tab it went to, for [`Self::close_terminal_pane`].
    ///
    /// The terminal tab owns the view from then on: it is dropped when it is
    /// replaced, closed or the tab closes. Keep only a `WeakEntity` to it,
    /// and end whatever it runs (an exec PTY) in its own release hook
    /// (`cx.on_release`), not in the Docker panel.
    ///
    /// The tab also closes the pane itself when its shell disconnects or
    /// fails, as Vue's `onDisconnected` closed the docker pane.
    pub fn show_terminal_pane(&self, pane: AnyView, cx: &mut App) -> Result<String, PaneRefused> {
        let (tab, view) = self.active_terminal(cx).ok_or(PaneRefused::NoTerminal)?;
        view.update(cx, |t, cx| t.set_bottom_pane(Some(pane), cx));
        Ok(tab.id)
    }

    /// Remove the pane from the tab [`Self::show_terminal_pane`] put it in.
    pub fn close_terminal_pane(&self, tab_id: &str, cx: &mut App) {
        let view = self
            .links
            .terminals
            .borrow()
            .get(tab_id)
            .and_then(WeakEntity::upgrade);
        if let Some(view) = view {
            view.update(cx, |t, cx| t.set_bottom_pane(None, cx));
        }
    }
}

/// Whether panel `kind` of `host_id` (and, per tab, of `tab_id`) is on
/// screen, given the right panel's state and the active tab.
pub fn panel_visible(
    open: bool,
    selected: PanelKind,
    active: Option<&crate::app::tabs::Tab>,
    kind: PanelKind,
    host_id: i64,
    tab_id: Option<&str>,
) -> bool {
    open && selected == kind
        && active.is_some_and(|t| {
            t.kind.has_right_panel()
                && t.host_id == Some(host_id)
                && tab_id.is_none_or(|id| id == t.id)
        })
}

// ---- the cache -----------------------------------------------------------

/// What a built panel is kept under.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum PanelKey {
    Tab(PanelKind, String),
    Host(PanelKind, i64),
}

/// The built panels. A panel is built the first time it is shown (which is
/// what keeps the security audit lazy) and kept, so switching panels or tabs
/// keeps its state, as the Vue `KeepAlive` did.
pub(crate) struct PanelCache {
    factory: Box<dyn PanelFactory>,
    views: HashMap<PanelKey, AnyView>,
}

impl PanelCache {
    pub(crate) fn new(factory: Box<dyn PanelFactory>) -> Self {
        Self {
            factory,
            views: HashMap::new(),
        }
    }

    /// The panel `kind` beside `tab`, built now if it was not yet.
    /// `make_ctx` is called only when building.
    pub(crate) fn get_or_build(
        &mut self,
        kind: PanelKind,
        tab_id: &str,
        host_id: i64,
        make_ctx: impl FnOnce() -> PanelContext,
        window: &mut Window,
        cx: &mut App,
    ) -> AnyView {
        let key = PanelKey::of(kind, tab_id, host_id);
        if let Some(view) = self.views.get(&key) {
            return view.clone();
        }
        let view = self.factory.build(kind, make_ctx(), window, cx);
        self.views.insert(key, view.clone());
        view
    }

    /// An SSH tab closed: drop its per-tab panels, and the host's panels
    /// when it was the host's last SSH tab.
    pub(crate) fn tab_closed(&mut self, tab_id: &str, host_id: i64, last_for_host: bool) {
        self.views
            .retain(|key, _| key.survives(tab_id, host_id, last_for_host));
    }
}

impl PanelKey {
    fn of(kind: PanelKind, tab_id: &str, host_id: i64) -> Self {
        match kind.scope() {
            PanelScope::PerTab => Self::Tab(kind, tab_id.to_string()),
            PanelScope::PerHost => Self::Host(kind, host_id),
        }
    }

    /// Whether a panel under this key outlives the closing of `tab_id`.
    fn survives(&self, tab_id: &str, host_id: i64, last_for_host: bool) -> bool {
        match self {
            Self::Tab(_, id) => id != tab_id,
            Self::Host(_, h) => !(last_for_host && *h == host_id),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sftp_is_per_tab_and_the_rest_per_host() {
        assert_eq!(PanelKind::Sftp.scope(), PanelScope::PerTab);
        for kind in [PanelKind::Tunnels, PanelKind::Docker, PanelKind::Security] {
            assert_eq!(kind.scope(), PanelScope::PerHost);
        }
        let labels: Vec<_> = PanelKind::ALL.iter().map(|k| k.label()).collect();
        assert_eq!(labels, ["SFTP", "Tunnels", "Docker", "Security"]);
    }

    #[test]
    fn sftp_panels_go_with_their_tab_and_host_panels_with_the_last_tab() {
        let sftp = PanelKey::of(PanelKind::Sftp, "ssh-1", 7);
        let docker = PanelKey::of(PanelKind::Docker, "ssh-1", 7);
        assert_eq!(docker, PanelKey::of(PanelKind::Docker, "ssh-2", 7));
        assert_ne!(sftp, PanelKey::of(PanelKind::Sftp, "ssh-2", 7));
        // Another tab on the host is still open.
        assert!(!sftp.survives("ssh-1", 7, false));
        assert!(docker.survives("ssh-1", 7, false));
        // The host's last SSH tab.
        assert!(!docker.survives("ssh-1", 7, true));
        // Other tabs and hosts are untouched.
        assert!(sftp.survives("ssh-9", 7, true));
        assert!(PanelKey::of(PanelKind::Security, "x", 8).survives("ssh-1", 7, true));
    }

    #[test]
    fn a_panel_is_visible_only_beside_its_own_host_and_tab() {
        use crate::app::tab_kind::TabKind;
        use crate::app::tabs::Tab;
        let ssh = Tab::new("ssh-1", Some(7), TabKind::Ssh, "box");
        let on = |open, selected, active: Option<&Tab>, kind, host, tab: Option<&str>| {
            panel_visible(open, selected, active, kind, host, tab)
        };
        let d = PanelKind::Docker;
        assert!(on(true, d, Some(&ssh), d, 7, None));
        assert!(!on(false, d, Some(&ssh), d, 7, None), "panel hidden");
        assert!(
            !on(true, PanelKind::Sftp, Some(&ssh), d, 7, None),
            "other tab"
        );
        assert!(!on(true, d, Some(&ssh), d, 8, None), "other host");
        assert!(!on(true, d, None, d, 7, None));
        let s = PanelKind::Sftp;
        assert!(on(true, s, Some(&ssh), s, 7, Some("ssh-1")));
        assert!(
            !on(true, s, Some(&ssh), s, 7, Some("ssh-2")),
            "other SSH tab"
        );
        let local = Tab::new("local-1", None, TabKind::Local, "Local");
        assert!(!on(true, d, Some(&local), d, 7, None));
    }

    #[test]
    fn panel_events_route_by_session_and_shell_events_do_not() {
        let progress = CoreEvent::SftpProgress {
            sftp_session_id: "f".into(),
            file: "/x".into(),
            bytes_transferred: 1,
            total_bytes: 2,
        };
        assert_eq!(
            EventKey::of(&progress),
            Some(EventKey::SftpTransfer("f".into()))
        );
        let exec = CoreEvent::PtyError {
            kind: PtyKind::ExecPty,
            id: "p".into(),
            error: "e".into(),
        };
        assert_eq!(EventKey::of(&exec), Some(EventKey::ExecPty("p".into())));
        let shell = CoreEvent::PtyConnected {
            kind: PtyKind::Shell,
            id: "s".into(),
        };
        assert_eq!(EventKey::of(&shell), None);
    }

    #[test]
    fn datastore_progress_routes_by_op_id_and_kind() {
        use termdrop_core::events::{MongoProgress, RedisProgress};
        let mongo = CoreEvent::MongoProgress(MongoProgress {
            op_id: "op".into(),
            db: "d".into(),
            collection: String::new(),
            stage: "dumping".into(),
            synced: 0,
            total: 1,
            percent: 5,
            detail: None,
        });
        assert_eq!(EventKey::of(&mongo), Some(EventKey::MongoOp("op".into())));
        let cancelled = CoreEvent::MongoCancelled {
            op_id: "op".into(),
            db: "d".into(),
        };
        assert_eq!(
            EventKey::of(&cancelled),
            Some(EventKey::MongoOp("op".into()))
        );
        let redis = CoreEvent::RedisProgress(RedisProgress {
            op_id: "op".into(),
            db: 0,
            stage: "exporting".into(),
            processed: 1,
            total: 2,
            percent: 50,
            detail: String::new(),
        });
        // Same id, other kind: a Redis backup never drives a MongoDB bar.
        assert_eq!(EventKey::of(&redis), Some(EventKey::RedisOp("op".into())));
        assert_eq!(
            EventKey::of(&CoreEvent::RedisCancelled {
                op_id: "op".into(),
                db: 0
            }),
            Some(EventKey::RedisOp("op".into()))
        );
    }

    #[test]
    fn a_refused_pane_says_what_to_do() {
        assert_eq!(
            PaneRefused::NoTerminal.message(),
            "Open a terminal on this host to show the pane"
        );
    }

    #[test]
    fn routes_add_and_remove_by_token() {
        let mut routes = PanelRoutes::default();
        let key = EventKey::ExecPty("p".into());
        let a = routes.add(key.clone(), Rc::new(|_, _| true));
        let b = routes.add(key.clone(), Rc::new(|_, _| true));
        assert_eq!(routes.callbacks(&key).len(), 2);
        routes.remove(&key, a);
        assert_eq!(routes.len(), 1);
        routes.remove_dead(&key, &[b]);
        assert_eq!(routes.len(), 0);
        assert!(routes.subs.is_empty(), "an empty key was left behind");
    }

    #[test]
    fn a_dropped_subscription_unsubscribes() {
        let hub = Rc::new(RefCell::new(Hub::default()));
        let key = EventKey::SftpTransfer("f".into());
        let token = hub
            .borrow_mut()
            .panels
            .add(key.clone(), Rc::new(|_, _| true));
        let sub = PanelSubscription {
            hub: Rc::downgrade(&hub),
            key: key.clone(),
            token,
        };
        assert_eq!(hub.borrow().panels.len(), 1);
        drop(sub);
        assert_eq!(hub.borrow().panels.len(), 0);
    }
}
