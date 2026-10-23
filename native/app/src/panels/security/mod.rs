//! The Security panel (`SecurityPanel.vue`). Per host, and lazy: building
//! the view is the panel being opened, which is the only time an audit may
//! start. See `../README.md`.
//!
//! Only this folder is the Security agent's to edit.

use gpui_kit::{AnyView, App, Window};

use super::PanelContext;

/// The panel's view. A placeholder until the panel is built.
pub fn build(ctx: PanelContext, _window: &mut Window, cx: &mut App) -> AnyView {
    super::placeholder(
        "Security",
        format!("The security audit of host {} renders here.", ctx.host_id),
        cx,
    )
}
