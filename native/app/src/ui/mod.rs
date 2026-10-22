//! The UI kit every panel builds on: the theme and our wrappers over GPUI
//! Kit components, each keeping one of the Vue app's rules.
//!
//! - [`theme`]: tokens and the `Palette`; views name a token, never a colour.
//! - [`icon_button::IconButton`]: an icon-only button needs a label.
//! - [`empty_state::EmptyState`]: empty, filtered and loading stay distinct.
//! - [`panel_header::PanelHeader`]: one title strip for every panel.
//! - [`toast::toast`]: a notification whose kind is required.
//! - [`dialogs`]: confirm and prompt, answering exactly once.
//! - [`host_key_dialog`]: trust decisions, with no accept path on a mismatch.

pub mod dialogs;
pub mod empty_state;
pub mod host_key_dialog;
pub mod icon_button;
pub mod panel_header;
pub mod theme;
pub mod toast;
