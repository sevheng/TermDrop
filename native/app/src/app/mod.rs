//! Application state and the pure rules behind it.
//!
//! Everything here except `model` is free of GPUI and tested like the JS
//! utilities it ports, case for case.

pub mod display;
pub mod host_grouping;
pub mod host_kind;
pub mod host_row_state;
pub mod list_nav;
pub mod model;
pub mod prefs;
pub mod tab_kind;
pub mod tabs;

pub use model::{AppEvent, AppModel};
