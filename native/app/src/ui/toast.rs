//! Toasts, over GPUI Kit's `Notification`. Port of `utils/toast.js`: the
//! kind is required, because a message whose severity the reader has to
//! guess from its wording is the toast's whole failure mode.

use gpui_kit::component::WindowExt as _;
use gpui_kit::component::notification::Notification;
use gpui_kit::{App, SharedString, Window};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToastKind {
    Info,
    Success,
    Warning,
    Error,
}

pub fn toast(window: &mut Window, cx: &mut App, message: impl Into<SharedString>, kind: ToastKind) {
    let message = message.into();
    let note = match kind {
        ToastKind::Info => Notification::info(message),
        ToastKind::Success => Notification::success(message),
        ToastKind::Warning => Notification::warning(message),
        // An error stays until it is dismissed: it is the one kind that may
        // need reading twice, or copying into a bug report.
        ToastKind::Error => Notification::error(message).autohide(false),
    };
    window.push_notification(note, cx);
}
