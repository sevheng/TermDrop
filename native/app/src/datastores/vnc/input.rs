//! Pointer and keyboard for the remote screen: the port of
//! `src/composables/useVncInput.js`, as a state machine that returns what to
//! send rather than sending it, so it can be tested without a session.
//!
//! The three behaviours that make a remote desktop feel broken when missing
//! are all here: a drag that leaves the image keeps dragging (the view
//! routes moves and releases to this while a button is held), a key held
//! when focus is lost does not stay held ([`Input::release_all`]), and a
//! pointer stream is not forwarded event for event ([`MOVE_INTERVAL`]).

use std::time::{Duration, Instant};

use gpui_kit::{Keystroke, Modifiers};

use super::keysym::{
    ALT_L, CAPS_LOCK, CONTROL_L, KeySource, MODIFIER_KEYSYMS, char_keysym, is_altgr_text,
    keystroke_keysym, modifier_changes,
};
use super::layout::{Buttons, WHEEL_DOWN, WHEEL_LEFT, WHEEL_RIGHT, WHEEL_UP, WheelDelta};
use super::layout::{wheel_events, wheel_ticks};

/// The shortest gap between two pointer sends: 125 Hz, above the rate at
/// which more reports help and well inside one display frame, so the
/// trailing send is never perceptible.
pub const MOVE_INTERVAL: Duration = Duration::from_millis(8);

/// One message for the session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Send {
    Pointer { x: u16, y: u16, mask: u8 },
    Key { keysym: u32, down: bool },
}

/// What a pointer move asks of the caller.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Move {
    /// Send these now.
    Now(Vec<Send>),
    /// Call [`Input::flush_move`] after this long, unless a timer is
    /// already armed (then nothing to do).
    ArmTrailing(Duration),
    /// A trailing send is already armed; it will carry this position.
    Wait,
}

/// What a key press did.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct KeyOutcome {
    pub sends: Vec<Send>,
    /// The press was the remote's: the app must not act on it. Always true:
    /// a viewer that lets the app keep its shortcuts cannot be used for
    /// anything real, and Ctrl+Alt+Shift is the way out.
    pub handled: bool,
}

#[derive(Default)]
pub struct Input {
    pub view_only: bool,
    buttons: Buttons,
    /// The latest pointer position, in framebuffer pixels, not yet sent.
    pending: Option<(u16, u16)>,
    trailing_armed: bool,
    last_sent_at: Option<Instant>,
    last_sent: Option<(u16, u16, u8)>,
    /// The last position anything was sent at, for wheel clicks.
    wheel: (f32, f32),
    /// Keys held, by GPUI key name, with the keysym their press sent.
    held: Vec<(String, u32)>,
    /// Modifier keysyms this side pressed and has not released.
    modifiers_down: Vec<u32>,
    modifiers: Modifiers,
    capslock: Option<bool>,
}

impl Input {
    pub fn new(view_only: bool) -> Self {
        Self {
            view_only,
            ..Self::default()
        }
    }

    pub fn buttons(&self) -> Buttons {
        self.buttons
    }

    // ---- pointer -------------------------------------------------------

    /// Leading edge, then a trailing flush: after any pause the move goes
    /// out at once (the first move of a gesture is the one the eye waits
    /// on); only a continuous drag is rate-limited.
    pub fn pointer_move(&mut self, at: (u16, u16), now: Instant) -> Move {
        if self.view_only {
            return Move::Now(Vec::new());
        }
        self.pending = Some(at);
        let since = self
            .last_sent_at
            .map_or(MOVE_INTERVAL, |t| now.saturating_duration_since(t));
        if since >= MOVE_INTERVAL {
            return Move::Now(self.flush_move(now));
        }
        if self.trailing_armed {
            return Move::Wait;
        }
        self.trailing_armed = true;
        Move::ArmTrailing(MOVE_INTERVAL - since)
    }

    /// Send the pending position, if it is news. Also what the trailing
    /// timer calls. A position the server already has (many raw moves land
    /// on one remote pixel when scaled down) is not sent again.
    pub fn flush_move(&mut self, now: Instant) -> Vec<Send> {
        self.trailing_armed = false;
        let Some((x, y)) = self.pending.take() else {
            return Vec::new();
        };
        let mask = self.buttons.mask();
        if self.last_sent == Some((x, y, mask)) {
            return Vec::new();
        }
        self.last_sent = Some((x, y, mask));
        self.last_sent_at = Some(now);
        vec![Send::Pointer { x, y, mask }]
    }

    /// A press or release. Never rate-limited, and it carries its own
    /// position: the click lands where the user aimed, and flushing here
    /// disarms the trailing send, so no stale position can follow it.
    pub fn buttons_changed(&mut self, at: (u16, u16), buttons: Buttons, now: Instant) -> Vec<Send> {
        if self.view_only {
            return Vec::new();
        }
        self.pending = Some(at);
        self.buttons = buttons;
        self.last_sent = None;
        self.flush_move(now)
    }

    /// Wheel deltas, positive down/right (the DOM's sign). RFB has no scroll
    /// event: each whole click is a press and release of a wheel button.
    pub fn wheel(&mut self, at: (u16, u16), dx: WheelDelta, dy: WheelDelta) -> Vec<Send> {
        if self.view_only {
            return Vec::new();
        }
        let (ty, ry) = wheel_ticks(self.wheel.1, dy);
        let (tx, rx) = wheel_ticks(self.wheel.0, dx);
        self.wheel = (rx, ry);
        let held = self.buttons.mask();
        wheel_events(ty, WHEEL_DOWN, WHEEL_UP, held)
            .into_iter()
            .chain(wheel_events(tx, WHEEL_RIGHT, WHEEL_LEFT, held))
            .map(|mask| Send::Pointer {
                x: at.0,
                y: at.1,
                mask,
            })
            .collect()
    }

    // ---- keyboard ------------------------------------------------------

    /// The held modifiers changed. Returns what to send, and whether this
    /// was Ctrl+Alt+Shift, the one deliberate way to hand the keyboard back
    /// to the app (everything is released first).
    pub fn modifiers_changed(&mut self, new: Modifiers, capslock: bool) -> (Vec<Send>, bool) {
        let old = std::mem::replace(&mut self.modifiers, new);
        let caps_was = self.capslock.replace(capslock);
        if new.control && new.alt && new.shift {
            return (self.release_all(), true);
        }
        if self.view_only {
            return (Vec::new(), false);
        }
        let mut sends = Vec::new();
        for (keysym, down) in modifier_changes(old, new) {
            if down {
                if !self.modifiers_down.contains(&keysym) {
                    self.modifiers_down.push(keysym);
                }
                sends.push(Send::Key { keysym, down });
            } else if let Some(i) = self.modifiers_down.iter().position(|k| *k == keysym) {
                self.modifiers_down.remove(i);
                sends.push(Send::Key { keysym, down });
            }
        }
        // Caps Lock arrives as a state, not a key: one press toggles the
        // remote's too. The first report only learns the state.
        if caps_was.is_some_and(|was| was != capslock) {
            sends.push(Send::Key {
                keysym: CAPS_LOCK,
                down: true,
            });
            sends.push(Send::Key {
                keysym: CAPS_LOCK,
                down: false,
            });
        }
        (sends, false)
    }

    pub fn key_down(&mut self, ks: &Keystroke, prefer_character_input: bool) -> KeyOutcome {
        let mut out = KeyOutcome {
            sends: Vec::new(),
            handled: true,
        };
        if self.view_only {
            return out;
        }
        if is_altgr_text(ks, prefer_character_input) {
            // That Ctrl+Alt was AltGr. Both have already gone out, so take
            // them back rather than send Ctrl+Alt+@; the character already
            // has AltGr applied.
            for keysym in [CONTROL_L, ALT_L] {
                if let Some(i) = self.modifiers_down.iter().position(|k| *k == keysym) {
                    self.modifiers_down.remove(i);
                    out.sends.push(Send::Key {
                        keysym,
                        down: false,
                    });
                }
            }
        }
        let Some((keysym, source)) = keystroke_keysym(ks) else {
            return out;
        };
        // A key whose release cannot be matched to its press is tapped:
        // a composed character (its press is named after the result, its
        // release after the key), or a symbol typed with Shift (`!` goes
        // down, and if Shift is let go first `1` comes up). A key left down
        // on the remote autorepeats there until the session ends; GPUI
        // repeats the press while the key is held, so a tap loses nothing.
        let shifted_symbol = source == KeySource::Typed
            && self.modifiers.shift
            && ks.key.chars().count() == 1
            && !ks.key.chars().any(char::is_alphabetic);
        let composed = source == KeySource::Typed && ks.key.chars().count() > 1;
        out.sends.push(Send::Key { keysym, down: true });
        if shifted_symbol || composed {
            out.sends.push(Send::Key {
                keysym,
                down: false,
            });
            return out;
        }
        if !self.held.iter().any(|(k, _)| *k == ks.key) {
            self.held.push((ks.key.clone(), keysym));
        }
        out
    }

    pub fn key_up(&mut self, key: &str) -> Vec<Send> {
        if self.view_only {
            return Vec::new();
        }
        match self.held.iter().position(|(k, _)| k == key) {
            Some(i) => {
                let (_, keysym) = self.held.remove(i);
                vec![Send::Key {
                    keysym,
                    down: false,
                }]
            }
            None => Vec::new(),
        }
    }

    /// Text from the input method (a commit, or a dead key that composed
    /// nothing): each character pressed and released.
    pub fn text(&mut self, text: &str) -> Vec<Send> {
        if self.view_only {
            return Vec::new();
        }
        text.chars()
            .filter(|c| !c.is_control())
            .flat_map(|c| {
                let keysym = char_keysym(c);
                [
                    Send::Key { keysym, down: true },
                    Send::Key {
                        keysym,
                        down: false,
                    },
                ]
            })
            .collect()
    }

    /// Release everything still held. Every modifier is released whether or
    /// not this side saw it go down: Alt-Tab away with Alt held and the
    /// release goes to the other window, which is exactly the stuck key
    /// this exists for.
    pub fn release_all(&mut self) -> Vec<Send> {
        let mut sends: Vec<Send> = self
            .held
            .drain(..)
            .map(|(_, keysym)| Send::Key {
                keysym,
                down: false,
            })
            .collect();
        sends.extend(MODIFIER_KEYSYMS.iter().map(|&keysym| Send::Key {
            keysym,
            down: false,
        }));
        self.modifiers_down.clear();
        sends
    }

    /// Focus left: forget the modifiers too, so the next focus starts from
    /// what the platform reports rather than a press that never ended.
    pub fn blurred(&mut self) -> Vec<Send> {
        self.modifiers = Modifiers::default();
        self.release_all()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t0() -> Instant {
        Instant::now()
    }

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    fn pointers(sends: &[Send]) -> Vec<(u16, u16, u8)> {
        sends
            .iter()
            .filter_map(|s| match *s {
                Send::Pointer { x, y, mask } => Some((x, y, mask)),
                _ => None,
            })
            .collect()
    }

    fn key(key: &str, key_char: Option<&str>) -> Keystroke {
        Keystroke {
            modifiers: Modifiers::default(),
            key: key.into(),
            key_char: key_char.map(Into::into),
        }
    }

    const LEFT: Buttons = Buttons {
        left: true,
        middle: false,
        right: false,
    };

    #[test]
    fn a_press_goes_out_at_its_own_position() {
        let now = t0();
        let mut input = Input::new(false);
        input.pointer_move((10, 10), now);
        let sent = input.buttons_changed((400, 300), LEFT, now + ms(1));
        assert_eq!(pointers(&sent), vec![(400, 300, 1)]);
    }

    #[test]
    fn the_first_move_of_a_gesture_is_sent_at_once() {
        let mut input = Input::new(false);
        let Move::Now(sent) = input.pointer_move((100, 100), t0()) else {
            panic!("the first move must not wait");
        };
        assert_eq!(pointers(&sent), vec![(100, 100, 0)]);
    }

    #[test]
    fn a_burst_is_rate_limited_but_its_final_position_survives() {
        let now = t0();
        let mut input = Input::new(false);
        let mut sent = Vec::new();
        let mut armed = 0;
        for i in 1..=20u16 {
            match input.pointer_move((i, i), now + Duration::from_micros(i as u64 * 100)) {
                Move::Now(s) => sent.extend(s),
                Move::ArmTrailing(_) => armed += 1,
                Move::Wait => {}
            }
        }
        assert_eq!(pointers(&sent).len(), 1, "leading edge only");
        assert_eq!(armed, 1, "one trailing timer, not one per move");
        sent.extend(input.flush_move(now + MOVE_INTERVAL));
        assert_eq!(pointers(&sent).last(), Some(&(20, 20, 0)));
    }

    #[test]
    fn the_trailing_delay_is_what_is_left_of_the_interval() {
        let now = t0();
        let mut input = Input::new(false);
        input.pointer_move((1, 1), now);
        assert_eq!(
            input.pointer_move((2, 2), now + ms(3)),
            Move::ArmTrailing(ms(5))
        );
    }

    #[test]
    fn does_not_resend_a_position_the_server_has() {
        let now = t0();
        let mut input = Input::new(false);
        input.pointer_move((50, 50), now);
        let Move::Now(again) = input.pointer_move((50, 50), now + ms(20)) else {
            panic!("idle long enough to send at once");
        };
        assert!(again.is_empty());
    }

    #[test]
    fn no_trailing_move_lands_after_a_button_event() {
        let now = t0();
        let mut input = Input::new(false);
        input.pointer_move((10, 10), now);
        input.pointer_move((20, 20), now + ms(1));
        let sent = input.buttons_changed((400, 300), LEFT, now + ms(2));
        assert_eq!(pointers(&sent), vec![(400, 300, 1)]);
        // The timer fires anyway: it has nothing left to send.
        assert!(input.flush_move(now + ms(9)).is_empty());
    }

    #[test]
    fn sends_nothing_at_all_when_view_only() {
        let now = t0();
        let mut input = Input::new(true);
        assert!(input.buttons_changed((1, 1), LEFT, now).is_empty());
        assert_eq!(input.pointer_move((5, 5), now), Move::Now(Vec::new()));
        assert!(input.key_down(&key("a", Some("a")), false).sends.is_empty());
        assert!(
            input
                .wheel((1, 1), WheelDelta::Pixels(0.), WheelDelta::Pixels(500.))
                .is_empty()
        );
        assert!(input.text("x").is_empty());
    }

    #[test]
    fn a_wheel_click_is_a_button_press_and_release() {
        let mut input = Input::new(false);
        let sent = input.wheel((3, 4), WheelDelta::Pixels(0.), WheelDelta::Pixels(100.));
        assert_eq!(
            pointers(&sent),
            vec![(3, 4, WHEEL_DOWN), (3, 4, 0), (3, 4, WHEEL_DOWN), (3, 4, 0)]
        );
        let sent = input.wheel((3, 4), WheelDelta::Pixels(-50.), WheelDelta::Pixels(0.));
        assert_eq!(pointers(&sent), vec![(3, 4, WHEEL_LEFT), (3, 4, 0)]);
    }

    #[test]
    fn sends_a_keysym_down_and_up() {
        let mut input = Input::new(false);
        let out = input.key_down(&key("a", Some("a")), false);
        assert!(out.handled);
        assert_eq!(
            out.sends,
            vec![Send::Key {
                keysym: 0x61,
                down: true
            }]
        );
        assert_eq!(
            input.key_up("a"),
            vec![Send::Key {
                keysym: 0x61,
                down: false
            }]
        );
        assert!(input.key_up("a").is_empty(), "released once");
    }

    #[test]
    fn a_key_whose_release_cannot_be_matched_is_tapped() {
        let mut input = Input::new(false);
        let shift = Modifiers {
            shift: true,
            ..Default::default()
        };
        input.modifiers_changed(shift, false);
        let out = input.key_down(&key("!", Some("!")), false);
        assert_eq!(
            out.sends,
            vec![
                Send::Key {
                    keysym: 0x21,
                    down: true
                },
                Send::Key {
                    keysym: 0x21,
                    down: false
                }
            ]
        );
        // A composed character too.
        let out = input.key_down(&key("eacute", Some("é")), false);
        assert_eq!(out.sends.len(), 2);
        // But a shifted letter is held: its name does not change.
        let out = input.key_down(&key("a", Some("A")), false);
        assert_eq!(out.sends.len(), 1);
        assert_eq!(input.key_up("a").len(), 1);
    }

    #[test]
    fn releases_everything_still_held_when_focus_is_lost() {
        let mut input = Input::new(false);
        let shift = Modifiers {
            shift: true,
            ..Default::default()
        };
        input.modifiers_changed(shift, false);
        input.key_down(&key("a", Some("A")), false);
        let ups = input.blurred();
        assert!(
            ups.iter()
                .all(|s| matches!(s, Send::Key { down: false, .. }))
        );
        let syms: Vec<u32> = ups
            .iter()
            .filter_map(|s| match *s {
                Send::Key { keysym, .. } => Some(keysym),
                _ => None,
            })
            .collect();
        assert!(syms.contains(&0x41));
        assert!(syms.contains(&0xffe1));
    }

    #[test]
    fn hands_the_keyboard_back_on_the_escape_combination() {
        let mut input = Input::new(false);
        let (_, release) = input.modifiers_changed(
            Modifiers {
                control: true,
                alt: true,
                shift: true,
                ..Default::default()
            },
            false,
        );
        assert!(release);
        // Even in view-only mode: it is the way out, not input.
        let mut input = Input::new(true);
        let (_, release) = input.modifiers_changed(
            Modifiers {
                control: true,
                alt: true,
                shift: true,
                ..Default::default()
            },
            false,
        );
        assert!(release);
    }

    #[test]
    fn modifiers_go_down_and_up_once() {
        let mut input = Input::new(false);
        let ctrl = Modifiers {
            control: true,
            ..Default::default()
        };
        let (down, _) = input.modifiers_changed(ctrl, false);
        assert_eq!(
            down,
            vec![Send::Key {
                keysym: CONTROL_L,
                down: true
            }]
        );
        let (up, _) = input.modifiers_changed(Modifiers::default(), false);
        assert_eq!(
            up,
            vec![Send::Key {
                keysym: CONTROL_L,
                down: false
            }]
        );
    }

    #[test]
    fn an_altgr_character_takes_back_the_ctrl_alt_it_sent() {
        let mut input = Input::new(false);
        let ctrl_alt = Modifiers {
            control: true,
            alt: true,
            ..Default::default()
        };
        input.modifiers_changed(ctrl_alt, false);
        let out = input.key_down(
            &Keystroke {
                modifiers: ctrl_alt,
                key: "q".into(),
                key_char: Some("@".into()),
            },
            true,
        );
        assert_eq!(
            out.sends,
            vec![
                Send::Key {
                    keysym: CONTROL_L,
                    down: false
                },
                Send::Key {
                    keysym: ALT_L,
                    down: false
                },
                Send::Key {
                    keysym: 0x40,
                    down: true
                },
            ]
        );
        // The platform's own release of Ctrl and Alt then sends nothing more.
        let (up, _) = input.modifiers_changed(Modifiers::default(), false);
        assert!(up.is_empty());
    }

    #[test]
    fn caps_lock_toggles_follow_the_state() {
        let mut input = Input::new(false);
        let (first, _) = input.modifiers_changed(Modifiers::default(), false);
        assert!(first.is_empty(), "the first report only learns the state");
        let (toggled, _) = input.modifiers_changed(Modifiers::default(), true);
        assert_eq!(
            toggled,
            vec![
                Send::Key {
                    keysym: CAPS_LOCK,
                    down: true
                },
                Send::Key {
                    keysym: CAPS_LOCK,
                    down: false
                }
            ]
        );
    }

    #[test]
    fn input_method_text_is_typed_a_character_at_a_time() {
        let mut input = Input::new(false);
        assert_eq!(
            input.text("é中"),
            vec![
                Send::Key {
                    keysym: 0xe9,
                    down: true
                },
                Send::Key {
                    keysym: 0xe9,
                    down: false
                },
                Send::Key {
                    keysym: 0x0100_4e2d,
                    down: true
                },
                Send::Key {
                    keysym: 0x0100_4e2d,
                    down: false
                },
            ]
        );
    }
}
