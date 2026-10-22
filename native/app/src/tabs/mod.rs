//! What fills a tab when the app runs on the real backend: a terminal for
//! SSH and local tabs, and for the kinds not ported yet a notice saying so.
//! Phase 4 of docs/gpui-migration-plan.md.
//!
//! [`CoreContent`] is the shell's `TabContentFactory`. It owns the routing of
//! core events to the terminal they belong to, and tears each session down
//! when its tab closes.

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
use terminal::{Target, TerminalTab};

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
#[derive(Default)]
pub(crate) struct Hub {
    routes: HashMap<String, WeakEntity<TerminalTab>>,
    orphans: HashMap<String, Vec<CoreEvent>>,
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
/// Docker exec PTYs are routed by the panel that will own them (Phase 5).
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
    terminals: RefCell<HashMap<String, WeakEntity<TerminalTab>>>,
}

impl CoreContent {
    pub fn new(services: Arc<Services>, events: async_channel::Receiver<CoreEvent>) -> Self {
        Self {
            services,
            events: RefCell::new(Some(events)),
            hub: SharedHub::default(),
            model: RefCell::new(None),
            terminals: RefCell::new(HashMap::new()),
        }
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

    fn tab_closed(&self, tab: &Tab, cx: &mut App) {
        let view = self.terminals.borrow_mut().remove(&tab.id);
        if let Some(view) = view.and_then(|v| v.upgrade()) {
            view.update(cx, |view, cx| view.shutdown(cx));
        }
    }
}
