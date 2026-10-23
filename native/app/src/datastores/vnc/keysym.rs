//! GPUI keystrokes to X11 keysyms. Port of `src/utils/keysym.js`.
//!
//! The table is only for keys that produce no character. Every printable
//! character follows two rules, which is why there is no Unicode table:
//! Latin-1 (U+0020-U+00FF) keysyms *are* the code points, and everything else
//! is `0x01000000 + code point`.
//!
//! GPUI differs from the DOM in two ways that matter here. Modifier keys
//! never arrive as key presses, only as a changed set of held modifiers
//! (`ModifiersChangedEvent`), with no left or right; so this app sends the
//! left-hand keysym for each. And keypad keys share their names with the main
//! row, so the keypad's own keysyms cannot be told apart (the Vue table
//! could, from `event.code`).

use gpui_kit::{Keystroke, Modifiers};

pub const SHIFT_L: u32 = 0xffe1;
pub const CONTROL_L: u32 = 0xffe3;
pub const CAPS_LOCK: u32 = 0xffe5;
pub const ALT_L: u32 = 0xffe9;
pub const META_L: u32 = 0xffeb;
pub const DELETE: u32 = 0xffff;

/// Keysyms for the modifier keys, released unconditionally on blur.
pub const MODIFIER_KEYSYMS: [u32; 9] = [
    0xffe1, 0xffe2, 0xffe3, 0xffe4, 0xffe9, 0xffea, 0xffeb, 0xffec, 0xffe5,
];

/// Ctrl+Alt+Del, which no window system will deliver as a key press.
pub const CTRL_ALT_DELETE: [u32; 3] = [CONTROL_L, ALT_L, DELETE];

/// A GPUI key name to the keysym of a key that produces no character.
/// Names are GPUI's (`keystroke_from_xkb` on Linux; lower-cased xkb names
/// for the rest).
pub fn named_keysym(key: &str) -> Option<u32> {
    let sym = match key {
        "escape" => 0xff1b,
        "tab" => 0xff09,
        "backspace" => 0xff08,
        "enter" => 0xff0d,
        "insert" => 0xff63,
        "delete" => DELETE,
        "home" => 0xff50,
        "end" => 0xff57,
        "pageup" => 0xff55,
        "pagedown" => 0xff56,
        "left" => 0xff51,
        "up" => 0xff52,
        "right" => 0xff53,
        "down" => 0xff54,
        "menu" => 0xff67,
        "num_lock" => 0xff7f,
        "scroll_lock" => 0xff14,
        "pause" => 0xff13,
        "print" => 0xff61,
        "caps_lock" => CAPS_LOCK,
        // Keypad operators keep their own names after `kp_` is dropped.
        "divide" => 0xffaf,
        "multiply" => 0xffaa,
        "subtract" => 0xffad,
        "add" => 0xffab,
        "decimal" => 0xffae,
        "space" => 0x20,
        _ => return function_key(key),
    };
    Some(sym)
}

/// F1-F35 run contiguously from 0xFFBE.
fn function_key(key: &str) -> Option<u32> {
    let n: u32 = key.strip_prefix('f')?.parse().ok()?;
    (1..=35).contains(&n).then(|| 0xffbe + n - 1)
}

/// The keysym for one character: the two rules.
pub fn char_keysym(c: char) -> u32 {
    let cp = c as u32;
    if (0x20..=0xff).contains(&cp) {
        cp
    } else {
        0x0100_0000 + cp
    }
}

/// The one printable character of `text`, if it is exactly one.
fn single_char(text: &str) -> Option<char> {
    let mut chars = text.chars();
    let c = chars.next()?;
    (chars.next().is_none() && !c.is_control()).then_some(c)
}

/// Where a key press's keysym came from, which decides how it is sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeySource {
    /// A key with no character (Enter, an arrow, F5).
    Named,
    /// The character the key typed (`key_char`, with the layout and Shift
    /// already applied, which is the symbol the remote should get).
    Typed,
    /// The key's own name, when it typed nothing: a letter with Ctrl held.
    Bare,
}

/// The keysym for a key press, or `None` for a key with no symbol (a dead
/// key mid-composition, a media key): sending anything there would be a
/// guess.
pub fn keystroke_keysym(ks: &Keystroke) -> Option<(u32, KeySource)> {
    if let Some(sym) = named_keysym(&ks.key) {
        return Some((sym, KeySource::Named));
    }
    if let Some(c) = ks.key_char.as_deref().and_then(single_char) {
        return Some((char_keysym(c), KeySource::Typed));
    }
    single_char(&ks.key).map(|c| (char_keysym(c), KeySource::Bare))
}

/// The modifier keysyms to press and release going from `old` to `new`, as
/// `(keysym, down)`.
pub fn modifier_changes(old: Modifiers, new: Modifiers) -> Vec<(u32, bool)> {
    [
        (old.control, new.control, CONTROL_L),
        (old.shift, new.shift, SHIFT_L),
        (old.alt, new.alt, ALT_L),
        (old.platform, new.platform, META_L),
    ]
    .into_iter()
    .filter(|(was, is, _)| was != is)
    .map(|(_, is, sym)| (sym, is))
    .collect()
}

/// Whether a key press is an AltGr character. Windows reports AltGr as
/// Ctrl+Alt, so German AltGr+Q (`@`) arrives with both held; GPUI marks such
/// keys `prefer_character_input`, and the second test catches the same
/// shape without the flag (a genuine Ctrl+Alt+letter never types anything
/// but the letter). The Vue panel spotted the same thing from the 20 ms gap
/// between `ControlLeft` and `AltRight`; GPUI sends no separate events.
pub fn is_altgr_text(ks: &Keystroke, prefer_character_input: bool) -> bool {
    let m = &ks.modifiers;
    if m.platform {
        return false;
    }
    let Some(produced) = ks.key_char.as_deref().filter(|c| single_char(c).is_some()) else {
        return false;
    };
    if prefer_character_input {
        return true;
    }
    m.control && m.alt && !produced.eq_ignore_ascii_case(&ks.key)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ks(key: &str, key_char: Option<&str>) -> Keystroke {
        Keystroke {
            modifiers: Modifiers::default(),
            key: key.into(),
            key_char: key_char.map(Into::into),
        }
    }

    fn sym(key: &str, key_char: Option<&str>) -> Option<u32> {
        keystroke_keysym(&ks(key, key_char)).map(|(s, _)| s)
    }

    #[test]
    fn maps_ascii_characters_to_their_own_code_points() {
        assert_eq!(sym("a", Some("a")), Some(0x61));
        assert_eq!(sym("a", Some("A")), Some(0x41));
        assert_eq!(sym("1", Some("1")), Some(0x31));
        assert_eq!(sym("!", Some("!")), Some(0x21));
        assert_eq!(sym("space", Some(" ")), Some(0x20));
    }

    #[test]
    fn reads_the_typed_character_not_the_key_name() {
        // On a French layout the key labelled Q types "a".
        assert_eq!(sym("q", Some("a")), Some(0x61));
    }

    #[test]
    fn maps_latin_1_directly_and_the_rest_into_the_unicode_range() {
        assert_eq!(sym("e", Some("é")), Some(0xe9));
        assert_eq!(sym("y", Some("ÿ")), Some(0xff));
        assert_eq!(sym("e", Some("€")), Some(0x0100_20ac));
        assert_eq!(sym("a", Some("中")), Some(0x0100_4e2d));
        assert_eq!(sym("q", Some("й")), Some(0x0100_0439));
        assert_eq!(sym("a", Some("\u{1f600}")), Some(0x0101_f600));
    }

    #[test]
    fn maps_named_keys() {
        assert_eq!(sym("enter", None), Some(0xff0d));
        assert_eq!(sym("escape", None), Some(0xff1b));
        assert_eq!(sym("backspace", None), Some(0xff08));
        assert_eq!(sym("up", None), Some(0xff52));
        assert_eq!(sym("delete", None), Some(0xffff));
        assert_eq!(sym("home", None), Some(0xff50));
        // Named even though Enter and Tab carry a key_char on some platforms.
        assert_eq!(sym("tab", Some("\t")), Some(0xff09));
    }

    #[test]
    fn numbers_the_function_keys_contiguously_from_f1() {
        assert_eq!(sym("f1", None), Some(0xffbe));
        assert_eq!(sym("f11", None), Some(0xffc8));
        assert_eq!(sym("f12", None), Some(0xffc9));
        assert_eq!(sym("f36", None), None);
        assert_eq!(sym("fx", None), None);
    }

    #[test]
    fn a_ctrl_letter_sends_the_letter() {
        // Ctrl+C types no character; the remote still needs the C.
        let k = Keystroke {
            modifiers: Modifiers {
                control: true,
                ..Default::default()
            },
            key: "c".into(),
            key_char: None,
        };
        assert_eq!(keystroke_keysym(&k), Some((0x63, KeySource::Bare)));
    }

    #[test]
    fn refuses_to_guess_at_a_key_it_has_no_symbol_for() {
        assert_eq!(sym("dead_acute", None), None);
        assert_eq!(sym("xf86audioraisevolume", None), None);
        assert_eq!(sym("", None), None);
    }

    #[test]
    fn the_sources_are_told_apart() {
        assert_eq!(
            keystroke_keysym(&ks("left", None)).map(|x| x.1),
            Some(KeySource::Named)
        );
        assert_eq!(
            keystroke_keysym(&ks("eacute", Some("é"))).map(|x| x.1),
            Some(KeySource::Typed)
        );
    }

    #[test]
    fn modifier_changes_press_and_release_the_left_hand_keys() {
        let ctrl = Modifiers {
            control: true,
            ..Default::default()
        };
        let ctrl_shift = Modifiers {
            control: true,
            shift: true,
            ..Default::default()
        };
        assert_eq!(
            modifier_changes(Modifiers::default(), ctrl),
            vec![(CONTROL_L, true)]
        );
        assert_eq!(modifier_changes(ctrl, ctrl_shift), vec![(SHIFT_L, true)]);
        assert_eq!(
            modifier_changes(ctrl_shift, Modifiers::default()),
            vec![(CONTROL_L, false), (SHIFT_L, false)]
        );
        // Every one of them is in the release-all list.
        for (sym, _) in modifier_changes(
            Modifiers::default(),
            Modifiers {
                control: true,
                alt: true,
                shift: true,
                platform: true,
                function: false,
            },
        ) {
            assert!(MODIFIER_KEYSYMS.contains(&sym));
        }
    }

    #[test]
    fn spells_ctrl_alt_del_from_the_same_keysyms() {
        assert_eq!(
            CTRL_ALT_DELETE,
            [0xffe3, 0xffe9, sym("delete", None).unwrap()]
        );
    }

    #[test]
    fn spots_an_altgr_character() {
        let altgr_q = Keystroke {
            modifiers: Modifiers {
                control: true,
                alt: true,
                ..Default::default()
            },
            key: "q".into(),
            key_char: Some("@".into()),
        };
        assert!(is_altgr_text(&altgr_q, false));
        assert!(is_altgr_text(&altgr_q, true));
        let ctrl_alt_q = Keystroke {
            key_char: Some("q".into()),
            ..altgr_q.clone()
        };
        assert!(!is_altgr_text(&ctrl_alt_q, false));
        assert!(!is_altgr_text(&ks("left", None), true));
    }
}
