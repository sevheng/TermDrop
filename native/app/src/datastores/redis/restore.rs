//! Confirming a restore: port of `RedisRestoreDialog.vue`.
//!
//! Shows what the file says about itself before anything is written,
//! because "which backup is this and where is it about to go" is the
//! question the user has at this moment. Emptying the database first is
//! opt-in, said in the bad tone, and turns the button into "Empty and
//! restore".

use std::rc::Rc;

use gpui_kit::component::WindowExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::select::{Select, SelectEvent, SelectState};
use gpui_kit::component::{IndexPath, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, AppContext as _, Context, Entity, IntoElement, ParentElement, Render, SharedString,
    Styled, Subscription, Window, div, px,
};
use termdrop_core::redis_backup::BackupHeader;

use super::logic::RESTORE_DB_COUNT;
use crate::panels::common::text_2xs;
use crate::ui::choice::{Choice, index_of};
use crate::ui::theme::{mono_font, palette};

/// What the user chose, handed back on confirm.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestoreChoice {
    pub path: String,
    pub target_db: i64,
    pub replace: bool,
    pub flush_first: bool,
}

type DbSelect = Entity<SelectState<Vec<Choice<i64>>>>;
type OnConfirm = Rc<dyn Fn(RestoreChoice, &mut Window, &mut App)>;

pub struct RestoreDialog {
    path: String,
    header: BackupHeader,
    warning: Option<String>,
    target_db: i64,
    replace: bool,
    flush_first: bool,
    db_select: DbSelect,
    on_confirm: OnConfirm,
    answered: bool,
    _subscription: Subscription,
}

impl RestoreDialog {
    fn new(
        path: String,
        header: BackupHeader,
        warning: Option<String>,
        target_db: i64,
        on_confirm: OnConfirm,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let choices: Vec<Choice<i64>> = (0..RESTORE_DB_COUNT)
            .map(|i| Choice::new(i, format!("db{i}")))
            .collect();
        let ix = index_of(&choices, &target_db);
        let db_select = cx.new(|cx| SelectState::new(choices, ix.map(IndexPath::new), window, cx));
        let subscription = cx.subscribe(
            &db_select,
            |this, _, event: &SelectEvent<Vec<Choice<i64>>>, cx| {
                if let SelectEvent::Confirm(Some(db)) = event {
                    this.target_db = *db;
                    cx.notify();
                }
            },
        );
        Self {
            path,
            header,
            warning,
            target_db,
            replace: false,
            flush_first: false,
            db_select,
            on_confirm,
            answered: false,
            _subscription: subscription,
        }
    }

    /// Restore was pressed: close, then hand the choice over, once.
    fn confirm(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if std::mem::replace(&mut self.answered, true) {
            return;
        }
        let choice = RestoreChoice {
            path: self.path.clone(),
            target_db: self.target_db,
            replace: self.replace,
            flush_first: self.flush_first,
        };
        let on_confirm = self.on_confirm.clone();
        window.close_dialog(cx);
        on_confirm(choice, window, cx);
    }
}

impl Render for RestoreDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = *palette(cx);
        let mono = mono_font(cx);
        let fact = |label: &'static str, value: String, mono: Option<SharedString>| {
            let mut v = div().text_color(p.ink).min_w_0().truncate().child(value);
            if let Some(mono) = mono {
                v = v.font_family(mono);
            }
            h_flex()
                .justify_between()
                .gap_3()
                .child(div().flex_none().text_color(p.ink_2).child(label))
                .child(v)
        };
        let ok = Button::new("redis-restore-ok").label(if self.flush_first {
            "Empty and restore"
        } else {
            "Restore"
        });
        let ok = if self.flush_first {
            ok.danger()
        } else {
            ok.primary()
        };
        v_flex()
            .gap_3()
            .child(
                div()
                    .text_xs()
                    .text_color(p.ink_2)
                    .child(SharedString::from(self.path.clone())),
            )
            .child(
                v_flex()
                    .gap_1()
                    .p_3()
                    .rounded_sm()
                    .bg(p.canvas)
                    .text_xs()
                    .child(fact("Taken from", self.header.source.clone(), None))
                    .child(fact(
                        "Redis version",
                        self.header.redis_version.clone(),
                        None,
                    ))
                    .child(fact("Created", self.header.created_at.clone(), None))
                    .when_some(self.header.pattern.clone(), |col, pattern| {
                        col.child(fact("Pattern", pattern, Some(mono.clone())))
                    }),
            )
            .when_some(self.warning.clone(), |col, warning| {
                col.child(div().text_xs().text_color(p.warn).child(warning))
            })
            .child(
                v_flex()
                    .gap_1()
                    .child(
                        div()
                            .text_xs()
                            .text_color(p.ink_2)
                            .child("Restore into database"),
                    )
                    .child(Select::new(&self.db_select).w_full()),
            )
            .child(
                v_flex()
                    .gap_0p5()
                    .child(
                        Checkbox::new("redis-restore-replace")
                            .label("Overwrite keys that already exist")
                            .checked(self.replace)
                            .on_click(cx.listener(|this, checked: &bool, _, cx| {
                                this.replace = *checked;
                                cx.notify();
                            })),
                    )
                    .child(text_2xs(div().pl(px(24.)).text_color(p.ink_2)).child(
                        "Without this, a key that is already there is left alone and counted \
                         as skipped.",
                    )),
            )
            .child(
                v_flex()
                    .gap_0p5()
                    .child(
                        Checkbox::new("redis-restore-flush")
                            .label("Empty the database first")
                            .checked(self.flush_first)
                            .on_click(cx.listener(|this, checked: &bool, _, cx| {
                                this.flush_first = *checked;
                                cx.notify();
                            })),
                    )
                    .child(text_2xs(div().pl(px(24.)).text_color(p.bad)).child(format!(
                        "Deletes every key in db{} before restoring. This cannot be undone.",
                        self.target_db
                    ))),
            )
            // In the view rather than the dialog's footer, so the button
            // follows the flush box as it is ticked.
            .child(
                h_flex()
                    .pt_2()
                    .justify_end()
                    .gap_2()
                    .child(
                        Button::new("redis-restore-cancel")
                            .ghost()
                            .label("Cancel")
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                    .child(
                        ok.on_click(cx.listener(|this, _, window, cx| this.confirm(window, cx))),
                    ),
            )
    }
}

/// Open the dialog. `on_confirm` runs once, on Restore; Cancel, Escape and
/// the backdrop only close it.
#[allow(clippy::too_many_arguments)]
pub fn open_restore_dialog(
    window: &mut Window,
    cx: &mut App,
    connection_name: String,
    path: String,
    header: BackupHeader,
    warning: Option<String>,
    target_db: i64,
    on_confirm: impl Fn(RestoreChoice, &mut Window, &mut App) + 'static,
) {
    let on_confirm: OnConfirm = Rc::new(on_confirm);
    let view =
        cx.new(|cx| RestoreDialog::new(path, header, warning, target_db, on_confirm, window, cx));
    window.open_dialog(cx, move |dialog, _, _| {
        dialog
            .title(format!("Restore into {connection_name}"))
            .w(px(512.))
            .child(view.clone())
    });
}
