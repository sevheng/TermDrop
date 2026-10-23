//! Toasts, over GPUI Kit's `Notification`. Port of `utils/toast.js` and
//! `ToastContainer.vue`: the kind is required, because a message whose
//! severity the reader has to guess from its wording is the toast's whole
//! failure mode.
//!
//! A repeat of a toast still on screen counts up (`×N`) instead of stacking,
//! and restarts its timer, as `ToastContainer.vue`'s `addToast` does: a remote
//! desktop rings its bell once per BEL, and `yes $'\a'` used to fill the
//! window with identical toasts. Toasts that differ in message or kind still
//! stack. Info, success and warning toasts last 4 s; an error stays until it
//! is dismissed (a deliberate native difference: it is the one kind that may
//! need reading twice, or copying into a bug report).

use std::collections::HashMap;
use std::time::Duration;

use gpui_kit::component::WindowExt as _;
use gpui_kit::component::notification::Notification;
use gpui_kit::{App, ElementId, Global, SharedString, Window};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ToastKind {
    Info,
    Success,
    Warning,
    Error,
}

/// How long a toast that hides itself stays up, from its last repeat. The
/// Vue container's default.
pub const TOAST_DURATION: Duration = Duration::from_secs(4);

/// The type half of every toast's notification id. Kit replaces a
/// notification pushed again under the same id, which is what lets a repeat
/// update the one on screen.
struct ToastId;

/// One toast on screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Shown {
    /// The id half that names this toast's notification.
    pub key: u64,
    /// How many times it has been raised while on screen.
    pub count: u32,
    /// Bumped on every repeat, so a timer armed for an earlier one does not
    /// hide the toast its successor restarted.
    pub generation: u64,
}

/// Which toasts are on screen, by (kind, message). Pure, so the coalescing
/// rule is tested without a window.
#[derive(Debug, Default)]
pub struct ToastLedger {
    shown: HashMap<(ToastKind, String), Shown>,
    next: u64,
}

impl Global for ToastLedger {}

impl ToastLedger {
    /// Record that `message` was raised. A repeat of one still on screen
    /// keeps its key and counts up; anything else gets a fresh key.
    pub fn raise(&mut self, kind: ToastKind, message: &str) -> Shown {
        self.next += 1;
        let generation = self.next;
        let entry = self
            .shown
            .entry((kind, message.to_string()))
            .and_modify(|s| {
                s.count += 1;
                s.generation = generation;
            })
            .or_insert(Shown {
                key: generation,
                count: 1,
                generation,
            });
        *entry
    }

    /// The toast left the screen (dismissed, or hidden by its timer). Only
    /// the generation that is showing counts: a notification replaced by a
    /// repeat is not this one.
    pub fn closed(&mut self, kind: ToastKind, message: &str, generation: u64) {
        let k = (kind, message.to_string());
        if self
            .shown
            .get(&k)
            .is_some_and(|s| s.generation == generation)
        {
            self.shown.remove(&k);
        }
    }

    /// Whether `generation` is still the showing one, for its timer.
    pub fn is_current(&self, kind: ToastKind, message: &str, generation: u64) -> bool {
        self.shown
            .get(&(kind, message.to_string()))
            .is_some_and(|s| s.generation == generation)
    }
}

/// The text a toast shows: the message, then `×N` once it has repeated.
pub fn toast_text(message: &str, count: u32) -> String {
    if count > 1 {
        format!("{message}  ×{count}")
    } else {
        message.to_string()
    }
}

pub fn toast(window: &mut Window, cx: &mut App, message: impl Into<SharedString>, kind: ToastKind) {
    let message: SharedString = message.into();
    let shown = cx
        .default_global::<ToastLedger>()
        .raise(kind, message.as_ref());
    let text = toast_text(&message, shown.count);
    let note = match kind {
        ToastKind::Info => Notification::info(text),
        ToastKind::Success => Notification::success(text),
        ToastKind::Warning => Notification::warning(text),
        ToastKind::Error => Notification::error(text),
    };
    let key = ElementId::from(SharedString::from(format!("toast-{}", shown.key)));
    let closed_message = message.clone();
    let generation = shown.generation;
    let note = note
        .id1::<ToastId>(key.clone())
        // Kit's own timer is a fixed 5 s from the first push; ours is 4 s
        // from the latest repeat.
        .autohide(false)
        .on_close(move |_, cx| {
            cx.default_global::<ToastLedger>()
                .closed(kind, &closed_message, generation);
        });
    window.push_notification(note, cx);

    if kind == ToastKind::Error {
        return;
    }
    window
        .spawn(cx, async move |cx| {
            cx.background_executor().timer(TOAST_DURATION).await;
            let _ = cx.update(|window, cx| {
                if cx
                    .default_global::<ToastLedger>()
                    .is_current(kind, &message, generation)
                {
                    window.remove_notification1::<ToastId>(key, cx);
                }
            });
        })
        .detach();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_a_repeated_toast_instead_of_stacking_it() {
        let mut ledger = ToastLedger::default();
        let first = ledger.raise(ToastKind::Info, "kiosk is asking for attention");
        let mut last = first;
        for _ in 1..50 {
            last = ledger.raise(ToastKind::Info, "kiosk is asking for attention");
        }
        assert_eq!(last.key, first.key, "one notification, updated in place");
        assert_eq!(last.count, 50);
        assert_eq!(
            toast_text("kiosk is asking for attention", last.count),
            "kiosk is asking for attention  ×50"
        );
    }

    #[test]
    fn keeps_a_repeated_toast_up_for_as_long_as_it_keeps_repeating() {
        let mut ledger = ToastLedger::default();
        let a = ledger.raise(ToastKind::Info, "bell");
        let b = ledger.raise(ToastKind::Info, "bell");
        // The first timer fires: it is no longer the current generation.
        assert!(!ledger.is_current(ToastKind::Info, "bell", a.generation));
        assert!(ledger.is_current(ToastKind::Info, "bell", b.generation));
        // A stale close does not forget the toast that is showing.
        ledger.closed(ToastKind::Info, "bell", a.generation);
        assert_eq!(ledger.raise(ToastKind::Info, "bell").count, 3);
    }

    #[test]
    fn starts_over_once_the_toast_has_gone() {
        let mut ledger = ToastLedger::default();
        let a = ledger.raise(ToastKind::Error, "boom");
        ledger.closed(ToastKind::Error, "boom", a.generation);
        let b = ledger.raise(ToastKind::Error, "boom");
        assert_eq!(b.count, 1);
        assert_ne!(b.key, a.key);
        assert_eq!(toast_text("boom", 1), "boom");
    }

    #[test]
    fn still_stacks_toasts_that_differ_in_message_or_severity() {
        let mut ledger = ToastLedger::default();
        let keys = [
            ledger.raise(ToastKind::Success, "saved").key,
            ledger.raise(ToastKind::Error, "saved").key,
            ledger.raise(ToastKind::Success, "deleted").key,
        ];
        assert_ne!(keys[0], keys[1]);
        assert_ne!(keys[0], keys[2]);
        assert_ne!(keys[1], keys[2]);
    }
}
