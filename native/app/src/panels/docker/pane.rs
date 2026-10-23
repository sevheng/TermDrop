//! The logs / exec pane under the terminal: the Docker pane of
//! `TerminalTab.vue`. The Docker panel builds it and hands it to the active
//! terminal tab (`PanelContext::show_terminal_pane`), which owns it from then
//! on; the pane ends its own PTY when it is released.
//!
//! Each run (opening, or toggling Follow on a logs pane, which Vue did by
//! reopening the pane) is one exec PTY with a fresh terminal view.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{
    Icon, Selectable as _, Sizable as _, h_flex, resizable::HandleEdge, resize_handle, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, AppContext as _, Axis, Context, DragMoveEvent, Empty, Entity, Focusable as _,
    InteractiveElement as _, IntoElement, ParentElement as _, Render, Styled as _, Subscription,
    Task, Window, div, px,
};
use termdrop_core::CoreEvent;
use termdrop_terminal::{TerminalOptions, TerminalView};

use super::logic::{
    KEY_FLUSH_MS, KeyBatch, PANE_HEIGHT, PaneKind, clamp_pane_height, exec_command,
    is_quiet_write_error, logs_command, pane_title, pty_error_text, start_failed_text,
};
use crate::app::{AppEvent, AppModel};
use crate::panels::common::text_2xs;
use crate::panels::{EventKey, PanelContext, PanelSubscription};
use crate::tabs::term_palette;
use crate::ui::icon_button::IconButton;
use crate::ui::theme::palette;

/// The drag payload for the pane's top edge.
#[derive(Clone)]
struct ResizePane;

impl Render for ResizePane {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        Empty
    }
}

/// One exec PTY. Ending it (a new run, the pane released) sets `closed`, so
/// a connect still in flight disconnects what it produces instead of
/// leaving a PTY nobody owns.
struct Run {
    id: String,
    closed: Arc<AtomicBool>,
    _events: PanelSubscription,
    _connect: Task<()>,
}

pub struct DockerPane {
    ctx: PanelContext,
    kind: PaneKind,
    container: String,
    /// For an exec pane: the shell `docker_inspect_shell` chose.
    shell: String,
    /// The terminal tab the pane sits in, once shown.
    tab_id: Option<String>,
    following: bool,
    height: f32,
    terminal: Option<Entity<TerminalView>>,
    run: Option<Run>,
    _subscriptions: Vec<Subscription>,
}

impl DockerPane {
    pub fn new(
        ctx: PanelContext,
        kind: PaneKind,
        container: String,
        shell: String,
        cx: &mut App,
    ) -> Entity<Self> {
        let model = ctx.model.clone();
        cx.new(|cx| {
            cx.on_release(|this: &mut Self, _| this.end_run()).detach();
            let subscriptions = vec![
                cx.subscribe(&model, Self::on_app_event),
                cx.observe(&model, Self::on_model_changed),
            ];
            Self {
                ctx,
                kind,
                container,
                shell,
                tab_id: None,
                following: false,
                height: PANE_HEIGHT,
                terminal: None,
                run: None,
                _subscriptions: subscriptions,
            }
        })
    }

    /// The pane is in `tab_id`'s terminal now: start its PTY.
    pub fn shown_in(
        &mut self,
        tab_id: String,
        follow: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.tab_id = Some(tab_id);
        self.start(follow, window, cx);
    }

    /// Start a run: a fresh terminal and a new exec PTY, replacing any run
    /// before it (`openDockerPane` closed the old pane first).
    fn start(&mut self, follow: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.end_run();
        let command = match self.kind {
            PaneKind::Logs => logs_command(&self.container, follow),
            PaneKind::Exec => exec_command(&self.container, &self.shell),
        };
        self.following = self.kind == PaneKind::Logs && follow;

        let id = self.ctx.new_pty_session_id();
        let closed = Arc::new(AtomicBool::new(false));
        let terminal = self.new_terminal(&id, &closed, window, cx);

        // Subscribed before connecting: the id is ours, so nothing is missed.
        let events = {
            let run_id = id.clone();
            self.ctx.subscribe(
                EventKey::ExecPty(id.clone()),
                cx.entity().downgrade(),
                move |this: &mut Self, event, cx| this.on_pty_event(&run_id, event, cx),
            )
        };

        let core = self.ctx.core().clone();
        let sink = self.ctx.events();
        let host_id = self.ctx.host_id;
        let job = {
            let id = id.clone();
            let closed = closed.clone();
            self.ctx.spawn(async move {
                let result = core
                    .exec_pty_connect(sink, host_id, id.clone(), command)
                    .await;
                // The pane went away while this ran: end what it started.
                if result.is_ok() && closed.load(Ordering::SeqCst) {
                    let _ = core.exec_pty_disconnect(&id);
                }
                result
            })
        };
        let connect = {
            let run_id = id.clone();
            cx.spawn(async move |this, cx| {
                let result = job.await.unwrap_or_else(|e| Err(e.to_string()));
                let _ = this.update(cx, |this, cx| this.connected(&run_id, result, cx));
            })
        };

        if self.kind == PaneKind::Exec {
            window.focus(&terminal.focus_handle(cx), cx);
        }
        self.terminal = Some(terminal);
        self.run = Some(Run {
            id,
            closed,
            _events: events,
            _connect: connect,
        });
        cx.notify();
    }

    fn new_terminal(
        &self,
        id: &str,
        closed: &Arc<AtomicBool>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<TerminalView> {
        let (font_size, theme) = {
            let m = self.ctx.model.read(cx);
            (m.font_size(), m.theme())
        };
        // Keystrokes are gathered on a flat 16 ms timer and written in one
        // go, as the Vue docker pane did.
        let input = {
            let services = self.ctx.services.clone();
            let batch = Arc::new(Mutex::new(KeyBatch::default()));
            let id = id.to_string();
            let closed = closed.clone();
            Box::new(move |bytes: Vec<u8>| {
                let arm = batch.lock().unwrap_or_else(|e| e.into_inner()).push(&bytes);
                if !arm {
                    return;
                }
                let core = services.core.clone();
                let batch = batch.clone();
                let id = id.clone();
                let closed = closed.clone();
                services.rt.spawn_blocking(move || {
                    std::thread::sleep(Duration::from_millis(KEY_FLUSH_MS));
                    let data = batch.lock().unwrap_or_else(|e| e.into_inner()).take();
                    if data.is_empty() || closed.load(Ordering::SeqCst) {
                        return;
                    }
                    if let Err(e) = core.exec_pty_write_bytes(&id, data)
                        && !is_quiet_write_error(&e)
                    {
                        tracing::warn!("exec_pty_write failed: {e}");
                    }
                });
            })
        };
        // There is no exec PTY resize.
        let on_resize = Box::new(|_: u16, _: u16| {});
        TerminalView::new(
            window,
            cx,
            TerminalOptions {
                font_size: px(f32::from(font_size)),
                palette: term_palette(theme),
                ..TerminalOptions::default()
            },
            input,
            on_resize,
        )
    }

    fn connected(&mut self, run_id: &str, result: Result<String, String>, cx: &mut Context<Self>) {
        if self.run.as_ref().is_none_or(|r| r.id != run_id) {
            return;
        }
        let Some(terminal) = self.terminal.clone() else {
            return;
        };
        let feed = terminal.read(cx).feed();
        match result {
            Ok(_) => {
                let services = &self.ctx.services;
                if let Err(e) = services
                    .bridge
                    .attach_exec_pty(&services.core, run_id, feed)
                {
                    tracing::warn!("Could not attach the docker pane's output: {e}");
                }
            }
            Err(e) => {
                tracing::error!("exec_pty_connect failed: {e}");
                feed.feed(start_failed_text(&e).as_bytes());
            }
        }
    }

    fn on_pty_event(&mut self, run_id: &str, event: &CoreEvent, cx: &mut Context<Self>) {
        if self.run.as_ref().is_none_or(|r| r.id != run_id) {
            return;
        }
        match event {
            CoreEvent::PtyError { error, .. } => {
                if let Some(t) = &self.terminal {
                    t.read(cx).feed().feed(pty_error_text(error).as_bytes());
                }
            }
            CoreEvent::PtyDisconnected { .. } => {
                self.following = false;
                // An exec pane closes when its shell exits.
                if self.kind == PaneKind::Exec {
                    self.close(cx);
                }
                cx.notify();
            }
            _ => {}
        }
    }

    /// End the current run's PTY, if any. Safe to call twice.
    fn end_run(&mut self) {
        let Some(run) = self.run.take() else {
            return;
        };
        run.closed.store(true, Ordering::SeqCst);
        let services = self.ctx.services.clone();
        services.bridge.detach_exec_pty(&run.id);
        let core = services.core.clone();
        let id = run.id;
        services.rt.spawn_blocking(move || {
            let _ = core.exec_pty_disconnect(&id);
        });
    }

    /// Take the pane out of its tab. Releasing it ends the PTY.
    fn close(&mut self, cx: &mut Context<Self>) {
        self.end_run();
        if let Some(tab_id) = self.tab_id.clone() {
            let ctx = self.ctx.clone();
            cx.defer(move |cx| ctx.close_terminal_pane(&tab_id, cx));
        }
    }

    fn toggle_follow(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.kind != PaneKind::Logs {
            return;
        }
        let follow = !self.following;
        self.start(follow, window, cx);
    }

    fn on_app_event(&mut self, _: Entity<AppModel>, event: &AppEvent, cx: &mut Context<Self>) {
        let Some(terminal) = self.terminal.clone() else {
            return;
        };
        match event {
            AppEvent::FontSizeChanged(size) => {
                let size = px(f32::from(*size));
                terminal.update(cx, |t, cx| t.set_font_size(size, cx));
            }
            AppEvent::ThemeChanged(theme) => {
                let palette = term_palette(*theme);
                terminal.update(cx, |t, cx| t.set_palette(palette, cx));
            }
            _ => {}
        }
    }

    /// The shell under the pane disconnected: close it, as Vue's
    /// `onDisconnected` did.
    fn on_model_changed(&mut self, model: Entity<AppModel>, cx: &mut Context<Self>) {
        let Some(tab_id) = self.tab_id.as_deref() else {
            return;
        };
        if self.run.is_none() {
            return;
        }
        let ended = model
            .read(cx)
            .tab(tab_id)
            .is_none_or(|t| !t.connected && !t.connecting);
        if ended {
            self.close(cx);
        }
    }
}

impl Render for DockerPane {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = *palette(cx);
        let is_logs = self.kind == PaneKind::Logs;
        let following = self.following;
        let icon = if is_logs {
            IconName::FileText
        } else {
            IconName::Terminal
        };
        let header = h_flex()
            .flex_none()
            .justify_between()
            .px_2()
            .py_1()
            .border_b_1()
            .border_color(p.line)
            .child(
                text_2xs(h_flex().gap_1p5().min_w_0().text_color(p.ink))
                    .child(Icon::new(icon).xsmall())
                    .child(
                        div()
                            .truncate()
                            .child(pane_title(self.kind, &self.container)),
                    ),
            )
            .child(
                h_flex()
                    .gap_1p5()
                    .when(is_logs, |row| {
                        // A tint, not a fill: the good tone cannot carry a
                        // white label (Vue: `bg-good/20 text-good`).
                        let (label, tone) = if following {
                            ("\u{25cf} Following", p.good)
                        } else {
                            ("Follow", p.ink_2)
                        };
                        row.child(
                            Button::new("docker-pane-follow")
                                .ghost()
                                .xsmall()
                                .selected(following)
                                .child(text_2xs(div().text_color(tone)).child(label))
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.toggle_follow(window, cx)
                                })),
                        )
                    })
                    .child(
                        IconButton::new("docker-pane-close", IconName::X, "Close pane")
                            .on_click(cx.listener(|this, _, _, cx| this.close(cx))),
                    ),
            );

        v_flex()
            .id("docker-pane")
            .relative()
            .w_full()
            .h(px(self.height))
            .bg(p.canvas)
            .on_drag_move(cx.listener(|this, e: &DragMoveEvent<ResizePane>, _, cx| {
                let height = f32::from(e.bounds.bottom() - e.event.position.y);
                this.height = clamp_pane_height(height);
                cx.notify();
            }))
            .child(header)
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .when_some(self.terminal.clone(), |d, t| d.child(t)),
            )
            .child(
                resize_handle::<ResizePane, ResizePane>("docker-pane-edge", Axis::Vertical)
                    .inside(HandleEdge::Leading)
                    .on_drag(ResizePane, |drag, _, _, cx| cx.new(|_| drag.clone())),
            )
    }
}
