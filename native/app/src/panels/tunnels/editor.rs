//! The Add / Edit Port Forward dialog: `PortForwardModal.vue` plus the
//! save that `MainWindow.vue`'s `onForwardSaved` did around it.
//!
//! The dialog saves for itself rather than handing the row to a parent:
//! there is no parent component here. A save that succeeds toasts, tells
//! the open Tunnels panels through [`super::ForwardBus`] (the Vue
//! `port-forward-added` event) and closes; one that fails toasts "Failed to
//! save: ..." and stays open with what was typed, as the Vue modal did.

use std::cell::Cell;
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{Disableable as _, Sizable as _, WindowExt as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, Entity, Focusable as _, HighlightStyle, IntoElement,
    ParentElement, Render, SharedString, Styled, StyledText, Subscription, Task, Window, div, px,
};
use termdrop_core::db::PortForward;

use super::logic::{
    Field, ForwardForm, ForwardKind, Prefill, check_field, dialog_title, save_label, saved_message,
    socks_hint, validate_all,
};
use crate::core_backend::Services;
use crate::ui::theme::{Palette, palette};
use crate::ui::toast::{ToastKind, toast};

/// Which forward the dialog is for.
#[derive(Debug, Clone)]
pub enum EditorMode {
    /// A new forward, blank or prefilled (the terminal's "Forward <port>").
    Add(Option<Prefill>),
    /// A saved forward, from the panel's Edit button.
    Edit(PortForward),
}

/// The text inputs, one per field.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum In {
    Name,
    LocalHost,
    LocalPort,
    RemoteHost,
    RemotePort,
}

impl In {
    /// The field a blur validates (`@blur="validateField(...)"`); the local
    /// host has no check.
    fn field(self) -> Option<Field> {
        match self {
            Self::Name => Some(Field::Name),
            Self::LocalHost => None,
            Self::LocalPort => Some(Field::LocalPort),
            Self::RemoteHost => Some(Field::RemoteHost),
            Self::RemotePort => Some(Field::RemotePort),
        }
    }
}

struct ForwardEditor {
    services: Arc<Services>,
    host_id: i64,
    /// The rule being edited; `None` when adding.
    edit_id: Option<i64>,
    form: ForwardForm,
    errors: BTreeMap<Field, &'static str>,
    inputs: HashMap<In, Entity<InputState>>,
    saving: bool,
    task: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl ForwardEditor {
    fn new(
        services: Arc<Services>,
        host_id: i64,
        mode: EditorMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let (form, edit_id) = match &mode {
            EditorMode::Add(None) => (ForwardForm::default(), None),
            EditorMode::Add(Some(p)) => (ForwardForm::from_prefill(p), None),
            EditorMode::Edit(f) => (ForwardForm::from_forward(f), Some(f.id)),
        };
        let specs = [
            (In::Name, form.name.clone(), "MySQL Tunnel"),
            (In::LocalHost, form.local_host.clone(), ""),
            (In::LocalPort, form.local_port.clone(), "13306"),
            (In::RemoteHost, form.remote_host.clone(), "localhost"),
            (In::RemotePort, form.remote_port.clone(), "3306"),
        ];
        let mut inputs = HashMap::new();
        let mut subscriptions = Vec::new();
        for (which, value, placeholder) in specs {
            let input = cx.new(|cx| {
                let mut s = InputState::new(window, cx).placeholder(placeholder);
                if !value.is_empty() {
                    s = s.default_value(value);
                }
                s
            });
            subscriptions.push(cx.subscribe_in(
                &input,
                window,
                move |this: &mut Self, input, event: &InputEvent, _, cx| match event {
                    InputEvent::Change => {
                        let value = input.read(cx).value().to_string();
                        this.set_value(which, value);
                        cx.notify();
                    }
                    InputEvent::Blur => {
                        if let Some(field) = which.field() {
                            check_field(&this.form, field, &mut this.errors);
                            cx.notify();
                        }
                    }
                    _ => {}
                },
            ));
            inputs.insert(which, input);
        }
        Self {
            services,
            host_id,
            edit_id,
            form,
            errors: BTreeMap::new(),
            inputs,
            saving: false,
            task: None,
            _subscriptions: subscriptions,
        }
    }

    fn editing(&self) -> bool {
        self.edit_id.is_some()
    }

    fn set_value(&mut self, which: In, value: String) {
        let slot = match which {
            In::Name => &mut self.form.name,
            In::LocalHost => &mut self.form.local_host,
            In::LocalPort => &mut self.form.local_port,
            In::RemoteHost => &mut self.form.remote_host,
            In::RemotePort => &mut self.form.remote_port,
        };
        *slot = value;
    }

    fn set_kind(&mut self, kind: ForwardKind, cx: &mut Context<Self>) {
        self.form.kind = kind;
        cx.notify();
    }

    /// `onSave` and `onForwardSaved`: validate, write the row off the UI
    /// thread, then toast and close, or toast and stay open.
    fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.saving {
            return;
        }
        let valid = validate_all(&self.form, &mut self.errors);
        cx.notify();
        if !valid {
            return;
        }
        self.saving = true;
        let row = self.form.to_new(self.host_id);
        let host_id = self.host_id;
        let edit_id = self.edit_id;
        let editing = self.editing();
        let core = self.services.core.clone();
        let job = self.services.rt.spawn_blocking(move || match edit_id {
            Some(id) => core.update_port_forward(id, &row),
            None => core.add_port_forward(&row).map(|_| ()),
        });
        self.task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = job.await.unwrap_or_else(|e| Err(e.to_string()));
            let _ = cx.update(|window, cx| {
                let _ = this.update(cx, |this, cx| {
                    this.saving = false;
                    cx.notify();
                });
                match result {
                    Ok(()) => {
                        toast(window, cx, saved_message(editing), ToastKind::Success);
                        super::notify_saved(host_id, cx);
                        window.close_dialog(cx);
                    }
                    Err(e) => toast(window, cx, format!("Failed to save: {e}"), ToastKind::Error),
                }
            });
        }));
    }

    // ---- layout ----------------------------------------------------------

    fn input(&self, which: In) -> Input {
        Input::new(&self.inputs[&which])
    }

    /// A labelled field: the label (with a red star when required), the
    /// control, then its error.
    fn field(&self, label: &'static str, required: bool, which: In, p: &Palette) -> AnyElement {
        let error = which.field().and_then(|f| self.errors.get(&f).copied());
        v_flex()
            .gap_1p5()
            .min_w_0()
            .child(
                h_flex()
                    .gap_1()
                    .text_xs()
                    .text_color(p.ink_2)
                    .child(label)
                    .when(required, |d| d.child(div().text_color(p.bad).child("*"))),
            )
            .child(self.input(which))
            .when_some(error, |d, msg| {
                d.child(div().text_xs().text_color(p.bad).child(msg))
            })
            .into_any_element()
    }

    fn kind_toggle(&self, p: &Palette, cx: &mut Context<Self>) -> AnyElement {
        let button = |id: &'static str, label: &'static str, kind: ForwardKind| {
            let b = Button::new(id).small().label(label).flex_1();
            let b = if self.form.kind == kind {
                b.primary()
            } else {
                b.ghost()
            };
            b.on_click(cx.listener(move |this, _, _, cx| this.set_kind(kind, cx)))
        };
        v_flex()
            .gap_1p5()
            .child(div().text_xs().text_color(p.ink_2).child("Type"))
            .child(
                h_flex()
                    .gap_1()
                    .p_1()
                    .rounded_sm()
                    .bg(p.input)
                    .child(button("forward-kind-local", "Local", ForwardKind::Local))
                    .child(button(
                        "forward-kind-dynamic",
                        "Dynamic (SOCKS)",
                        ForwardKind::Dynamic,
                    )),
            )
            .into_any_element()
    }

    /// One run of text, so it wraps as a sentence; the address carries the
    /// code chip's fill (Vue's `<code class="bg-input">`).
    fn socks_hint(&self, p: &Palette) -> AnyElement {
        let (text, address) = socks_hint(&self.form);
        div()
            .text_xs()
            .text_color(p.ink_2)
            .child(StyledText::new(text).with_highlights([(
                address,
                HighlightStyle {
                    color: Some(p.ink),
                    background_color: Some(p.input),
                    ..Default::default()
                },
            )]))
            .into_any_element()
    }
}

impl Render for ForwardEditor {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = *palette(cx);
        let local = self.form.kind == ForwardKind::Local;
        let address_row = h_flex()
            .gap_3()
            .items_start()
            .child(div().w_2_3().min_w_0().child(self.field(
                "Local Host",
                false,
                In::LocalHost,
                &p,
            )))
            .child(div().w_1_3().min_w_0().child(self.field(
                "Local Port",
                true,
                In::LocalPort,
                &p,
            )));
        let remote_row = h_flex()
            .gap_3()
            .items_start()
            .child(div().w_2_3().min_w_0().child(self.field(
                "Remote Host",
                true,
                In::RemoteHost,
                &p,
            )))
            .child(div().w_1_3().min_w_0().child(self.field(
                "Remote Port",
                true,
                In::RemotePort,
                &p,
            )));
        let saving = self.saving;
        let editing = self.editing();

        v_flex()
            .gap_4()
            .child(self.field("Name", true, In::Name, &p))
            .child(self.kind_toggle(&p, cx))
            .child(address_row)
            .map(|d| {
                if local {
                    d.child(remote_row)
                } else {
                    d.child(self.socks_hint(&p))
                }
            })
            .child(
                h_flex()
                    .justify_end()
                    .gap_2()
                    .mt_2()
                    .child(
                        Button::new("forward-cancel")
                            .ghost()
                            .label("Cancel")
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                    .child(
                        Button::new("forward-save")
                            .primary()
                            .label(save_label(saving, editing))
                            .loading(saving)
                            .disabled(saving)
                            .on_click(cx.listener(|this, _, window, cx| this.save(window, cx))),
                    ),
            )
    }
}

/// Open the dialog for `host_id`. The panel's Add and Edit buttons and the
/// terminal's "Forward <port>" entry all come here.
pub fn open_editor(
    window: &mut Window,
    cx: &mut App,
    services: Arc<Services>,
    host_id: i64,
    mode: EditorMode,
) {
    let editing = matches!(mode, EditorMode::Edit(_));
    let editor = cx.new(|cx| ForwardEditor::new(services, host_id, mode, window, cx));
    let first_build = Cell::new(true);
    window.open_dialog(cx, move |dialog, window, cx| {
        // Focus the name once, on the first build only: a rebuild must not
        // pull focus back from wherever the user moved it.
        if first_build.replace(false) {
            let name = editor.read(cx).inputs.get(&In::Name).cloned();
            if let Some(name) = name {
                name.focus_handle(cx).focus(window, cx);
            }
        }
        let on_enter = editor.clone();
        dialog
            .title(SharedString::from(dialog_title(editing)))
            .w(px(448.))
            .child(editor.clone())
            // Enter in a field reaches the dialog as Confirm; save rather
            // than let the default close the dialog unsaved. The save
            // closes it when the row is written.
            .on_ok(move |_, window, cx| {
                on_enter.update(cx, |e, cx| e.save(window, cx));
                false
            })
    });
}
