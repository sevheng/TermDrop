//! What a datastore tab shows until its folder's real view replaces it:
//! the host and its (masked) address, and that the browser is not built
//! yet. Delete the call in `mongo/mod.rs` or `redis/mod.rs` when the real
//! view lands; delete this file when neither uses it.

use gpui_kit::assets::IconName;
use gpui_kit::component::Icon;
use gpui_kit::{
    AnyView, App, AppContext as _, Context, IntoElement, ParentElement, Render, SharedString,
    Styled, Window, div, px,
};

use super::ServiceTabContext;
use crate::ui::empty_state::EmptyState;
use crate::ui::theme::palette;

pub(crate) struct Placeholder {
    ctx: ServiceTabContext,
    address: String,
}

pub(crate) fn build(ctx: ServiceTabContext, _window: &mut Window, cx: &mut App) -> AnyView {
    let address = ctx.display_uri(cx);
    cx.new(|_| Placeholder { ctx, address }).into()
}

impl Render for Placeholder {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = *palette(cx);
        let label = self.ctx.kind.label();
        let icon = match self.ctx.kind {
            super::DatastoreKind::Mongo => IconName::Database,
            super::DatastoreKind::Redis => IconName::Layers,
        };
        let name = self.ctx.host_name(cx).unwrap_or_default();
        let hint = if self.address.is_empty() {
            format!("{name}: the {label} browser is still being built in the native app.")
        } else {
            format!(
                "{name} ({}): the {label} browser is still being built in the native app.",
                self.address
            )
        };
        let ctx = self.ctx.clone();
        div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .bg(p.canvas)
            .child(
                div().w(px(440.)).child(
                    EmptyState::empty(
                        SharedString::from(format!("datastore-{}", self.ctx.tab_id)),
                        format!("{label} is not in the native app yet"),
                        "Close tab",
                        move |_, _, cx| ctx.close_tab(cx),
                    )
                    .icon(Icon::new(icon))
                    .hint(hint),
                ),
            )
    }
}
