//! What every panel formats and draws the same way: ports of
//! `src/utils/format.js`, the SFTP panel's permission and date columns, the
//! security panel's "checked 5m ago", and `src/utils/listStyles.js`.
//!
//! Use these rather than a copy in a panel folder, so the panels keep one
//! look and one wording.

use chrono::{Datelike as _, Local, TimeZone as _};
use gpui_kit::{FontWeight, Hsla, Styled, px};

use crate::ui::theme::Palette;

// ---- numbers -------------------------------------------------------------

/// Human-readable byte count: "512 B", "1.5 KB", "2.0 MB", "1.0 GB".
/// `formatBytes`.
pub fn format_bytes(bytes: u64) -> String {
    const K: f64 = 1024.0;
    let b = bytes as f64;
    if b < K {
        format!("{bytes} B")
    } else if b < K * K {
        format!("{:.1} KB", b / K)
    } else if b < K * K * K {
        format!("{:.1} MB", b / (K * K))
    } else {
        format!("{:.1} GB", b / (K * K * K))
    }
}

/// Signed transfer rate; the unit follows the magnitude so a negative value
/// (a counter reset) keeps its sign: "-2.0 KB/s". `formatRate`.
pub fn format_rate(bytes_per_sec: f64) -> String {
    const K: f64 = 1024.0;
    let abs = bytes_per_sec.abs();
    if abs < K {
        format!("{bytes_per_sec:.0} B/s")
    } else if abs < K * K {
        format!("{:.1} KB/s", bytes_per_sec / K)
    } else {
        format!("{:.1} MB/s", bytes_per_sec / (K * K))
    }
}

/// Unsigned transfer rate, for values that are never negative (an upload's
/// speed). `formatSpeed`: differs from [`format_rate`] only below zero, where
/// it stays in "B/s".
pub fn format_speed(bytes_per_sec: f64) -> String {
    const K: f64 = 1024.0;
    if bytes_per_sec < K {
        format!("{bytes_per_sec:.0} B/s")
    } else if bytes_per_sec < K * K {
        format!("{:.1} KB/s", bytes_per_sec / K)
    } else {
        format!("{:.1} MB/s", bytes_per_sec / (K * K))
    }
}

/// A transfer's progress as a whole percentage, 0..=100. An empty file is
/// done as soon as it starts.
pub fn percent(done: u64, total: u64) -> u8 {
    if total == 0 {
        return 100;
    }
    ((done.min(total) as f64 / total as f64) * 100.0).round() as u8
}

// ---- files ---------------------------------------------------------------

/// `ls -l`'s mode column: "drwxr-xr-x". Unknown permissions (the server did
/// not say) read as ten dashes. The SFTP panel's `formatPermissions`.
pub fn format_permissions(perm: Option<u32>, is_dir: bool) -> String {
    let Some(perm) = perm else {
        return "----------".into();
    };
    let mut s = String::with_capacity(10);
    s.push(if is_dir { 'd' } else { '-' });
    for shift in [6u32, 3, 0] {
        let bits = (perm >> shift) & 0o7;
        s.push(if bits & 0o4 != 0 { 'r' } else { '-' });
        s.push(if bits & 0o2 != 0 { 'w' } else { '-' });
        s.push(if bits & 0o1 != 0 { 'x' } else { '-' });
    }
    s
}

const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// A calendar date as the SFTP list shows it: "Oct 23" within `this_year`,
/// "Oct 23, 2024" otherwise (`toLocaleDateString` in en-US, which is what
/// the Vue panel rendered on an English system).
pub fn format_civil_date(year: i32, month: u32, day: u32, this_year: i32) -> String {
    let m = MONTHS[(month.clamp(1, 12) - 1) as usize];
    if year == this_year {
        format!("{m} {day}")
    } else {
        format!("{m} {day}, {year}")
    }
}

/// A file's mtime (Unix seconds, as `SftpFile::modified` carries it) in the
/// machine's local time zone; "-" when the server gave none. The SFTP
/// panel's `formatDate`.
pub fn format_file_date(mtime: Option<u64>) -> String {
    let Some(secs) = mtime.filter(|s| *s != 0) else {
        return "-".into();
    };
    let Some(date) = i64::try_from(secs)
        .ok()
        .and_then(|s| Local.timestamp_opt(s, 0).single())
    else {
        return "-".into();
    };
    format_civil_date(date.year(), date.month(), date.day(), Local::now().year())
}

/// "just now", "12s ago", "5m ago", "3h ago": how long since something was
/// checked. The security panel's `timeAgo` (it never shows days).
pub fn format_elapsed(secs: u64) -> String {
    if secs < 5 {
        return "just now".into();
    }
    if secs < 60 {
        return format!("{secs}s ago");
    }
    let minutes = secs / 60;
    if minutes < 60 {
        return format!("{minutes}m ago");
    }
    format!("{}h ago", minutes / 60)
}

/// The last component of a remote path ("/" stays "/").
pub fn base_name(path: &str) -> &str {
    let trimmed = path.trim_end_matches('/');
    if trimmed.is_empty() {
        return if path.is_empty() { "" } else { "/" };
    }
    trimmed.rsplit('/').next().unwrap_or(trimmed)
}

/// The directory a remote path is in: "/var/log/syslog" -> "/var/log",
/// "/etc" -> "/", "/" -> "/".
pub fn parent_dir(path: &str) -> String {
    let trimmed = path.trim_end_matches('/');
    match trimmed.rfind('/') {
        Some(0) | None => "/".into(),
        Some(i) => trimmed[..i].into(),
    }
}

/// A remote name as one line of plain text: each control character (a
/// newline, ESC, DEL, C1) becomes `?`, as `ls` shows it. A file name may hold
/// any of them, and drawn raw a newline breaks the row into two lines. Only
/// for display: the real name still addresses the file.
pub fn display_name(name: &str) -> String {
    name.chars()
        .map(|c| if c.is_control() { '?' } else { c })
        .collect()
}

/// `dir` joined with `name` without doubling the slash.
pub fn join_remote(dir: &str, name: &str) -> String {
    if dir.ends_with('/') {
        format!("{dir}{name}")
    } else {
        format!("{dir}/{name}")
    }
}

// ---- lists (`listStyles.js`) ---------------------------------------------
//
// Fifteen lists in the Vue app had fifteen header styles until listStyles.js
// gave them one. These are its values as GPUI styles: call them on the
// element and keep layout (widths, flex) at the call site.

/// The `text-2xs` tier: 10px on 13px leading. Never a bare size: a size with
/// no leading is what cost the Vue sidebar ~7px per row.
pub fn text_2xs<E: Styled>(e: E) -> E {
    e.text_size(px(10.)).line_height(px(13.))
}

/// The header row (`LIST_HEAD` + `LIST_HEAD_CELL`'s rule): uppercase micro
/// text on the surface tint, the strongest line in the list under it.
/// Uppercase the label itself; GPUI has no text-transform.
pub fn list_head<E: Styled>(e: E, p: &Palette) -> E {
    text_2xs(e)
        .font_weight(FontWeight::MEDIUM)
        .text_color(p.ink_2)
        .bg(p.surface)
        .px_2()
        .py_1()
        .border_b_1()
        .border_color(p.line)
}

/// The separator under a body row (`border-line/50`).
pub fn row_separator(p: &Palette) -> Hsla {
    p.line.opacity(0.5)
}

/// A body row: the half-strength separator, `raised` on hover when
/// `interactive`, and `selected` (never `raised`, which is hover, and never
/// `active`, which means the active tab) when selected. `rowClass`.
///
/// Hover needs an element with an id (`.id(...)`) to take effect; apply
/// this after `.id`.
pub fn list_row<E: Styled + gpui_kit::InteractiveElement>(
    e: E,
    p: &Palette,
    selected: bool,
    interactive: bool,
) -> E {
    let raised = p.raised;
    let sel = p.selected;
    let mut e = e.border_b_1().border_color(row_separator(p));
    if selected {
        e = e.bg(sel);
    }
    if interactive {
        e = e
            .cursor_pointer()
            .hover(move |s| if selected { s.bg(sel) } else { s.bg(raised) });
    }
    e
}

/// A body cell (`LIST_CELL`): xs text in ink.
pub fn list_cell<E: Styled>(e: E, p: &Palette) -> E {
    e.px_2().py_1().text_xs().text_color(p.ink)
}

/// A secondary cell (`LIST_CELL_MUTED`).
pub fn list_cell_muted<E: Styled>(e: E, p: &Palette) -> E {
    e.px_2().py_1().text_xs().text_color(p.ink_2)
}

/// A column of numbers (`NUM`): right-aligned. GPUI's default UI font has
/// no tabular-figures switch; numbers that must not jitter between rows
/// (sizes, rates) should also take a fixed width at the call site.
pub fn num<E: Styled>(e: E) -> E {
    e.flex().justify_end()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_name_keeps_one_line() {
        assert_eq!(display_name("plain name.txt"), "plain name.txt");
        assert_eq!(display_name("a b 'c\" d\nnew.txt"), "a b 'c\" d?new.txt");
        assert_eq!(
            display_name("\u{1b}[31mred\r\t\u{7f}\u{9b}"),
            "?[31mred????"
        );
        assert_eq!(display_name("..\\..\\x"), "..\\..\\x");
        assert_eq!(display_name("caf\u{e9} \u{1f600}"), "caf\u{e9} \u{1f600}");
    }

    #[test]
    fn bytes_match_format_js() {
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(512), "512 B");
        assert_eq!(format_bytes(1023), "1023 B");
        assert_eq!(format_bytes(1024), "1.0 KB");
        assert_eq!(format_bytes(1536), "1.5 KB");
        assert_eq!(format_bytes(2 * 1024 * 1024), "2.0 MB");
        assert_eq!(format_bytes(1024 * 1024 * 1024), "1.0 GB");
        assert_eq!(format_bytes(5 * 1024 * 1024 * 1024 * 1024), "5120.0 GB");
    }

    #[test]
    fn rates_keep_their_sign_and_speeds_stay_in_bytes_below_zero() {
        assert_eq!(format_rate(0.0), "0 B/s");
        assert_eq!(format_rate(1023.0), "1023 B/s");
        assert_eq!(format_rate(-2048.0), "-2.0 KB/s");
        assert_eq!(format_rate(3.0 * 1024.0 * 1024.0), "3.0 MB/s");
        assert_eq!(format_speed(1536.0), "1.5 KB/s");
        assert_eq!(format_speed(-2048.0), "-2048 B/s");
        assert_eq!(format_speed(5.0 * 1024.0 * 1024.0), "5.0 MB/s");
    }

    #[test]
    fn percent_is_clamped_and_an_empty_file_is_done() {
        assert_eq!(percent(0, 0), 100);
        assert_eq!(percent(1, 3), 33);
        assert_eq!(percent(2, 3), 67);
        assert_eq!(percent(10, 5), 100);
    }

    #[test]
    fn permissions_read_like_ls() {
        assert_eq!(format_permissions(Some(0o755), true), "drwxr-xr-x");
        assert_eq!(format_permissions(Some(0o100644), false), "-rw-r--r--");
        assert_eq!(format_permissions(Some(0o000), false), "----------");
        assert_eq!(format_permissions(Some(0o777), false), "-rwxrwxrwx");
        assert_eq!(format_permissions(None, true), "----------");
    }

    #[test]
    fn dates_drop_the_year_only_within_this_one() {
        assert_eq!(format_civil_date(2026, 10, 23, 2026), "Oct 23");
        assert_eq!(format_civil_date(2024, 1, 5, 2026), "Jan 5, 2024");
        assert_eq!(format_file_date(None), "-");
        assert_eq!(format_file_date(Some(0)), "-");
        assert_ne!(format_file_date(Some(1_700_000_000)), "-");
    }

    #[test]
    fn elapsed_matches_the_security_panel() {
        assert_eq!(format_elapsed(0), "just now");
        assert_eq!(format_elapsed(4), "just now");
        assert_eq!(format_elapsed(5), "5s ago");
        assert_eq!(format_elapsed(59), "59s ago");
        assert_eq!(format_elapsed(60), "1m ago");
        assert_eq!(format_elapsed(3599), "59m ago");
        assert_eq!(format_elapsed(7200), "2h ago");
    }

    #[test]
    fn remote_paths_split_and_join() {
        assert_eq!(base_name("/var/log/syslog"), "syslog");
        assert_eq!(base_name("/var/log/"), "log");
        assert_eq!(base_name("/"), "/");
        assert_eq!(parent_dir("/var/log/syslog"), "/var/log");
        assert_eq!(parent_dir("/etc"), "/");
        assert_eq!(parent_dir("/"), "/");
        assert_eq!(join_remote("/", "etc"), "/etc");
        assert_eq!(join_remote("/home/me", "a b"), "/home/me/a b");
    }
}
