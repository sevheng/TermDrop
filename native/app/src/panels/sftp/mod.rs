//! The SFTP panel (`SftpPanel.vue`, `FloatingEditor.vue`). Per tab: built
//! once the tab's SFTP session is up, with `ctx.sftp_session_id` set. See
//! `../README.md`.
//!
//! Only this folder is the SFTP agent's to edit.

use gpui_kit::{AnyView, App, Window};

use super::PanelContext;

/// The panel's view. A placeholder until the panel is built.
pub fn build(ctx: PanelContext, _window: &mut Window, cx: &mut App) -> AnyView {
    let id = ctx.sftp_session_id.unwrap_or_default();
    super::placeholder(
        "SFTP",
        format!("The file browser for SFTP session {id} renders here."),
        cx,
    )
}
