//! The Security panel (`SecurityPanel.vue` plus the store's
//! `securityReports`). Per host, and lazy: building the view is the panel
//! being opened, which is the only time an audit starts on its own. See
//! `../README.md`.
//!
//! - **Once per host.** The frame builds this view the first time the panel
//!   is shown for a host and keeps it until the host's last SSH tab closes,
//!   which is when the Vue store dropped its report. So starting the audit
//!   in [`build`] runs it once per host, and the report dies at the right
//!   moment with no extra code.
//! - **An error is never retried by itself**: only Retry or Refresh, both
//!   with `force`, run it again.
//! - **Remediation goes through `PanelContext::insert_command` and nothing
//!   else**, which types it with no newline after a one-time confirm. The
//!   command shown and sent is `check.remediation.command` exactly as the
//!   core built it from its own literals; nothing here builds one.

mod report;

use std::collections::HashSet;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::{Icon, Sizable, h_flex, v_flex};
use gpui_kit::{
    AnyElement, AnyView, App, AppContext as _, ClipboardItem, Context, FontWeight, Hsla,
    InteractiveElement as _, IntoElement, ParentElement, Render, SharedString,
    StatefulInteractiveElement as _, Styled, Subscription, Task, Window, div, px,
};
use termdrop_core::security::{SecurityCheck, SecurityReport};

use self::report::{Audit, AuditState, Grade, Status, Tone};
use super::PanelContext;
use super::common::{row_separator, text_2xs};
use crate::ui::empty_state::EmptyState;
use crate::ui::icon_button::IconButton;
use crate::ui::panel_header::PanelHeader;
use crate::ui::theme::{Palette, palette};

/// How often the "updated N ago" label is re-read while the panel is shown.
const CLOCK: Duration = Duration::from_secs(15);

/// The panel's view. Building it is opening the panel, so the audit starts
/// here (from the core's per-host cache when it has a fresh report).
pub fn build(ctx: PanelContext, _window: &mut Window, cx: &mut App) -> AnyView {
    let view = cx.new(|cx| SecurityPanel::new(ctx, cx));
    view.update(cx, |panel, cx| panel.run(false, cx));
    view.into()
}

pub struct SecurityPanel {
    ctx: PanelContext,
    audit: Audit,
    /// Which findings have their guidance open, by check name. Cleared when
    /// a run starts: a new report can renumber or drop checks.
    expanded: HashSet<String>,
    /// The UI half of the running audit. Dropping it cancels only that half;
    /// the core finishes the run and caches it.
    task: Option<Task<()>>,
    was_shown: bool,
    _clock: Task<()>,
    _model: Subscription,
}

impl SecurityPanel {
    fn new(ctx: PanelContext, cx: &mut Context<Self>) -> Self {
        // Re-render on a timer so the age label ages instead of freezing at
        // "just now"; only while on screen, where it is read.
        let clock = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(CLOCK).await;
                let alive = this.update(cx, |this, cx| {
                    if this.ctx.is_shown(cx) {
                        cx.notify();
                    }
                });
                if alive.is_err() {
                    break;
                }
            }
        });
        // Being shown again refreshes the age at once (Vue's `onActivated`),
        // and whether "Send to terminal" is offered follows the active tab.
        let model = cx.observe(&ctx.model, |this: &mut Self, _, cx| {
            let shown = this.ctx.is_shown(cx);
            if shown || shown != this.was_shown {
                cx.notify();
            }
            this.was_shown = shown;
        });
        let was_shown = ctx.is_shown(cx);
        Self {
            ctx,
            audit: Audit::default(),
            expanded: HashSet::new(),
            task: None,
            was_shown,
            _clock: clock,
            _model: model,
        }
    }

    /// Run the audit. `force` skips the core's per-host cache (Refresh and
    /// Retry); the lazy first run takes a cached report if one is fresh.
    fn run(&mut self, force: bool, cx: &mut Context<Self>) {
        let run = self.audit.begin();
        self.expanded.clear();
        let core = self.ctx.core().clone();
        let host_id = self.ctx.host_id;
        let job = self
            .ctx
            .spawn(async move { core.run_security_audit(host_id, force).await });
        self.task = Some(cx.spawn(async move |this, cx| {
            let result = job.await.unwrap_or_else(|e| Err(e.to_string()));
            let _ = this.update(cx, |this, cx| {
                if this.audit.finish(run, result) {
                    cx.notify();
                }
            });
        }));
        cx.notify();
    }

    fn toggle(&mut self, name: &str, cx: &mut Context<Self>) {
        if !self.expanded.remove(name) {
            self.expanded.insert(name.to_string());
        }
        cx.notify();
    }

    fn header(&self, cx: &mut Context<Self>) -> PanelHeader {
        let mut header = PanelHeader::new("Security");
        if let Some(r) = self.audit.report() {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            header = header
                .subtitle(report::updated_label(r.generated_at, now))
                .meta(report::checks_label(r));
        }
        header.action(
            IconButton::new(
                "security-refresh",
                Icon::new(IconName::RefreshCw),
                "Re-run the audit",
            )
            .pending(self.audit.is_loading())
            .on_click(cx.listener(|this, _, _, cx| this.run(true, cx))),
        )
    }

    fn body(&self, cx: &mut Context<Self>) -> AnyElement {
        match &self.audit.state {
            AuditState::Loading => div()
                .p_2()
                .child(
                    EmptyState::loading("security-loading", "Running security audit\u{2026}")
                        .hint("Switch tabs freely \u{2014} results will appear here"),
                )
                .into_any_element(),
            AuditState::Failed(error) => div()
                .p_2()
                .child(
                    EmptyState::error(
                        "security-error",
                        "Audit failed",
                        "Retry",
                        cx.listener(|this, _, _, cx| this.run(true, cx)),
                    )
                    .icon(Icon::new(IconName::ShieldAlert))
                    .hint(error.clone()),
                )
                .into_any_element(),
            AuditState::Idle => div()
                .p_2()
                .child(
                    EmptyState::empty(
                        "security-empty",
                        "No audit has run for this host",
                        "Run audit",
                        cx.listener(|this, _, _, cx| this.run(true, cx)),
                    )
                    .icon(Icon::new(IconName::Shield))
                    .hint("The audit runs privileged probes on the server"),
                )
                .into_any_element(),
            AuditState::Ready(r) => self.report_view(r, cx).into_any_element(),
        }
    }

    fn report_view(&self, r: &SecurityReport, cx: &mut Context<Self>) -> impl IntoElement {
        let p = *palette(cx);
        let can_send = self.ctx.can_insert(cx);
        let rows: Vec<AnyElement> = r
            .checks
            .iter()
            .enumerate()
            .map(|(ix, check)| self.check_row(ix, check, can_send, &p, cx))
            .collect();
        v_flex()
            .child(score_header(r, &p))
            .child(v_flex().children(rows))
    }

    fn check_row(
        &self,
        ix: usize,
        check: &SecurityCheck,
        can_send: bool,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let status = Status::parse(&check.status);
        let (badge_bg, tone) = tone_colors(status.tone(), p);
        let mono = crate::ui::theme::mono_font(cx);

        let mut col = v_flex()
            .flex_1()
            .min_w_0()
            .child(
                h_flex()
                    .gap_1p5()
                    .child(div().text_xs().text_color(p.ink).child(check.name.clone()))
                    .child(
                        text_2xs(div())
                            .px_1()
                            .rounded_sm()
                            .font_weight(FontWeight::MEDIUM)
                            .bg(badge_bg)
                            .text_color(tone)
                            .child(report::badge_text(&check.status)),
                    ),
            )
            .child(
                text_2xs(div())
                    .mt_0p5()
                    .text_color(p.ink_2)
                    .child(check.message.clone()),
            );
        if let Some(detail) = &check.detail {
            col = col.child(
                text_2xs(div())
                    .mt_0p5()
                    .font_family(mono.clone())
                    .text_color(p.ink_3)
                    .child(detail.clone()),
            );
        }

        if let Some(fix) = &check.remediation {
            let open = self.expanded.contains(&check.name);
            let name = check.name.clone();
            let accent = p.accent;
            col = col.child(
                text_2xs(h_flex())
                    .id(("security-toggle", ix))
                    .mt_1()
                    .gap_0p5()
                    .cursor_pointer()
                    .text_color(accent)
                    .hover(|s| s.underline())
                    .child(
                        Icon::new(if open {
                            IconName::ChevronDown
                        } else {
                            IconName::ChevronRight
                        })
                        .size(px(10.)),
                    )
                    .child("What to do")
                    .on_click(cx.listener(move |this, _, _, cx| this.toggle(&name, cx))),
            );
            if open {
                col = col.child(self.remediation(ix, fix, can_send, &mono, p));
            }
        }

        h_flex()
            .px_2()
            .py_1p5()
            .gap_2()
            .items_start()
            .border_b_1()
            .border_color(row_separator(p))
            .child(
                div().mt_0p5().flex_none().child(
                    Icon::new(status_icon(status))
                        .size(px(14.))
                        .text_color(tone),
                ),
            )
            .child(col)
            .into_any_element()
    }

    fn remediation(
        &self,
        ix: usize,
        fix: &termdrop_core::security::Remediation,
        can_send: bool,
        mono: &SharedString,
        p: &Palette,
    ) -> AnyElement {
        let mut block = v_flex().mt_1().mb_0p5().child(
            text_2xs(div())
                .text_color(p.ink_2)
                .child(fix.summary.clone()),
        );
        let Some(command) = fix.command.clone() else {
            return block.into_any_element();
        };
        let copy = {
            let command = command.clone();
            Button::new(("security-copy", ix))
                .ghost()
                .xsmall()
                .icon(IconName::Copy)
                .label("Copy")
                .on_click(move |_, _, cx| {
                    // Clipboard access can fail; the command is on screen
                    // either way, as in Vue.
                    cx.write_to_clipboard(ClipboardItem::new_string(command.clone()));
                })
        };
        let mut actions = h_flex().gap_2().mt_1().child(copy);
        if can_send {
            let ctx = self.ctx.clone();
            let command = command.clone();
            actions = actions.child(
                Button::new(("security-send", ix))
                    .ghost()
                    .xsmall()
                    .icon(IconName::SquareTerminal)
                    .label("Send to terminal")
                    .on_click(move |_, window, cx| {
                        // The only way a command reaches a shell. A refusal
                        // here (the tab disconnected between render and
                        // click) types nothing, as Vue's guard returns.
                        if let Err(why) = ctx.insert_command(&command, window, cx) {
                            tracing::debug!(?why, "security: command not sent");
                        }
                    }),
            );
        }
        block = block
            .child(
                text_2xs(div())
                    .mt_1()
                    .px_1p5()
                    .py_1()
                    .rounded_sm()
                    .border_1()
                    .border_color(p.line)
                    .bg(p.surface)
                    .font_family(mono.clone())
                    .text_color(p.ink)
                    .child(command),
            )
            .child(actions);
        if can_send {
            block = block.child(
                text_2xs(div())
                    .mt_0p5()
                    .text_color(p.ink_3)
                    .child("Typed at the prompt without running. Press Enter yourself."),
            );
        }
        block.into_any_element()
    }
}

impl Render for SecurityPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = *palette(cx);
        v_flex()
            .size_full()
            .bg(p.canvas)
            .child(self.header(cx))
            .child(
                div()
                    .id("security-body")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(self.body(cx)),
            )
    }
}

/// The score circle, grade, summary and breakdown.
fn score_header(r: &SecurityReport, p: &Palette) -> impl IntoElement {
    let grade = Grade::of(r);
    let (bg, fg) = tone_colors(grade.tone(), p);
    let breakdown = report::breakdown(&r.checks);
    let mut col = v_flex()
        .items_center()
        .child(
            div()
                .size(px(56.))
                .mb_1()
                .rounded_full()
                .flex()
                .items_center()
                .justify_center()
                .bg(bg)
                .text_color(fg)
                .text_lg()
                .font_weight(FontWeight::BOLD)
                .child(report::score_text(r)),
        )
        .child(
            text_2xs(div())
                .text_color(p.ink_2)
                .child(grade.label().to_uppercase()),
        )
        .child(
            text_2xs(div())
                .mt_1()
                .text_color(p.ink_2)
                .child(report::summary(r)),
        );
    if !breakdown.is_empty() {
        col = col.child(
            text_2xs(div())
                .mt_0p5()
                .text_color(p.ink_3)
                .child(breakdown),
        );
    }
    h_flex()
        .justify_center()
        .py_4()
        .border_b_1()
        .border_color(p.line)
        .child(col)
}

/// `(fill, text)` for a tone: the tone at 20% under the tone itself, or the
/// input surface under `ink-2` when there is nothing to grade.
fn tone_colors(tone: Tone, p: &Palette) -> (Hsla, Hsla) {
    match tone {
        Tone::Good => (p.good.opacity(0.2), p.good),
        Tone::Warn => (p.warn.opacity(0.2), p.warn),
        Tone::Bad => (p.bad.opacity(0.2), p.bad),
        Tone::Muted => (p.input, p.ink_2),
    }
}

fn status_icon(status: Status) -> IconName {
    match status {
        Status::Pass => IconName::ShieldCheck,
        Status::Warn => IconName::TriangleAlert,
        Status::Fail => IconName::CircleX,
        Status::Unknown => IconName::Shield,
    }
}
