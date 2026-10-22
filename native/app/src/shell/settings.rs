//! The settings dialog: theme and terminal font size. Port of the parts of
//! `SettingsPanel.vue` the shell owns; both are saved as they change, in the
//! `theme` and `font_size` settings the Tauri app reads.

use gpui_kit::assets::IconName;
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::{Selectable as _, Sizable as _, h_flex, v_flex};
use gpui_kit::{
    App, AppContext as _, Context, Entity, IntoElement, ParentElement, Render, Styled,
    Subscription, Window, div, px,
};

use crate::app::AppModel;
use crate::app::prefs::{FONT_SIZE_RANGE, ThemeChoice};
use crate::ui::icon_button::IconButton;
use crate::ui::theme::palette;

pub struct SettingsView {
    model: Entity<AppModel>,
    _observe: Subscription,
}

impl SettingsView {
    fn new(model: Entity<AppModel>, cx: &mut Context<Self>) -> Self {
        // Its own entity, observing the model, so the dialog reflects a change
        // the moment it is made rather than when the dialog layer next rebuilds.
        let observe = cx.observe(&model, |_, _, cx| cx.notify());
        Self {
            model,
            _observe: observe,
        }
    }
}

fn row(label: &'static str, control: impl IntoElement, cx: &App) -> impl IntoElement {
    let p = *palette(cx);
    h_flex()
        .justify_between()
        .gap_4()
        .child(div().text_sm().text_color(p.ink).child(label))
        .child(control)
}

impl Render for SettingsView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let model = self.model.read(cx);
        let theme = model.theme();
        let size = model.font_size();
        let p = *palette(cx);

        let theme_button = |id: &'static str, label: &'static str, choice: ThemeChoice| {
            let m = self.model.clone();
            Button::new(id)
                .small()
                .label(label)
                .icon(match choice {
                    ThemeChoice::Dark => IconName::Moon,
                    ThemeChoice::Light => IconName::Sun,
                })
                .selected(theme == choice)
                .when_selected(theme == choice)
                .on_click(move |_, _, cx| m.update(cx, |m, cx| m.set_theme(choice, cx)))
        };
        let m_down = self.model.clone();
        let m_up = self.model.clone();

        v_flex()
            .gap_4()
            .child(row(
                "Theme",
                h_flex()
                    .gap_1()
                    .child(theme_button("theme-dark", "Dark", ThemeChoice::Dark))
                    .child(theme_button("theme-light", "Light", ThemeChoice::Light)),
                cx,
            ))
            .child(row(
                "Terminal font size",
                h_flex()
                    .gap_1()
                    .child(
                        IconButton::new("font-smaller", IconName::Minus, "Smaller terminal font")
                            .disabled(size <= FONT_SIZE_RANGE.0)
                            .on_click(move |_, _, cx| {
                                m_down.update(cx, |m, cx| m.set_font_size(size - 1, cx))
                            }),
                    )
                    .child(
                        div()
                            .w(px(40.))
                            .text_center()
                            .text_sm()
                            .text_color(p.ink)
                            .child(format!("{size}px")),
                    )
                    .child(
                        IconButton::new("font-larger", IconName::Plus, "Larger terminal font")
                            .disabled(size >= FONT_SIZE_RANGE.1)
                            .on_click(move |_, _, cx| {
                                m_up.update(cx, |m, cx| m.set_font_size(size + 1, cx))
                            }),
                    ),
                cx,
            ))
    }
}

/// A primary fill on the selected option, so the current choice reads at a
/// glance in both themes.
trait WhenSelected: Sized {
    fn when_selected(self, selected: bool) -> Self;
}

impl WhenSelected for Button {
    fn when_selected(self, selected: bool) -> Self {
        if selected {
            self.primary()
        } else {
            self.ghost()
        }
    }
}

pub fn open_settings(window: &mut Window, cx: &mut App, model: Entity<AppModel>) {
    let view = cx.new(|cx| SettingsView::new(model, cx));
    window.open_dialog(cx, move |dialog, _, _| {
        dialog.title("Settings").w(px(420.)).child(view.clone())
    });
}
