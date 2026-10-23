//! The Tunnels panel (`PortForwardPanel.vue`) is not ported yet: it says so
//! rather than showing a list that pretends to work.

use gpui_kit::assets::IconName;
use gpui_kit::component::Icon;
use gpui_kit::{
    AnyView, App, AppContext as _, Context, Entity, IntoElement, ParentElement, Render, Styled,
    Window, div,
};

use super::PanelContext;
use crate::app::AppModel;
use crate::ui::empty_state::EmptyState;

pub fn build(ctx: PanelContext, _window: &mut Window, cx: &mut App) -> AnyView {
    cx.new(|_| Tunnels { model: ctx.model }).into()
}

struct Tunnels {
    model: Entity<AppModel>,
}

impl Render for Tunnels {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let model = self.model.clone();
        div().size_full().p_2().child(
            EmptyState::empty(
                "tunnels-not-ported",
                "Tunnels are not in the native app yet",
                "Hide side panel",
                move |_, _, cx| model.update(cx, |m, cx| m.toggle_right_panel(cx)),
            )
            .icon(Icon::new(IconName::ArrowLeftRight))
            .hint(
                "Port forwards are still being ported. Manage them in the TermDrop app for now; \
                 it reads the same saved rules.",
            ),
        )
    }
}
