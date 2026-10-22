//! Importing hosts from `~/.ssh/config`. Port of `SshConfigImportDialog.vue`:
//! every parsed host with a checkbox, imported in one call, then each
//! `ProxyJump` linked to the saved host it names.
//!
//! One rule is new: a host that is already saved (same name, address and
//! port) starts unticked and says so, so importing the same file twice does
//! not double the list. It can still be ticked.

use gpui_kit::component::WindowExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, AppContext as _, Context, Entity, InteractiveElement as _, IntoElement, ParentElement,
    Render, SharedString, StatefulInteractiveElement as _, Styled, Window, div, px,
};

use crate::app::AppModel;
use crate::app::host_import::{
    SummaryTone, already_saved, import_button_label, link_proxy_jumps, ssh_config_import_host,
    summarize_import, unresolved_jumps_message,
};
use crate::backend::{ImportEntry, SshConfigEntry};
use crate::ui::theme::palette;
use crate::ui::toast::{ToastKind, toast};

pub fn summary_toast(tone: SummaryTone) -> ToastKind {
    match tone {
        SummaryTone::Info => ToastKind::Info,
        SummaryTone::Success => ToastKind::Success,
        SummaryTone::Warning => ToastKind::Warning,
    }
}

pub struct SshImportDialog {
    model: Entity<AppModel>,
    entries: Vec<SshConfigEntry>,
    saved: Vec<bool>,
    selected: Vec<bool>,
    done: bool,
}

impl SshImportDialog {
    fn new(model: Entity<AppModel>, entries: Vec<SshConfigEntry>, cx: &App) -> Self {
        let hosts = model.read(cx).hosts().to_vec();
        let saved: Vec<bool> = entries.iter().map(|e| already_saved(e, &hosts)).collect();
        let selected = saved.iter().map(|s| !s).collect();
        Self {
            model,
            entries,
            saved,
            selected,
            done: false,
        }
    }

    fn selected_count(&self) -> usize {
        self.selected.iter().filter(|s| **s).count()
    }

    /// Import the ticked hosts. Returns whether the dialog should close.
    fn confirm(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.done {
            return false;
        }
        let picked: Vec<SshConfigEntry> = self
            .entries
            .iter()
            .zip(&self.selected)
            .filter(|(_, s)| **s)
            .map(|(e, _)| e.clone())
            .collect();
        if picked.is_empty() {
            self.done = true;
            return true;
        }
        let skipped = self
            .saved
            .iter()
            .zip(&self.selected)
            .filter(|(saved, sel)| **saved && !**sel)
            .count();
        let entries = picked
            .iter()
            .map(|e| ImportEntry {
                host: ssh_config_import_host(e),
                replace_id: None,
            })
            .collect();
        let summary = match self.model.update(cx, |m, cx| m.import_hosts(entries, cx)) {
            Ok(summary) => summary,
            Err(e) => {
                toast(window, cx, format!("Import failed: {e}"), ToastKind::Error);
                return false;
            }
        };
        self.done = true;
        let (message, tone) = summarize_import(&summary, skipped);
        toast(window, cx, message, summary_toast(tone));
        self.link_jump_hosts(&picked, window, cx);
        true
    }

    /// Point imported hosts at their `ProxyJump` host, now that the hosts a
    /// hop can name have ids. Anything that cannot be matched safely is
    /// listed rather than guessed.
    fn link_jump_hosts(
        &mut self,
        imported: &[SshConfigEntry],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !imported.iter().any(|e| e.proxy_jump.is_some()) {
            return;
        }
        let (links, mut unresolved) = link_proxy_jumps(imported, self.model.read(cx).hosts());
        if !links.is_empty() {
            self.model.update(cx, |m, cx| {
                for link in &links {
                    if let Err(e) = m.set_jump_host(link.host_id, link.jump_id) {
                        let name = m
                            .host(link.host_id)
                            .map(|h| h.name.clone())
                            .unwrap_or_default();
                        unresolved.push((name, e));
                    }
                }
                m.reload_hosts(cx);
            });
        }
        if let Some(message) = unresolved_jumps_message(&unresolved) {
            toast(window, cx, message, ToastKind::Warning);
        }
    }
}

impl Render for SshImportDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = *palette(cx);
        let rows = self.entries.iter().enumerate().map(|(i, e)| {
            let subtitle: SharedString =
                format!("{}@{}:{} · {}", e.username, e.host, e.port, e.auth_type).into();
            h_flex()
                .gap_2()
                .px_2()
                .py_1p5()
                .rounded_sm()
                .hover(|s| s.bg(p.raised))
                .child(
                    Checkbox::new(SharedString::from(format!("ssh-import-{i}")))
                        .checked(self.selected[i])
                        .accessibility_label(e.name.clone())
                        .on_click(cx.listener(move |this, checked: &bool, _, cx| {
                            this.selected[i] = *checked;
                            cx.notify();
                        })),
                )
                .child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .child(
                            div()
                                .text_xs()
                                .text_color(p.ink)
                                .truncate()
                                .child(e.name.clone()),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(p.ink_2)
                                .truncate()
                                .child(subtitle),
                        ),
                )
                .when(self.saved[i], |row| {
                    row.child(
                        div()
                            .flex_none()
                            .text_xs()
                            .text_color(p.ink_3)
                            .child("Already saved"),
                    )
                })
        });
        // The buttons live in the view rather than the dialog's footer: the
        // label counts the ticked hosts, and the footer is only rebuilt when
        // the dialog layer is, not when a box is ticked.
        let count = self.selected_count();
        v_flex()
            .gap_3()
            .child(
                div()
                    .id("ssh-import-list")
                    .max_h(px(360.))
                    .overflow_y_scroll()
                    .child(v_flex().gap_0p5().children(rows)),
            )
            .child(
                h_flex()
                    .justify_end()
                    .gap_2()
                    .pt_3()
                    .border_t_1()
                    .border_color(p.line)
                    .child(
                        Button::new("ssh-import-cancel")
                            .ghost()
                            .label("Cancel")
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                    .child(
                        Button::new("ssh-import-confirm")
                            .primary()
                            .label(import_button_label(count))
                            .on_click(cx.listener(|this, _, window, cx| {
                                if this.confirm(window, cx) {
                                    window.close_dialog(cx);
                                }
                            })),
                    ),
            )
    }
}

/// Parse `~/.ssh/config` and, when it has hosts, offer them.
pub fn open_ssh_import(window: &mut Window, cx: &mut App, model: Entity<AppModel>) {
    let entries = match model.read(cx).parse_ssh_config() {
        Ok(entries) => entries,
        Err(e) => {
            toast(
                window,
                cx,
                format!("Failed to parse SSH config: {e}"),
                ToastKind::Error,
            );
            return;
        }
    };
    if entries.is_empty() {
        toast(
            window,
            cx,
            "No hosts found in ~/.ssh/config",
            ToastKind::Warning,
        );
        return;
    }
    let view = cx.new(|cx| SshImportDialog::new(model, entries, cx));
    window.open_dialog(cx, move |dialog, _, _| {
        let on_ok = view.clone();
        dialog
            .title("Import from ~/.ssh/config")
            .w(px(440.))
            .child(view.clone())
            .on_ok(move |_, window, cx| {
                if on_ok.update(cx, |v, cx| v.confirm(window, cx)) {
                    window.close_dialog(cx);
                }
                false
            })
    });
}
