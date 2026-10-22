//! The confirm and prompt dialogs, over GPUI Kit's `Dialog`. Ports of
//! `ConfirmDialog.vue` / `useConfirmDialog` and `PromptDialog.vue`.
//!
//! Both are plain functions that open a dialog and call back once with the
//! answer. Every way out of a dialog answers exactly once: the buttons, Enter
//! in the prompt, and Escape / the backdrop / the close icon, which count as
//! cancel. The Vue prompt answered `null` on cancel through a window event;
//! here it is `None` to the callback.

use std::cell::Cell;
use std::rc::Rc;

use gpui_kit::component::WindowExt as _;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::{
    App, AppContext as _, Focusable as _, ParentElement, SharedString, Styled, Window, div,
};

use super::theme::palette;

#[derive(Debug, Clone)]
pub struct ConfirmOptions {
    pub title: SharedString,
    pub message: SharedString,
    pub confirm_label: SharedString,
    /// A destructive confirm takes the `bad-solid` fill.
    pub danger: bool,
}

impl ConfirmOptions {
    pub fn new(title: impl Into<SharedString>, message: impl Into<SharedString>) -> Self {
        Self {
            title: title.into(),
            message: message.into(),
            confirm_label: "Confirm".into(),
            danger: false,
        }
    }

    pub fn confirm_label(mut self, label: impl Into<SharedString>) -> Self {
        self.confirm_label = label.into();
        self
    }

    pub fn danger(mut self) -> Self {
        self.danger = true;
        self
    }
}

/// Ask, then call `on_answer(true)` on confirm and `on_answer(false)` on any
/// other way out.
pub fn confirm(
    window: &mut Window,
    cx: &mut App,
    options: ConfirmOptions,
    on_answer: impl Fn(bool, &mut Window, &mut App) + 'static,
) {
    let answered = Rc::new(Cell::new(false));
    let on_answer = Rc::new(on_answer);
    window.open_dialog(cx, move |dialog, _, cx| {
        let p = *palette(cx);
        let answer = {
            let answered = answered.clone();
            let on_answer = on_answer.clone();
            move |yes: bool, window: &mut Window, cx: &mut App| {
                if answered.replace(true) {
                    return;
                }
                window.close_dialog(cx);
                on_answer(yes, window, cx);
            }
        };
        let cancel = answer.clone();
        let ok = answer.clone();
        let mut ok_button = Button::new("confirm-ok").label(options.confirm_label.clone());
        ok_button = if options.danger {
            ok_button.danger()
        } else {
            ok_button.primary()
        };
        dialog
            .title(options.title.clone())
            .w(gpui_kit::px(420.))
            .child(
                div()
                    .text_sm()
                    .text_color(p.ink_2)
                    .child(options.message.clone()),
            )
            .footer(
                h_flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        Button::new("confirm-cancel")
                            .ghost()
                            .label("Cancel")
                            .on_click(move |_, w, cx| cancel(false, w, cx)),
                    )
                    .child(ok_button.on_click(move |_, w, cx| ok(true, w, cx))),
            )
            .on_close(move |_, w, cx| answer(false, w, cx))
    });
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PromptKind {
    #[default]
    Text,
    /// Masked input. Used for the keyring-miss password retry and, later, key
    /// passphrases.
    Password,
}

#[derive(Debug, Clone)]
pub struct PromptOptions {
    pub title: SharedString,
    pub message: SharedString,
    pub placeholder: SharedString,
    pub kind: PromptKind,
    pub confirm_label: SharedString,
    pub initial: SharedString,
}

impl PromptOptions {
    pub fn new(title: impl Into<SharedString>) -> Self {
        Self {
            title: title.into(),
            message: SharedString::default(),
            placeholder: SharedString::default(),
            kind: PromptKind::Text,
            confirm_label: "OK".into(),
            initial: SharedString::default(),
        }
    }

    pub fn message(mut self, message: impl Into<SharedString>) -> Self {
        self.message = message.into();
        self
    }

    pub fn placeholder(mut self, placeholder: impl Into<SharedString>) -> Self {
        self.placeholder = placeholder.into();
        self
    }

    pub fn password(mut self) -> Self {
        self.kind = PromptKind::Password;
        self
    }

    pub fn confirm_label(mut self, label: impl Into<SharedString>) -> Self {
        self.confirm_label = label.into();
        self
    }

    pub fn initial(mut self, value: impl Into<SharedString>) -> Self {
        self.initial = value.into();
        self
    }
}

/// Ask for a line of text. `on_answer(Some(text))` on OK or Enter,
/// `on_answer(None)` on cancel. An empty answer is still `Some("")`: whether
/// that is acceptable is the caller's decision, as it was in the Vue app.
pub fn prompt(
    window: &mut Window,
    cx: &mut App,
    options: PromptOptions,
    on_answer: impl Fn(Option<String>, &mut Window, &mut App) + 'static,
) {
    // Created before opening: the builder can run again on every rebuild of
    // the dialog layer, and an input made inside it would lose its text.
    let input = cx.new(|cx| {
        let mut state = InputState::new(window, cx)
            .placeholder(options.placeholder.clone())
            .masked(options.kind == PromptKind::Password);
        if !options.initial.is_empty() {
            state = state.default_value(options.initial.clone());
        }
        state
    });
    let answered = Rc::new(Cell::new(false));
    let on_answer = Rc::new(on_answer);
    let answer = {
        let answered = answered.clone();
        let on_answer = on_answer.clone();
        let input = input.downgrade();
        move |submit: bool, window: &mut Window, cx: &mut App| {
            if answered.replace(true) {
                return;
            }
            let value = submit
                .then(|| input.upgrade().map(|i| i.read(cx).value().to_string()))
                .flatten();
            window.close_dialog(cx);
            on_answer(value, window, cx);
        }
    };

    let on_enter = answer.clone();
    window
        .subscribe(&input, cx, move |_, event: &InputEvent, window, cx| {
            if let InputEvent::PressEnter { .. } = event {
                on_enter(true, window, cx);
            }
        })
        .detach();

    let first_build = Cell::new(true);
    window.open_dialog(cx, move |dialog, window, cx| {
        let p = *palette(cx);
        // Focus the field once, on the first build only: a rebuild must not
        // steal focus back from wherever the user moved it.
        if first_build.replace(false) {
            input.focus_handle(cx).focus(window, cx);
        }
        let cancel = answer.clone();
        let ok = answer.clone();
        let close = answer.clone();
        dialog
            .title(options.title.clone())
            .w(gpui_kit::px(420.))
            .child(
                v_flex()
                    .gap_3()
                    .when_message(&options.message, p.ink_2)
                    .child(Input::new(&input)),
            )
            .footer(
                h_flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        Button::new("prompt-cancel")
                            .ghost()
                            .label("Cancel")
                            .on_click(move |_, w, cx| cancel(false, w, cx)),
                    )
                    .child(
                        Button::new("prompt-ok")
                            .primary()
                            .label(options.confirm_label.clone())
                            .on_click(move |_, w, cx| ok(true, w, cx)),
                    ),
            )
            .on_close(move |_, w, cx| close(false, w, cx))
    });
}

/// A message line above a dialog's body, when there is one.
trait WhenMessage: ParentElement + Sized {
    fn when_message(self, message: &SharedString, color: gpui_kit::Hsla) -> Self;
}

impl<E: ParentElement + Sized> WhenMessage for E {
    fn when_message(self, message: &SharedString, color: gpui_kit::Hsla) -> Self {
        if message.is_empty() {
            self
        } else {
            self.child(div().text_sm().text_color(color).child(message.clone()))
        }
    }
}
