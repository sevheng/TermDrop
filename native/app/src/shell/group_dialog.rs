//! Creating and renaming a group. Port of `GroupModal.vue`: one name field,
//! checked as it was (required, not taken, and on a rename, different), with
//! Enter submitting.

use std::cell::Cell;

use gpui_kit::component::WindowExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, AppContext as _, Context, Entity, Focusable as _, IntoElement, ParentElement, Render,
    Styled, Subscription, Window, div, px,
};

use crate::app::AppModel;
use crate::app::host_menu::group_name_error;
use crate::ui::theme::palette;
use crate::ui::toast::{ToastKind, toast};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GroupDialogMode {
    Create,
    /// Renaming the group with this name.
    Rename(String),
}

pub struct GroupDialog {
    mode: GroupDialogMode,
    model: Entity<AppModel>,
    input: Entity<InputState>,
    error: Option<&'static str>,
    /// Set once a save succeeded. Enter reaches both the field and the
    /// dialog's Confirm; only the first may close the dialog, or the second
    /// would close whatever is beneath it.
    done: bool,
    _subscription: Subscription,
}

impl GroupDialog {
    fn new(
        mode: GroupDialogMode,
        model: Entity<AppModel>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let (placeholder, initial) = match &mode {
            GroupDialogMode::Create => ("e.g. Production", String::new()),
            GroupDialogMode::Rename(current) => ("New name", current.clone()),
        };
        let input = cx.new(|cx| {
            let s = InputState::new(window, cx).placeholder(placeholder);
            if initial.is_empty() {
                s
            } else {
                s.default_value(initial)
            }
        });
        let subscription = cx.subscribe(&input, |this, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) && this.error.is_some() {
                this.error = None;
                cx.notify();
            }
        });
        Self {
            mode,
            model,
            input,
            error: None,
            done: false,
            _subscription: subscription,
        }
    }

    /// Check and apply. Returns whether the dialog should close.
    fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.done {
            return false;
        }
        let name = self.input.read(cx).value().trim().to_string();
        let current = match &self.mode {
            GroupDialogMode::Create => None,
            GroupDialogMode::Rename(c) => Some(c.clone()),
        };
        let existing: Vec<String> = self
            .model
            .read(cx)
            .group_names()
            .into_iter()
            .filter(|g| Some(g) != current.as_ref())
            .collect();
        if let Some(msg) = group_name_error(&name, &existing, current.as_deref()) {
            self.error = Some(msg);
            cx.notify();
            return false;
        }
        match current {
            None => self.model.update(cx, |m, cx| m.add_custom_group(&name, cx)),
            Some(old) => {
                if let Err(e) = self
                    .model
                    .update(cx, |m, cx| m.rename_group(&old, &name, cx))
                {
                    toast(
                        window,
                        cx,
                        format!("Could not rename the group: {e}"),
                        ToastKind::Error,
                    );
                    return false;
                }
            }
        }
        self.done = true;
        true
    }
}

impl Render for GroupDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = *palette(cx);
        v_flex()
            .gap_1()
            .child(
                h_flex()
                    .gap_1()
                    .text_xs()
                    .text_color(p.ink_2)
                    .child("Group Name")
                    .child(div().text_color(p.bad).child("*")),
            )
            .child(Input::new(&self.input))
            .when_some(self.error, |d, msg| {
                d.child(div().text_xs().text_color(p.bad).child(msg))
            })
    }
}

pub fn open_group_dialog(
    window: &mut Window,
    cx: &mut App,
    model: Entity<AppModel>,
    mode: GroupDialogMode,
) {
    let rename = matches!(mode, GroupDialogMode::Rename(_));
    let view = cx.new(|cx| GroupDialog::new(mode, model, window, cx));
    let input = view.read(cx).input.clone();
    // Enter in the field submits, as `@keydown.enter="onSave"` did.
    {
        let view = view.downgrade();
        window
            .subscribe(&input, cx, move |_, event: &InputEvent, window, cx| {
                if let InputEvent::PressEnter { .. } = event
                    && let Some(view) = view.upgrade()
                    && view.update(cx, |v, cx| v.save(window, cx))
                {
                    window.close_dialog(cx);
                }
            })
            .detach();
    }
    let first_build = Cell::new(true);
    window.open_dialog(cx, move |dialog, window, cx| {
        if first_build.replace(false) {
            input.focus_handle(cx).focus(window, cx);
        }
        let on_save = view.clone();
        let on_ok = view.clone();
        dialog
            .title(if rename { "Rename Group" } else { "New Group" })
            .w(px(340.))
            .child(view.clone())
            .footer(
                h_flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        Button::new("group-cancel")
                            .ghost()
                            .label("Cancel")
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                    .child(
                        Button::new("group-save")
                            .primary()
                            .label(if rename { "Rename" } else { "Create" })
                            .on_click(move |_, window, cx| {
                                if on_save.update(cx, |v, cx| v.save(window, cx)) {
                                    window.close_dialog(cx);
                                }
                            }),
                    ),
            )
            // Enter reaches the dialog as Confirm too; `save` answers only
            // once, so whichever runs second does nothing.
            .on_ok(move |_, window, cx| {
                if on_ok.update(cx, |v, cx| v.save(window, cx)) {
                    window.close_dialog(cx);
                }
                false
            })
    });
}
