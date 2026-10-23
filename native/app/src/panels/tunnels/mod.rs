//! The Tunnels panel (`PortForwardPanel.vue`) and the forward dialog
//! (`PortForwardModal.vue`). Per host. See `../README.md`.
//!
//! Saved forwards are rows in `port_forwards`; a running one lives in the
//! core's `forward_manager`, keyed by rule id, and keeps running after the
//! host's tabs close, as in the Tauri app. The panel reads each rule's
//! status after every load, as the Vue one did.
//!
//! Two entry points for the terminal's context menu:
//! [`detect_forward`] (is the selection a port worth offering, and what is
//! the entry called) and [`open_forward_editor`] (the dialog, prefilled).
//! A save from the dialog reloads the open panel for that host through
//! [`ForwardBus`], the port of the Vue `port-forward-added` event.

mod detect;
mod editor;
mod logic;

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{Disableable as _, Icon, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, AnyView, App, AppContext as _, Context, Entity, EventEmitter, FontWeight, Global,
    InteractiveElement as _, IntoElement, ParentElement as _, Render, SharedString,
    StatefulInteractiveElement as _, Styled as _, Subscription, Task, Window, div, px,
};
use termdrop_core::db::PortForward;

pub use self::detect::{ForwardHint, detect_forward};
pub use self::editor::EditorMode;
pub use self::logic::{ForwardKind, Prefill};

use self::logic::{browser_url, can_open, count_meta, listen_line, status_label, target_line};
use super::PanelContext;
use super::common::text_2xs;
use crate::core_backend::Services;
use crate::ui::empty_state::EmptyState;
use crate::ui::icon_button::IconButton;
use crate::ui::panel_header::PanelHeader;
use crate::ui::theme::{Palette, palette};
use crate::ui::toast::{ToastKind, toast};

// ---- the saved-forward bus ----------------------------------------------

/// A forward for `host_id` was added or edited: the Vue
/// `port-forward-added` window event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ForwardsChanged {
    pub host_id: i64,
}

/// Carries [`ForwardsChanged`] from the dialog to the panels. One per app,
/// created on first use.
pub struct ForwardBus;

impl EventEmitter<ForwardsChanged> for ForwardBus {}

struct BusGlobal(Entity<ForwardBus>);

impl Global for BusGlobal {}

/// The app's [`ForwardBus`]; subscribe with `cx.subscribe(&bus(cx), ...)`.
pub fn bus(cx: &mut App) -> Entity<ForwardBus> {
    if let Some(bus) = cx.try_global::<BusGlobal>() {
        return bus.0.clone();
    }
    let bus = cx.new(|_| ForwardBus);
    cx.set_global(BusGlobal(bus.clone()));
    bus
}

fn notify_saved(host_id: i64, cx: &mut App) {
    bus(cx).update(cx, |_, cx| cx.emit(ForwardsChanged { host_id }));
}

/// Open the Add Port Forward dialog for `host_id`, blank or with
/// `prefill` (the terminal's "Forward <port>":
/// `detect_forward(selection)?.prefill()`). Offer it for an SSH tab's own
/// host only, as `MainWindow.vue` did.
pub fn open_forward_editor(
    window: &mut Window,
    cx: &mut App,
    services: Arc<Services>,
    host_id: i64,
    prefill: Option<Prefill>,
) {
    editor::open_editor(window, cx, services, host_id, EditorMode::Add(prefill));
}

// ---- the panel -----------------------------------------------------------

/// The panel's view, built the first time the Tunnels tab is shown for a
/// host.
pub fn build(ctx: PanelContext, _window: &mut Window, cx: &mut App) -> AnyView {
    let view = cx.new(|cx| TunnelsPanel::new(ctx, cx));
    view.update(cx, |panel, cx| panel.load(cx));
    view.into()
}

/// What each row's buttons are doing, so a second click waits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RowAction {
    Start,
    Stop,
    Delete,
}

struct TunnelsPanel {
    ctx: PanelContext,
    forwards: Vec<PortForward>,
    /// `activeStatus`, by rule id.
    active: HashMap<i64, bool>,
    /// Shown instead of the empty state until the first answer (the Vue
    /// panel once claimed "none" while still asking).
    loading: bool,
    load_seq: u64,
    load_task: Option<Task<()>>,
    busy: HashSet<i64>,
    _bus: Subscription,
}

impl TunnelsPanel {
    fn new(ctx: PanelContext, cx: &mut Context<Self>) -> Self {
        let host_id = ctx.host_id;
        let bus = bus(cx);
        let sub = cx.subscribe(&bus, move |this: &mut Self, _, ev: &ForwardsChanged, cx| {
            if ev.host_id == host_id {
                this.load(cx);
            }
        });
        Self {
            ctx,
            forwards: Vec::new(),
            active: HashMap::new(),
            loading: false,
            load_seq: 0,
            load_task: None,
            busy: HashSet::new(),
            _bus: sub,
        }
    }

    fn services(&self) -> Arc<Services> {
        self.ctx.services.clone()
    }

    /// `loadForwards`: the rules, then each one's status.
    fn load(&mut self, cx: &mut Context<Self>) {
        self.loading = true;
        self.load_seq += 1;
        let seq = self.load_seq;
        let core = self.ctx.core().clone();
        let host_id = self.ctx.host_id;
        let job = self.ctx.spawn_blocking(move || {
            core.get_port_forwards(host_id).map(|forwards| {
                let active: HashMap<i64, bool> = forwards
                    .iter()
                    .map(|f| (f.id, core.port_forward_status(f.id)))
                    .collect();
                (forwards, active)
            })
        });
        self.load_task = Some(cx.spawn(async move |this, cx| {
            let result = job.await.unwrap_or_else(|e| Err(e.to_string()));
            let _ = this.update(cx, |this, cx| {
                if seq != this.load_seq {
                    return;
                }
                this.loading = false;
                this.load_task = None;
                match result {
                    Ok((forwards, active)) => {
                        this.forwards = forwards;
                        this.active.extend(active);
                    }
                    // The Vue panel let this reject unseen and kept its
                    // list; so does this one, but it is logged.
                    Err(e) => tracing::error!("get_port_forwards failed: {e}"),
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn is_active(&self, id: i64) -> bool {
        self.active.get(&id).copied().unwrap_or(false)
    }

    /// Start, Stop or Delete one rule off the UI thread. Start reads the
    /// keyring and binds the listener; each connection through it opens an
    /// SSH session that checks the host key and fails closed.
    fn run(&mut self, action: RowAction, id: i64, window: &mut Window, cx: &mut Context<Self>) {
        if !self.busy.insert(id) {
            return;
        }
        cx.notify();
        let core = self.ctx.core().clone();
        let job = self.ctx.spawn_blocking(move || match action {
            RowAction::Start => core.start_port_forward(id),
            RowAction::Stop => {
                core.stop_port_forward(id);
                Ok(())
            }
            RowAction::Delete => core.delete_port_forward(id),
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = job.await.unwrap_or_else(|e| Err(e.to_string()));
            let _ = cx.update(|window, cx| {
                this.update(cx, |this, cx| {
                    this.busy.remove(&id);
                    match (action, result) {
                        (RowAction::Start, Ok(())) => {
                            this.active.insert(id, true);
                            toast(window, cx, "Port forward started", ToastKind::Success);
                        }
                        (RowAction::Stop, Ok(())) => {
                            this.active.insert(id, false);
                            toast(window, cx, "Port forward stopped", ToastKind::Success);
                        }
                        (RowAction::Delete, Ok(())) => {
                            this.active.remove(&id);
                            this.load(cx);
                            toast(window, cx, "Port forward deleted", ToastKind::Success);
                        }
                        (RowAction::Start, Err(e)) => toast(
                            window,
                            cx,
                            format!("Failed to start: {e}"),
                            ToastKind::Error,
                        ),
                        (RowAction::Stop, Err(e)) => {
                            toast(window, cx, format!("Failed to stop: {e}"), ToastKind::Error)
                        }
                        (RowAction::Delete, Err(e)) => toast(
                            window,
                            cx,
                            format!("Failed to delete: {e}"),
                            ToastKind::Error,
                        ),
                    }
                    cx.notify();
                })
            });
        })
        .detach();
    }

    fn add(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        editor::open_editor(
            window,
            cx,
            self.services(),
            self.ctx.host_id,
            EditorMode::Add(None),
        );
    }

    fn edit(&mut self, forward: PortForward, window: &mut Window, cx: &mut Context<Self>) {
        editor::open_editor(
            window,
            cx,
            self.services(),
            self.ctx.host_id,
            EditorMode::Edit(forward),
        );
    }

    // ---- rendering --------------------------------------------------------

    fn render_body(&self, p: &Palette, cx: &mut Context<Self>) -> AnyElement {
        if self.loading && self.forwards.is_empty() {
            return div()
                .p_2()
                .child(EmptyState::loading(
                    "tunnels-loading",
                    "Loading forwards\u{2026}",
                ))
                .into_any_element();
        }
        if self.forwards.is_empty() {
            return div()
                .p_2()
                .child(
                    EmptyState::empty(
                        "tunnels-empty",
                        "No port forwards",
                        "Add a forward",
                        cx.listener(|this, _, window, cx| this.add(window, cx)),
                    )
                    .icon(Icon::new(IconName::Network))
                    .hint("Forward a remote port to this machine"),
                )
                .into_any_element();
        }
        div()
            .id("tunnels-list")
            .size_full()
            .overflow_y_scroll()
            .px_2()
            .py_1()
            .children(self.forwards.iter().map(|f| self.render_card(f, p, cx)))
            .into_any_element()
    }

    fn render_card(&self, f: &PortForward, p: &Palette, cx: &mut Context<Self>) -> AnyElement {
        let id = f.id;
        let active = self.is_active(id);
        let busy = self.busy.contains(&id);
        let key = |what: &str| SharedString::from(format!("tunnel-{what}-{id}"));

        let (badge_text, badge_fill) = if active {
            (p.good, p.good.opacity(0.2))
        } else {
            (p.ink_2, p.input)
        };
        let badge = text_2xs(div())
            .flex_none()
            .px(px(6.))
            .py(px(2.))
            .rounded_sm()
            .font_weight(FontWeight::MEDIUM)
            .bg(badge_fill)
            .text_color(badge_text)
            .child(status_label(active));

        let line = |icon: IconName, text: String| {
            h_flex()
                .gap_1()
                .min_w_0()
                .child(Icon::new(icon).size(px(9.)))
                .child(div().min_w_0().truncate().child(text))
        };

        let toggle =
            if active {
                Button::new(key("stop"))
                    .xsmall()
                    .label("Stop")
                    .flex_1()
                    .loading(busy)
                    .disabled(busy)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.run(RowAction::Stop, id, window, cx)
                    }))
            } else {
                Button::new(key("start"))
                    .xsmall()
                    .primary()
                    .label("Start")
                    .flex_1()
                    .loading(busy)
                    .disabled(busy)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.run(RowAction::Start, id, window, cx)
                    }))
            };
        let editing = f.clone();
        let url = browser_url(f);

        v_flex()
            .id(key("card"))
            .mb_2()
            .p_2()
            .gap_1()
            .bg(p.surface)
            .border_1()
            .border_color(p.line)
            .rounded_sm()
            .child(
                h_flex()
                    .justify_between()
                    .gap_2()
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .text_xs()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(p.ink)
                            .child(f.name.clone()),
                    )
                    .child(badge),
            )
            .child(
                text_2xs(v_flex().gap_0p5().text_color(p.ink_2))
                    .child(line(IconName::ArrowRightLeft, listen_line(f)))
                    .when_some(target_line(f), |d, target| {
                        d.child(line(IconName::ArrowRight, target))
                    }),
            )
            .child(
                h_flex()
                    .mt_1()
                    .gap_1()
                    .child(toggle)
                    .child(
                        IconButton::new(key("edit"), IconName::Pencil, "Edit").on_click(
                            cx.listener(move |this, _, window, cx| {
                                this.edit(editing.clone(), window, cx)
                            }),
                        ),
                    )
                    .when(can_open(f, active), |d| {
                        d.child(
                            IconButton::new(key("open"), IconName::ExternalLink, "Open in browser")
                                .tone(p.accent)
                                .on_click(move |_, _, cx| cx.open_url(&url)),
                        )
                    })
                    .child(
                        IconButton::new(key("delete"), IconName::Trash, "Delete")
                            .tone(p.bad)
                            .disabled(busy)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.run(RowAction::Delete, id, window, cx)
                            })),
                    ),
            )
            .into_any_element()
    }
}

impl Render for TunnelsPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = *palette(cx);
        let header = PanelHeader::new("Port forwards")
            .meta(count_meta(self.forwards.len()))
            .action(
                IconButton::new("tunnels-add", IconName::Plus, "Add a forward")
                    .on_click(cx.listener(|this, _, window, cx| this.add(window, cx))),
            );
        v_flex().size_full().bg(p.canvas).child(header).child(
            div()
                .flex_1()
                .min_h(px(80.))
                .relative()
                .child(self.render_body(&p, cx)),
        )
    }
}
