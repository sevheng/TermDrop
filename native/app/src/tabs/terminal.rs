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
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::menu::{ContextMenuExt as _, PopupMenu, PopupMenuItem};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyView, App, AppContext as _, Context, Entity, Focusable as _, InteractiveElement as _,
    IntoElement, KeyDownEvent, ParentElement, Render, SharedString,
    StatefulInteractiveElement as _, Styled, Subscription, Task, WeakEntity, Window, div, px,
};
use termdrop_core::CoreEvent;
use termdrop_terminal::{
    Palette as TermPalette, SearchDirection, TerminalEvent, TerminalOptions, TerminalView,
};

use super::SharedHub;
use super::status_bar::{Link, StatusBar};
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

pub(crate) fn term_palette(theme: ThemeChoice) -> TermPalette {
    match theme {
        ThemeChoice::Dark => TermPalette::dark(),
        ThemeChoice::Light => TermPalette::light(),
    }
}

/// A tab's SFTP session id, and whether the tab has closed.
#[derive(Debug, Default)]
pub(super) struct SftpSlot {
    pub closed: bool,
    pub id: Option<String>,
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
    /// The SFTP session the file panel uses, once `sftp_connect` returned.
    /// Passed to `ssh_reconnect` (which replaces it under the same id) and
    /// disconnected with the tab. Shared with the connect task, so a tab
    /// closed while it runs still has its session disconnected.
    pub(super) sftp: Arc<Mutex<SftpSlot>>,
    pub(super) sftp_task: Option<Task<()>>,
    /// A panel's pane under the terminal: the Docker panel's logs or exec
    /// view. Owned here, so it goes with the tab.
    bottom_pane: Option<AnyView>,
    /// The host status bar (and the system panel it opens), SSH tabs only.
    status_bar: Option<Entity<StatusBar>>,
    /// The find bar (Ctrl+F): open, its query box, and "Aa".
    find_open: bool,
    find_input: Entity<InputState>,
    find_case: bool,
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
        let status_bar = match target {
            Target::Ssh { host_id } => Some(StatusBar::new(
                services.clone(),
                model.clone(),
                tab_id.clone(),
                host_id,
                window,
                cx,
            )),
            Target::Local { .. } => None,
        };
        cx.new(|cx| {
            let find_input = cx.new(|cx| InputState::new(window, cx).placeholder("Find..."));
            let subscriptions = vec![
                cx.subscribe_in(&model, window, Self::on_app_event),
                cx.subscribe_in(&terminal, window, Self::on_terminal_event),
                // Enter finds the next match, Shift+Enter the previous.
                cx.subscribe(&find_input, |this, _, event: &InputEvent, cx| {
                    if let InputEvent::PressEnter { shift, .. } = event {
                        let direction = if *shift {
                            SearchDirection::Previous
                        } else {
                            SearchDirection::Next
                        };
                        this.find(direction, cx);
                    }
                }),
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
                sftp: Arc::default(),
                sftp_task: None,
                bottom_pane: None,
                status_bar,
                find_open: false,
                find_input,
                find_case: false,
                _subscriptions: subscriptions,
            }
        })
    }

    /// The core session id, once connected.
    pub(crate) fn session_id(&self) -> Option<String> {
        lock(&self.session).clone()
    }

    /// Type `text` at the prompt and focus the terminal so Enter goes to the
    /// shell. The panels' `insert_command` has already checked it; nothing
    /// here adds a newline.
    pub(crate) fn insert_text(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(id) = self.session_id()
            && let Err(e) = self.services.core.ssh_write(&id, text.to_string())
        {
            tracing::warn!("Could not type into the terminal: {e}");
        }
        window.focus(&self.terminal.focus_handle(cx), cx);
    }

    /// Put a panel's view under the terminal, or remove it with `None`.
    pub(crate) fn set_bottom_pane(&mut self, pane: Option<AnyView>, cx: &mut Context<Self>) {
        self.bottom_pane = pane;
        cx.notify();
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
        let link = match status {
            Status::Connecting => Link::Connecting,
            Status::Connected => Link::Connected,
            Status::Ended(_) | Status::Failed(_) => Link::Ended,
        };
        if let Some(bar) = &self.status_bar {
            bar.update(cx, |bar, cx| bar.set_link(link, cx));
        }
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
            // A pane under the terminal (docker logs/exec) goes with the
            // shell, as Vue's `onDisconnected` closed it; dropping it ends
            // its PTY.
            CoreEvent::PtyDisconnected { .. } => {
                self.bottom_pane = None;
                self.set_status(Status::Ended(None), cx);
            }
            CoreEvent::PtyError { error, .. } => {
                self.bottom_pane = None;
                self.set_status(Status::Ended(Some(error)), cx);
            }
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
        self.sftp_task = None;
        self.bottom_pane = None;
        // Its polls stop with it.
        self.status_bar = None;
        let sftp_id = {
            let mut slot = lock(&self.sftp);
            slot.closed = true;
            slot.id.take()
        };
        if let Some(sftp_id) = sftp_id {
            self.disconnect_sftp(sftp_id);
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

    // ---- find bar ----------------------------------------------------------

    /// `openSearch`: show the bar and put the cursor in it.
    fn open_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.find_open = true;
        self.find_input
            .update(cx, |input, cx| input.focus(window, cx));
        cx.notify();
    }

    /// `closeSearch`: hide the bar, empty it, drop the highlights and give
    /// the keyboard back to the terminal.
    fn close_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.find_open = false;
        self.find_input
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.terminal.update(cx, |t, cx| t.clear_search(cx));
        window.focus(&self.terminal.focus_handle(cx), cx);
        cx.notify();
    }

    /// `findNext` / `findPrevious`; an empty query does nothing.
    fn find(&mut self, direction: SearchDirection, cx: &mut Context<Self>) {
        let query = self.find_input.read(cx).value().to_string();
        if query.is_empty() {
            return;
        }
        let case = self.find_case;
        self.terminal
            .update(cx, |t, cx| t.find(&query, case, direction, cx));
    }

    /// The terminal's own shortcuts, taken before the keys reach the
    /// program, as `attachCustomKeyEventHandler` did: Ctrl+F opens the find
    /// bar, Ctrl+Shift+A selects everything, Esc closes an open find bar.
    fn on_key_capture(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let ks = &event.keystroke;
        let m = &ks.modifiers;
        let key = ks.key.to_lowercase();
        if m.control && !m.alt && !m.platform {
            if m.shift && key == "a" {
                self.terminal.update(cx, |t, cx| t.select_all(cx));
                cx.stop_propagation();
                return;
            }
            if !m.shift && key == "f" {
                self.open_find(window, cx);
                cx.stop_propagation();
                return;
            }
        }
        if self.find_open && key == "escape" && !m.modified() {
            self.close_find(window, cx);
            cx.stop_propagation();
        }
    }

    fn render_find_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let p = *palette(cx);
        let glyph = |id: &'static str, text: &'static str| {
            let ink = p.ink;
            div()
                .id(id)
                .px_1()
                .text_xs()
                .text_color(p.ink_2)
                .cursor_pointer()
                .hover(move |s| s.text_color(ink))
                .child(text)
        };
        h_flex()
            .id("terminal-find")
            // Clicks on the bar are its own, not the terminal's under it.
            .occlude()
            .absolute()
            .top_2()
            .right_2()
            .gap_2()
            .p_2()
            .bg(p.surface)
            .border_1()
            .border_color(p.line)
            .rounded_sm()
            .shadow_lg()
            .child(
                div()
                    .w(px(160.))
                    .child(Input::new(&self.find_input).xsmall()),
            )
            .child(
                glyph("find-prev", "\u{2191}")
                    .tooltip(|window, cx| Tooltip::new("Previous").build(window, cx))
                    .on_click(
                        cx.listener(|this, _, _, cx| this.find(SearchDirection::Previous, cx)),
                    ),
            )
            .child(
                glyph("find-next", "\u{2193}")
                    .tooltip(|window, cx| Tooltip::new("Next").build(window, cx))
                    .on_click(cx.listener(|this, _, _, cx| this.find(SearchDirection::Next, cx))),
            )
            .child(
                Checkbox::new("find-case")
                    .xsmall()
                    .label("Aa")
                    .checked(self.find_case)
                    .on_click(cx.listener(|this, checked: &bool, _, cx| {
                        this.find_case = *checked;
                        // The highlights were for the other case rule; the
                        // next Enter draws them again.
                        this.terminal.update(cx, |t, cx| t.clear_search(cx));
                        cx.notify();
                    })),
            )
            .child(
                glyph("find-close", "\u{d7}")
                    .on_click(cx.listener(|this, _, window, cx| this.close_find(window, cx))),
            )
    }

    /// The right-click menu (`showContextMenu`): Copy, Paste, Select All,
    /// and on an SSH tab a "Forward ..." item when the selection names a
    /// port. While the program takes the mouse the click is reported to it
    /// instead and no menu opens, unless Shift is held.
    fn context_menu(this: &WeakEntity<Self>, menu: PopupMenu, cx: &mut App) -> PopupMenu {
        let Ok((terminal, host_id)) = this.read_with(cx, |tab, _| {
            let host_id = match tab.target {
                Target::Ssh { host_id } => Some(host_id),
                Target::Local { .. } => None,
            };
            (tab.terminal.clone(), host_id)
        }) else {
            return menu;
        };
        let selection = terminal.read(cx).selection_text().unwrap_or_default();
        let (copy, paste, all) = (terminal.clone(), terminal.clone(), terminal);
        let menu = menu
            .item(PopupMenuItem::new("Copy").on_click(move |_, _, cx| {
                copy.update(cx, |t, cx| t.copy(cx));
            }))
            .item(PopupMenuItem::new("Paste").on_click(move |_, _, cx| {
                paste.update(cx, |t, cx| t.paste_from_clipboard(cx));
            }))
            .item(PopupMenuItem::new("Select All").on_click(move |_, _, cx| {
                all.update(cx, |t, cx| t.select_all(cx));
            }));
        match host_id.and_then(|host_id| forward_menu_item(host_id, &selection)) {
            Some(item) => menu.separator().item(item),
            None => menu,
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

/// The "Forward ..." item for the terminal's selection (TUN-4), on an SSH
/// tab. A hook for the port-forward work: until it lands there is no
/// forward editor to open, so there is no item. Wire it as
/// `detect_forward(selection)` -> a `PopupMenuItem` with its label, whose
/// click opens the forward editor for `host_id` prefilled as
/// `createForwardFromSelection` did.
fn forward_menu_item(_host_id: i64, _selection: &str) -> Option<PopupMenuItem> {
    None
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
            .child(
                div()
                    .id("terminal-area")
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .capture_key_down(cx.listener(Self::on_key_capture))
                    .child(
                        div()
                            .id("terminal-surface")
                            .size_full()
                            .child(self.terminal.clone())
                            .context_menu({
                                let this = cx.entity().downgrade();
                                move |menu, _, cx| Self::context_menu(&this, menu, cx)
                            }),
                    )
                    .when(self.find_open, |d| d.child(self.render_find_bar(cx))),
            )
            .when_some(self.bottom_pane.clone(), |col, pane| {
                // The pane sizes itself (its own height and resize edge).
                col.child(
                    div()
                        .flex_none()
                        .border_t_1()
                        .border_color(p.line)
                        .child(pane),
                )
            })
            .when_some(self.status_bar.clone(), |col, bar| col.child(bar))
    }
}
