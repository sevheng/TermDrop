//! Guards for text a panel types into a live shell. Port of
//! `src/utils/terminalInsert.js`.
//!
//! The security panel can put a remediation command at the user's prompt.
//! That shell is frequently root-capable, so the one thing that must never
//! happen is the app pressing Enter on the user's behalf:
//!
//! 1. The text is written without a trailing newline; the user reads it and
//!    submits it. [`strip_submit`] enforces that.
//! 2. Nothing containing a control character may be inserted at all. A
//!    newline in the middle would submit the part before it, and ESC can
//!    drive the terminal's own control sequences.
//!
//! The backend already restricts remediation commands to its own literals;
//! this is a second, independent check at the boundary that writes to the
//! shell. [`super::PanelContext::insert_command`] is the only caller.

use crate::app::tabs::Tab;

/// Longer than this is not reviewable at a glance, so it is not offered.
pub const MAX_INSERT_LENGTH: usize = 512;

/// C0, DEL and C1: newline, carriage return, tab, escape and friends. The
/// same set as the Vue guard's `/[\x00-\x1F\x7F-\x9F]/`.
fn is_control(c: char) -> bool {
    matches!(c, '\u{00}'..='\u{1F}' | '\u{7F}'..='\u{9F}')
}

/// Whether `text` is safe to type at a shell prompt for the user to review.
pub fn is_insertable_command(text: &str) -> bool {
    // Tested before trimming, on purpose: trimming a trailing newline away
    // and then calling the result safe would accept exactly the input this
    // guard exists to reject.
    if text.chars().any(is_control) {
        return false;
    }
    let trimmed = text.trim();
    // Length in UTF-16 units, as JavaScript's `length` counts.
    !trimmed.is_empty() && trimmed.encode_utf16().count() <= MAX_INSERT_LENGTH
}

/// The exact text to write. Never ends in a newline: the command lands at
/// the prompt and nothing runs until the user presses Enter.
pub fn strip_submit(text: &str) -> &str {
    text.trim()
}

/// Whether a tab can take typed text: a shell kind (never a datastore or VNC
/// tab) with a live session.
pub fn can_receive_command(tab: Option<&Tab>) -> bool {
    tab.is_some_and(|t| t.kind.accepts_commands() && t.connected)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::tab_kind::TabKind;

    #[test]
    fn rejects_any_control_character_even_a_trailing_newline() {
        for bad in [
            "ls\n",
            "ls\r",
            "ls\nrm -rf /",
            "a\tb",
            "\u{1b}[2J",
            "x\u{7f}",
            "x\u{9b}",
            "\0",
        ] {
            assert!(!is_insertable_command(bad), "{bad:?} was accepted");
        }
    }

    #[test]
    fn accepts_plain_commands_and_caps_their_length() {
        assert!(is_insertable_command("sudo apt-get upgrade"));
        assert!(is_insertable_command("  ss -tlnp  "));
        assert!(!is_insertable_command("   "));
        assert!(!is_insertable_command(""));
        assert!(is_insertable_command(&"a".repeat(MAX_INSERT_LENGTH)));
        assert!(!is_insertable_command(&"a".repeat(MAX_INSERT_LENGTH + 1)));
        // Surrounding spaces do not count against the cap.
        assert!(is_insertable_command(&format!(
            "  {}  ",
            "a".repeat(MAX_INSERT_LENGTH)
        )));
    }

    #[test]
    fn the_written_text_never_ends_in_a_newline() {
        assert_eq!(strip_submit("  echo hi  "), "echo hi");
    }

    #[test]
    fn only_a_connected_shell_receives_commands() {
        let mut ssh = Tab::new("a", Some(1), TabKind::Ssh, "a");
        assert!(!can_receive_command(Some(&ssh)), "not connected yet");
        ssh.connected = true;
        assert!(can_receive_command(Some(&ssh)));
        for kind in [TabKind::Mongodb, TabKind::Redis, TabKind::Vnc] {
            let t = Tab::new("b", Some(1), kind, "b");
            assert!(t.connected);
            assert!(!can_receive_command(Some(&t)), "{kind:?}");
        }
        assert!(!can_receive_command(None));
    }
}
