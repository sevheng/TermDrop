//! What fills a tab when the app runs on the real backend: a terminal for
//! SSH and local tabs, the datastore views (`crate::datastores`) for MongoDB
//! and Redis tabs, and for VNC, not ported yet, a notice saying so.
//! Phase 4 of docs/gpui-migration-plan.md.
//!
//! [`CoreContent`] is the shell's `TabContentFactory`. It owns the routing of
//! core events to the terminal (or right-hand panel) they belong to, builds
//! and keeps the right panel's views, and tears each session down when its
//! tab closes.

mod local;
mod ssh;
mod terminal;
mod unsupported;

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::{AnyView, App, AppContext as _, Entity, WeakEntity, Window};
use termdrop_core::{CoreEvent, PtyKind};

use crate::app::tab_kind::TabKind;
use crate::app::tabs::Tab;
use crate::app::{AppModel, LocalShellChoice};
use crate::backend::TabContentFactory;
use crate::core_backend::Services;
use crate::datastores::{
    DatastoreFactory, DatastoreKind, DefaultDatastores, ServiceTabContext, release_host,
    release_on_close,
};
use crate::panels::{
    DefaultPanels, EventKey, PanelCache, PanelContext, PanelKind, PanelLinks, PanelRoutes,
    dispatch_to_panels,
};
use terminal::Target;
pub(crate) use terminal::{TerminalTab, term_palette};

/// The open terminal views by tab id, shared with the panels so they can
/// type into the active one or put a pane under it.
pub(crate) type Terminals = Rc<RefCell<HashMap<String, WeakEntity<TerminalTab>>>>;

/// How many events to hold for a session that has no view yet. A session
/// sends a handful before its view registers (connected, maybe an error), so
/// this only bounds a session whose view never comes.
const MAX_ORPHANS: usize = 16;

/// Routes core events, by session id, to the terminal that owns the session.
///
/// A connect call returns the session id only after the session's thread has
/// started, so its first events can reach the UI thread before the view
/// knows its own id. Those are held as orphans and handed over on
/// [`Hub::register`].
///
/// Panel events (SFTP progress, docker exec PTYs) go to the panels that
/// subscribed through `PanelContext::subscribe`; their ids are known before
/// anything is emitted, so they need no orphans.
#[derive(Default)]
pub(crate) struct Hub {
    routes: HashMap<String, WeakEntity<TerminalTab>>,
    orphans: HashMap<String, Vec<CoreEvent>>,
    pub(crate) panels: PanelRoutes,
}

pub(crate) type SharedHub = Rc<RefCell<Hub>>;

impl Hub {
    /// Route `session_id` to `view` from now on, returning what arrived
    /// before it registered, for the view to apply in order.
    pub(crate) fn register(
        &mut self,
        session_id: &str,
        view: WeakEntity<TerminalTab>,
    ) -> Vec<CoreEvent> {
        self.routes.insert(session_id.to_string(), view);
        self.orphans.remove(session_id).unwrap_or_default()
    }

    pub(crate) fn unregister(&mut self, session_id: &str) {
        self.routes.remove(session_id);
        self.orphans.remove(session_id);
    }
}

/// The terminal session an event is about, if it is one a tab handles.
/// Docker exec PTYs and SFTP progress go to panels instead (`EventKey::of`).
fn session_of(event: &CoreEvent) -> Option<&str> {
    match event {
        CoreEvent::PtyConnected {
            kind: PtyKind::Shell,
            id,
        }
        | CoreEvent::PtyDisconnected {
            kind: PtyKind::Shell,
            id,
        }
        | CoreEvent::PtyError {
            kind: PtyKind::Shell,
            id,
            ..
        } => Some(id),
        CoreEvent::SshReconnected { session_id } => Some(session_id),
        _ => None,
    }
}

fn dispatch(hub: &SharedHub, event: CoreEvent, cx: &mut App) {
    if let Some(key) = EventKey::of(&event) {
        dispatch_to_panels(hub, key, &event, cx);
        return;
    }
    let Some(id) = session_of(&event).map(str::to_string) else {
        return;
    };
    let route = hub.borrow().routes.get(&id).cloned();
    match route {
        Some(view) => {
            // The borrow is released first: the view may register or
            // unregister while it handles the event.
            if view
                .update(cx, |view, cx| view.on_core_event(event, cx))
                .is_err()
            {
                hub.borrow_mut().unregister(&id);
            }
        }
        None => {
            let mut hub = hub.borrow_mut();
            let held = hub.orphans.entry(id).or_default();
            if held.len() < MAX_ORPHANS {
                held.push(event);
            }
        }
    }
}

/// The real tab factory.
pub struct CoreContent {
    services: Arc<Services>,
    events: RefCell<Option<async_channel::Receiver<CoreEvent>>>,
    hub: SharedHub,
    model: RefCell<Option<WeakEntity<AppModel>>>,
    terminals: Terminals,
    panels: RefCell<PanelCache>,
    links: Rc<PanelLinks>,
    datastores: Box<dyn DatastoreFactory>,
}

impl CoreContent {
    pub fn new(services: Arc<Services>, events: async_channel::Receiver<CoreEvent>) -> Self {
        let hub = SharedHub::default();
        let terminals = Terminals::default();
        Self {
            services,
            events: RefCell::new(Some(events)),
            links: PanelLinks::new(hub.clone(), terminals.clone()),
            hub,
            model: RefCell::new(None),
            terminals,
            panels: RefCell::new(PanelCache::new(Box::new(DefaultPanels))),
            datastores: Box::new(DefaultDatastores),
        }
    }

    fn terminal(&self, tab_id: &str) -> Option<Entity<TerminalTab>> {
        self.terminals
            .borrow()
            .get(tab_id)
            .and_then(WeakEntity::upgrade)
    }
}

impl TabContentFactory for CoreContent {
    fn start(&self, model: &Entity<AppModel>, cx: &mut App) {
        *self.model.borrow_mut() = Some(model.downgrade());

        if let Some(rx) = self.events.borrow_mut().take() {
            let hub = self.hub.clone();
            cx.spawn(async move |cx| {
                while let Ok(event) = rx.recv().await {
                    cx.update(|cx| dispatch(&hub, event, cx));
                }
            })
            .detach();
        }

        // Detected once at launch for the new-terminal menu. The Vue app
        // re-detected each time the menu opened; a shell installed while the
        // app runs appears after a restart here.
        let services = self.services.clone();
        let detect = self
            .services
            .rt
            .spawn(async move { services.core.list_local_shells().await });
        let model = model.downgrade();
        cx.spawn(async move |cx| {
            let shells = match detect.await {
                Ok(Ok(shells)) => shells,
                Ok(Err(e)) => {
                    tracing::warn!("Shell detection failed: {e}");
                    return;
                }
                Err(e) => {
                    tracing::warn!("Shell detection task failed: {e}");
                    return;
                }
            };
            let choices = shells
                .into_iter()
                .map(|s| LocalShellChoice {
                    id: s.id,
                    label: s.label,
                })
                .collect();
            let _ = model.update(cx, |m, cx| m.set_local_shells(choices, cx));
        })
        .detach();
    }

    fn build(&self, tab: &Tab, window: &mut Window, cx: &mut App) -> AnyView {
        let model = self.model.borrow().as_ref().and_then(WeakEntity::upgrade);
        let target = match (tab.kind, tab.host_id) {
            (TabKind::Ssh, Some(host_id)) => Some(Target::Ssh { host_id }),
            (TabKind::Local, _) => Some(Target::Local {
                shell_id: tab.shell_id.clone(),
            }),
            _ => None,
        };
        if let (Some(kind), Some(host_id), Some(model)) =
            (DatastoreKind::of(tab.kind), tab.host_id, model.clone())
        {
            let ctx = ServiceTabContext::new(
                kind,
                tab.id.clone(),
                host_id,
                self.services.clone(),
                model,
                self.links.clone(),
            );
            return self.datastores.build(kind, ctx, window, cx);
        }
        match (target, model) {
            (Some(target), Some(model)) => {
                let view = TerminalTab::new(
                    self.services.clone(),
                    self.hub.clone(),
                    model,
                    tab,
                    target,
                    window,
                    cx,
                );
                self.terminals
                    .borrow_mut()
                    .insert(tab.id.clone(), view.downgrade());
                view.into()
            }
            _ => cx
                .new(|_| unsupported::Unsupported::new(tab, self.model.borrow().clone()))
                .into(),
        }
    }

    fn tab_closed(&self, tab: &Tab, last_of_kind_for_host: bool, cx: &mut App) {
        // `closeServiceTab`: the host's pooled connections (and a Redis
        // host's tunnel) go with its last tab of that kind.
        if let (Some(kind), Some(host_id)) = (
            release_on_close(tab.kind, last_of_kind_for_host),
            tab.host_id,
        ) {
            release_host(&self.services, kind, host_id);
        }
        let view = self.terminals.borrow_mut().remove(&tab.id);
        if let Some(view) = view.and_then(|v| v.upgrade()) {
            view.update(cx, |view, cx| view.shutdown(cx));
        }
        if tab.kind.has_right_panel()
            && let Some(host_id) = tab.host_id
        {
            self.panels
                .borrow_mut()
                .tab_closed(&tab.id, host_id, last_of_kind_for_host);
        }
    }

    fn right_panel(
        &self,
        tab: &Tab,
        kind: PanelKind,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<AnyView> {
        let host_id = tab.host_id.filter(|_| tab.kind.has_right_panel())?;
        let sftp_session_id = tab.sftp.session_id().map(str::to_string);
        // The SFTP panel needs its session; the frame shows the connecting or
        // failed state until there is one.
        if kind == PanelKind::Sftp && sftp_session_id.is_none() {
            return None;
        }
        let model = self.model.borrow().as_ref().and_then(WeakEntity::upgrade)?;
        let make_ctx = || {
            let per_tab = kind.scope() == crate::panels::PanelScope::PerTab;
            PanelContext::new(
                kind,
                host_id,
                per_tab.then(|| tab.id.clone()),
                if kind == PanelKind::Sftp {
                    sftp_session_id.clone()
                } else {
                    None
                },
                self.services.clone(),
                model,
                self.links.clone(),
            )
        };
        Some(
            self.panels
                .borrow_mut()
                .get_or_build(kind, &tab.id, host_id, make_ctx, window, cx),
        )
    }

    fn retry_sftp(&self, tab: &Tab, window: &mut Window, cx: &mut App) {
        if let Some(view) = self.terminal(&tab.id) {
            view.update(cx, |view, cx| view.retry_sftp(window, cx));
        }
    }

    fn floating(&self) -> Vec<AnyView> {
        self.links.floating()
    }
}
