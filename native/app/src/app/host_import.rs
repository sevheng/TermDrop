//! Importing hosts from a JSON export or from `~/.ssh/config`. Port of
//! `src/utils/hostImport.js`, case for case, plus the one rule the native
//! `~/.ssh/config` dialog adds: an entry already saved starts unticked.

use serde_json::{Map, Value};
use termdrop_core::uri::split_password;

use super::uri_form::is_mongo_uri;
use crate::backend::{HostSummary, ImportHost, ImportSummary, SshConfigEntry};

/// Larger than any real export; a guard against opening the wrong file.
pub const MAX_IMPORT_BYTES: u64 = 5 * 1024 * 1024;

/// Read an exported hosts file: a bare array, which is what export has always
/// written, or a `{ "hosts": [...] }` envelope. The errors explain rather than
/// surfacing a parser message.
pub fn parse_hosts_file(text: &str) -> Result<Vec<Map<String, Value>>, String> {
    let parsed: Value =
        serde_json::from_str(text).map_err(|_| "This file is not valid JSON".to_string())?;
    let hosts = match parsed {
        Value::Array(a) => a,
        Value::Object(mut o) => match o.remove("hosts") {
            Some(Value::Array(a)) => a,
            _ => return Err("This does not look like a TermDrop host export".into()),
        },
        _ => return Err("This does not look like a TermDrop host export".into()),
    };
    hosts
        .into_iter()
        .map(|h| match h {
            Value::Object(o) => Ok(o),
            _ => Err("The file contains an entry that is not a host".to_string()),
        })
        .collect()
}

/// JavaScript's `Number(x)`, for the values JSON can hold.
fn js_number(v: Option<&Value>) -> f64 {
    match v {
        None => f64::NAN,
        Some(Value::Null) => 0.0,
        Some(Value::Bool(b)) => f64::from(u8::from(*b)),
        Some(Value::Number(n)) => n.as_f64().unwrap_or(f64::NAN),
        Some(Value::String(s)) => {
            let t = s.trim();
            if t.is_empty() {
                0.0
            } else {
                t.parse().unwrap_or(f64::NAN)
            }
        }
        Some(Value::Array(_) | Value::Object(_)) => f64::NAN,
    }
}

/// JavaScript truthiness.
fn js_truthy(v: Option<&Value>) -> bool {
    match v {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_f64().is_some_and(|f| f != 0.0 && !f.is_nan()),
        Some(Value::String(s)) => !s.is_empty(),
        Some(Value::Array(_) | Value::Object(_)) => true,
    }
}

/// `x == null ? null : String(x)`.
fn js_opt_string(v: Option<&Value>) -> Option<String> {
    match v? {
        Value::Null => None,
        Value::String(s) => Some(s.clone()),
        Value::Object(_) => Some("[object Object]".into()),
        other => Some(other.to_string()),
    }
}

fn trimmed_str(v: Option<&Value>) -> String {
    match v {
        Some(Value::String(s)) => s.trim().to_string(),
        _ => String::new(),
    }
}

/// `normalizeImportHost`: only the fields the import accepts, coerced. A
/// stray `password` key in a hand-edited file never reaches the database,
/// and a MongoDB URI from before credentials moved to the keyring has its
/// password stripped rather than written back in plaintext.
pub fn normalize_import_host(raw: &Map<String, Value>) -> ImportHost {
    let port = js_number(raw.get("port"));
    let strip = |field: &str| {
        js_opt_string(raw.get(field)).map(|u| {
            if !u.is_empty() && is_mongo_uri(&u) {
                split_password(&u).0
            } else {
                u
            }
        })
    };
    ImportHost {
        name: trimmed_str(raw.get("name")),
        host: trimmed_str(raw.get("host")),
        // Not finite falls back to 22; a fraction goes to the backend's range
        // check as its whole part.
        port: if port.is_finite() { port as i64 } else { 22 },
        username: trimmed_str(raw.get("username")),
        auth_type: if raw.get("auth_type").and_then(Value::as_str) == Some("key") {
            "key".into()
        } else {
            "password".into()
        },
        key_path: js_opt_string(raw.get("key_path")),
        group: js_opt_string(raw.get("group")),
        favorite: i64::from(js_truthy(raw.get("favorite"))),
        mongo_uri: strip("mongo_uri"),
        mongo_local_uri: strip("mongo_local_uri"),
    }
}

/// A `~/.ssh/config` entry as the same raw record `normalize_import_host`
/// takes, as the Vue dialog passed it.
pub fn ssh_config_import_host(e: &SshConfigEntry) -> ImportHost {
    let mut raw = Map::new();
    raw.insert("name".into(), e.name.clone().into());
    raw.insert("host".into(), e.host.clone().into());
    raw.insert("port".into(), e.port.into());
    raw.insert("username".into(), e.username.clone().into());
    raw.insert("auth_type".into(), e.auth_type.clone().into());
    raw.insert(
        "key_path".into(),
        e.key_path.clone().map_or(Value::Null, Value::from),
    );
    normalize_import_host(&raw)
}

/// How a summary toast reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SummaryTone {
    Info,
    Success,
    Warning,
}

fn plural(n: usize, word: &str) -> String {
    format!("{n} {word}{}", if n == 1 { "" } else { "s" })
}

/// `summarizeImport`: one line saying what an import did. Zero counts are
/// left out, and the first failure is named so the message is actionable.
pub fn summarize_import(summary: &ImportSummary, skipped: usize) -> (String, SummaryTone) {
    let mut parts = Vec::new();
    if summary.added > 0 {
        parts.push(format!("Imported {}", plural(summary.added, "host")));
    }
    if summary.replaced > 0 {
        parts.push(format!("replaced {}", plural(summary.replaced, "host")));
    }
    if skipped > 0 {
        parts.push(format!("skipped {skipped}"));
    }
    if !summary.failed.is_empty() {
        parts.push(plural(summary.failed.len(), "failure"));
    }
    if parts.is_empty() {
        return ("Nothing to import".into(), SummaryTone::Info);
    }
    let mut message = parts.join(", ");
    if let Some((name, reason)) = summary.failed.first() {
        let name = if name.is_empty() { "unnamed" } else { name };
        message.push_str(&format!(": {name} ({reason})"));
        if summary.failed.len() > 1 {
            message.push_str(&format!(" and {} more", summary.failed.len() - 1));
        }
    }
    let tone = if summary.failed.is_empty() {
        SummaryTone::Success
    } else {
        SummaryTone::Warning
    };
    (message, tone)
}

/// A `ProxyJump` hop split into its parts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JumpSpec {
    pub user: Option<String>,
    pub host: String,
    pub port: Option<u16>,
}

/// `parseJumpSpec`: `alias`, `user@host`, `host:port`, `[v6]:port`.
pub fn parse_jump_spec(spec: &str) -> JumpSpec {
    let mut rest = spec.trim();
    let mut user = None;
    if let Some(at) = rest.rfind('@') {
        user = Some(rest[..at].to_string());
        rest = &rest[at + 1..];
    }
    if let Some(inner) = rest.strip_prefix('[')
        && let Some(close) = inner.find(']')
    {
        let tail = &inner[close + 1..];
        let port = match tail.strip_prefix(':') {
            Some(p) if !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()) => p.parse().ok(),
            None if tail.is_empty() => None,
            // Not the bracketed form after all; read it whole.
            _ => {
                return JumpSpec {
                    user,
                    host: rest.to_string(),
                    port: None,
                };
            }
        };
        return JumpSpec {
            user,
            host: inner[..close].to_string(),
            port,
        };
    }
    if let Some((h, p)) = rest.split_once(':')
        && !h.is_empty()
        && !p.is_empty()
        && p.bytes().all(|b| b.is_ascii_digit())
        && !h.contains(':')
    {
        return JumpSpec {
            user,
            host: h.to_string(),
            port: p.parse().ok(),
        };
    }
    JumpSpec {
        user,
        host: rest.to_string(),
        port: None,
    }
}

/// A host to point at its jump host after an import.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JumpLink {
    pub host_id: i64,
    pub jump_id: i64,
}

/// `linkProxyJumps`: point freshly imported hosts at their `ProxyJump` host.
///
/// Runs after the import, when the rows a hop could name have ids. A hop
/// matches a saved SSH host by name first, then by address, honouring a port
/// and user when the hop gives them. Anything ambiguous or unsupported is
/// reported as `(name, reason)` rather than guessed: connecting through the
/// wrong bastion is worse than not linking.
pub fn link_proxy_jumps(
    imported: &[SshConfigEntry],
    hosts: &[HostSummary],
) -> (Vec<JumpLink>, Vec<(String, String)>) {
    fn newest<'a>(m: &[&'a HostSummary]) -> Option<&'a HostSummary> {
        m.iter().copied().max_by_key(|h| h.id)
    }
    let is_ssh = |h: &&HostSummary| !h.host.is_empty();
    let mut links = Vec::new();
    let mut unresolved = Vec::new();

    for entry in imported {
        let Some(jump) = entry.proxy_jump.as_deref() else {
            continue;
        };
        let rows: Vec<_> = hosts
            .iter()
            .filter(is_ssh)
            .filter(|h| h.name == entry.name && h.host == entry.host && h.port == entry.port)
            .collect();
        let Some(row) = newest(&rows) else { continue };

        let spec = parse_jump_spec(jump);
        let by_name: Vec<_> = hosts
            .iter()
            .filter(is_ssh)
            .filter(|h| h.name == spec.host && h.id != row.id)
            .collect();
        let by_address: Vec<_> = hosts
            .iter()
            .filter(is_ssh)
            .filter(|h| {
                h.host == spec.host
                    && h.id != row.id
                    && spec.port.is_none_or(|p| h.port == p)
                    && spec.user.as_deref().is_none_or(|u| h.username == u)
            })
            .collect();
        let candidates = if by_name.is_empty() {
            by_address
        } else {
            by_name
        };
        match newest(&candidates) {
            Some(j) => links.push(JumpLink {
                host_id: row.id,
                jump_id: j.id,
            }),
            None => unresolved.push((entry.name.clone(), format!("no saved host matches {jump}"))),
        }
    }

    // One hop only: a jump host that is itself behind one, already or as
    // part of this import, would be refused at connect time.
    let jumped: Vec<i64> = hosts
        .iter()
        .filter(|h| h.jump_host_id.is_some())
        .map(|h| h.id)
        .chain(links.iter().map(|l| l.host_id))
        .collect();
    let links = links
        .into_iter()
        .filter(|l| {
            if !jumped.contains(&l.jump_id) {
                return true;
            }
            let name = hosts
                .iter()
                .find(|h| h.id == l.host_id)
                .map(|h| h.name.clone())
                .unwrap_or_default();
            unresolved.push((name, "its jump host has a jump host of its own".into()));
            false
        })
        .collect();
    (links, unresolved)
}

/// The toast for hops that could not be linked, or `None` when all were.
pub fn unresolved_jumps_message(unresolved: &[(String, String)]) -> Option<String> {
    let (name, reason) = unresolved.first()?;
    let more = if unresolved.len() > 1 {
        format!(" and {} more", unresolved.len() - 1)
    } else {
        String::new()
    };
    Some(format!(
        "Jump host not linked for {name} ({reason}){more}. Set it by editing the host."
    ))
}

/// Whether a `~/.ssh/config` entry is already a saved SSH host: same name,
/// address and port. The dialog starts those unticked, so importing the same
/// file twice does not double every host.
pub fn already_saved(entry: &SshConfigEntry, hosts: &[HostSummary]) -> bool {
    hosts.iter().any(|h| {
        !h.host.is_empty() && h.name == entry.name && h.host == entry.host && h.port == entry.port
    })
}

/// The import button's label.
pub fn import_button_label(selected: usize) -> String {
    format!(
        "Import {selected} host{}",
        if selected == 1 { "" } else { "s" }
    )
}

/// The export file's suggested name, `ssh-hosts-YYYY-MM-DD.json` (UTC, as
/// `toISOString` gave it).
pub fn export_file_name(now_unix_secs: i64) -> String {
    let days = now_unix_secs.div_euclid(86_400);
    let (y, m, d) = civil_from_days(days);
    format!("ssh-hosts-{y:04}-{m:02}-{d:02}.json")
}

/// Days since 1970-01-01 to a calendar date (Howard Hinnant's algorithm).
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::HostRecord;
    use serde_json::json;

    fn obj(v: Value) -> Map<String, Value> {
        v.as_object().unwrap().clone()
    }

    #[test]
    fn a_hosts_file_is_an_array_or_an_envelope() {
        assert_eq!(parse_hosts_file(r#"[{"name":"a"}]"#).unwrap().len(), 1);
        assert_eq!(parse_hosts_file("[]").unwrap().len(), 0);
        assert_eq!(
            parse_hosts_file(r#"{"version":1,"hosts":[{"name":"a"}]}"#)
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            parse_hosts_file("not json").unwrap_err(),
            "This file is not valid JSON"
        );
        assert!(
            parse_hosts_file(r#"{"a":1}"#)
                .unwrap_err()
                .contains("host export")
        );
        assert!(
            parse_hosts_file("[1,2]")
                .unwrap_err()
                .contains("not a host")
        );
        assert!(
            parse_hosts_file("[null]")
                .unwrap_err()
                .contains("not a host")
        );
    }

    #[test]
    fn normalizing_keeps_known_fields_and_coerces_them() {
        let out = normalize_import_host(&obj(
            json!({"name": "  a  ", "host": " h ", "port": "2222", "favorite": true, "password": "x"}),
        ));
        assert_eq!((out.name.as_str(), out.host.as_str()), ("a", "h"));
        assert_eq!(out.port, 2222);
        assert_eq!(out.favorite, 1);
        assert_eq!(out.auth_type, "password");
        assert_eq!(normalize_import_host(&obj(json!({}))).port, 22);
        assert_eq!(normalize_import_host(&obj(json!({"port": "abc"}))).port, 22);
        assert_eq!(normalize_import_host(&obj(json!({"port": null}))).port, 0);
        assert_eq!(
            normalize_import_host(&obj(json!({"auth_type": "key"}))).auth_type,
            "key"
        );
        assert_eq!(
            normalize_import_host(&obj(json!({"auth_type": "agent"}))).auth_type,
            "password"
        );
        let empty = normalize_import_host(&obj(json!({"name": "a"})));
        assert_eq!(
            (empty.key_path, empty.group, empty.mongo_uri),
            (None, None, None)
        );
        assert_eq!(
            normalize_import_host(&obj(json!({"group": 5})))
                .group
                .as_deref(),
            Some("5")
        );
    }

    #[test]
    fn an_old_export_has_its_mongo_password_stripped() {
        let out = normalize_import_host(&obj(json!({
            "name": "m",
            "mongo_uri": "mongodb://u:hunter2@h:27017",
            "mongo_local_uri": "mongodb://u:pw@localhost",
        })));
        assert_eq!(out.mongo_uri.as_deref(), Some("mongodb://u@h:27017"));
        assert_eq!(
            out.mongo_local_uri.as_deref(),
            Some("mongodb://u@localhost")
        );
    }

    #[test]
    fn summaries_pluralise_and_name_the_first_failure() {
        let s = |added, replaced, failed: &[(&str, &str)]| ImportSummary {
            added,
            replaced,
            failed: failed
                .iter()
                .map(|(a, b)| (a.to_string(), b.to_string()))
                .collect(),
        };
        assert_eq!(
            summarize_import(&s(1, 0, &[]), 0),
            ("Imported 1 host".into(), SummaryTone::Success)
        );
        assert_eq!(summarize_import(&s(3, 0, &[]), 0).0, "Imported 3 hosts");
        assert_eq!(
            summarize_import(&s(2, 1, &[]), 0).0,
            "Imported 2 hosts, replaced 1 host"
        );
        assert_eq!(
            summarize_import(&s(1, 0, &[]), 4).0,
            "Imported 1 host, skipped 4"
        );
        let one = summarize_import(&s(1, 0, &[("bad", "Port")]), 0);
        assert_eq!(
            one,
            (
                "Imported 1 host, 1 failure: bad (Port)".into(),
                SummaryTone::Warning
            )
        );
        assert_eq!(
            summarize_import(&s(0, 0, &[("a", "Port"), ("b", "Name")]), 0).0,
            "2 failures: a (Port) and 1 more"
        );
        assert_eq!(
            summarize_import(&s(0, 0, &[("", "Name")]), 0).0,
            "1 failure: unnamed (Name)"
        );
        assert_eq!(
            summarize_import(&ImportSummary::default(), 0),
            ("Nothing to import".into(), SummaryTone::Info)
        );
    }

    #[test]
    fn jump_specs_split_into_parts() {
        let s = |u: Option<&str>, h: &str, p: Option<u16>| JumpSpec {
            user: u.map(String::from),
            host: h.into(),
            port: p,
        };
        assert_eq!(parse_jump_spec("bastion"), s(None, "bastion", None));
        assert_eq!(
            parse_jump_spec("me@b.example"),
            s(Some("me"), "b.example", None)
        );
        assert_eq!(parse_jump_spec("b:2222"), s(None, "b", Some(2222)));
        assert_eq!(
            parse_jump_spec("me@[::1]:22"),
            s(Some("me"), "::1", Some(22))
        );
        assert_eq!(parse_jump_spec("[fe80::1]"), s(None, "fe80::1", None));
    }

    fn saved(id: i64, name: &str, host: &str, jump: Option<i64>) -> HostSummary {
        HostRecord {
            id,
            name: name.into(),
            host: host.into(),
            port: 22,
            username: "me".into(),
            jump_host_id: jump,
            ..Default::default()
        }
        .summary()
    }

    fn entry(name: &str, host: &str, jump: Option<&str>) -> SshConfigEntry {
        SshConfigEntry {
            name: name.into(),
            host: host.into(),
            port: 22,
            username: "me".into(),
            auth_type: "key".into(),
            key_path: None,
            proxy_jump: jump.map(String::from),
        }
    }

    #[test]
    fn proxy_jumps_link_by_name_then_address() {
        let hosts = vec![
            saved(1, "bastion", "b.example", None),
            saved(2, "db", "10.0.0.2", None),
            saved(3, "cache", "10.0.0.3", None),
        ];
        let imported = vec![
            entry("db", "10.0.0.2", Some("bastion")),
            entry("cache", "10.0.0.3", Some("me@b.example:22")),
            entry("lost", "10.0.0.9", Some("nowhere")),
        ];
        let (links, unresolved) = link_proxy_jumps(&imported, &hosts);
        assert_eq!(
            links,
            [
                JumpLink {
                    host_id: 2,
                    jump_id: 1
                },
                JumpLink {
                    host_id: 3,
                    jump_id: 1
                }
            ]
        );
        // "lost" has no row (it was not imported), so it is not reported.
        assert!(unresolved.is_empty());

        let hosts = vec![
            saved(1, "bastion", "b.example", None),
            saved(2, "db", "10.0.0.2", None),
            saved(4, "lost", "10.0.0.9", None),
        ];
        let (_, unresolved) = link_proxy_jumps(&imported, &hosts);
        assert_eq!(
            unresolved,
            [(
                "lost".to_string(),
                "no saved host matches nowhere".to_string()
            )]
        );
        assert_eq!(
            unresolved_jumps_message(&unresolved).unwrap(),
            "Jump host not linked for lost (no saved host matches nowhere). Set it by editing the host."
        );
    }

    #[test]
    fn a_chained_jump_is_refused() {
        let hosts = vec![
            saved(1, "outer", "o.example", None),
            saved(2, "inner", "i.example", Some(1)),
            saved(3, "db", "10.0.0.2", None),
        ];
        let (links, unresolved) =
            link_proxy_jumps(&[entry("db", "10.0.0.2", Some("inner"))], &hosts);
        assert!(links.is_empty());
        assert_eq!(unresolved[0].1, "its jump host has a jump host of its own");
    }

    #[test]
    fn entries_already_saved_are_recognised() {
        let hosts = vec![saved(1, "bastion", "b.example", None)];
        assert!(already_saved(&entry("bastion", "b.example", None), &hosts));
        assert!(!already_saved(
            &entry("bastion", "other.example", None),
            &hosts
        ));
        assert!(!already_saved(&entry("box", "b.example", None), &hosts));
    }

    #[test]
    fn a_config_entry_imports_like_the_vue_dialog_sent_it() {
        let h = ssh_config_import_host(&SshConfigEntry {
            key_path: Some("~/.ssh/id".into()),
            ..entry(" box ", "h", Some("bastion"))
        });
        assert_eq!(
            (h.name.as_str(), h.port, h.auth_type.as_str()),
            ("box", 22, "key")
        );
        assert_eq!(h.key_path.as_deref(), Some("~/.ssh/id"));
    }

    #[test]
    fn labels_and_file_names() {
        assert_eq!(import_button_label(1), "Import 1 host");
        assert_eq!(import_button_label(0), "Import 0 hosts");
        // 2026-10-22T12:00:00Z
        assert_eq!(export_file_name(1_792_670_400), "ssh-hosts-2026-10-22.json");
        assert_eq!(export_file_name(0), "ssh-hosts-1970-01-01.json");
    }
}
