//! What both datastore tabs format, detect and draw the same way. Use these
//! rather than a copy in `mongo/` or `redis/`, so the two tabs keep one look
//! and one wording.
//!
//! - Numbers: [`format_count`] (`toLocaleString()` in the Vue tabs) and the
//!   panels' [`format_bytes`].
//! - The missing-password detector behind `ServiceTabContext::run_with_secret`:
//!   [`missing_secret`].
//! - Backup/restore progress: [`OpProgress`] folded from the core's events,
//!   and [`op_progress_row`] to draw it with its Cancel button.
//! - A JSON viewer that keeps field order: [`json_lines`] (pure) and
//!   [`json_view`]; and [`Json`], a parsed value that keeps it too.
//! - A three-state checkbox (none / some / all) for a group whose members
//!   are ticked one by one: [`CheckState`] and [`tri_check`]. GPUI Kit's
//!   `Checkbox` has no indeterminate state.
//! - The desktop notice after a run: [`completed_summary`], posted with
//!   `ServiceTabContext::notify_if_unfocused`.
//!
//! Lists: GPUI's `uniform_list` is the virtualised list (the SFTP panel's
//! file list uses it); `gpui_kit::component::{table, tree}` are there too. No
//! wrapper is added here, because each tab's rows differ.

use gpui_kit::assets::IconName;
use gpui_kit::component::progress::Progress;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{Icon, h_flex, v_flex};
use gpui_kit::{
    App, ClickEvent, ElementId, FontWeight, Hsla, InteractiveElement, IntoElement, ParentElement,
    SharedString, StatefulInteractiveElement, Styled, Window, div, prelude::FluentBuilder as _, px,
};
use termdrop_core::CoreEvent;

use super::DatastoreKind;
pub use super::json::{Json, js_number};
pub use crate::panels::common::{
    format_bytes, list_cell, list_cell_muted, list_head, list_row, num, row_separator, text_2xs,
};
use crate::ui::icon_button::IconButton;
use crate::ui::theme::Palette;

// ---- a group's checkbox ---------------------------------------------------

/// How many of a group's members are ticked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckState {
    None,
    /// Some but not all: drawn as a dash.
    Some,
    All,
}

/// A 14px three-state box: empty, a dash (some) or a tick (all). `tip` says
/// what a click does; a click on "some" is the caller's to define (the
/// MongoDB tree ticks the rest, as Vue's indeterminate checkbox did).
pub fn tri_check(
    id: impl Into<ElementId>,
    state: CheckState,
    tip: impl Into<SharedString>,
    p: &Palette,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let tip: SharedString = tip.into();
    let filled = state != CheckState::None;
    div()
        .id(id)
        .flex_none()
        .size(px(14.))
        .rounded_sm()
        .border_1()
        .border_color(if filled { p.accent_solid } else { p.line })
        .when(filled, |d| d.bg(p.accent_solid))
        .flex()
        .items_center()
        .justify_center()
        .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
        .when(filled, |d| {
            d.child(
                Icon::new(if state == CheckState::All {
                    IconName::Check
                } else {
                    IconName::Minus
                })
                .size(px(10.))
                .text_color(gpui_kit::white()),
            )
        })
        .on_click(on_click)
}

// ---- notifications --------------------------------------------------------

/// The body of the one desktop notice for a finished multi-database run
/// (`useMongoBackup.js`'s `notifyCompleted`): `None` when nothing succeeded,
/// since a run that failed or was cancelled has already said so.
pub fn completed_summary(verb: &str, done: &[String]) -> Option<String> {
    match done {
        [] => None,
        [one] => Some(format!("{verb} {one}")),
        many => Some(format!("{verb} {} databases", many.len())),
    }
}

// ---- numbers -------------------------------------------------------------

/// A count with thousands separators, as the Vue tabs' `toLocaleString()`
/// shows it in an English locale: "1,234,567".
pub fn format_count(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// "1 key" / "2 keys": a count and the noun that agrees with it.
pub fn count_noun(n: u64, singular: &str, plural: &str) -> String {
    format!(
        "{} {}",
        format_count(n),
        if n == 1 { singular } else { plural }
    )
}

// ---- missing secrets -----------------------------------------------------

/// Which password a failed call was missing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MissingSecret {
    /// The datastore's own password (`mongo-remote-<id>` / `redis-<id>` in
    /// the keyring): ask, store it, retry.
    Datastore,
    /// The SSH password of the host a Redis connection tunnels through.
    /// Only `redis_connect` can take it (`ssh_password`); once the tunnel is
    /// up it is reused, so no later call needs it.
    Bastion,
}

/// Whether `err` says a stored password is missing, and whose.
///
/// The core's wording: `keyring retrieve failed for MongoDB: ...` /
/// `keyring retrieve failed for Redis: ...` for the datastore's own secret
/// (`state::load_mongo_uri` / `load_redis_uri`), and the bare
/// `keyring retrieve failed: ...` or `No matching entry` from
/// `crypto::get_password` for an SSH password. The Vue tabs match the bare
/// substrings only (`utils/secretPrompt.js`) and so would ask for, and store,
/// a bastion's password as the Redis one; the native tabs tell them apart.
pub fn missing_secret(kind: DatastoreKind, err: &str) -> Option<MissingSecret> {
    let own = match kind {
        DatastoreKind::Mongo => err.contains("keyring retrieve failed for MongoDB"),
        DatastoreKind::Redis => err.contains("keyring retrieve failed for Redis"),
        // A VNC password that is not stored is not a keyring error: the
        // handshake runs with an empty one and the server rejects it
        // (`vnc::conn::failure_reason`). `run_with_secret` asks only when
        // none is stored.
        DatastoreKind::Vnc => {
            err.starts_with("the VNC password was rejected")
                || err.starts_with("the user name or password was rejected")
        }
    };
    if own {
        return Some(MissingSecret::Datastore);
    }
    if !(err.contains("keyring retrieve failed") || err.contains("No matching entry")) {
        return None;
    }
    Some(match kind {
        // A MongoDB host has no tunnel: any keyring miss is its own secret,
        // as the Vue tab assumed.
        DatastoreKind::Mongo => MissingSecret::Datastore,
        // Redis and VNC keep their own secrets out of the plain keyring
        // error, so a bare one is the tunnel's SSH host.
        DatastoreKind::Redis | DatastoreKind::Vnc => MissingSecret::Bastion,
    })
}

/// The password to ask for after a call failed with `err`, or `None` to
/// give up and show the error. Each secret is asked for at most once per
/// call: a second miss of the same one means the answer did not take, and
/// asking again would loop. A *different* one is asked for, though: a new
/// Redis host tunnelled through a bastion whose password is not stored
/// either misses its own password first and the SSH one after, and showing
/// the raw keyring error in between only made the user press Reconnect.
pub fn next_secret_prompt(
    kind: DatastoreKind,
    err: &str,
    asked: &[MissingSecret],
) -> Option<MissingSecret> {
    missing_secret(kind, err).filter(|need| !asked.contains(need))
}

// ---- operation progress --------------------------------------------------

/// Where a backup or restore stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpState {
    Running,
    /// Cancel was pressed; the core has not confirmed yet.
    Cancelling,
    /// The core emitted its cancelled event (the call then fails with
    /// `cancelled`).
    Cancelled,
    Done,
    Failed(String),
}

/// One backup or restore, folded from `MongoProgress` / `RedisProgress`
/// events. Keep one per op id in the view and feed it every event its
/// `subscribe_op` subscription delivers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpProgress {
    pub op_id: String,
    /// What the row says, e.g. "Backing up db0". The tab picks it.
    pub title: String,
    /// The core's stage (`dumping`, `restoring`, `exporting`, `done`...).
    pub stage: String,
    /// What is being worked on: a MongoDB collection; empty for Redis.
    pub item: String,
    /// 0..=100. MongoDB's CLI tools report an estimate while they run.
    pub percent: u8,
    /// Whether `percent` means anything yet. A Redis restore reads its file
    /// as it goes and only learns the key count at the end, so the core
    /// sends `total: 0` and a percent that stays 0 until it is done; the row
    /// then shows the count without a "0%" beside 600,000 restored keys.
    pub percent_known: bool,
    /// "current / total" as the tool or the core printed it, or built from
    /// the counts when it did not.
    pub detail: String,
    pub state: OpState,
}

impl OpProgress {
    pub fn new(op_id: impl Into<String>, title: impl Into<String>) -> Self {
        Self {
            op_id: op_id.into(),
            title: title.into(),
            stage: String::new(),
            item: String::new(),
            percent: 0,
            percent_known: true,
            detail: String::new(),
            state: OpState::Running,
        }
    }

    /// Apply one event. Events for another op id are ignored, so a stray one
    /// cannot move this bar. Returns whether anything changed.
    pub fn apply(&mut self, event: &CoreEvent) -> bool {
        match event {
            CoreEvent::MongoProgress(p) if p.op_id == self.op_id => {
                self.stage = p.stage.clone();
                self.item = p.collection.clone();
                self.percent = p.percent.min(100) as u8;
                self.detail = p
                    .detail
                    .clone()
                    .unwrap_or_else(|| format!("{} / {}", p.synced, p.total));
                if p.stage == "done" && self.state == OpState::Running {
                    self.state = OpState::Done;
                }
                true
            }
            CoreEvent::RedisProgress(p) if p.op_id == self.op_id => {
                self.stage = p.stage.clone();
                self.percent = p.percent.min(100) as u8;
                self.percent_known = p.total > 0 || p.stage == "done";
                self.detail = if p.detail.is_empty() {
                    if p.total > 0 {
                        format!("{} / {}", format_count(p.processed), format_count(p.total))
                    } else {
                        format_count(p.processed)
                    }
                } else {
                    p.detail.clone()
                };
                true
            }
            CoreEvent::MongoCancelled { op_id, .. } | CoreEvent::RedisCancelled { op_id, .. }
                if *op_id == self.op_id =>
            {
                self.state = OpState::Cancelled;
                true
            }
            _ => false,
        }
    }

    /// The call returned. `Err("cancelled")` (the core's exact string) is a
    /// cancel, not a failure.
    pub fn finish(&mut self, result: Result<(), &str>) {
        self.state = match result {
            Ok(()) => {
                self.percent = 100;
                self.percent_known = true;
                OpState::Done
            }
            Err("cancelled") => OpState::Cancelled,
            Err(e) => OpState::Failed(e.to_string()),
        };
    }

    /// Cancel was pressed (call `ctx.cancel_op` too).
    pub fn cancelling(&mut self) {
        if self.state == OpState::Running {
            self.state = OpState::Cancelling;
        }
    }

    pub fn is_active(&self) -> bool {
        matches!(self.state, OpState::Running | OpState::Cancelling)
    }

    /// The line under the bar: stage and item while running, the outcome
    /// after.
    pub fn status_line(&self) -> String {
        match &self.state {
            OpState::Running => [self.stage.as_str(), self.item.as_str()]
                .into_iter()
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
                .join(" "),
            OpState::Cancelling => "Cancelling\u{2026}".into(),
            OpState::Cancelled => "Cancelled".into(),
            OpState::Done => "Done".into(),
            OpState::Failed(e) => e.clone(),
        }
    }
}

/// A progress row: title, bar, status line and detail, and a Cancel button
/// while the operation runs. `on_cancel` should call
/// `OpProgress::cancelling` and `ctx.cancel_op(op_id)`.
pub fn op_progress_row(
    id: impl Into<SharedString>,
    op: &OpProgress,
    p: &Palette,
    on_cancel: impl Fn(&mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let id: SharedString = id.into();
    let tone: Hsla = match op.state {
        OpState::Failed(_) => p.bad,
        OpState::Cancelled | OpState::Cancelling => p.warn,
        OpState::Done => p.good,
        OpState::Running => p.accent,
    };
    let active = op.is_active();
    v_flex()
        .gap_1()
        .px_3()
        .py_2()
        .child(
            h_flex()
                .gap_2()
                .items_center()
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_sm()
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(p.ink)
                        .child(SharedString::from(op.title.clone())),
                )
                .when(op.percent_known, |row| {
                    row.child(
                        num(text_2xs(div()))
                            .text_color(p.ink_2)
                            .child(format!("{}%", op.percent)),
                    )
                })
                .when_active(active, || {
                    IconButton::new(
                        ElementId::Name(format!("{id}-cancel").into()),
                        IconName::X,
                        "Cancel",
                    )
                    .disabled(op.state == OpState::Cancelling)
                    .on_click(move |_, window, cx| on_cancel(window, cx))
                }),
        )
        .child(
            Progress::new(ElementId::Name(format!("{id}-bar").into()))
                .value(f32::from(op.percent))
                .color(tone)
                .h(px(4.)),
        )
        .child(
            h_flex()
                .gap_2()
                .child(
                    text_2xs(div())
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_color(if matches!(op.state, OpState::Failed(_)) {
                            p.bad
                        } else {
                            p.ink_2
                        })
                        .child(op.status_line()),
                )
                .child(
                    num(text_2xs(div()))
                        .text_color(p.ink_3)
                        .child(op.detail.clone()),
                ),
        )
}

/// `.child(f())` only when `on`, for builders that are not `FluentBuilder`.
trait WhenActive: ParentElement + Sized {
    fn when_active<E: IntoElement>(self, on: bool, f: impl FnOnce() -> E) -> Self {
        if on { self.child(f()) } else { self }
    }
}

impl<T: ParentElement + Sized> WhenActive for T {}

// ---- JSON ----------------------------------------------------------------

/// What a piece of a JSON line is, for colouring.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JsonTok {
    /// An object key, with its quotes.
    Key,
    /// A string value, with its quotes and escapes as written.
    Str,
    Number,
    /// `true` / `false`.
    Bool,
    Null,
    /// Brackets, braces, `:` and `,`.
    Punct,
}

/// One line of pretty-printed JSON.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JsonLine {
    pub indent: usize,
    pub parts: Vec<(JsonTok, String)>,
}

impl JsonLine {
    /// The line as plain text, indented two spaces a level (for copying).
    pub fn text(&self) -> String {
        let mut s = "  ".repeat(self.indent);
        for (_, t) in &self.parts {
            s.push_str(t);
        }
        s
    }
}

/// Pretty-print `text` as JSON, two spaces a level, **keeping field order**
/// (a MongoDB document's order is part of it; `serde_json::Value` would sort
/// the keys). Re-indents the tokens as written, so numbers and extended-JSON
/// wrappers (`{"$numberLong": "1"}`) are shown exactly. `None` when `text`
/// is not JSON, for the caller to show it raw.
///
/// Stops after `max_lines` lines and reports whether it did, so a huge value
/// cannot stall the UI thread.
pub fn json_lines(text: &str, max_lines: usize) -> Option<(Vec<JsonLine>, bool)> {
    let tokens = lex(text)?;
    if !balanced(&tokens) {
        return None;
    }
    let mut lines = Vec::new();
    let mut cur = JsonLine {
        indent: 0,
        parts: Vec::new(),
    };
    let mut indent = 0usize;
    let mut i = 0;
    while i < tokens.len() {
        if lines.len() >= max_lines {
            return Some((lines, true));
        }
        let (kind, ref t) = tokens[i];
        match t.as_str() {
            "{" | "[" => {
                let close = if t == "{" { "}" } else { "]" };
                if tokens.get(i + 1).is_some_and(|(_, n)| n == close) {
                    cur.parts.push((JsonTok::Punct, format!("{t}{close}")));
                    i += 2;
                    continue;
                }
                cur.parts.push((JsonTok::Punct, t.clone()));
                indent += 1;
                lines.push(std::mem::replace(
                    &mut cur,
                    JsonLine {
                        indent,
                        parts: Vec::new(),
                    },
                ));
            }
            "}" | "]" => {
                indent = indent.saturating_sub(1);
                if !cur.parts.is_empty() {
                    lines.push(std::mem::replace(
                        &mut cur,
                        JsonLine {
                            indent,
                            parts: Vec::new(),
                        },
                    ));
                }
                cur.indent = indent;
                cur.parts.push((JsonTok::Punct, t.clone()));
            }
            "," => {
                cur.parts.push((JsonTok::Punct, ",".into()));
                lines.push(std::mem::replace(
                    &mut cur,
                    JsonLine {
                        indent,
                        parts: Vec::new(),
                    },
                ));
            }
            ":" => cur.parts.push((JsonTok::Punct, ": ".into())),
            _ => {
                let kind =
                    if kind == JsonTok::Str && tokens.get(i + 1).is_some_and(|(_, n)| n == ":") {
                        JsonTok::Key
                    } else {
                        kind
                    };
                cur.parts.push((kind, t.clone()));
            }
        }
        i += 1;
    }
    if !cur.parts.is_empty() {
        lines.push(cur);
    }
    Some((lines, false))
}

fn lex(text: &str) -> Option<Vec<(JsonTok, String)>> {
    let mut out = Vec::new();
    let mut chars = text.char_indices().peekable();
    while let Some(&(start, c)) = chars.peek() {
        match c {
            c if c.is_whitespace() => {
                chars.next();
            }
            '{' | '}' | '[' | ']' | ':' | ',' => {
                chars.next();
                out.push((JsonTok::Punct, c.to_string()));
            }
            '"' => {
                chars.next();
                let mut end = None;
                let mut escaped = false;
                for (i, c) in chars.by_ref() {
                    if escaped {
                        escaped = false;
                    } else if c == '\\' {
                        escaped = true;
                    } else if c == '"' {
                        end = Some(i + 1);
                        break;
                    } else if c.is_control() {
                        return None;
                    }
                }
                out.push((JsonTok::Str, text[start..end?].to_string()));
            }
            c if c == '-' || c.is_ascii_digit() || c.is_ascii_alphabetic() => {
                let mut end = start;
                while let Some(&(i, c)) = chars.peek() {
                    if c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.') {
                        end = i + c.len_utf8();
                        chars.next();
                    } else {
                        break;
                    }
                }
                let word = &text[start..end];
                let kind = match word {
                    "true" | "false" => JsonTok::Bool,
                    "null" => JsonTok::Null,
                    w if w.parse::<f64>().is_ok() => JsonTok::Number,
                    _ => return None,
                };
                out.push((kind, word.to_string()));
            }
            _ => return None,
        }
    }
    (!out.is_empty()).then_some(out)
}

/// Brackets nest properly. Not a full validator: the text is meant to be
/// JSON (the core produced it); this only refuses what would draw wrongly.
fn balanced(tokens: &[(JsonTok, String)]) -> bool {
    let mut stack = Vec::new();
    for (kind, t) in tokens {
        if *kind != JsonTok::Punct {
            continue;
        }
        match t.as_str() {
            "{" => stack.push('}'),
            "[" => stack.push(']'),
            "}" | "]" if stack.pop() != t.chars().next() => return false,
            _ => {}
        }
    }
    stack.is_empty()
}

/// The colour of each token kind, from the palette.
pub fn json_colour(kind: JsonTok, p: &Palette) -> Hsla {
    match kind {
        JsonTok::Key => p.accent,
        JsonTok::Str => p.good,
        JsonTok::Number => p.warn,
        JsonTok::Bool | JsonTok::Null => p.bad,
        JsonTok::Punct => p.ink_2,
    }
}

/// Draw `lines` (from [`json_lines`]) in `mono`, coloured by token. Plain
/// elements, not a list: for a long value put it in a scrolling container,
/// or render rows from `lines` with `uniform_list` yourself.
pub fn json_view(lines: &[JsonLine], p: &Palette, mono: SharedString) -> impl IntoElement {
    v_flex()
        .font_family(mono)
        .text_xs()
        .children(lines.iter().map(|line| {
            h_flex()
                .pl(px(14. * line.indent as f32))
                .children(line.parts.iter().map(|(kind, t)| {
                    div()
                        .text_color(json_colour(*kind, p))
                        .child(SharedString::from(t.clone()))
                }))
        }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_finished_run_is_summarised_once() {
        assert_eq!(completed_summary("Backed up", &[]), None);
        assert_eq!(
            completed_summary("Backed up", &["shopdb".into()]).as_deref(),
            Some("Backed up shopdb")
        );
        assert_eq!(
            completed_summary("Restored", &["a".into(), "b".into()]).as_deref(),
            Some("Restored 2 databases")
        );
    }
    use termdrop_core::events::{MongoProgress, RedisProgress};

    #[test]
    fn counts_get_thousands_separators() {
        assert_eq!(format_count(0), "0");
        assert_eq!(format_count(999), "999");
        assert_eq!(format_count(1000), "1,000");
        assert_eq!(format_count(1234567), "1,234,567");
        assert_eq!(count_noun(1, "key", "keys"), "1 key");
        assert_eq!(count_noun(2500, "key", "keys"), "2,500 keys");
    }

    #[test]
    fn each_missing_secret_is_asked_for_once_per_call() {
        use DatastoreKind::*;
        let own = "keyring retrieve failed for Redis password: No matching entry";
        let ssh = "keyring retrieve failed: no fallback password for host";
        // Nothing asked yet: ask for whichever is missing.
        assert_eq!(
            next_secret_prompt(Redis, own, &[]),
            Some(MissingSecret::Datastore)
        );
        // The Redis password was given and the bastion's is missing next.
        assert_eq!(
            next_secret_prompt(Redis, ssh, &[MissingSecret::Datastore]),
            Some(MissingSecret::Bastion)
        );
        // The same one missing again: the answer did not take; stop.
        assert_eq!(
            next_secret_prompt(Redis, own, &[MissingSecret::Datastore]),
            None
        );
        assert_eq!(
            next_secret_prompt(
                Redis,
                ssh,
                &[MissingSecret::Datastore, MissingSecret::Bastion]
            ),
            None
        );
        // Any other error is shown as it is.
        assert_eq!(next_secret_prompt(Mongo, "connection refused", &[]), None);
    }

    #[test]
    fn a_missing_secret_is_told_apart_from_a_missing_bastion_password() {
        use DatastoreKind::*;
        use MissingSecret::*;
        let redis = "keyring retrieve failed for Redis: keyring retrieve failed: x";
        let mongo = "keyring retrieve failed for MongoDB: No matching entry";
        let ssh = "keyring retrieve failed: x; fallback retrieve failed: y";
        assert_eq!(missing_secret(Redis, redis), Some(Datastore));
        assert_eq!(missing_secret(Mongo, mongo), Some(Datastore));
        assert_eq!(missing_secret(Redis, ssh), Some(Bastion));
        assert_eq!(
            missing_secret(Redis, "No matching entry found"),
            Some(Bastion)
        );
        // A MongoDB host has no tunnel; any miss is its own password.
        assert_eq!(missing_secret(Mongo, ssh), Some(Datastore));
        assert_eq!(missing_secret(Redis, "connection refused"), None);
        assert_eq!(missing_secret(Mongo, "cancelled"), None);
        // A remote desktop: its own password is a rejection, the tunnel's a
        // keyring miss.
        assert_eq!(
            missing_secret(Vnc, "the VNC password was rejected"),
            Some(Datastore)
        );
        assert_eq!(
            missing_secret(Vnc, "the user name or password was rejected: locked"),
            Some(Datastore)
        );
        assert_eq!(missing_secret(Vnc, ssh), Some(Bastion));
        assert_eq!(missing_secret(Vnc, "could not connect to x:5901"), None);
    }

    fn mongo(op: &str, stage: &str, percent: u64, detail: Option<&str>) -> CoreEvent {
        CoreEvent::MongoProgress(MongoProgress {
            op_id: op.into(),
            db: "d".into(),
            collection: "users".into(),
            stage: stage.into(),
            synced: 3,
            total: 9,
            percent,
            detail: detail.map(Into::into),
        })
    }

    #[test]
    fn progress_follows_its_own_op_only() {
        let mut op = OpProgress::new("a", "Backing up d");
        assert!(!op.apply(&mongo("b", "dumping", 50, None)));
        assert_eq!(op.percent, 0);
        assert!(op.apply(&mongo("a", "dumping", 40, Some("1/2"))));
        assert_eq!((op.percent, op.detail.as_str()), (40, "1/2"));
        assert_eq!(op.status_line(), "dumping users");
        op.apply(&mongo("a", "dumping", 150, None));
        assert_eq!((op.percent, op.detail.as_str()), (100, "3 / 9"));
        op.apply(&mongo("a", "done", 100, None));
        assert_eq!(op.state, OpState::Done);
    }

    #[test]
    fn a_cancel_is_not_a_failure() {
        let mut op = OpProgress::new("a", "Restoring");
        op.cancelling();
        assert_eq!(op.status_line(), "Cancelling\u{2026}");
        assert!(op.is_active());
        op.apply(&CoreEvent::RedisCancelled {
            op_id: "a".into(),
            db: 0,
        });
        assert_eq!(op.state, OpState::Cancelled);
        op.finish(Err("cancelled"));
        assert_eq!(op.state, OpState::Cancelled);
        op.finish(Err("connection reset"));
        assert_eq!(op.state, OpState::Failed("connection reset".into()));
        assert!(!op.is_active());
    }

    #[test]
    fn redis_progress_builds_a_detail_when_the_core_gives_none() {
        let mut op = OpProgress::new("r", "Backing up db0");
        op.apply(&CoreEvent::RedisProgress(RedisProgress {
            op_id: "r".into(),
            db: 0,
            stage: "exporting".into(),
            processed: 1500,
            total: 3000,
            percent: 50,
            detail: String::new(),
        }));
        assert_eq!(op.detail, "1,500 / 3,000");
        assert_eq!(op.status_line(), "exporting");
        assert!(op.percent_known);
    }

    #[test]
    fn a_restore_without_a_total_shows_no_percent_until_it_is_done() {
        let restoring = |processed, total, stage: &str, percent| {
            CoreEvent::RedisProgress(RedisProgress {
                op_id: "r".into(),
                db: 6,
                stage: stage.into(),
                processed,
                total,
                percent,
                detail: format!("{processed} keys restored"),
            })
        };
        let mut op = OpProgress::new("r", "Restoring");
        op.apply(&restoring(578_200, 0, "restoring", 0));
        assert!(!op.percent_known);
        assert_eq!(op.detail, "578200 keys restored");
        op.apply(&restoring(1_000_000, 1_000_000, "done", 100));
        assert!(op.percent_known);
        assert_eq!(op.percent, 100);

        let mut op = OpProgress::new("r", "Restoring");
        op.apply(&restoring(10, 0, "restoring", 0));
        op.finish(Ok(()));
        assert!(op.percent_known);
    }

    fn texts(lines: &[JsonLine]) -> Vec<String> {
        lines.iter().map(JsonLine::text).collect()
    }

    #[test]
    fn json_is_reindented_in_field_order() {
        let (lines, cut) = json_lines(
            r#"{"z":1,"a":{"$oid":"x"},"l":[],"o":{},"b":[true,null,-1.5e3]}"#,
            100,
        )
        .unwrap();
        assert!(!cut);
        assert_eq!(
            texts(&lines),
            [
                "{",
                "  \"z\": 1,",
                "  \"a\": {",
                "    \"$oid\": \"x\"",
                "  },",
                "  \"l\": [],",
                "  \"o\": {},",
                "  \"b\": [",
                "    true,",
                "    null,",
                "    -1.5e3",
                "  ]",
                "}",
            ]
        );
        assert_eq!(lines[1].parts[0].0, JsonTok::Key);
        assert_eq!(lines[3].parts[2].0, JsonTok::Str);
    }

    #[test]
    fn json_keeps_escapes_and_scalars() {
        let (lines, _) = json_lines(r#""a \"quoted\" \\ word""#, 10).unwrap();
        assert_eq!(texts(&lines), [r#""a \"quoted\" \\ word""#]);
        assert_eq!(lines[0].parts[0].0, JsonTok::Str);
        let (lines, _) = json_lines("42", 10).unwrap();
        assert_eq!(lines[0].parts[0].0, JsonTok::Number);
    }

    #[test]
    fn text_that_is_not_json_is_refused() {
        for bad in [
            "", "hello", "{\"a\":1", "[1}", "{'a': 1}", "\"open", "1 # x",
        ] {
            assert!(json_lines(bad, 10).is_none(), "{bad:?}");
        }
    }

    #[test]
    fn a_long_value_stops_at_the_cap() {
        let text = format!("[{}]", vec!["1"; 1000].join(","));
        let (lines, cut) = json_lines(&text, 50).unwrap();
        assert!(cut);
        assert_eq!(lines.len(), 50);
    }
}
