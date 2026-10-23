//! The remote-desktop tab: port of `VncPanel.vue` and `useVncInput.js`.
//!
//! The tab owns its session, as the Vue panel did: it opens as a tab at
//! once and shows its own spinner and error. The pieces:
//!
//! - [`frame`]: the wire format the core writes (`vncFrame.js`).
//! - [`framebuffer`]: the screen, BGRA, updated in place, as tiles.
//! - [`renderer`]: one thread per tab that parses, decodes JPEGs and builds
//!   the changed tiles' images, so the UI thread only swaps them in.
//! - [`screen`]: the element that paints the tiles and takes the pointer.
//! - [`layout`], [`input`], [`keysym`]: the pure parts (`vncLayout.js`,
//!   `useVncInput.js`, `keysym.js`).
//!
//! The rules from `CLAUDE.md` that the renderer must keep:
//!
//! - **The ack is the flow control.** Each frame is acknowledged with
//!   `vnc_frame_ack` once it is applied (its tiles swapped in), as the Vue
//!   renderer acked after drawing; the session requests the next frame only
//!   then (or after its 250 ms timeout).
//! - **A hidden tab says so** with `vnc_set_paused` (the tab is not the
//!   active one, or the window is not visible), and the ack of a frame that
//!   arrived while hidden is withheld until it is shown again, when a full
//!   refresh repaints whatever was missed.
//! - **Attaching the sink forces a full update** (the core invalidates its
//!   shadow), so a new tab never starts on a picture it does not have.
//! - **JPEG rectangles reach us undecoded** and are decoded here; one that
//!   will not decode is skipped, and the core's shadow already treats its
//!   area as unknown, so it is sent again when next touched.
//! - **A frame that will not parse ends the session**: the stream is out of
//!   step, and every later frame would be garbage.

mod frame;
mod framebuffer;
mod input;
mod keysym;
mod layout;
mod renderer;
mod screen;

use std::ops::Range;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{Disableable as _, Icon, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, AnyView, AnyWindowHandle, App, AppContext as _, Bounds, ClipboardItem, Context,
    EntityInputHandler, FocusHandle, Focusable, Hsla, InteractiveElement as _, IntoElement,
    KeyBinding, KeyDownEvent, KeyUpEvent, ModifiersChangedEvent, MouseButton, NoAction,
    ParentElement as _, Pixels, Point, Render, RenderImage, ScrollDelta, ScrollWheelEvent,
    SharedString, Styled as _, Subscription, Task, UTF16Selection, Window, WindowVisibility, div,
};
use termdrop_core::CoreEvent;
use termdrop_core::vnc::VncSessionInfo;

use self::input::{Input, Move, Send};
use self::keysym::CTRL_ALT_DELETE;
use self::layout::{FitMode, WheelDelta};
use self::renderer::{Message, Update};
use self::screen::{Geometry, VncScreen};
use super::{OpSubscription, ServiceTabContext};
use crate::app::AppEvent;
use crate::ui::empty_state::EmptyState;
use crate::ui::icon_button::IconButton;
use crate::ui::panel_header::PanelHeader;
use crate::ui::tag::{Tag, TagTone};
use crate::ui::theme::{Palette, palette};
use crate::ui::toast::{ToastKind, toast};

/// The key context of the focused screen. Every chord the app binds is
/// rebound to `NoAction` here ([`init`]), so while the screen has focus
/// Tab, Ctrl+W, Ctrl+Tab and the rest reach the remote, as they did in the
/// Vue canvas; Ctrl+Alt+Shift hands the keyboard back.
pub const KEY_CONTEXT: &str = "VncScreen";

const FIT_SETTING: &str = "vnc.fitMode";
const STATS_SETTING: &str = "vnc.showStats";
const STATS_HINT: &str = "Frames per second, the gap between one frame arriving and the next, \
     the time spent applying it, and its size. The gap minus the apply time is time spent \
     waiting on the remote. Run with RUST_LOG=termdrop::vnc=debug to see that split exactly.";

/// Give the remote every chord the app has bound. Call after every other
/// binding is registered (it copies the keymap as it stands).
pub fn init(cx: &mut App) {
    let chords: Vec<String> = {
        let keymap = cx.key_bindings();
        let keymap = keymap.borrow();
        let mut chords: Vec<String> = keymap
            .bindings()
            .map(|b| {
                b.keystrokes()
                    .iter()
                    .map(|k| k.inner().unparse())
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .filter(|c| !c.is_empty())
            .collect();
        chords.sort();
        chords.dedup();
        chords
    };
    cx.bind_keys(
        chords
            .iter()
            .map(|c| KeyBinding::new(c, NoAction {}, Some(KEY_CONTEXT))),
    );
}

/// The tab's view, built once when the tab opens and dropped when it closes.
pub fn build(ctx: ServiceTabContext, window: &mut Window, cx: &mut App) -> AnyView {
    cx.new(|cx| VncTab::new(ctx, window, cx)).into()
}

/// A live session and what hangs off it.
struct Session {
    id: String,
    info: VncSessionInfo,
    _events: OpSubscription,
    _frames: Task<()>,
}

/// A connect between its start and its delivery to the view.
#[derive(Debug, Default)]
struct PendingConnect {
    /// The tab went before the result could be delivered.
    abandoned: bool,
    /// The session the core opened, until the view takes it.
    session: Option<String>,
}

impl PendingConnect {
    /// The core opened `session_id`. False when the tab has already gone,
    /// so the caller must close it.
    fn connected(this: &Mutex<Self>, session_id: &str) -> bool {
        let mut p = this.lock().unwrap_or_else(|e| e.into_inner());
        if p.abandoned {
            return false;
        }
        p.session = Some(session_id.to_string());
        true
    }

    /// The view took the result.
    fn claim(this: &Mutex<Self>) {
        this.lock().unwrap_or_else(|e| e.into_inner()).session = None;
    }

    /// The tab is going. Returns a session opened but not yet taken, which
    /// the caller must close; one still connecting will close itself.
    fn abandon(this: &Mutex<Self>) -> Option<String> {
        let mut p = this.lock().unwrap_or_else(|e| e.into_inner());
        p.abandoned = true;
        p.session.take()
    }
}

/// The once-a-second readout behind `vnc.showStats`.
#[derive(Default)]
struct Stats {
    frames: u32,
    bytes: usize,
    apply: Duration,
    gap: Duration,
    last_frame: Option<Instant>,
    line: String,
}

impl Stats {
    fn record(&mut self, update: &Update) {
        let now = Instant::now();
        if let Some(last) = self.last_frame.replace(now) {
            self.gap += now - last;
        }
        self.frames += 1;
        self.bytes += update.bytes;
        self.apply += update.apply;
    }

    fn publish(&mut self) {
        let Self {
            frames,
            bytes,
            apply,
            gap,
            ..
        } = std::mem::take(self);
        self.line = if frames == 0 {
            "0fps \u{b7} 0ms gap \u{b7} 0ms paint \u{b7} idle".into()
        } else {
            let n = frames as f64;
            let per_frame = bytes as f64 / n;
            let size = if per_frame > 512.0 * 1024.0 {
                format!("{:.1}MB/frame", per_frame / (1024.0 * 1024.0))
            } else {
                format!("{}KB/frame", (per_frame / 1024.0).round())
            };
            format!(
                "{frames}fps \u{b7} {}ms gap \u{b7} {:.1}ms paint \u{b7} {size}",
                (gap.as_secs_f64() * 1000.0 / n).round(),
                apply.as_secs_f64() * 1000.0 / n,
            )
        };
        if frames > 0 {
            self.last_frame = Some(Instant::now());
        }
    }
}

pub(crate) struct VncTab {
    ctx: ServiceTabContext,
    /// The screen's focus: while it has it, every key goes to the remote.
    focus: FocusHandle,
    /// The tab's, outside the screen's key context.
    tab_focus: FocusHandle,
    window: AnyWindowHandle,

    session: Option<Session>,
    connecting: bool,
    connected: bool,
    error: Option<String>,
    connect_task: Option<Task<()>>,
    /// The connect in flight, shared with its core future so a session that
    /// connects after the tab has gone is closed rather than left open.
    pending: Option<Arc<Mutex<PendingConnect>>>,

    view_only: bool,
    fit: FitMode,
    /// The desktop's size, from the handshake, a resize event, or the last
    /// applied frame. For the header.
    size: (u16, u16),
    /// The size of the framebuffer the tiles were built for, which is what
    /// the screen lays them out by. A resize event arrives before the
    /// render thread has applied the frame that rebuilds the grid; laying
    /// the old tiles out at the new size drew a stretched copy of the old
    /// screen until it landed.
    drawn_size: (u16, u16),
    /// The screen as tiles: [`framebuffer::TILE`]-pixel images, row-major,
    /// `grid.0` across. `None` until a frame has drawn it.
    grid: (usize, usize),
    tiles: Vec<Option<Arc<RenderImage>>>,
    /// A frame applied while the tab or window was hidden, acknowledged when
    /// it is shown again rather than waiting out the session's timeout.
    withheld_ack: Option<u32>,
    /// What this side last told the session.
    paused: bool,

    input: Input,
    trailing: Option<Task<()>>,
    has_focus: bool,
    /// Input-method text being composed.
    marked_text: Option<String>,
    accent: Hsla,

    stats: Option<Stats>,
    _stats_timer: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl VncTab {
    fn new(ctx: ServiceTabContext, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus = cx.focus_handle();
        let tab_focus = cx.focus_handle();
        let fit =
            FitMode::from_setting(ctx.setting(cx, FIT_SETTING).as_deref()).unwrap_or(FitMode::Fit);
        let show_stats = ctx.setting(cx, STATS_SETTING).as_deref() == Some("true");

        let subscriptions = vec![
            // `watch(isActive)`: a tab out of sight stops the stream, and
            // coming back resumes it with a full refresh.
            cx.subscribe_in(
                &ctx.model,
                window,
                |this, _, event: &AppEvent, window, cx| match event {
                    AppEvent::ActiveTabChanged(_) => this.visibility_changed(window, cx),
                    AppEvent::HostsChanged => cx.notify(),
                    _ => {}
                },
            ),
            // `visibilitychange`: a minimised or covered window too.
            cx.observe_window_visibility(window, |this, _: WindowVisibility, window, cx| {
                this.visibility_changed(window, cx)
            }),
            // A focused element in a window that loses focus blurs in a
            // browser; GPUI keeps the focus, so release the keys here.
            cx.observe_window_activation(window, |this, window, cx| {
                if !window.is_window_active() {
                    let sends = this.input.blurred();
                    this.send(sends);
                    cx.notify();
                }
            }),
            cx.on_focus(&focus, window, |this, _, cx| {
                this.has_focus = true;
                cx.notify();
            }),
            cx.on_blur(&focus, window, |this, _, cx| {
                this.has_focus = false;
                let sends = this.input.blurred();
                this.send(sends);
                cx.notify();
            }),
            // `onBeforeUnmount`: the tab's own session goes with it. The
            // frame then runs `vnc_disconnect_host` if it was the host's last
            // remote-desktop tab.
            cx.on_release(|this, cx| {
                this.abandon_connect();
                this.disconnect_session();
                // The tiles' textures stay in the window's atlas until they
                // are dropped from it; the view has no window here.
                let tiles: Vec<_> = this.tiles.drain(..).flatten().collect();
                let handle = this.window;
                cx.defer(move |cx| {
                    let _ = handle.update(cx, |_, window, _| {
                        for tile in tiles {
                            let _ = window.drop_image(tile);
                        }
                    });
                });
            }),
        ];

        let stats_timer = show_stats.then(|| {
            cx.spawn(async move |this, cx| {
                loop {
                    cx.background_executor().timer(Duration::from_secs(1)).await;
                    let alive = this.update(cx, |this, cx| {
                        if let Some(stats) = this.stats.as_mut() {
                            stats.publish();
                            cx.notify();
                        }
                    });
                    if alive.is_err() {
                        break;
                    }
                }
            })
        });

        let mut this = Self {
            accent: palette(cx).accent,
            ctx,
            focus,
            tab_focus,
            window: window.window_handle(),
            session: None,
            connecting: false,
            connected: false,
            error: None,
            connect_task: None,
            pending: None,
            view_only: false,
            fit,
            size: (0, 0),
            drawn_size: (0, 0),
            grid: (0, 0),
            tiles: Vec::new(),
            withheld_ack: None,
            paused: false,
            input: Input::new(false),
            trailing: None,
            has_focus: false,
            marked_text: None,
            stats: show_stats.then(Stats::default),
            _stats_timer: stats_timer,
            _subscriptions: subscriptions,
        };
        this.connect(window, cx);
        this
    }

    fn session_id(&self) -> Option<&str> {
        self.session.as_ref().map(|s| s.id.as_str())
    }

    /// Whether the screen can be seen: the active tab, in a visible window.
    fn on_screen(&self, window: &Window, cx: &App) -> bool {
        self.ctx.is_shown(cx) && window.is_visible()
    }

    fn set_connected(&mut self, connected: bool, cx: &mut Context<Self>) {
        self.connected = connected;
        // The host row reads the tab, not this view: without this it stayed
        // green through a refused password or a dropped server.
        let (id, connecting) = (self.ctx.tab_id.clone(), self.connecting);
        self.ctx
            .model
            .update(cx, |m, cx| m.set_tab_status(&id, connected, connecting, cx));
        cx.notify();
    }

    // ---- the connection ------------------------------------------------

    fn connect(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.connecting {
            return;
        }
        self.disconnect_session();
        self.error = None;
        self.connecting = true;
        self.set_connected(false, cx);
        let host_id = self.ctx.host_id;
        let events = self.ctx.events();
        let pending = Arc::new(Mutex::new(PendingConnect::default()));
        self.pending = Some(pending.clone());
        self.connect_task = Some(self.ctx.run_with_secret(
            window,
            cx,
            move |core, ssh_password| {
                let (events, pending) = (events.clone(), pending.clone());
                async move {
                    let info = core
                        .vnc_connect(events, host_id, None, ssh_password)
                        .await?;
                    if PendingConnect::connected(&pending, &info.session_id) {
                        Ok(info)
                    } else {
                        // The tab went while this was connecting: nothing
                        // will ever take the session, so close it (and its
                        // tunnel, if it was the host's last).
                        let _ = core.vnc_disconnect(&info.session_id).await;
                        Err("cancelled".into())
                    }
                }
            },
            |this, result, window, cx| this.connected(result, window, cx),
        ));
    }

    /// The tab is going: close a session that has connected but not yet
    /// been delivered, and have one still connecting close itself.
    fn abandon_connect(&mut self) {
        if let Some(pending) = self.pending.take()
            && let Some(id) = PendingConnect::abandon(&pending)
        {
            let core = self.ctx.core().clone();
            self.ctx
                .spawn(async move { core.vnc_disconnect(&id).await });
        }
    }

    fn connected(
        &mut self,
        result: Result<VncSessionInfo, String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.connecting = false;
        // Delivered: the session is this view's to close from here on.
        if let Some(pending) = self.pending.take() {
            PendingConnect::claim(&pending);
        }
        let info = match result {
            Ok(info) => info,
            Err(e) => {
                self.error = Some(e);
                self.set_connected(false, cx);
                return;
            }
        };
        let id = info.session_id.clone();
        let (events, held) =
            self.ctx
                .subscribe_session(&id, cx.entity().downgrade(), |this, event, cx| {
                    this.session_event(event, cx)
                });

        // The sink goes in before any frame is requested: the session asks
        // for none until it has one, and attaching it forces a full update.
        // A session this view could not take is closed, not left open.
        let attached = renderer::start(info.width, info.height).and_then(|(sink, frames)| {
            self.ctx.core().vnc_open_frame_channel(&id, sink)?;
            Ok(frames)
        });
        let frames = match attached {
            Ok(frames) => frames,
            Err(e) => {
                let core = self.ctx.core().clone();
                self.ctx
                    .spawn(async move { core.vnc_disconnect(&id).await });
                self.error = Some(e);
                self.set_connected(false, cx);
                return;
            }
        };
        let frames_task = cx.spawn_in(window, async move |this, cx| {
            while let Ok(message) = frames.recv().await {
                if this
                    .update_in(cx, |this, window, cx| this.frame(message, window, cx))
                    .is_err()
                {
                    return;
                }
            }
            // The render thread is gone. Normally because the session
            // dropped its sink, which its own event reports; a thread that
            // died says so with `Message::Stopped` first.
        });

        self.size = (info.width, info.height);
        self.drawn_size = self.size;
        self.view_only = info.view_only;
        self.input = Input::new(info.view_only);
        self.drop_tiles(window);
        self.withheld_ack = None;
        self.paused = false;
        self.session = Some(Session {
            id,
            info,
            _events: events,
            _frames: frames_task,
        });
        self.set_connected(true, cx);
        if !self.on_screen(window, cx) {
            self.set_paused(true);
        }
        // What the session said before this view could listen.
        for event in held {
            self.session_event(&event, cx);
        }
    }

    /// `disconnect()`: release the keys, close the session in the
    /// background. The tunnel goes with the host's last session.
    fn disconnect_session(&mut self) {
        let sends = self.input.release_all();
        self.send(sends);
        self.trailing = None;
        if let Some(session) = self.session.take() {
            let core = self.ctx.core().clone();
            self.ctx
                .spawn(async move { core.vnc_disconnect(&session.id).await });
        }
    }

    fn session_event(&mut self, event: &CoreEvent, cx: &mut Context<Self>) {
        let shown = self.ctx.is_shown(cx);
        match event {
            CoreEvent::VncResize { width, height, .. } => {
                self.size = (*width, *height);
                cx.notify();
            }
            // The session's thread has ended; closing it here takes it out
            // of the core's registry and lets the host's tunnel go if it
            // was the last one.
            CoreEvent::VncError { error, .. } => {
                self.error = Some(error.clone());
                self.disconnect_session();
                self.set_connected(false, cx);
            }
            CoreEvent::VncDisconnected { .. } => {
                self.disconnect_session();
                self.set_connected(false, cx);
            }
            // The remote asked for attention. A toast rather than a sound,
            // and only from the tab in front.
            CoreEvent::VncBell { .. } if shown => {
                let name = self
                    .ctx
                    .host_name(cx)
                    .unwrap_or_else(|| "Remote desktop".into());
                self.toast_later(
                    format!("{name} is asking for attention"),
                    ToastKind::Info,
                    cx,
                );
            }
            // Only while the tab is in front: a background tab quietly
            // replacing the system clipboard is genuinely nasty.
            CoreEvent::VncClipboard { text, .. } if shown => {
                cx.write_to_clipboard(ClipboardItem::new_string(text.clone()));
            }
            _ => {}
        }
    }

    /// A toast from a place with no window at hand (a session event).
    fn toast_later(&self, message: String, kind: ToastKind, cx: &mut Context<Self>) {
        let handle = self.window;
        cx.defer(move |cx| {
            let _ = handle.update(cx, |_, window, cx| toast(window, cx, message, kind));
        });
    }

    // ---- frames --------------------------------------------------------

    fn frame(&mut self, message: Message, window: &mut Window, cx: &mut Context<Self>) {
        if self.session.is_none() {
            return;
        }
        let update = match message {
            Message::Frame(update) => update,
            Message::Corrupt(e) => {
                self.error = Some(format!("Corrupt frame from the server: {e}"));
                self.disconnect_session();
                self.set_connected(false, cx);
                return;
            }
            Message::Stopped => {
                self.error = Some(renderer::STOPPED.into());
                self.disconnect_session();
                self.set_connected(false, cx);
                return;
            }
        };
        if update.reset || update.grid != self.grid {
            self.drop_tiles(window);
            self.grid = update.grid;
            self.tiles = vec![None; update.grid.0 * update.grid.1];
        }
        self.size = update.size;
        self.drawn_size = update.size;
        for (index, image) in &update.tiles {
            if let Some(slot) = self.tiles.get_mut(*index)
                && let Some(old) = slot.replace(image.clone())
            {
                // Out of the atlas now; the next paint uses the new one.
                let _ = window.drop_image(old);
            }
        }
        if let Some(stats) = self.stats.as_mut() {
            stats.record(&update);
        }
        cx.notify();
        self.ack(update.seq, window, cx);
    }

    /// `ackFrame`: tell the session it may request the next frame. Withheld
    /// while the screen cannot be seen, so a hidden tab stops pulling frames
    /// (with `vnc_set_paused` doing the rest).
    fn ack(&mut self, seq: u32, window: &Window, cx: &App) {
        if !self.on_screen(window, cx) {
            self.withheld_ack = Some(seq);
            return;
        }
        self.withheld_ack = None;
        if let Some(id) = self.session_id() {
            let _ = self.ctx.core().vnc_frame_ack(id, seq);
        }
    }

    fn drop_tiles(&mut self, window: &mut Window) {
        for tile in self.tiles.drain(..).flatten() {
            let _ = window.drop_image(tile);
        }
        self.grid = (0, 0);
    }

    fn set_paused(&mut self, paused: bool) {
        self.paused = paused;
        if let Some(id) = self.session_id() {
            let _ = self.ctx.core().vnc_set_paused(id, paused);
        }
    }

    fn visibility_changed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.session.is_none() {
            return;
        }
        let on_screen = self.on_screen(window, cx);
        if on_screen && self.paused {
            // `resumeStream`: resume, hand back a withheld ack rather than
            // wait out the timeout, and repaint everything missed.
            self.set_paused(false);
            if let Some(seq) = self.withheld_ack.take() {
                self.ack(seq, window, cx);
            }
            self.refresh();
        } else if !on_screen && !self.paused {
            self.set_paused(true);
            let sends = self.input.release_all();
            self.send(sends);
        }
    }

    // ---- actions -------------------------------------------------------

    fn refresh(&self) {
        if let Some(id) = self.session_id() {
            let _ = self.ctx.core().vnc_refresh(id);
        }
    }

    fn toggle_view_only(&mut self, cx: &mut Context<Self>) {
        self.view_only = !self.view_only;
        if self.view_only {
            let sends = self.input.release_all();
            self.send(sends);
        }
        self.input.view_only = self.view_only;
        if let Some(id) = self.session_id() {
            let _ = self.ctx.core().vnc_set_view_only(id, self.view_only);
        }
        cx.notify();
    }

    fn toggle_fit(&mut self, cx: &mut Context<Self>) {
        self.fit = self.fit.toggled();
        self.ctx.set_setting(FIT_SETTING, self.fit.setting(), cx);
        cx.notify();
    }

    fn send_ctrl_alt_del(&self) {
        if let Some(id) = self.session_id() {
            let _ = self.ctx.core().vnc_key_sequence(id, &CTRL_ALT_DELETE);
        }
    }

    fn send_clipboard(&self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.session_id() else {
            return;
        };
        let Some(text) = cx.read_from_clipboard().and_then(|c| c.text()) else {
            return;
        };
        if text.is_empty() {
            return;
        }
        match self.ctx.core().vnc_send_clipboard(id, &text) {
            Ok(true) => self.ctx.toast(
                window,
                cx,
                "Some characters could not be sent: VNC clipboard is Latin-1 only",
                ToastKind::Warning,
            ),
            Ok(false) => {}
            Err(e) => self.ctx.toast(
                window,
                cx,
                format!("Could not send the clipboard: {e}"),
                ToastKind::Error,
            ),
        }
    }

    // ---- input ---------------------------------------------------------

    fn send(&self, sends: Vec<Send>) {
        let Some(id) = self.session_id() else {
            return;
        };
        let core = self.ctx.core();
        for s in sends {
            // Errors only say the session is gone, which its own event
            // reports; the Vue panel ignored them too.
            let _ = match s {
                Send::Pointer { x, y, mask } => core.vnc_pointer(id, x, y, mask),
                Send::Key { keysym, down } => core.vnc_key(id, keysym, down),
            };
        }
    }

    fn mouse_button(
        &mut self,
        button: MouseButton,
        down: bool,
        position: Point<Pixels>,
        geo: Geometry,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mut buttons = self.input.buttons();
        let slot = match button {
            MouseButton::Left => &mut buttons.left,
            MouseButton::Middle => &mut buttons.middle,
            MouseButton::Right => &mut buttons.right,
            MouseButton::Navigate(_) => return,
        };
        if *slot == down {
            return;
        }
        // A release only counts for a press this screen took.
        *slot = down;
        if down {
            window.focus(&self.focus, cx);
        }
        let sends =
            self.input
                .buttons_changed(geo.framebuffer_pixel(position), buttons, Instant::now());
        self.trailing = None;
        self.send(sends);
    }

    fn mouse_move(
        &mut self,
        position: Point<Pixels>,
        hovered: bool,
        geo: Geometry,
        cx: &mut Context<Self>,
    ) {
        if !hovered && !self.input.buttons().any() {
            return;
        }
        if self.fit == FitMode::Actual && geo.pans() {
            cx.notify();
        }
        match self
            .input
            .pointer_move(geo.framebuffer_pixel(position), Instant::now())
        {
            Move::Now(sends) => self.send(sends),
            Move::Wait => {}
            Move::ArmTrailing(delay) => {
                self.trailing = Some(cx.spawn(async move |this, cx| {
                    cx.background_executor().timer(delay).await;
                    let _ = this.update(cx, |this, _| {
                        let sends = this.input.flush_move(Instant::now());
                        this.send(sends);
                    });
                }));
            }
        }
    }

    fn wheel(&mut self, event: &ScrollWheelEvent, geo: Geometry) {
        // GPUI's deltas are positive for up/left; the DOM's for down/right.
        let (dx, dy) = match event.delta {
            ScrollDelta::Pixels(p) => (
                WheelDelta::Pixels(-f32::from(p.x)),
                WheelDelta::Pixels(-f32::from(p.y)),
            ),
            ScrollDelta::Lines(p) => (WheelDelta::Lines(-p.x), WheelDelta::Lines(-p.y)),
        };
        let sends = self
            .input
            .wheel(geo.framebuffer_pixel(event.position), dx, dy);
        self.send(sends);
    }

    fn key_down(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        // While the input method is composing, keys belong to it.
        if self.marked_text.is_some() {
            return;
        }
        let out = self
            .input
            .key_down(&event.keystroke, event.prefer_character_input);
        self.send(out.sends);
        if out.handled {
            cx.stop_propagation();
        }
    }

    fn key_up(&mut self, event: &KeyUpEvent, _: &mut Window, cx: &mut Context<Self>) {
        let sends = self.input.key_up(&event.keystroke.key);
        self.send(sends);
        cx.stop_propagation();
    }

    fn modifiers_changed(
        &mut self,
        event: &ModifiersChangedEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (sends, release) = self
            .input
            .modifiers_changed(event.modifiers, event.capslock.on);
        self.send(sends);
        if release {
            // Ctrl+Alt+Shift: the one keyboard route back to the app. The
            // tab itself takes the focus, as the page body did when the
            // canvas blurred, so the app's shortcuts work again (with
            // nothing focused they would reach no one).
            window.focus(&self.tab_focus, cx);
            cx.notify();
        }
    }

    // ---- rendering -----------------------------------------------------

    fn render_header(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let name = self
            .ctx
            .host_name(cx)
            .unwrap_or_else(|| "Remote desktop".into());
        let connected = self.connected;
        let view_only = self.view_only;
        let tunnelled = self.session.as_ref().is_some_and(|s| s.info.tunnelled);
        let meta = if self.size.0 > 0 {
            format!("{}\u{d7}{}", self.size.0, self.size.1)
        } else {
            String::new()
        };
        let stats = self.stats.as_ref().map(|s| s.line.clone());
        PanelHeader::new(name)
            .subtitle(self.ctx.display_uri(cx))
            .subtitle_mono()
            .meta(meta)
            .when(tunnelled, |h| {
                h.badge(
                    Tag::new("vnc-tunnelled", "tunnelled")
                        .tone(TagTone::Accent)
                        .tooltip("Through an SSH tunnel"),
                )
            })
            .when(view_only, |h| {
                h.badge(Tag::new("vnc-view-only", "view only").tone(TagTone::Warn))
            })
            .when_some(stats.filter(|s| !s.is_empty()), |h, line| {
                h.badge(Tag::new("vnc-stats", line).tooltip(STATS_HINT))
            })
            .action(
                IconButton::new("vnc-refresh", IconName::RefreshCw, "Refresh screen")
                    .disabled(!connected)
                    .on_click(cx.listener(|this, _, _, _| this.refresh())),
            )
            .action(
                IconButton::new(
                    "vnc-view-only",
                    if view_only {
                        IconName::EyeOff
                    } else {
                        IconName::Eye
                    },
                    if view_only {
                        "Allow input"
                    } else {
                        "View only"
                    },
                )
                .disabled(!connected)
                .on_click(cx.listener(|this, _, _, cx| this.toggle_view_only(cx))),
            )
            .action(
                IconButton::new(
                    "vnc-fit",
                    IconName::Maximize2,
                    match self.fit {
                        FitMode::Fit => "Actual size",
                        FitMode::Actual => "Fit to window",
                    },
                )
                .on_click(cx.listener(|this, _, _, cx| this.toggle_fit(cx))),
            )
            .action(
                IconButton::new("vnc-cad", IconName::Command, "Send Ctrl+Alt+Delete")
                    .disabled(!connected || view_only)
                    .on_click(cx.listener(|this, _, _, _| this.send_ctrl_alt_del())),
            )
            .action(
                IconButton::new(
                    "vnc-clipboard",
                    IconName::ClipboardCopy,
                    "Send clipboard to remote",
                )
                .disabled(!connected || view_only)
                .on_click(cx.listener(|this, _, window, cx| this.send_clipboard(window, cx))),
            )
    }

    fn render_error(&self, error: &str, p: &Palette, cx: &mut Context<Self>) -> AnyElement {
        h_flex()
            .flex_none()
            .items_start()
            .gap_2()
            .px_3()
            .py_2()
            .bg(p.bad_bg)
            .border_b_1()
            .border_color(p.bad_line)
            .child(
                div()
                    .flex_none()
                    .pt_0p5()
                    .child(Icon::new(IconName::CircleAlert).small().text_color(p.bad)),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_xs()
                    .text_color(p.bad)
                    .child(SharedString::from(error.to_string())),
            )
            .child(
                Button::new("vnc-reconnect")
                    .ghost()
                    .xsmall()
                    .label("Reconnect")
                    .disabled(self.connecting)
                    .on_click(cx.listener(|this, _, window, cx| this.connect(window, cx))),
            )
            .into_any_element()
    }
}

impl Focusable for VncTab {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for VncTab {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = *palette(cx);
        self.accent = p.accent;
        let body: AnyElement = if self.connecting {
            EmptyState::loading("vnc-connecting", "Connecting\u{2026}").into_any_element()
        } else if self.connected {
            div()
                .id("vnc-screen")
                .key_context(KEY_CONTEXT)
                .track_focus(&self.focus)
                .size_full()
                .on_key_down(cx.listener(Self::key_down))
                .on_key_up(cx.listener(Self::key_up))
                .on_modifiers_changed(cx.listener(Self::modifiers_changed))
                .child(VncScreen::new(cx.entity()))
                .into_any_element()
        } else if self.error.is_none() {
            // The session ended without an error: the server hung up
            // politely. The Vue panel went blank here.
            EmptyState::empty(
                "vnc-ended",
                "Disconnected",
                "Reconnect",
                cx.listener(|this, _, window, cx| this.connect(window, cx)),
            )
            .icon(Icon::new(IconName::Monitor))
            .hint("The remote desktop session has ended.")
            .into_any_element()
        } else {
            div().into_any_element()
        };
        v_flex()
            .size_full()
            .track_focus(&self.tab_focus)
            .bg(p.canvas)
            .child(self.render_header(cx))
            .when_some(self.error.clone(), |col, error| {
                col.child(self.render_error(&error, &p, cx))
            })
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .overflow_hidden()
                    .child(body),
            )
    }
}

/// The input method: composed text is typed into the remote a character at
/// a time. Ordinary keys never reach it (the key handler takes them first).
impl EntityInputHandler for VncTab {
    fn text_for_range(
        &mut self,
        _range: Range<usize>,
        _adjusted_range: &mut Option<Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        None
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: 0..0,
            reversed: false,
        })
    }

    fn marked_text_range(
        &self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Range<usize>> {
        self.marked_text
            .as_ref()
            .map(|text| 0..text.encode_utf16().count())
    }

    fn unmark_text(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.marked_text = None;
        cx.notify();
    }

    fn replace_text_in_range(
        &mut self,
        _range: Option<Range<usize>>,
        text: &str,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.marked_text.take().is_some() {
            cx.notify();
        }
        let sends = self.input.text(text);
        self.send(sends);
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        _range: Option<Range<usize>>,
        new_text: &str,
        _new_selected_range: Option<Range<usize>>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.marked_text = (!new_text.is_empty()).then(|| new_text.to_string());
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        _range_utf16: Range<usize>,
        element_bounds: Bounds<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        // The candidate window goes at the screen's corner: the remote's
        // caret is not something this side can know.
        Some(element_bounds)
    }

    fn character_index_for_point(
        &mut self,
        _point: Point<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::{Mutex, PendingConnect, Stats};

    #[test]
    fn a_session_that_connects_after_the_tab_closed_is_closed() {
        // Closed while connecting: the connect closes its own session.
        let p = Mutex::new(PendingConnect::default());
        assert_eq!(PendingConnect::abandon(&p), None);
        assert!(!PendingConnect::connected(&p, "vnc-1"));

        // Closed after connecting, before delivery: the tab closes it.
        let p = Mutex::new(PendingConnect::default());
        assert!(PendingConnect::connected(&p, "vnc-2"));
        assert_eq!(PendingConnect::abandon(&p).as_deref(), Some("vnc-2"));

        // Delivered: the session is the view's, and closing the tab does
        // not close it a second time.
        let p = Mutex::new(PendingConnect::default());
        assert!(PendingConnect::connected(&p, "vnc-3"));
        PendingConnect::claim(&p);
        assert_eq!(PendingConnect::abandon(&p), None);
    }
    use super::renderer::Update;
    use std::time::Duration;

    #[test]
    fn the_stats_line_reads_as_the_vue_badge() {
        let mut stats = Stats::default();
        stats.publish();
        assert_eq!(
            stats.line,
            "0fps \u{b7} 0ms gap \u{b7} 0ms paint \u{b7} idle"
        );
        let update = |bytes| Update {
            seq: 0,
            size: (1, 1),
            grid: (1, 1),
            reset: false,
            tiles: Vec::new(),
            bytes,
            apply: Duration::from_micros(1500),
        };
        stats.record(&update(2048));
        stats.record(&update(2048));
        stats.publish();
        assert!(stats.line.starts_with("2fps \u{b7} "), "{}", stats.line);
        assert!(
            stats.line.ends_with("1.5ms paint \u{b7} 2KB/frame"),
            "{}",
            stats.line
        );
    }
}
