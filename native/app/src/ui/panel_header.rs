//! The title strip on every panel. Port of `PanelHeader.vue`: a title on the
//! left, the panel's icon buttons on the right, one height everywhere so
//! panels side by side line up.

use gpui_kit::component::h_flex;
use gpui_kit::{
    AnyElement, App, FontWeight, IntoElement, ParentElement, RenderOnce, SharedString, Styled,
    Window, px,
};

use super::theme::palette;

pub const PANEL_HEADER_H: f32 = 32.0;

#[derive(IntoElement)]
pub struct PanelHeader {
    title: SharedString,
    actions: Vec<AnyElement>,
}

impl PanelHeader {
    pub fn new(title: impl Into<SharedString>) -> Self {
        Self {
            title: title.into(),
            actions: Vec::new(),
        }
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
        h_flex()
            .h(px(PANEL_HEADER_H))
            .flex_none()
            .px_2()
            .justify_between()
            .border_b_1()
            .border_color(p.line)
            .child(
                gpui_kit::div()
                    .text_xs()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(p.ink)
                    .child(self.title),
            )
            .child(h_flex().gap_0p5().children(self.actions))
    }
}
