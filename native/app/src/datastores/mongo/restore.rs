//! The restore confirmation: the one destructive thing the tab does. Port of
//! `MongoRestoreDialog.vue`: what was found, what will be written, and
//! whether the target collections are dropped first (opt-in, no undo).

use std::rc::Rc;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::{WindowExt as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, AppContext as _, Context, FontWeight, InteractiveElement as _, IntoElement, ParentElement,
    Render, SharedString, StatefulInteractiveElement as _, Styled, Window, div, px,
};

use super::selection::{Entry, RestoreScope, restore_scope};
use crate::ui::theme::{Palette, mono_font, palette};

/// What a restore will read and write.
#[derive(Debug, Clone)]
pub struct RestoreRequest {
    pub input_path: String,
    pub is_archive: bool,
    /// The ticked collections (the restore's filter).
    pub entries: Vec<Entry>,
    /// What `scan_restore_folder` found; empty for an archive.
    pub source: Vec<(String, Vec<String>)>,
}

impl RestoreRequest {
    pub fn source_dbs(&self) -> Vec<String> {
        self.source.iter().map(|(d, _)| d.clone()).collect()
    }
}

type OnConfirm = Rc<dyn Fn(RestoreRequest, bool, &mut Window, &mut App)>;

/// Ask before restoring. `on_confirm(request, drop_first)` runs only on
/// Restore; every other way out does nothing.
pub fn confirm_restore(
    window: &mut Window,
    cx: &mut App,
    connection_name: String,
    request: RestoreRequest,
    on_confirm: impl Fn(RestoreRequest, bool, &mut Window, &mut App) + 'static,
) {
    let body = cx.new(|_| RestoreBody {
        request,
        drop_first: false,
        answered: false,
        on_confirm: Rc::new(on_confirm),
    });
    window.open_dialog(cx, move |dialog, _, cx| {
        let p = *palette(cx);
        dialog
            .w(px(460.))
            .title(
                h_flex().gap_1().child("Confirm restore into").child(
                    div()
                        .text_color(p.accent)
                        .child(SharedString::from(connection_name.clone())),
                ),
            )
            .child(body.clone())
    });
}

struct RestoreBody {
    request: RestoreRequest,
    drop_first: bool,
    answered: bool,
    on_confirm: OnConfirm,
}

impl RestoreBody {
    fn answer(&mut self, yes: bool, window: &mut Window, cx: &mut Context<Self>) {
        if std::mem::replace(&mut self.answered, true) {
            return;
        }
        window.close_dialog(cx);
        if yes {
            (self.on_confirm)(self.request.clone(), self.drop_first, window, cx);
        }
    }
}

fn listing(id: &'static str, rows: Vec<(String, String)>, p: &Palette) -> impl IntoElement {
    v_flex()
        .id(id)
        .max_h(px(128.))
        .overflow_y_scroll()
        .bg(p.canvas)
        .rounded_sm()
        .p_2()
        .gap_0p5()
        .text_xs()
        .children(rows.into_iter().map(|(db, colls)| {
            h_flex()
                .flex_wrap()
                .child(div().text_color(p.accent).child(SharedString::from(db)))
                .child(
                    div()
                        .text_color(p.ink)
                        .child(SharedString::from(format!(": {colls}"))),
                )
        }))
}

impl Render for RestoreBody {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = *palette(cx);
        let mono = mono_font(cx);
        let r = &self.request;
        let source_dbs = r.source_dbs();
        let scope = restore_scope(&r.entries, r.is_archive, &source_dbs);
        let quiet = |text: &str| {
            div()
                .text_color(p.ink_2)
                .child(SharedString::from(text.to_string()))
        };
        let mono_accent = |text: String| {
            div()
                .font_family(mono.clone())
                .text_color(p.accent)
                .child(SharedString::from(text))
        };

        let mut body = v_flex().gap_2().text_sm().text_color(p.ink).child(
            v_flex()
                .child(format!(
                    "Source {}:",
                    if r.is_archive { "archive" } else { "folder" }
                ))
                .child(
                    div()
                        .font_family(mono.clone())
                        .text_color(p.good)
                        .child(SharedString::from(r.input_path.clone())),
                ),
        );
        if !r.is_archive && !r.source.is_empty() {
            body = body.child(quiet("Data found in folder:")).child(listing(
                "mongo-restore-found",
                r.source
                    .iter()
                    .map(|(d, c)| (d.clone(), c.join(", ")))
                    .collect(),
                &p,
            ));
        }
        body =
            match scope {
                RestoreScope::Selected => body
                    .child(
                        h_flex()
                            .gap_1()
                            .flex_wrap()
                            .child(format!(
                                "Target filter database{}:",
                                if r.entries.len() > 1 { "s" } else { "" }
                            ))
                            .child(mono_accent(
                                r.entries
                                    .iter()
                                    .map(|e| e.db.clone())
                                    .collect::<Vec<_>>()
                                    .join(", "),
                            )),
                    )
                    .child(quiet("Collections to restore:"))
                    .child(listing(
                        "mongo-restore-picked",
                        r.entries
                            .iter()
                            .map(|e| (e.db.clone(), e.collections.join(", ")))
                            .collect(),
                        &p,
                    )),
                RestoreScope::OneDb(name) => body.child(
                    h_flex()
                        .gap_1()
                        .flex_wrap()
                        .text_color(p.accent)
                        .child("Will restore database")
                        .child(mono_accent(name))
                        .child("(all collections found in the folder)."),
                ),
                RestoreScope::AllDbs(names) => body.child(
                    h_flex()
                        .gap_1()
                        .flex_wrap()
                        .text_color(p.accent)
                        .child("Will restore all databases found in the folder:")
                        .child(mono_accent(names.join(", "))),
                ),
                RestoreScope::Everything => body.child(div().text_color(p.accent).child(
                    "No database selected \u{2014} everything in the source will be restored.",
                )),
            };

        let drop_first = self.drop_first;
        body.child(
            Checkbox::new("mongo-restore-drop")
                .checked(drop_first)
                .label("Drop existing collections in the target first")
                .on_click(cx.listener(|this, checked: &bool, _, cx| {
                    this.drop_first = *checked;
                    cx.notify();
                })),
        )
        .child(if drop_first {
            div().text_xs().text_color(p.bad).child(
                "Existing collections in the target database will be dropped before restoring. \
                 This cannot be undone.",
            )
        } else {
            div().text_xs().text_color(p.ink_2).child(
                "Existing documents are kept. Documents whose _id already exists will be \
                 reported as failures, not overwritten.",
            )
        })
        .child(
            h_flex()
                .justify_end()
                .gap_2()
                .pt_3()
                .child(
                    Button::new("mongo-restore-cancel")
                        .ghost()
                        .label("Cancel")
                        .on_click(
                            cx.listener(|this, _, window, cx| this.answer(false, window, cx)),
                        ),
                )
                .child(
                    Button::new("mongo-restore-ok")
                        .label("Restore")
                        .font_weight(FontWeight::MEDIUM)
                        .when(drop_first, |b| b.danger())
                        .when(!drop_first, |b| b.primary())
                        .on_click(cx.listener(|this, _, window, cx| this.answer(true, window, cx))),
                ),
        )
    }
}
