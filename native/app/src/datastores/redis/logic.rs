//! The Redis tab's pure parts: ports of `src/utils/redisKeys.js`,
//! `src/utils/redisBackup.js`, and the decisions `RedisPanel.vue`,
//! `RedisKeyList.vue`, `RedisKeyTree.vue` and `RedisValueView.vue` make
//! inline. Nothing here touches GPUI or the network, so all of it is tested.

use base64::Engine as _;
use termdrop_core::redis::{PrefixGroup, RedisBytes, RedisDbInfo, RedisOpSummary, RedisValuePage};
use termdrop_core::redis_backup::BackupHeader;

use crate::datastores::common::{format_bytes, format_count};
use crate::panels::common::display_name;
use crate::ui::toast::ToastKind;

/// The types the key list can filter by, in the Vue menu's order.
pub const KEY_TYPES: [&str; 6] = ["string", "list", "set", "zset", "hash", "stream"];

/// The databases the restore dialog offers (`dbOptions`: db0..db15).
pub const RESTORE_DB_COUNT: i64 = 16;

/// A cell of a value table is drawn at most this many bytes long. The core
/// returns hash, list and set members whole, and one element can be many
/// megabytes; laying that out on the UI thread would freeze the window. The
/// page itself is never trimmed, only the drawing of one cell, and the cell
/// says how much it left out.
pub const MAX_CELL_BYTES: usize = 16 * 1024;

/// The columns a tab takes when a string value is cut into display lines.
pub const TAB_COLS: usize = 4;

// ---- patterns --------------------------------------------------------------

/// Escape a literal for use inside a Redis glob pattern (`escapeGlob`).
///
/// Not optional: keys may contain `*`, `?`, `[`, `]` and `\`, and a prefix
/// group's pattern is built from a real key's bytes. Unescaped,
/// `report[2024]` would match nothing and `a*` half the keyspace.
pub fn escape_glob(literal: &str) -> String {
    let mut out = String::with_capacity(literal.len());
    for c in literal.chars() {
        if matches!(c, '\\' | '*' | '?' | '[' | ']') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// The MATCH pattern for one prefix group (`prefixPattern`).
pub fn prefix_pattern(prefix: &str) -> String {
    format!("{}:*", escape_glob(prefix))
}

// ---- SCAN paging -----------------------------------------------------------

/// A forward-only cursor stack (`createCursorStack`).
///
/// Redis cursors cannot be rewound and carry no page number, so going back
/// means remembering where each page started. Cursors stay strings: they are
/// u64 on the server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CursorStack {
    stack: Vec<String>,
}

impl Default for CursorStack {
    fn default() -> Self {
        Self {
            stack: vec!["0".into()],
        }
    }
}

impl CursorStack {
    /// The cursor the current page starts at.
    pub fn current(&self) -> &str {
        self.stack.last().map(String::as_str).unwrap_or("0")
    }

    /// Record the cursor the next page starts at.
    pub fn push(&mut self, cursor: impl Into<String>) {
        self.stack.push(cursor.into());
    }

    /// Step back one page, returning the cursor to load.
    pub fn back(&mut self) -> String {
        if self.stack.len() > 1 {
            self.stack.pop();
        }
        self.current().to_string()
    }

    pub fn can_go_back(&self) -> bool {
        self.stack.len() > 1
    }

    /// 1-based, for display.
    pub fn page_number(&self) -> usize {
        self.stack.len()
    }
}

/// How the scan's progress reads (`summarizeScan`). Says "of ~N" and never
/// claims completeness while the cursor is open: SCAN gives no total, and
/// the database's key count is only an estimate of what a filtered scan
/// finds.
pub fn summarize_scan(loaded: usize, total_keys: u64, done: bool) -> String {
    let loaded = loaded as u64;
    if done {
        return format!(
            "{} {}",
            format_count(loaded),
            if loaded == 1 { "key" } else { "keys" }
        );
    }
    if total_keys == 0 {
        return format!("{} so far", format_count(loaded));
    }
    format!("{} of ~{}", format_count(loaded), format_count(total_keys))
}

/// What the key list shows when a page holds no keys. SCAN adds a third
/// answer to "empty" and "filtered": this page may be empty while later ones
/// are not, which is why the hint and the "Next page" action matter here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmptyPage {
    pub filtered: bool,
    pub title: &'static str,
    pub hint: Option<&'static str>,
    /// The way out: "Next page" while the scan has more; once it is done,
    /// "Clear filter" for a filtered page and "Refresh" for an empty one
    /// (the Vue list offered nothing there; the native empty state always
    /// offers its fix).
    pub action: EmptyAction,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmptyAction {
    NextPage,
    ClearFilter,
    Refresh,
}

impl EmptyAction {
    pub fn label(self) -> &'static str {
        match self {
            Self::NextPage => "Next page",
            Self::ClearFilter => "Clear filter",
            Self::Refresh => "Refresh",
        }
    }
}

pub fn empty_page(pattern: &str, type_filter: &str, done: bool) -> EmptyPage {
    let filtered = !pattern.is_empty() || !type_filter.is_empty();
    EmptyPage {
        filtered,
        title: if filtered {
            "No keys match on this page"
        } else {
            "No keys on this page"
        },
        hint: (!done)
            .then_some("SCAN returns pages, not results \u{2014} there may be more further on"),
        action: match (done, filtered) {
            (false, _) => EmptyAction::NextPage,
            (true, true) => EmptyAction::ClearFilter,
            (true, false) => EmptyAction::Refresh,
        },
    }
}

// ---- keys and values -------------------------------------------------------

/// How a TTL reads (`formatTtl`). `-1` and `-2` are Redis's own sentinels,
/// and showing either as a duration would mislead.
pub fn format_ttl(ms: i64) -> String {
    if ms == -1 {
        return "no expiry".into();
    }
    if ms == -2 {
        return "expired".into();
    }
    if ms < 1000 {
        return format!("{ms}ms");
    }
    let total = ms / 1000;
    let days = total / 86400;
    let hours = (total % 86400) / 3600;
    let minutes = (total % 3600) / 60;
    let seconds = total % 60;
    let mut parts = Vec::new();
    if days > 0 {
        parts.push(format!("{days}d"));
    }
    if hours > 0 {
        parts.push(format!("{hours}h"));
    }
    if minutes > 0 {
        parts.push(format!("{minutes}m"));
    }
    // Seconds are noise once the value is big enough.
    if seconds > 0 && days == 0 && hours == 0 {
        parts.push(format!("{seconds}s"));
    }
    if parts.is_empty() {
        "0s".into()
    } else {
        parts.join(" ")
    }
}

/// A key type's display name (`formatKeyKind`).
pub fn format_key_kind(kind: &str) -> String {
    match kind {
        "string" => "String",
        "list" => "List",
        "set" => "Set",
        "zset" => "Sorted set",
        "hash" => "Hash",
        "stream" => "Stream",
        "none" => "Gone",
        "" => "Unknown",
        other => return other.to_string(),
    }
    .to_string()
}

/// Whether there is a viewer for this type (`isViewableKind`).
pub fn is_viewable_kind(kind: &str) -> bool {
    KEY_TYPES.contains(&kind)
}

/// A key as one line of text: `⟨binary N bytes⟩` when it is not UTF-8 (its
/// `text` is then base64, which is no name anyone knows the key by), and
/// otherwise its text with control characters shown as `?`. Display only:
/// the key is always addressed by its `key_b64`.
pub fn key_label(key: &RedisBytes) -> String {
    if key.binary {
        format!("\u{27e8}binary {} bytes\u{27e9}", key.bytes)
    } else {
        display_name(&key.text)
    }
}

/// The header of the value table's first column, if the type has one.
pub fn field_header(kind: &str) -> Option<&'static str> {
    match kind {
        "stream" => Some("ID"),
        "list" => Some("#"),
        "hash" => Some("Field"),
        _ => None,
    }
}

/// "1,234 bytes" for a string, "1,234 elements" otherwise.
pub fn total_label(page: &RedisValuePage) -> Option<String> {
    page.total.map(|t| {
        format!(
            "{} {}",
            format_count(t),
            if page.kind == "string" {
                "bytes"
            } else {
                "elements"
            }
        )
    })
}

/// The text Copy value may put on the clipboard: a string value that is
/// whole and text. A truncated value is only the first 256 KB preview and a
/// binary one is shown as base64, so copying either would hand over
/// something other than the value while claiming success.
pub fn copyable_value(page: &RedisValuePage) -> Option<&str> {
    if page.kind != "string" {
        return None;
    }
    let value = &page.entries.first()?.value;
    (!value.truncated && !value.binary).then_some(value.text.as_str())
}

/// The note above a value: how much of a truncated string is shown, or
/// that binary members are base64. Above the value, never under it: a
/// 256 KB preview is thousands of lines tall and a note at the bottom would
/// never be seen.
pub fn truncated_note(page: &RedisValuePage) -> Option<String> {
    if page.kind == "string"
        && let Some(value) = page.entries.first().map(|e| &e.value)
        && value.truncated
    {
        // Bytes, not characters: a multi-byte value would otherwise report
        // a smaller preview than it is. A binary preview's text is base64,
        // so count what it decodes to.
        let shown = if value.binary {
            base64::engine::general_purpose::STANDARD
                .decode(&value.text)
                .map(|b| b.len())
                .unwrap_or(value.text.len())
        } else {
            value.text.len()
        };
        return Some(format!(
            "Truncated \u{2014} showing the first {} of {}.",
            format_bytes(shown as u64),
            format_bytes(value.bytes)
        ));
    }
    let any_binary = page
        .entries
        .iter()
        .any(|e| e.value.binary || e.field.as_ref().is_some_and(|f| f.binary));
    any_binary.then(|| "Values that are not valid UTF-8 are shown as base64.".into())
}

/// How the next page of a value is asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MoreValue {
    /// Hashes and sets: `HSCAN`/`SSCAN` from this cursor.
    Cursor(String),
    /// Lists and sorted sets: an index window from this offset.
    Offset(i64),
}

/// Whether "Load more" is offered, and what it asks for. Hashes and sets
/// page by cursor until it comes back `"0"`; lists and sorted sets by index
/// until every element is shown. Streams (`XRANGE - + COUNT`) and strings
/// (one capped preview) have no next page.
///
/// The Vue view showed "Load more" on a list or sorted set even after the
/// last element, where it loaded an empty window; this stops at `total`.
pub fn more_value(page: &RedisValuePage) -> Option<MoreValue> {
    let shown = page.entries.len() as u64;
    match page.kind.as_str() {
        "hash" | "set" => (!page.done).then(|| MoreValue::Cursor(page.cursor.clone())),
        "list" | "zset" => page
            .total
            .is_none_or(|t| shown < t)
            .then_some(MoreValue::Offset(shown as i64)),
        _ => None,
    }
}

/// Add `next` to the page shown, as `loadValue(.., append)` did: the newer
/// page's cursor, total and TTL, and both pages' entries, untrimmed.
pub fn append_page(shown: RedisValuePage, next: RedisValuePage) -> RedisValuePage {
    let mut entries = shown.entries;
    entries.extend(next.entries);
    RedisValuePage { entries, ..next }
}

/// A cell's text, cut to [`MAX_CELL_BYTES`] on a character boundary, and how
/// many bytes were left out.
pub fn clip_cell(text: &str) -> (&str, usize) {
    if text.len() <= MAX_CELL_BYTES {
        return (text, 0);
    }
    let mut end = MAX_CELL_BYTES;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    (&text[..end], text.len() - end)
}

/// A string value split into the lines it is drawn as: at each newline, and
/// a line wider than `cols` monospace columns into pieces that fit (on
/// character boundaries; a wide character takes two columns, a combining
/// mark none). Joined back with newlines at the original breaks, the pieces
/// are the whole text.
///
/// The view wraps here rather than letting the text element wrap, as Vue's
/// `whitespace-pre-wrap break-all` did: every piece is then one row of the
/// same height, which is what lets a 256 KB preview be a virtualised list.
/// Left to GPUI, a column of long wrapping lines in a scroll container was
/// laid out one row high each and painted on top of one another.
pub fn display_lines(text: &str, cols: usize) -> Vec<String> {
    let cols = cols.max(1);
    let mut out = Vec::new();
    for line in text.split('\n') {
        let line = line.strip_suffix('\r').unwrap_or(line);
        let mut start = 0;
        let mut used = 0;
        for (at, ch) in line.char_indices() {
            let width = char_cols(ch);
            if used + width > cols && at > start {
                out.push(line[start..at].to_string());
                start = at;
                used = 0;
            }
            used += width;
        }
        out.push(line[start..].to_string());
    }
    out
}

/// The monospace columns one character takes.
fn char_cols(ch: char) -> usize {
    if ch == '\t' {
        return TAB_COLS;
    }
    unicode_width::UnicodeWidthChar::width(ch).unwrap_or(1)
}

// ---- databases and the tree ------------------------------------------------

/// The databases to list. A server with no keys reports none at all, but
/// db0 always exists.
pub fn databases_or_default(dbs: &[RedisDbInfo]) -> Vec<RedisDbInfo> {
    if dbs.is_empty() {
        vec![RedisDbInfo {
            index: 0,
            keys: 0,
            expires: 0,
        }]
    } else {
        dbs.to_vec()
    }
}

/// The key count `INFO keyspace` gave for `db`, or 0.
pub fn db_key_count(dbs: &[RedisDbInfo], db: i64) -> u64 {
    dbs.iter().find(|d| d.index == db).map_or(0, |d| d.keys)
}

/// One row of the keyspace tree, which is drawn as a virtual list: a flat
/// keyspace of 20,000 keys with no delimiter is 20,000 groups.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TreeRow {
    Db {
        index: i64,
        keys: u64,
        active: bool,
    },
    /// The active database's groups are being built.
    Grouping,
    /// The active database's group at this index in `KeyTree::groups`.
    Group(usize),
    /// The tree's scan was capped: these groups are part of the keyspace.
    Partial {
        scanned: u64,
    },
    /// The scan finished and found nothing.
    NoMatch,
}

/// The tree's rows: every database, and under the active one its groups
/// (or "Grouping keys…", and a closing note when partial or empty).
pub fn tree_rows(
    dbs: &[RedisDbInfo],
    active_db: i64,
    groups: &[PrefixGroup],
    grouping: bool,
    truncated: bool,
    scanned: u64,
) -> Vec<TreeRow> {
    let mut rows = Vec::new();
    for db in dbs {
        let active = db.index == active_db;
        rows.push(TreeRow::Db {
            index: db.index,
            keys: db.keys,
            active,
        });
        if !active {
            continue;
        }
        if grouping {
            rows.push(TreeRow::Grouping);
            continue;
        }
        rows.extend((0..groups.len()).map(TreeRow::Group));
        if truncated {
            rows.push(TreeRow::Partial { scanned });
        } else if groups.is_empty() {
            rows.push(TreeRow::NoMatch);
        }
    }
    rows
}

/// A group's label: its prefix, or `⟨binary⟩` when the prefix is not text.
pub fn group_label(group: &PrefixGroup) -> String {
    if group.binary {
        "\u{27e8}binary\u{27e9}".into()
    } else {
        display_name(&group.prefix)
    }
}

// ---- backup and restore ----------------------------------------------------

/// A file name that says what the backup holds (`defaultBackupName`): the
/// host, the database, the pattern and the local date.
pub fn default_backup_name(
    host_name: &str,
    db: i64,
    pattern: &str,
    date: (i32, u32, u32),
) -> String {
    let safe = slug(
        &host_name.to_lowercase(),
        |c| c.is_ascii_lowercase() || c.is_ascii_digit(),
        '-',
    );
    let safe = safe.trim_matches('-');
    let scope = if pattern.is_empty() {
        String::new()
    } else {
        format!("-{}", slug(pattern, |c| c.is_ascii_alphanumeric(), '_'))
    };
    let (y, m, d) = date;
    format!(
        "{}-db{db}{scope}-{y}-{m:02}-{d:02}.tdredis",
        if safe.is_empty() { "redis" } else { safe }
    )
}

/// Every run of characters `keep` refuses becomes one `sep`.
fn slug(text: &str, keep: impl Fn(char) -> bool, sep: char) -> String {
    let mut out = String::with_capacity(text.len());
    let mut in_run = false;
    for c in text.chars() {
        if keep(c) {
            out.push(c);
            in_run = false;
        } else if !in_run {
            out.push(sep);
            in_run = true;
        }
    }
    out
}

/// What to say when a backup or restore finishes (`summarizeOp`). Every
/// non-zero count is reported: a restore that skipped 12,000 existing keys
/// "succeeded", and saying only that would mislead. Any failed key makes it
/// a warning.
pub fn summarize_op(summary: &RedisOpSummary, verb: &str) -> (String, ToastKind) {
    let mut parts = vec![format!("{verb} {} keys", format_count(summary.written))];
    if summary.skipped_existing > 0 {
        parts.push(format!(
            "{} already existed and were left alone",
            format_count(summary.skipped_existing)
        ));
    }
    if !summary.skipped_big.is_empty() {
        parts.push(format!("{} too large to copy", summary.skipped_big.len()));
    }
    if summary.vanished > 0 {
        parts.push(format!(
            "{} expired while running",
            format_count(summary.vanished)
        ));
    }
    if !summary.failed.is_empty() {
        parts.push(format!("{} failed", summary.failed.len()));
    }
    let kind = if summary.failed.is_empty() {
        ToastKind::Success
    } else {
        ToastKind::Warning
    };
    (parts.join("; "), kind)
}

/// The toast for a backup or restore that ended in an error. The core's
/// cancel is the string `cancelled`; Vue tested for it with `includes`.
pub fn op_error_message(err: &str, noun: &str) -> (String, ToastKind) {
    if err.contains("cancelled") {
        (format!("{noun} cancelled"), ToastKind::Info)
    } else {
        (format!("{noun} failed: {err}"), ToastKind::Error)
    }
}

/// The warning before a restore, when the file came from another version
/// (`restoreWarning`). The core refuses an impossible restore itself.
pub fn restore_warning(header: &BackupHeader, server_version: &str) -> Option<String> {
    (!header.redis_version.is_empty()
        && !server_version.is_empty()
        && header.redis_version != server_version)
        .then(|| {
            format!(
                "This backup came from Redis {}; this server runs {server_version}.",
                header.redis_version
            )
        })
}

/// The database a restore targets first: the file's first, else the one
/// being browsed.
pub fn default_restore_db(header: &BackupHeader, active_db: i64) -> i64 {
    header.dbs.first().map_or(active_db, |d| i64::from(*d))
}

#[cfg(test)]
mod tests {
    use super::*;
    use termdrop_core::redis::{FailedKey, RedisEntry, SkippedKey};

    fn bytes(text: &str) -> RedisBytes {
        RedisBytes {
            text: text.into(),
            binary: false,
            bytes: text.len() as u64,
            truncated: false,
        }
    }

    fn page(kind: &str, n: usize, total: Option<u64>, cursor: &str) -> RedisValuePage {
        RedisValuePage {
            kind: kind.into(),
            entries: (0..n)
                .map(|i| RedisEntry {
                    field: None,
                    value: bytes(&i.to_string()),
                    score: None,
                })
                .collect(),
            total,
            ttl_ms: -1,
            encoding: None,
            cursor: cursor.into(),
            done: cursor == "0",
        }
    }

    #[test]
    fn glob_metacharacters_are_escaped() {
        assert_eq!(escape_glob("plain"), "plain");
        assert_eq!(escape_glob("report[2024]"), "report\\[2024\\]");
        assert_eq!(escape_glob("a*b?c\\d"), "a\\*b\\?c\\\\d");
        assert_eq!(prefix_pattern("user"), "user:*");
        assert_eq!(prefix_pattern("a*"), "a\\*:*");
    }

    #[test]
    fn the_cursor_stack_remembers_where_each_page_started() {
        let mut s = CursorStack::default();
        assert_eq!(
            (s.current(), s.page_number(), s.can_go_back()),
            ("0", 1, false)
        );
        s.push("17");
        s.push("18446744073709551615");
        assert_eq!(s.page_number(), 3);
        assert_eq!(s.back(), "17");
        assert_eq!(s.back(), "0");
        assert_eq!(s.back(), "0");
        assert!(!s.can_go_back());
    }

    #[test]
    fn scan_summaries_never_claim_a_total_they_do_not_have() {
        assert_eq!(summarize_scan(1, 50, true), "1 key");
        assert_eq!(summarize_scan(1200, 50, true), "1,200 keys");
        assert_eq!(summarize_scan(200, 0, false), "200 so far");
        assert_eq!(summarize_scan(200, 1_000_000, false), "200 of ~1,000,000");
    }

    #[test]
    fn an_empty_page_says_which_kind_of_empty_it_is() {
        let e = empty_page("", "", false);
        assert_eq!(
            (e.filtered, e.title, e.action),
            (false, "No keys on this page", EmptyAction::NextPage)
        );
        assert!(e.hint.is_some());
        let e = empty_page("user:*", "", true);
        assert_eq!(
            (e.filtered, e.title, e.hint, e.action),
            (
                true,
                "No keys match on this page",
                None,
                EmptyAction::ClearFilter
            )
        );
        assert!(empty_page("", "hash", true).filtered);
        assert_eq!(empty_page("", "", true).action, EmptyAction::Refresh);
    }

    #[test]
    fn ttls_read_like_the_vue_list() {
        assert_eq!(format_ttl(-1), "no expiry");
        assert_eq!(format_ttl(-2), "expired");
        assert_eq!(format_ttl(0), "0ms");
        assert_eq!(format_ttl(999), "999ms");
        assert_eq!(format_ttl(1000), "1s");
        assert_eq!(format_ttl(61_000), "1m 1s");
        assert_eq!(format_ttl(3_600_000 + 5_000), "1h");
        assert_eq!(format_ttl(90_061_000), "1d 1h 1m");
        assert_eq!(format_ttl(86_400_000), "1d");
    }

    #[test]
    fn kinds_have_names_and_viewers() {
        assert_eq!(format_key_kind("zset"), "Sorted set");
        assert_eq!(format_key_kind("none"), "Gone");
        assert_eq!(format_key_kind(""), "Unknown");
        assert_eq!(format_key_kind("ReJSON-RL"), "ReJSON-RL");
        assert!(is_viewable_kind("stream"));
        assert!(!is_viewable_kind("none"));
        assert!(!is_viewable_kind("ReJSON-RL"));
    }

    #[test]
    fn a_binary_key_is_labelled_never_shown_as_base64() {
        let key = RedisBytes {
            text: "AAEC".into(),
            binary: true,
            bytes: 3,
            truncated: false,
        };
        assert_eq!(key_label(&key), "\u{27e8}binary 3 bytes\u{27e9}");
        assert_eq!(key_label(&bytes("a\nb")), "a?b");
    }

    #[test]
    fn only_a_whole_text_string_can_be_copied() {
        let string = |value: RedisBytes| RedisValuePage {
            entries: vec![RedisEntry {
                field: None,
                value,
                score: None,
            }],
            ..page("string", 0, None, "0")
        };
        assert_eq!(copyable_value(&string(bytes("hello"))), Some("hello"));
        let truncated = RedisBytes {
            bytes: 5 * 1024 * 1024,
            truncated: true,
            ..bytes("preview")
        };
        assert_eq!(copyable_value(&string(truncated)), None);
        let binary = RedisBytes {
            binary: true,
            ..bytes("AAEC")
        };
        assert_eq!(copyable_value(&string(binary)), None);
        assert_eq!(copyable_value(&page("list", 1, Some(1), "0")), None);
        assert_eq!(copyable_value(&page("string", 0, None, "0")), None);
    }

    #[test]
    fn a_truncated_string_counts_bytes_not_characters() {
        let mut p = page("string", 0, Some(1_000_000), "0");
        p.entries.push(RedisEntry {
            field: None,
            value: RedisBytes {
                text: "\u{e9}".repeat(1024),
                binary: false,
                bytes: 1_000_000,
                truncated: true,
            },
            score: None,
        });
        assert_eq!(
            truncated_note(&p).unwrap(),
            "Truncated \u{2014} showing the first 2.0 KB of 976.6 KB."
        );
        p.entries[0].value = RedisBytes {
            text: base64::engine::general_purpose::STANDARD.encode([0xffu8; 2048]),
            binary: true,
            bytes: 4096,
            truncated: true,
        };
        assert!(
            truncated_note(&p)
                .unwrap()
                .contains("first 2.0 KB of 4.0 KB")
        );
    }

    #[test]
    fn binary_members_get_the_base64_note() {
        let mut p = page("hash", 2, Some(2), "0");
        assert_eq!(truncated_note(&p), None);
        p.entries[1].field = Some(RedisBytes {
            binary: true,
            ..bytes("AA==")
        });
        assert_eq!(
            truncated_note(&p).unwrap(),
            "Values that are not valid UTF-8 are shown as base64."
        );
    }

    #[test]
    fn load_more_follows_each_types_paging() {
        assert_eq!(
            more_value(&page("hash", 3, Some(10), "42")),
            Some(MoreValue::Cursor("42".into()))
        );
        assert_eq!(more_value(&page("set", 3, Some(3), "0")), None);
        assert_eq!(
            more_value(&page("list", 200, Some(500), "0")),
            Some(MoreValue::Offset(200))
        );
        assert_eq!(more_value(&page("zset", 500, Some(500), "0")), None);
        assert_eq!(
            more_value(&page("list", 5, None, "0")),
            Some(MoreValue::Offset(5))
        );
        assert_eq!(more_value(&page("stream", 200, Some(900), "0")), None);
        assert_eq!(more_value(&page("string", 1, Some(9), "0")), None);
    }

    #[test]
    fn appending_keeps_every_entry_and_the_newest_cursor() {
        let joined = append_page(page("hash", 3, Some(9), "5"), page("hash", 4, Some(9), "0"));
        assert_eq!(joined.entries.len(), 7);
        assert_eq!(joined.cursor, "0");
        assert!(joined.done);
    }

    #[test]
    fn cells_and_lines_are_cut_on_character_boundaries() {
        let short = "abc";
        assert_eq!(clip_cell(short), ("abc", 0));
        let long = "\u{e9}".repeat(MAX_CELL_BYTES); // 2 bytes each
        let (shown, hidden) = clip_cell(&long);
        assert_eq!(shown.len() + hidden, long.len());
        assert!(shown.len() <= MAX_CELL_BYTES);

        assert_eq!(display_lines("a\r\nb\n\nc", 80), ["a", "b", "", "c"]);
        assert_eq!(display_lines("", 80), [""]);
    }

    #[test]
    fn a_long_line_is_cut_to_the_columns_and_nothing_is_lost() {
        // The 256 KB preview of a 5 MB string: one line, thousands of rows.
        let line = "x".repeat(256 * 1024);
        let pieces = display_lines(&line, 48);
        assert_eq!(pieces.len(), (256 * 1024usize).div_ceil(48));
        assert!(pieces.iter().all(|p| p.len() <= 48));
        assert_eq!(pieces.concat(), line);

        // Two bytes each, one column each: cut by columns, not bytes.
        let accents = "\u{e9}".repeat(10);
        assert_eq!(
            display_lines(&accents, 4),
            ["\u{e9}".repeat(4), "\u{e9}".repeat(4), "\u{e9}".repeat(2)]
        );

        // A wide character takes two columns and is never split.
        assert_eq!(
            display_lines("\u{4e2d}\u{6587}ab", 3),
            ["\u{4e2d}", "\u{6587}a", "b"]
        );
        // A combining mark stays with its letter.
        assert_eq!(display_lines("ae\u{301}b", 2), ["ae\u{301}", "b"]);
        // A character wider than the row still gets a row of its own.
        assert_eq!(display_lines("\u{4e2d}", 1), ["\u{4e2d}"]);
        assert_eq!(display_lines("a\tb", 4), ["a", "\t", "b"]);

        let text = "first line\n".to_string() + &"y".repeat(100) + "\nlast";
        assert_eq!(display_lines(&text, 30).concat(), text.replace('\n', ""));
    }

    fn db(index: i64, keys: u64) -> RedisDbInfo {
        RedisDbInfo {
            index,
            keys,
            expires: 0,
        }
    }

    #[test]
    fn db0_is_always_listed() {
        assert_eq!(databases_or_default(&[]), [db(0, 0)]);
        assert_eq!(databases_or_default(&[db(2, 5)]), [db(2, 5)]);
        assert_eq!(db_key_count(&[db(0, 3), db(2, 5)], 2), 5);
        assert_eq!(db_key_count(&[db(0, 3)], 9), 0);
    }

    #[test]
    fn the_tree_opens_only_the_active_database() {
        let groups = vec![
            PrefixGroup {
                prefix: "user".into(),
                count: 9,
                binary: false,
            },
            PrefixGroup {
                prefix: "AA==".into(),
                count: 1,
                binary: true,
            },
        ];
        let dbs = [db(0, 10), db(1, 4)];
        assert_eq!(
            tree_rows(&dbs, 0, &groups, false, true, 20_000),
            [
                TreeRow::Db {
                    index: 0,
                    keys: 10,
                    active: true
                },
                TreeRow::Group(0),
                TreeRow::Group(1),
                TreeRow::Partial { scanned: 20_000 },
                TreeRow::Db {
                    index: 1,
                    keys: 4,
                    active: false
                },
            ]
        );
        assert_eq!(
            tree_rows(&dbs, 1, &[], true, false, 0)[2],
            TreeRow::Grouping
        );
        assert_eq!(
            tree_rows(&dbs, 1, &[], false, false, 0)[2],
            TreeRow::NoMatch
        );
        assert_eq!(group_label(&groups[1]), "\u{27e8}binary\u{27e9}");
    }

    #[test]
    fn backup_names_say_what_and_when() {
        assert_eq!(
            default_backup_name("Prod Cache #1", 0, "", (2026, 3, 7)),
            "prod-cache-1-db0-2026-03-07.tdredis"
        );
        assert_eq!(
            default_backup_name("!!", 3, "user:*", (2026, 10, 23)),
            "redis-db3-user_-2026-10-23.tdredis"
        );
        assert_eq!(
            default_backup_name("", 1, "a:b:*x", (2026, 1, 1)),
            "redis-db1-a_b_x-2026-01-01.tdredis"
        );
    }

    #[test]
    fn every_nonzero_count_is_reported() {
        let mut s = RedisOpSummary {
            written: 40_000,
            ..Default::default()
        };
        assert_eq!(
            summarize_op(&s, "Restored"),
            ("Restored 40,000 keys".into(), ToastKind::Success)
        );
        s.skipped_existing = 12_000;
        s.skipped_big = vec![SkippedKey {
            key: "big".into(),
            bytes: 1,
        }];
        s.vanished = 2;
        s.failed = vec![FailedKey {
            key: "k".into(),
            reason: "r".into(),
        }];
        assert_eq!(
            summarize_op(&s, "Backed up"),
            (
                "Backed up 40,000 keys; 12,000 already existed and were left alone; 1 too large \
                 to copy; 2 expired while running; 1 failed"
                    .into(),
                ToastKind::Warning
            )
        );
        assert_eq!(
            op_error_message("cancelled", "Backup"),
            ("Backup cancelled".into(), ToastKind::Info)
        );
        assert_eq!(
            op_error_message("boom", "Restore"),
            ("Restore failed: boom".into(), ToastKind::Error)
        );
    }

    fn header(version: &str, dbs: Vec<u8>) -> BackupHeader {
        BackupHeader {
            app: "TermDrop".into(),
            created_at: "now".into(),
            source: "redis://***@h".into(),
            redis_version: version.into(),
            dbs,
            pattern: None,
            consistency: "scan".into(),
        }
    }

    #[test]
    fn restore_warns_across_versions_and_targets_the_files_db() {
        assert_eq!(restore_warning(&header("7.2.4", vec![]), "7.2.4"), None);
        assert_eq!(restore_warning(&header("", vec![]), "7.2.4"), None);
        assert_eq!(
            restore_warning(&header("6.2.0", vec![]), "7.2.4").unwrap(),
            "This backup came from Redis 6.2.0; this server runs 7.2.4."
        );
        assert_eq!(default_restore_db(&header("", vec![3, 4]), 0), 3);
        assert_eq!(default_restore_db(&header("", vec![]), 5), 5);
    }
}
