//! The title strip on every panel. Port of `PanelHeader.vue`: a title, then
//! an optional subtitle and badges, then on the right an optional `meta`
//! (a count or status) and the panel's icon buttons. One height everywhere,
//! so panels side by side line up.

use gpui_kit::FontFeatures;
use gpui_kit::component::h_flex;
use gpui_kit::{
    AnyElement, App, FontWeight, IntoElement, ParentElement, RenderOnce, SharedString, Styled,
    Window, div, px,
};

use super::theme::{mono_font, palette};
use crate::panels::common::text_2xs;

pub const PANEL_HEADER_H: f32 = 32.0;

#[derive(IntoElement)]
pub struct PanelHeader {
    title: SharedString,
    subtitle: Option<SharedString>,
    subtitle_mono: bool,
    meta: Option<SharedString>,
    badges: Vec<AnyElement>,
    actions: Vec<AnyElement>,
}

impl PanelHeader {
    pub fn new(title: impl Into<SharedString>) -> Self {
        Self {
            title: title.into(),
            subtitle: None,
            subtitle_mono: false,
            meta: None,
            badges: Vec::new(),
            actions: Vec::new(),
        }
    }

    /// Quieter text after the title ("updated 5m ago", a path). Truncates
    /// first when the strip is narrow. An empty string shows nothing.
    pub fn subtitle(mut self, subtitle: impl Into<SharedString>) -> Self {
        let subtitle = subtitle.into();
        self.subtitle = (!subtitle.is_empty()).then_some(subtitle);
        self
    }

    /// Set the subtitle in the monospace font, for a path or an address.
    pub fn subtitle_mono(mut self) -> Self {
        self.subtitle_mono = true;
        self
    }

    /// A right-aligned count or status ("4 containers"), in tabular figures
    /// so it does not jitter as it changes. An empty string shows nothing.
    pub fn meta(mut self, meta: impl Into<SharedString>) -> Self {
        let meta = meta.into();
        self.meta = (!meta.is_empty()).then_some(meta);
        self
    }

    /// A control or tag after the subtitle, on the left (Vue's `badges`
    /// slot): a "Show all" toggle, a status tag.
    pub fn badge(mut self, badge: impl IntoElement) -> Self {
        self.badges.push(badge.into_any_element());
        self
    }

    /// An action on the right, normally an `IconButton`.
    pub fn action(mut self, action: impl IntoElement) -> Self {
        self.actions.push(action.into_any_element());
        self
    }
}

impl RenderOnce for PanelHeader {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let p = *palette(cx);
        let mono = self.subtitle_mono.then(|| mono_font(cx));
        h_flex()
            .h(px(PANEL_HEADER_H))
            .flex_none()
            .px_2()
            .gap_2()
            .border_b_1()
            .border_color(p.line)
            .child(
                div()
                    .flex_none()
                    .text_xs()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(p.ink)
                    .child(self.title),
            )
            .children(self.subtitle.map(|subtitle| {
                let mut el = text_2xs(div()).min_w_0().truncate().text_color(p.ink_3);
                if let Some(mono) = mono {
                    el = el.font_family(mono);
                }
                el.child(subtitle)
            }))
            .children(self.badges)
            .child(div().flex_1().min_w_0())
            .children(self.meta.map(|meta| {
                text_2xs(div())
                    .flex_none()
                    .font_features(tabular_figures())
                    .text_color(p.ink_3)
                    .child(meta)
            }))
            .child(h_flex().flex_none().gap_0p5().children(self.actions))
    }
}

/// `tnum`: every digit the same width, so a count does not shift the row
/// as it changes (Tailwind's `tabular-nums`).
pub fn tabular_figures() -> FontFeatures {
    FontFeatures(std::sync::Arc::new(vec![("tnum".into(), 1)]))
}
