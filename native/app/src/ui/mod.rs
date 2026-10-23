//! The UI kit every panel builds on: the theme and our wrappers over GPUI
//! Kit components, each keeping one of the Vue app's rules.
//!
//! - [`fonts`]: the bundled TTFs, registered before the first window.
//! - [`theme`]: tokens and the `Palette`; views name a token, never a colour.
//! - [`icon_button::IconButton`]: an icon-only button needs a label.
//! - [`empty_state::EmptyState`]: empty, filtered and loading stay distinct.
//! - [`panel_header::PanelHeader`]: one title strip for every panel.
//! - [`toast::toast`]: a notification whose kind is required.
//! - [`dialogs`]: confirm and prompt, answering exactly once.
//! - [`host_key_dialog`]: trust decisions, with no accept path on a mismatch.
//! - [`choice::Choice`]: a `Select` entry with a hint.
//! - [`file_dialog`]: native open/save dialogs, several filters each.
//! - [`notify::notify_if_unfocused`]: a desktop notice only when the window
//!   is not focused.
//! - [`tag::Tag`]: a header's word badge, toned by token, with a tooltip.

pub mod choice;
pub mod dialogs;
pub mod empty_state;
pub mod file_dialog;
pub mod fonts;
pub mod host_key_dialog;
pub mod icon_button;
pub mod notify;
pub mod panel_header;
pub mod tag;
pub mod theme;
pub mod toast;
