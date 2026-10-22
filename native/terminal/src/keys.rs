//! Keystrokes to the bytes a terminal program expects (xterm conventions).
//!
//! Only keys that are not plain text are encoded here. Printable characters,
//! with or without Shift, are left to GPUI's input handler so IME composition,
//! dead keys and AltGr layouts reach the terminal as the text they produce.

use gpui_kit::Keystroke;

/// Terminal modes that change what a key sends.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct KeyModes {
    /// DECCKM: cursor keys send `ESC O x` instead of `ESC [ x`.
    pub app_cursor: bool,
    /// Alt (Option on macOS) prefixes the key with ESC. On macOS Option
    /// normally types characters such as `ß`, so the caller turns this off
    /// there unless the user asked for Option-as-Meta.
    pub alt_is_meta: bool,
}

/// The xterm modifier parameter: 1 + Shift(1) + Alt(2) + Ctrl(4).
fn modifier_param(ks: &Keystroke) -> u8 {
    let m = &ks.modifiers;
    1 + m.shift as u8 + (m.alt as u8) * 2 + (m.control as u8) * 4
}

/// `ESC [ 1 ; m x` with modifiers, else `ESC [ x` (or `ESC O x` in
/// application mode, which xterm only applies to unmodified keys).
fn cursor_key(final_byte: u8, ks: &Keystroke, app: bool) -> Vec<u8> {
    let m = modifier_param(ks);
    if m > 1 {
        format!("\x1b[1;{m}{}", final_byte as char).into_bytes()
    } else if app {
        vec![0x1b, b'O', final_byte]
    } else {
        vec![0x1b, b'[', final_byte]
    }
}

/// `ESC [ n ~`, or `ESC [ n ; m ~` with modifiers.
fn tilde_key(n: u8, ks: &Keystroke) -> Vec<u8> {
    let m = modifier_param(ks);
    if m > 1 {
        format!("\x1b[{n};{m}~").into_bytes()
    } else {
        format!("\x1b[{n}~").into_bytes()
    }
}

/// F1-F4 are SS3 keys unmodified and CSI keys with modifiers.
fn ss3_function_key(final_byte: u8, ks: &Keystroke) -> Vec<u8> {
    let m = modifier_param(ks);
    if m > 1 {
        format!("\x1b[1;{m}{}", final_byte as char).into_bytes()
    } else {
        vec![0x1b, b'O', final_byte]
    }
}

/// The C0 control code Ctrl+`key` produces, if any.
fn control_code(key: &str) -> Option<u8> {
    let mut chars = key.chars();
    let c = chars.next()?;
    if chars.next().is_some() {
        return match key {
            "space" => Some(0x00),
            _ => None,
        };
    }
    match c {
        'a'..='z' => Some(c as u8 - b'a' + 1),
        '@' | '2' => Some(0x00),
        '[' | '3' => Some(0x1b),
        '\\' | '4' => Some(0x1c),
        ']' | '5' => Some(0x1d),
        '^' | '6' => Some(0x1e),
        '_' | '-' | '/' | '7' => Some(0x1f),
        '8' | '?' => Some(0x7f),
        _ => None,
    }
}

/// Prefix with ESC when Alt acts as Meta.
fn meta(alt: bool, mut bytes: Vec<u8>) -> Vec<u8> {
    if alt {
        bytes.insert(0, 0x1b);
    }
    bytes
}

/// Whether `ks` is an AltGr character that must go to the input handler as
/// text instead of being encoded. Windows reports AltGr as Ctrl+Alt, so
/// German AltGr+Q (`@`) would otherwise reach `encode_keystroke` as Ctrl+Alt+Q
/// and send ESC ^Q. GPUI marks such keys with `prefer_character_input`; the
/// second test catches the same shape when that flag is not set: Ctrl and Alt
/// held and a printable character that is not just the key itself, which a
/// genuine Ctrl+Alt+letter (Meta+Ctrl) never produces.
pub fn is_altgr_text(ks: &Keystroke, prefer_character_input: bool) -> bool {
    let m = &ks.modifiers;
    if m.platform {
        return false;
    }
    let printable = ks
        .key_char
        .as_deref()
        .is_some_and(|c| !c.is_empty() && !c.chars().any(char::is_control));
    if !printable {
        return false;
    }
    if prefer_character_input {
        return true;
    }
    let produced = ks.key_char.as_deref().unwrap_or_default();
    m.control && m.alt && !produced.eq_ignore_ascii_case(&ks.key)
}

/// The bytes for `ks`, or `None` when the key is ordinary text (left to the
/// input handler) or belongs to the application (Cmd/Super shortcuts).
pub fn encode_keystroke(ks: &Keystroke, modes: KeyModes) -> Option<Vec<u8>> {
    let m = &ks.modifiers;
    if m.platform {
        return None;
    }
    let alt = m.alt && modes.alt_is_meta;
    let key = ks.key.as_str();

    let special = match key {
        "up" => Some(cursor_key(b'A', ks, modes.app_cursor)),
        "down" => Some(cursor_key(b'B', ks, modes.app_cursor)),
        "right" => Some(cursor_key(b'C', ks, modes.app_cursor)),
        "left" => Some(cursor_key(b'D', ks, modes.app_cursor)),
        "home" => Some(cursor_key(b'H', ks, modes.app_cursor)),
        "end" => Some(cursor_key(b'F', ks, modes.app_cursor)),
        "insert" => Some(tilde_key(2, ks)),
        "delete" => Some(tilde_key(3, ks)),
        "pageup" => Some(tilde_key(5, ks)),
        "pagedown" => Some(tilde_key(6, ks)),
        "f1" => Some(ss3_function_key(b'P', ks)),
        "f2" => Some(ss3_function_key(b'Q', ks)),
        "f3" => Some(ss3_function_key(b'R', ks)),
        "f4" => Some(ss3_function_key(b'S', ks)),
        "f5" => Some(tilde_key(15, ks)),
        "f6" => Some(tilde_key(17, ks)),
        "f7" => Some(tilde_key(18, ks)),
        "f8" => Some(tilde_key(19, ks)),
        "f9" => Some(tilde_key(20, ks)),
        "f10" => Some(tilde_key(21, ks)),
        "f11" => Some(tilde_key(23, ks)),
        "f12" => Some(tilde_key(24, ks)),
        "enter" => Some(meta(alt, vec![b'\r'])),
        "escape" => Some(meta(alt, vec![0x1b])),
        "tab" if m.shift => Some(b"\x1b[Z".to_vec()),
        "tab" => Some(meta(alt, vec![b'\t'])),
        // Ctrl+Backspace sends ^H, which shells bind to delete-word; plain
        // Backspace sends DEL, as every modern terminal does.
        "backspace" if m.control => Some(meta(alt, vec![0x08])),
        "backspace" => Some(meta(alt, vec![0x7f])),
        _ => None,
    };
    if special.is_some() {
        return special;
    }

    if m.control {
        // Ctrl+Shift+letter is the same control code: terminals cannot see
        // Shift on C0 codes.
        return control_code(key).map(|code| meta(alt, vec![code]));
    }

    if alt {
        // Alt+key as Meta: ESC then the character the key would type.
        let text = match key {
            "space" => " ".to_string(),
            _ => ks
                .key_char
                .clone()
                .filter(|c| !c.is_empty())
                .unwrap_or_else(|| key.to_string()),
        };
        if text.chars().count() == 1 {
            return Some(meta(true, text.into_bytes()));
        }
    }

    None
}

/// Whether `ks` is the copy shortcut: Cmd+C on macOS, Ctrl+Shift+C elsewhere
/// (Ctrl+C has to stay SIGINT).
pub fn is_copy(ks: &Keystroke) -> bool {
    let m = &ks.modifiers;
    if cfg!(target_os = "macos") {
        m.platform && !m.control && !m.alt && ks.key == "c"
    } else {
        (m.control && m.shift && !m.alt && ks.key == "c")
            || (m.control && !m.shift && ks.key == "insert")
    }
}

/// Whether `ks` is the paste shortcut: Cmd+V on macOS, Ctrl+Shift+V or
/// Shift+Insert elsewhere.
pub fn is_paste(ks: &Keystroke) -> bool {
    let m = &ks.modifiers;
    if cfg!(target_os = "macos") {
        m.platform && !m.control && !m.alt && ks.key == "v"
    } else {
        (m.control && m.shift && !m.alt && ks.key == "v")
            || (m.shift && !m.control && ks.key == "insert")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn enc(s: &str) -> Option<Vec<u8>> {
        enc_with(
            s,
            KeyModes {
                app_cursor: false,
                alt_is_meta: true,
            },
        )
    }

    fn enc_with(s: &str, modes: KeyModes) -> Option<Vec<u8>> {
        encode_keystroke(&Keystroke::parse(s).unwrap(), modes)
    }

    #[test]
    fn arrows_follow_cursor_mode() {
        assert_eq!(enc("up").unwrap(), b"\x1b[A");
        assert_eq!(enc("left").unwrap(), b"\x1b[D");
        let app = KeyModes {
            app_cursor: true,
            alt_is_meta: true,
        };
        assert_eq!(enc_with("up", app).unwrap(), b"\x1bOA");
        assert_eq!(enc_with("end", app).unwrap(), b"\x1bOF");
        // Modified cursor keys are CSI in either mode.
        assert_eq!(enc_with("ctrl-right", app).unwrap(), b"\x1b[1;5C");
        assert_eq!(enc("shift-up").unwrap(), b"\x1b[1;2A");
        assert_eq!(enc("alt-left").unwrap(), b"\x1b[1;3D");
    }

    #[test]
    fn navigation_keys() {
        assert_eq!(enc("home").unwrap(), b"\x1b[H");
        assert_eq!(enc("end").unwrap(), b"\x1b[F");
        assert_eq!(enc("pageup").unwrap(), b"\x1b[5~");
        assert_eq!(enc("pagedown").unwrap(), b"\x1b[6~");
        assert_eq!(enc("insert").unwrap(), b"\x1b[2~");
        assert_eq!(enc("delete").unwrap(), b"\x1b[3~");
        assert_eq!(enc("ctrl-delete").unwrap(), b"\x1b[3;5~");
    }

    #[test]
    fn function_keys() {
        assert_eq!(enc("f1").unwrap(), b"\x1bOP");
        assert_eq!(enc("f4").unwrap(), b"\x1bOS");
        assert_eq!(enc("shift-f1").unwrap(), b"\x1b[1;2P");
        assert_eq!(enc("f5").unwrap(), b"\x1b[15~");
        assert_eq!(enc("f10").unwrap(), b"\x1b[21~");
        assert_eq!(enc("f11").unwrap(), b"\x1b[23~");
        assert_eq!(enc("f12").unwrap(), b"\x1b[24~");
        assert_eq!(enc("ctrl-f12").unwrap(), b"\x1b[24;5~");
    }

    #[test]
    fn editing_keys() {
        assert_eq!(enc("enter").unwrap(), b"\r");
        assert_eq!(enc("alt-enter").unwrap(), b"\x1b\r");
        assert_eq!(enc("backspace").unwrap(), b"\x7f");
        assert_eq!(enc("ctrl-backspace").unwrap(), b"\x08");
        assert_eq!(enc("alt-backspace").unwrap(), b"\x1b\x7f");
        assert_eq!(enc("tab").unwrap(), b"\t");
        assert_eq!(enc("shift-tab").unwrap(), b"\x1b[Z");
        assert_eq!(enc("escape").unwrap(), b"\x1b");
    }

    #[test]
    fn control_combos() {
        assert_eq!(enc("ctrl-c").unwrap(), [0x03]);
        assert_eq!(enc("ctrl-a").unwrap(), [0x01]);
        assert_eq!(enc("ctrl-z").unwrap(), [0x1a]);
        assert_eq!(enc("ctrl-[").unwrap(), [0x1b]);
        assert_eq!(enc("ctrl-\\").unwrap(), [0x1c]);
        assert_eq!(enc("ctrl-space").unwrap(), [0x00]);
        assert_eq!(enc("ctrl-/").unwrap(), [0x1f]);
        assert_eq!(enc("ctrl-alt-x").unwrap(), [0x1b, 0x18]);
    }

    #[test]
    fn alt_is_meta() {
        assert_eq!(enc("alt-b").unwrap(), b"\x1bb");
        assert_eq!(enc("alt-.").unwrap(), b"\x1b.");
        // Option on macOS types characters unless Option-as-Meta is on.
        let mac = KeyModes {
            app_cursor: false,
            alt_is_meta: false,
        };
        assert_eq!(enc_with("alt-b", mac), None);
    }

    #[test]
    fn plain_text_is_left_to_the_input_handler() {
        assert_eq!(enc("a"), None);
        assert_eq!(enc("shift-a"), None);
        assert_eq!(enc("space"), None);
        assert_eq!(enc("cmd-k"), None);
    }

    #[test]
    fn altgr_characters_are_text() {
        // Windows: German AltGr+Q arrives as ctrl-alt-q typing '@'.
        let altgr_q = Keystroke {
            key_char: Some("@".into()),
            ..Keystroke::parse("ctrl-alt-q").unwrap()
        };
        assert!(is_altgr_text(&altgr_q, true));
        assert!(is_altgr_text(&altgr_q, false));
        let altgr_7 = Keystroke {
            key_char: Some("{".into()),
            ..Keystroke::parse("ctrl-alt-7").unwrap()
        };
        assert!(is_altgr_text(&altgr_7, false));
        // A real Ctrl+Alt+letter still encodes as Meta+Ctrl.
        let ctrl_alt_x = Keystroke {
            key_char: Some("x".into()),
            ..Keystroke::parse("ctrl-alt-x").unwrap()
        };
        assert!(!is_altgr_text(&ctrl_alt_x, false));
        assert!(!is_altgr_text(
            &Keystroke::parse("ctrl-alt-x").unwrap(),
            false
        ));
        assert!(!is_altgr_text(&Keystroke::parse("ctrl-c").unwrap(), false));
        // No printable character: nothing for the input handler to insert.
        assert!(!is_altgr_text(&Keystroke::parse("up").unwrap(), true));
    }

    #[test]
    fn clipboard_shortcuts() {
        let ks = |s: &str| Keystroke::parse(s).unwrap();
        if cfg!(target_os = "macos") {
            assert!(is_copy(&ks("cmd-c")));
            assert!(is_paste(&ks("cmd-v")));
        } else {
            assert!(is_copy(&ks("ctrl-shift-c")));
            assert!(!is_copy(&ks("ctrl-c")));
            assert!(is_paste(&ks("ctrl-shift-v")));
            assert!(is_paste(&ks("shift-insert")));
            assert!(!is_paste(&ks("ctrl-v")));
        }
    }
}
