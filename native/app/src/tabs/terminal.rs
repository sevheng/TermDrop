//! One terminal tab: a `TerminalView` wired to a core session. SSH and
//! local tabs share everything here, as they share the core's handle and
//! events; only connecting differs (`ssh.rs`, `local.rs`).
//!
//! Keystrokes go to `Core::ssh_write_bytes`, grid changes to `ssh_resize`, output
//! comes back through a byte sink the bridge attaches. Closing the tab calls
//! `ssh_disconnect`, which drops the host's exec session and caches when it
//! was the host's last terminal, exactly as the Tauri command does.

use std::sync::{Arc, Mutex, MutexGuard};

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, AppContext as _, Context, Entity, Focusable as _, IntoElement, ParentElement, Render,
    SharedString, Styled, Subscription, Task, Window, div, px,
};
use termdrop_core::CoreEvent;
use termdrop_terminal::{Palette as TermPalette, TerminalEvent, TerminalOptions, TerminalView};

use super::SharedHub;
use crate::app::prefs::ThemeChoice;
use crate::app::tabs::Tab;
use crate::app::{AppEvent, AppModel};
use crate::core_backend::Services;
use crate::ui::theme::palette;

/// What the tab connects to.
#[derive(Debug, Clone)]
pub enum Target {
    Ssh {
        host_id: i64,
    },
    /// `None` is the default shell; otherwise an id from `local_shells`.
    Local {
        shell_id: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub(super) enum Status {
    Connecting,
    Connected,
    /// The session ended, from the far side or with an error. Its id is
    /// kept, so Reconnect reuses it and the tab stays put.
    Ended(Option<String>),
    /// Connecting failed before there was a session. Retry starts afresh.
    Failed(String),
}

/// Locks that only hold plain values: a panic elsewhere cannot leave them
/// half-written, so a poisoned lock is still safe to read.
pub(super) fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn term_palette(theme: ThemeChoice) -> TermPalette {
    match theme {
        ThemeChoice::Dark => TermPalette::dark(),
        ThemeChoice::Light => TermPalette::light(),
    }
}

pub struct TerminalTab {
    pub(super) services: Arc<Services>,
    pub(super) hub: SharedHub,
    pub(super) model: Entity<AppModel>,
    pub(super) tab_id: String,
    pub(super) target: Target,
    pub(super) terminal: Entity<TerminalView>,
    /// The core session id, once there is one. Shared with the input and
    /// resize closures, which the terminal may call from any thread.
    pub(super) session: Arc<Mutex<Option<String>>>,
    /// The grid size last laid out, sent on connect and after a reconnect
    /// (which the core starts at 80x24).
    pub(super) size: Arc<Mutex<(u16, u16)>>,
    pub(super) status: Status,
    /// Set when the tab closes. A connect still in flight then disconnects
    /// whatever it produces instead of adopting it.
    pub(super) closed: bool,
    pub(super) pending: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl TerminalTab {
    pub fn new(
        services: Arc<Services>,
        hub: SharedHub,
        model: Entity<AppModel>,
        tab: &Tab,
        target: Target,
        window: &mut Window,
        cx: &mut App,
    ) -> Entity<Self> {
        let (font_size, theme) = {
            let m = model.read(cx);
            (m.font_size(), m.theme())
        };
        let session: Arc<Mutex<Option<String>>> = Arc::default();
        let size = Arc::new(Mutex::new((80u16, 24u16)));

        // Keystrokes before a session exists are dropped, as xterm's were
        // before `ssh_connect` resolved.
        let input = {
            let core = services.core.clone();
            let session = session.clone();
            Box::new(move |bytes: Vec<u8>| {
                let id = lock(&session).clone();
                if let Some(id) = id {
                    // Bytes, not text: a legacy mouse report's coordinates
                    // are raw bytes that are not valid UTF-8 past column 94.
                    let _ = core.ssh_write_bytes(&id, bytes);
                }
            })
        };
        let on_resize = {
            let core = services.core.clone();
            let session = session.clone();
            let size = size.clone();
            Box::new(move |cols: u16, rows: u16| {
                *lock(&size) = (cols, rows);
                let id = lock(&session).clone();
                if let Some(id) = id {
                    let _ = core.ssh_resize(&id, u32::from(cols), u32::from(rows));
                }
            })
        };
        let terminal = TerminalView::new(
            window,
            cx,
            TerminalOptions {
                font_size: px(f32::from(font_size)),
                palette: term_palette(theme),
                ..TerminalOptions::default()
            },
            input,
            on_resize,
        );

        let tab_id = tab.id.clone();
        cx.new(|cx| {
            let subscriptions = vec![
                cx.subscribe_in(&model, window, Self::on_app_event),
                cx.subscribe_in(&terminal, window, Self::on_terminal_event),
            ];
            // Deferred: this runs inside the window's handling of the tab
            // opening, and connecting updates the model.
            cx.defer_in(window, |this: &mut Self, window, cx| {
                window.focus(&this.terminal.focus_handle(cx), cx);
                this.connect_fresh(window, cx);
            });
            Self {
                services,
                hub,
                model,
                tab_id,
                target,
                terminal,
                session,
                size,
                status: Status::Connecting,
                closed: false,
                pending: None,
                _subscriptions: subscriptions,
            }
        })
    }

    pub(super) fn session_id(&self) -> Option<String> {
        lock(&self.session).clone()
    }

    fn connect_fresh(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.target.clone() {
            Target::Ssh { host_id } => self.connect_ssh(host_id, Default::default(), window, cx),
            Target::Local { shell_id } => self.connect_local(shell_id, window, cx),
        }
    }

    pub(super) fn set_status(&mut self, status: Status, cx: &mut Context<Self>) {
        let (connected, connecting) = match status {
            Status::Connecting => (false, true),
            Status::Connected => (true, false),
            Status::Ended(_) | Status::Failed(_) => (false, false),
        };
        self.status = status;
        let id = self.tab_id.clone();
        self.model
            .update(cx, |m, cx| m.set_tab_status(&id, connected, connecting, cx));
        cx.notify();
    }

    /// Take ownership of a session the core just started: route its output
    /// and events here and send it the current grid size.
    pub(super) fn adopt_session(&mut self, session_id: String, cx: &mut Context<Self>) {
        *lock(&self.session) = Some(session_id.clone());
        self.attach_output(&session_id, cx);
        let early = self
            .hub
            .borrow_mut()
            .register(&session_id, cx.entity().downgrade());
        for event in early {
            self.on_core_event(event, cx);
        }
        self.send_size(&session_id);
    }

    pub(super) fn attach_output(&self, session_id: &str, cx: &App) {
        let feed = self.terminal.read(cx).feed();
        if let Err(e) = self
            .services
            .bridge
            .attach(&self.services.core, session_id, feed)
        {
            tracing::warn!("Could not attach terminal output: {e}");
        }
    }

    pub(super) fn send_size(&self, session_id: &str) {
        let (cols, rows) = *lock(&self.size);
        let _ = self
            .services
            .core
            .ssh_resize(session_id, u32::from(cols), u32::from(rows));
    }

    /// A session event the hub routed here.
    pub(super) fn on_core_event(&mut self, event: CoreEvent, cx: &mut Context<Self>) {
        match event {
            CoreEvent::PtyConnected { .. } => {
                if let Some(id) = self.session_id() {
                    self.send_size(&id);
                }
                self.set_status(Status::Connected, cx);
            }
            CoreEvent::PtyDisconnected { .. } => self.set_status(Status::Ended(None), cx),
            CoreEvent::PtyError { error, .. } => self.set_status(Status::Ended(Some(error)), cx),
            // The reconnect call's own result re-attaches output; the
            // connected event that follows flips the status.
            _ => {}
        }
    }

    /// Reconnect a session that ended, or start over after a failure.
    fn retry(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match &self.status {
            Status::Ended(_) if self.session_id().is_some() => self.reconnect(None, window, cx),
            _ => self.connect_fresh(window, cx),
        }
        window.focus(&self.terminal.focus_handle(cx), cx);
    }

    /// The tab closed: end the session. `ssh_disconnect` can drop an
    /// `ssh2::Session`, which talks to the server, so it runs off this thread.
    pub fn shutdown(&mut self, cx: &mut Context<Self>) {
        self.closed = true;
        self.pending = None;
        if let Target::Ssh { host_id } = self.target
            && self.model.read(cx).connecting_host_id() == Some(host_id)
        {
            self.model
                .update(cx, |m, cx| m.set_connecting_host(None, cx));
        }
        if let Some(id) = lock(&self.session).take() {
            self.disconnect(id);
        }
    }

    pub(super) fn disconnect(&self, session_id: String) {
        self.hub.borrow_mut().unregister(&session_id);
        self.services.bridge.detach(&session_id);
        let core = self.services.core.clone();
        self.services.rt.spawn_blocking(move || {
            if let Err(e) = core.ssh_disconnect(&session_id) {
                tracing::warn!("Disconnect failed: {e}");
            }
        });
    }

    fn on_app_event(
        &mut self,
        _: &Entity<AppModel>,
        event: &AppEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            AppEvent::ActiveTabChanged(Some(id)) if *id == self.tab_id => {
                window.focus(&self.terminal.focus_handle(cx), cx);
            }
            AppEvent::FontSizeChanged(size) => {
                let size = px(f32::from(*size));
                self.terminal.update(cx, |t, cx| t.set_font_size(size, cx));
            }
            AppEvent::ThemeChanged(theme) => {
                let palette = term_palette(*theme);
                self.terminal.update(cx, |t, cx| t.set_palette(palette, cx));
            }
            _ => {}
        }
    }

    fn on_terminal_event(
        &mut self,
        _: &Entity<TerminalView>,
        event: &TerminalEvent,
        _: &mut Window,
        _: &mut Context<Self>,
    ) {
        match event {
            // OSC 52 is deliberately not honoured: xterm.js in the Tauri app
            // has no clipboard addon, so a remote program could never write
            // the local clipboard there, and it should not start here.
            TerminalEvent::ClipboardStore(_) => {}
            // The tab keeps its host's name, as the Vue tab did.
            TerminalEvent::TitleChanged(_) | TerminalEvent::Bell | TerminalEvent::Exited => {}
        }
    }

    fn banner(&self) -> Option<(SharedString, &'static str)> {
        match &self.status {
            Status::Connecting | Status::Connected => None,
            Status::Ended(None) => Some(("Disconnected.".into(), "Reconnect")),
            Status::Ended(Some(e)) => Some((format!("Disconnected: {e}").into(), "Reconnect")),
            Status::Failed(e) => Some((e.clone().into(), "Retry")),
        }
    }
}

impl Render for TerminalTab {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = *palette(cx);
        let banner = self.banner();
        v_flex()
            .size_full()
            .bg(p.canvas)
            .when_some(banner, |col, (message, action)| {
                col.child(
                    h_flex()
                        .flex_none()
                        .gap_3()
                        .px_3()
                        .py_1p5()
                        .bg(p.bad_bg)
                        .border_b_1()
                        .border_color(p.bad_line)
                        .text_xs()
                        .text_color(p.ink)
                        .child(div().flex_1().min_w_0().child(message))
                        .child(
                            Button::new("terminal-retry")
                                .primary()
                                .xsmall()
                                .label(action)
                                .on_click(
                                    cx.listener(|this, _, window, cx| this.retry(window, cx)),
                                ),
                        ),
                )
            })
            .child(div().flex_1().min_h_0().child(self.terminal.clone()))
    }
}
