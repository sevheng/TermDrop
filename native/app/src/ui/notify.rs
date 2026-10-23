//! Desktop notifications for long work that finished while the user looked
//! elsewhere. Port of `utils/notify.js` (`notifyIfUnfocused`).
//!
//! The toast that already accompanies these only works if the user is
//! looking at the app, and a backup big enough to be worth announcing is
//! exactly the one they switched away from. So this posts only when the
//! window is **not** focused, and the toast covers the case where it is:
//! firing both would say the same thing twice.
//!
//! It never fails. GPUI posts through the platform's notification center
//! (`notify-rust` over D-Bus on Linux) on its own thread and only logs a
//! refusal, so a desktop without a notification daemon (WSLg among them)
//! cannot turn a backup that succeeded into a visible failure.

use gpui_kit::{App, SharedString, SystemNotification, Window};

/// Post `title` / `body` to the desktop, unless this window has focus.
pub fn notify_if_unfocused(
    window: &Window,
    cx: &App,
    title: impl Into<SharedString>,
    body: impl Into<SharedString>,
) {
    if window.is_window_active() {
        return;
    }
    cx.show_system_notification(SystemNotification {
        // A fresh tag each time: two backups finishing are two notices, as
        // the Tauri plugin's untagged notifications were, not one replacing
        // the other.
        tag: uuid::Uuid::new_v4().to_string().into(),
        title: title.into(),
        body: body.into(),
        actions: Vec::new(),
    });
}
