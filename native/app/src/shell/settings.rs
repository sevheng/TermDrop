//! The settings dialog. Port of `SettingsPanel.vue`: terminal font size
//! (10-24), appearance, download path and the version line, written only on
//! Save, in the `font_size`, `download_path` and `theme` settings the Tauri
//! app reads. Picking a theme previews it at once; Cancel, Escape or the
//! close icon put the saved one back. Save also tells every terminal its
//! font size (`terminal-settings-changed`).
//!
//! `download_path` is stored and shown but, as in the Tauri app, nothing
//! reads it yet: SFTP downloads go to the Downloads folder. The update check
//! and its "Last checked" line are not here: the preview has no updater.

use gpui_kit::assets::IconName;
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::slider::{Slider, SliderEvent, SliderState};
use gpui_kit::component::{Icon, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, AppContext as _, Context, Entity, FontWeight, InteractiveElement as _, IntoElement,
    ParentElement, Render, SharedString, StatefulInteractiveElement as _, Styled, Subscription,
    Window, div, px,
};

use crate::app::AppModel;
use crate::app::prefs::{FONT_SIZE_SLIDER, SettingsDraft, ThemeChoice};
use crate::ui::theme::{self, mono_font, palette};

/// The version line's value: this build's own.
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

pub struct SettingsView {
    model: Entity<AppModel>,
    draft: SettingsDraft,
    slider: Entity<SliderState>,
    download_path: Entity<InputState>,
    _subscriptions: Vec<Subscription>,
}

impl SettingsView {
    fn new(model: Entity<AppModel>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let m = model.read(cx);
        let draft = SettingsDraft::open(m.font_size(), m.download_path(), m.theme());
        let slider = cx.new(|_| {
            SliderState::new()
                .min(f32::from(FONT_SIZE_SLIDER.0))
                .max(f32::from(FONT_SIZE_SLIDER.1))
                .step(1.)
                .default_value(f32::from(draft.slider_value()))
        });
        let initial_path = draft.download_path.clone();
        let download_path = cx.new(|cx| {
            let mut state = InputState::new(window, cx).placeholder("~/Downloads");
            state.set_value(initial_path, window, cx);
            state
        });
        let subscriptions = vec![
            cx.subscribe(&slider, |this, _, event: &SliderEvent, cx| {
                let (SliderEvent::Change(value) | SliderEvent::Release(value)) = event;
                this.draft.font_size = value.start().round() as u16;
                cx.notify();
            }),
            cx.subscribe(&download_path, |this, input, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    this.draft.download_path = input.read(cx).value().to_string();
                }
            }),
        ];
        Self {
            model,
            draft,
            slider,
            download_path,
            _subscriptions: subscriptions,
        }
    }

    /// Preview a theme without saving it, as `applyTheme` on the Vue ref.
    fn pick_theme(&mut self, choice: ThemeChoice, cx: &mut Context<Self>) {
        self.draft.theme = choice;
        theme::install(choice, cx);
        cx.notify();
    }

    fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let draft = self.draft.clone();
        self.model.update(cx, |m, cx| m.save_settings(&draft, cx));
        window.close_dialog(cx);
    }
}

fn field_label(text: impl Into<SharedString>, cx: &App) -> impl IntoElement {
    let p = *palette(cx);
    div().text_xs().text_color(p.ink_2).child(text.into())
}

impl Render for SettingsView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = *palette(cx);
        let mono = mono_font(cx);
        let current = self.draft.theme;

        let mut appearance = h_flex().gap_2();
        for choice in [ThemeChoice::Dark, ThemeChoice::Light] {
            let selected = current == choice;
            appearance = appearance.child(
                h_flex()
                    .id(SharedString::from(format!("theme-{}", choice.as_str())))
                    .flex_1()
                    .justify_center()
                    .gap_2()
                    .px_3()
                    .py_2()
                    .rounded_sm()
                    .border_1()
                    .text_xs()
                    .cursor_pointer()
                    .when_else(
                        selected,
                        |d| d.border_color(p.accent).bg(p.selected).text_color(p.ink),
                        |d| {
                            d.border_color(p.line)
                                .bg(p.input)
                                .text_color(p.ink_2)
                                .hover(|s| s.text_color(p.ink))
                        },
                    )
                    .child(
                        Icon::new(match choice {
                            ThemeChoice::Dark => IconName::Moon,
                            ThemeChoice::Light => IconName::Sun,
                        })
                        .xsmall(),
                    )
                    .child(match choice {
                        ThemeChoice::Dark => "Dark",
                        ThemeChoice::Light => "Light",
                    })
                    .on_click(cx.listener(move |this, _, _, cx| this.pick_theme(choice, cx))),
            );
        }

        v_flex()
            .gap_2()
            .child(
                v_flex()
                    .gap_1()
                    .child(field_label(self.draft.font_size_label(), cx))
                    .child(Slider::new(&self.slider).accessibility_label("Terminal font size")),
            )
            .child(
                v_flex()
                    .gap_2()
                    .child(field_label("Appearance", cx))
                    .child(appearance),
            )
            .child(
                v_flex()
                    .gap_1()
                    .child(field_label("Download Path (leave empty for default)", cx))
                    .child(Input::new(&self.download_path).small()),
            )
            .child(
                div().mt_2().pt_4().border_t_1().border_color(p.line).child(
                    h_flex()
                        .gap_1()
                        .text_xs()
                        .text_color(p.ink)
                        .child("Version:")
                        .child(
                            div()
                                .font_family(mono)
                                .font_weight(FontWeight::NORMAL)
                                .child(APP_VERSION),
                        ),
                ),
            )
            .child(
                h_flex()
                    .mt_4()
                    .justify_end()
                    .gap_2()
                    .child(
                        Button::new("settings-cancel")
                            .ghost()
                            .label("Cancel")
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                    .child(
                        Button::new("settings-save")
                            .primary()
                            .label("Save")
                            .on_click(cx.listener(|this, _, window, cx| this.save(window, cx))),
                    ),
            )
    }
}

pub fn open_settings(window: &mut Window, cx: &mut App, model: Entity<AppModel>) {
    let view = cx.new(|cx| SettingsView::new(model.clone(), window, cx));
    window.open_dialog(cx, move |dialog, _, _| {
        let model = model.clone();
        dialog
            .title("Settings")
            .w(px(384.))
            .child(view.clone())
            // Every way out puts the saved theme back. After Save that is the
            // one just saved, so this undoes only an unsaved preview, as the
            // Vue dialog's `watch(show)` does.
            .on_close(move |_, _, cx| {
                let saved = model.read(cx).theme();
                theme::install(saved, cx);
            })
    });
}
