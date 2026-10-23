//! The Docker panel (`DockerPanel.vue`, plus the logs/exec pane that
//! `TerminalTab.vue` hosts). Per host. See `../README.md`.
//!
//! Only this folder is the Docker agent's to edit.

use gpui_kit::{AnyView, App, Window};

use super::PanelContext;

/// The panel's view. A placeholder until the panel is built.
pub fn build(ctx: PanelContext, _window: &mut Window, cx: &mut App) -> AnyView {
    super::placeholder(
        "Docker",
        format!("The containers on host {} render here.", ctx.host_id),
        cx,
    )
}
