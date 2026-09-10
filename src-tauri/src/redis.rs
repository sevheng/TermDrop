//! Redis connections: browsing a keyspace, and backing it up.
//!
//! Mirrors `mongodb.rs` in shape and in convention — every result is
//! `Result<T, String>`, every error is redacted through [`crate::uri`], and the
//! exact string `"cancelled"` is what a cancelled operation returns.
//!
//! Two rules run through the whole module:
//!
//! - **Nothing here may be unbounded.** `KEYS`, `HGETALL`, `SMEMBERS` and
//!   `LRANGE key 0 -1` are all O(N) with an unbounded reply, and running one
//!   against a production keyspace stalls the server for every other client.
//!   Every read below is a `SCAN` variant or an explicit range, capped by the
//!   constants in this module.
//! - **Keys are bytes, not text.** A key that is not valid UTF-8 must still be
//!   addressable, so keys cross the IPC boundary as both a display string and
//!   their exact bytes, and every follow-up call names the key by the latter.
//!
//! The module is `mod redis`, which shadows the `redis` crate inside this
//! crate. `mongodb.rs` has the same collision and solves it the same way: the
//! crate is always `::redis::`, as in `main.rs`'s `::mongodb::Client`.

use ::redis::aio::MultiplexedConnection;
use ::redis::{ConnectionInfo, IntoConnectionInfo};
use base64::Engine;
use serde::Serialize;
use std::time::Duration;

/// How long to wait for a server before giving up.
///
/// Matches the MongoDB path's 8s: the driver's own default is far longer, and
/// that is a long time to stare at a spinner for a host that is not there.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(8);

// ---------------------------------------------------------------------------
// URI handling
// ---------------------------------------------------------------------------

/// Whether a URI names TLS (`rediss://`).
pub fn is_tls_uri(uri: &str) -> bool {
    uri.trim_start()
        .to_ascii_lowercase()
        .starts_with("rediss://")
}

/// The database index from a URI's path, if it names one.
///
/// `redis://host:6379/2` is db 2. A missing or unparseable path is `None`,
/// which the caller reads as db 0.
pub fn uri_db_index(uri: &str) -> Option<i64> {
    let after_scheme = uri.split_once("://")?.1;
    let path = after_scheme.split_once('/')?.1;
    let path = path.split(['?', '#']).next()?;
    path.parse::<i64>().ok().filter(|d| *d >= 0)
}

/// The byte range of a URI's host and port, i.e. the authority with any
/// userinfo removed.
fn hostport_range(uri: &str) -> Option<(usize, usize)> {
    let scheme_end = uri.find("://")?;
    let start = scheme_end + 3;
    let end = uri[start..]
        .find(['/', '?', '#'])
        .map(|idx| start + idx)
        .unwrap_or(uri.len());
    // Userinfo, when present, ends at the last `@` inside the authority.
    let host_start = uri[start..end]
        .rfind('@')
        .map(|at| start + at + 1)
        .unwrap_or(start);
    Some((host_start, end))
}

/// Point a Redis URI at the local end of a tunnel, returning the rewritten URI
/// and the host it originally named.
///
/// Only the host and port change: the scheme, userinfo, database path and
/// query are all preserved, because the credential and the db index still
/// apply — it is the same server, reached by a different route.
pub fn retarget_uri(uri: &str, local_port: u16) -> Result<(String, String), String> {
    let (start, end) =
        hostport_range(uri).ok_or_else(|| format!("not a URI: {}", crate::uri::redact_uri(uri)))?;
    let original = &uri[start..end];
    if original.is_empty() {
        return Err("the Redis URI names no host to tunnel to".to_string());
    }
    let host = original
        .rsplit_once(':')
        // A colon inside `[::1]` is part of the address, not a port separator.
        .filter(|(before, _)| !before.ends_with(']') || before.starts_with('['))
        .map(|(h, _)| h)
        .unwrap_or(original);
    let rewritten = format!("{}127.0.0.1:{}{}", &uri[..start], local_port, &uri[end..]);
    Ok((rewritten, host.to_string()))
}

/// The host and port a Redis URI names, for a tunnel's remote end.
pub fn uri_endpoint(uri: &str) -> Result<(String, u16), String> {
    let (start, end) =
        hostport_range(uri).ok_or_else(|| format!("not a URI: {}", crate::uri::redact_uri(uri)))?;
    let authority = &uri[start..end];
    if authority.is_empty() {
        return Err("the Redis URI names no host".to_string());
    }
    // IPv6 literals are bracketed, and their colons are not port separators.
    if let Some(rest) = authority.strip_prefix('[') {
        let (addr, tail) = rest
            .split_once(']')
            .ok_or_else(|| "unterminated IPv6 address in the Redis URI".to_string())?;
        let port = match tail.strip_prefix(':') {
            Some(p) => p
                .parse::<u16>()
                .map_err(|_| format!("bad port in the Redis URI: {}", p))?,
            None => 6379,
        };
        return Ok((addr.to_string(), port));
    }
    match authority.rsplit_once(':') {
        Some((host, port)) => {
            let port = port
                .parse::<u16>()
                .map_err(|_| format!("bad port in the Redis URI: {}", port))?;
            Ok((host.to_string(), port))
        }
        None => Ok((authority.to_string(), 6379)),
    }
}

// ---------------------------------------------------------------------------
// INFO parsing
// ---------------------------------------------------------------------------

/// One database's key counts, as reported by `INFO keyspace`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RedisDbInfo {
    pub index: i64,
    pub keys: u64,
    pub expires: u64,
}

/// A field from an `INFO` reply, which is `key:value` lines separated by CRLF.
pub fn info_field<'a>(info: &'a str, key: &str) -> Option<&'a str> {
    info.lines()
        .filter(|l| !l.starts_with('#'))
        .find_map(|l| l.split_once(':').filter(|(k, _)| *k == key))
        .map(|(_, v)| v.trim_end())
}

/// Parse `INFO keyspace`, whose body is `db0:keys=12,expires=3,avg_ttl=0`.
///
/// Databases with no keys are simply absent from the reply, so this reports
/// what exists rather than what is configured; the database *count* comes from
/// `CONFIG GET databases`, which managed providers often disable.
pub fn parse_keyspace_info(info: &str) -> Vec<RedisDbInfo> {
    let mut out = Vec::new();
    for line in info.lines() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix("db") else {
            continue;
        };
        let Some((index, fields)) = rest.split_once(':') else {
            continue;
        };
        let Ok(index) = index.parse::<i64>() else {
            continue;
        };
        let mut keys = 0u64;
        let mut expires = 0u64;
        for field in fields.split(',') {
            match field.split_once('=') {
                Some(("keys", v)) => keys = v.parse().unwrap_or(0),
                Some(("expires", v)) => expires = v.parse().unwrap_or(0),
                _ => {}
            }
        }
        out.push(RedisDbInfo {
            index,
            keys,
            expires,
        });
    }
    out.sort_by_key(|d| d.index);
    out
}

/// A Redis version string as `(major, minor, patch)`.
pub fn parse_version(version: &str) -> Option<(u32, u32, u32)> {
    let mut parts = version.trim().split('.');
    let major = parts.next()?.parse().ok()?;
    // A missing minor or patch is 0, so "7" parses rather than failing.
    let minor = parts.next().and_then(|p| p.parse().ok()).unwrap_or(0);
    let patch = parts.next().and_then(|p| {
        p.chars()
            .take_while(char::is_ascii_digit)
            .collect::<String>()
            .parse()
            .ok()
    });
    Some((major, minor, patch.unwrap_or(0)))
}

/// Whether a version string is at least `major.minor`.
///
/// Used to gate features rather than to guess at compatibility: `SCAN … TYPE`
/// needs 6.0, and falling back is cheap, so an unparseable version is treated
/// as too old.
pub fn version_at_least(version: &str, major: u32, minor: u32) -> bool {
    match parse_version(version) {
        Some((ma, mi, _)) => (ma, mi) >= (major, minor),
        None => false,
    }
}

/// What the app knows about a connected server.
#[derive(Debug, Clone, Serialize)]
pub struct RedisServerInfo {
    pub version: String,
    /// `standalone`, `cluster` or `sentinel`.
    pub mode: String,
    pub databases: Vec<RedisDbInfo>,
    /// How many databases the server is configured for, when it will say.
    pub database_count: Option<i64>,
    /// Whether this connection runs through an SSH tunnel.
    pub tunnelled: bool,
    /// The connection URI with its credentials masked, for the panel header.
    pub display_uri: String,
    /// True when `SCAN … TYPE` is available (Redis 6.0+).
    pub supports_scan_type: bool,
}

// ---------------------------------------------------------------------------
// Connecting
// ---------------------------------------------------------------------------

/// Build the driver's connection info, forcing the database index.
///
/// The index is baked in here rather than issued as a `SELECT` at runtime:
/// `MultiplexedConnection` pipelines commands from different tasks over one
/// socket, so a `SELECT` from one pane would silently change the database under
/// another. One connection per `(host, db)` is what makes that safe.
fn connection_info(uri: &str, db: i64) -> Result<ConnectionInfo, String> {
    let info = uri.into_connection_info().map_err(|e| {
        format!(
            "parse uri: {}",
            crate::uri::redact_uris_in_text(&e.to_string())
        )
    })?;
    let redis = info.redis_settings().clone().set_db(db);
    Ok(info.set_redis_settings(redis))
}

/// Open a multiplexed connection to one database.
pub async fn build_client(uri: &str, db: i64) -> Result<MultiplexedConnection, String> {
    let client = ::redis::Client::open(connection_info(uri, db)?).map_err(|e| {
        format!(
            "create client: {}",
            crate::uri::redact_uris_in_text(&e.to_string())
        )
    })?;

    let connect = client.get_multiplexed_async_connection();
    match tokio::time::timeout(CONNECT_TIMEOUT, connect).await {
        Ok(Ok(conn)) => Ok(conn),
        Ok(Err(e)) => Err(format!(
            "connect: {}",
            crate::uri::redact_uris_in_text(&e.to_string())
        )),
        Err(_) => Err(format!(
            "connect: timed out after {} seconds",
            CONNECT_TIMEOUT.as_secs()
        )),
    }
}

/// Read `INFO` and the keyspace, plus the database count when the server will
/// report it.
pub async fn server_info(
    conn: &mut MultiplexedConnection,
    display_uri: String,
    tunnelled: bool,
) -> Result<RedisServerInfo, String> {
    let server: String = ::redis::cmd("INFO")
        .arg("server")
        .query_async(conn)
        .await
        .map_err(|e| format!("INFO: {}", crate::uri::redact_uris_in_text(&e.to_string())))?;

    let keyspace: String = ::redis::cmd("INFO")
        .arg("keyspace")
        .query_async(conn)
        .await
        .map_err(|e| {
            format!(
                "INFO keyspace: {}",
                crate::uri::redact_uris_in_text(&e.to_string())
            )
        })?;

    let version = info_field(&server, "redis_version")
        .unwrap_or("unknown")
        .to_string();
    let mode = info_field(&server, "redis_mode")
        .unwrap_or("standalone")
        .to_string();

    // CONFIG is frequently disabled on managed Redis, and a missing database
    // count is not an error -- the keyspace list still works without it.
    let database_count: Option<i64> = ::redis::cmd("CONFIG")
        .arg("GET")
        .arg("databases")
        .query_async::<Vec<String>>(conn)
        .await
        .ok()
        .and_then(|pair| pair.get(1).and_then(|v| v.parse().ok()));

    Ok(RedisServerInfo {
        version: version.clone(),
        mode,
        databases: parse_keyspace_info(&keyspace),
        database_count,
        tunnelled,
        display_uri,
        supports_scan_type: version_at_least(&version, 6, 0),
    })
}

// ---------------------------------------------------------------------------
// Bounds
// ---------------------------------------------------------------------------

/// Keys per page of the browser, mirroring MongoDB's `MAX_FIND_LIMIT`.
pub const MAX_KEYS_PER_PAGE: i64 = 200;
/// Elements per page when viewing one collection-shaped key.
pub const MAX_ELEMENTS_PER_PAGE: i64 = 200;
/// How much of a single value is ever fetched. `GET` on a 512 MiB string would
/// pull all of it across the socket to render a preview.
pub const MAX_VALUE_PREVIEW_BYTES: usize = 256 * 1024;
/// How many `SCAN` round trips one browser page may cost.
///
/// `COUNT` is only a hint, and a heavily filtered `MATCH` can return nothing
/// for many iterations. Without a ceiling one page request could scan an entire
/// keyspace; with it, the page comes back with a cursor to resume from.
pub const MAX_SCAN_ITERATIONS: u32 = 50;
/// How many keys the prefix tree will scan before reporting a partial result.
pub const TREE_SCAN_CAP: u64 = 20_000;
/// Above this, a key is recorded as skipped rather than dumped. `DUMP` of a
/// very large key serialises it synchronously, stalling every other client.
pub const MAX_DUMP_KEY_BYTES: u64 = 64 * 1024 * 1024;

// ---------------------------------------------------------------------------
// Binary-safe values
// ---------------------------------------------------------------------------

/// A Redis key or value on its way to the frontend.
///
/// Redis keys and values are arbitrary bytes and JSON strings are not, so a
/// value that is not valid UTF-8 is carried as base64 with `binary` set. That
/// matters most for *keys*: decoding one lossily would produce a name that
/// cannot be fetched again, which looks like a working feature and is not.
#[derive(Debug, Clone, Serialize)]
pub struct RedisBytes {
    /// UTF-8 text, or standard base64 when `binary`.
    pub text: String,
    pub binary: bool,
    /// The true length in bytes, before any truncation.
    pub bytes: u64,
    pub truncated: bool,
}

impl RedisBytes {
    /// Render bytes we hold in full.
    fn whole(raw: &[u8]) -> Self {
        Self::new(raw, raw.len() as u64, false)
    }

    /// Render a prefix of a value whose full length is known separately.
    fn partial(raw: &[u8], total: u64) -> Self {
        let truncated = (raw.len() as u64) < total;
        Self::new(raw, total, truncated)
    }

    fn new(raw: &[u8], total: u64, truncated: bool) -> Self {
        match std::str::from_utf8(raw) {
            Ok(text) => Self {
                text: text.to_string(),
                binary: false,
                bytes: total,
                truncated,
            },
            Err(_) => Self {
                text: base64::engine::general_purpose::STANDARD.encode(raw),
                binary: true,
                bytes: total,
                truncated,
            },
        }
    }
}

/// The exact bytes of a key, as the frontend sends them back.
fn encode_key(raw: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(raw)
}

/// Decode a key the frontend sent back.
pub fn decode_key(key_b64: &str) -> Result<Vec<u8>, String> {
    base64::engine::general_purpose::STANDARD
        .decode(key_b64)
        .map_err(|_| "malformed key".to_string())
}

// ---------------------------------------------------------------------------
// Key browsing
// ---------------------------------------------------------------------------

/// One key in the browser's list.
#[derive(Debug, Clone, Serialize)]
pub struct RedisKeySummary {
    pub key: RedisBytes,
    pub key_b64: String,
    /// `string`, `list`, `set`, `zset`, `hash`, `stream`, or `none` when the
    /// key expired between the scan and the follow-up pipeline.
    pub kind: String,
    /// Milliseconds until expiry; `-1` never expires, `-2` already gone.
    pub ttl_ms: i64,
    pub memory_bytes: Option<u64>,
}

/// One page of the keyspace.
#[derive(Debug, Clone, Serialize)]
pub struct RedisScanPage {
    pub keys: Vec<RedisKeySummary>,
    /// The cursor to resume from, as a decimal string. `"0"` means the
    /// iteration finished — see [`scan_keys`] for why this is not a number.
    pub cursor: String,
    pub done: bool,
}

/// Run `SCAN` until it has enough keys, the iteration finishes, or the
/// round-trip ceiling is hit.
///
/// The cursor is a **string** all the way to the frontend and back. Redis
/// cursors are u64, which exceeds `Number.MAX_SAFE_INTEGER`, so parsing one as
/// a JS number would corrupt it on a large keyspace and silently restart the
/// iteration.
///
/// Looping here rather than in the frontend is what makes a page useful:
/// `COUNT` is a hint, so a single `SCAN` may legitimately return zero keys with
/// a non-zero cursor, and a UI that took that as "no results" would be wrong.
async fn scan_key_names(
    conn: &mut MultiplexedConnection,
    cursor: &str,
    pattern: Option<&str>,
    type_filter: Option<&str>,
    limit: i64,
    supports_scan_type: bool,
) -> Result<(Vec<Vec<u8>>, String), String> {
    let limit = limit.clamp(1, MAX_KEYS_PER_PAGE);
    let mut cursor = cursor.to_string();
    let mut found: Vec<Vec<u8>> = Vec::new();

    for _ in 0..MAX_SCAN_ITERATIONS {
        let mut cmd = ::redis::cmd("SCAN");
        cmd.arg(&cursor);
        if let Some(p) = pattern {
            cmd.arg("MATCH").arg(p);
        }
        // Ask for more than one page's worth: MATCH filters server-side after
        // COUNT is applied, so a selective pattern returns far fewer.
        cmd.arg("COUNT").arg(limit * 5);
        if supports_scan_type {
            if let Some(t) = type_filter {
                cmd.arg("TYPE").arg(t);
            }
        }

        let (next, batch): (String, Vec<Vec<u8>>) = cmd
            .query_async(conn)
            .await
            .map_err(|e| format!("SCAN: {}", crate::uri::redact_uris_in_text(&e.to_string())))?;

        found.extend(batch);
        cursor = next;

        if cursor == "0" || found.len() as i64 >= limit {
            break;
        }
    }

    found.truncate(limit as usize);
    Ok((found, cursor))
}

/// Attach type, TTL and optionally memory to a batch of keys in one round trip.
///
/// Naively this is three commands per key — 600 round trips for a 200-key page.
/// One pipeline makes it one.
async fn describe_keys(
    conn: &mut MultiplexedConnection,
    keys: Vec<Vec<u8>>,
    with_memory: bool,
) -> Result<Vec<RedisKeySummary>, String> {
    if keys.is_empty() {
        return Ok(Vec::new());
    }

    let mut pipe = ::redis::pipe();
    for key in &keys {
        pipe.cmd("TYPE").arg(key);
        pipe.cmd("PTTL").arg(key);
        if with_memory {
            pipe.cmd("MEMORY").arg("USAGE").arg(key);
        }
    }

    let replies: Vec<::redis::Value> = pipe.query_async(conn).await.map_err(|e| {
        format!(
            "describe keys: {}",
            crate::uri::redact_uris_in_text(&e.to_string())
        )
    })?;

    let stride = if with_memory { 3 } else { 2 };
    let mut out = Vec::with_capacity(keys.len());
    for (i, key) in keys.iter().enumerate() {
        let kind = replies
            .get(i * stride)
            .and_then(value_to_string)
            .unwrap_or_else(|| "none".to_string());
        let ttl_ms = replies
            .get(i * stride + 1)
            .and_then(value_to_i64)
            .unwrap_or(-1);
        // MEMORY USAGE is disabled on several managed providers; a missing
        // answer is not an error, the column simply stays blank.
        let memory_bytes = if with_memory {
            replies
                .get(i * stride + 2)
                .and_then(value_to_i64)
                .and_then(|v| u64::try_from(v).ok())
        } else {
            None
        };

        out.push(RedisKeySummary {
            key: RedisBytes::whole(key),
            key_b64: encode_key(key),
            kind,
            ttl_ms,
            memory_bytes,
        });
    }
    Ok(out)
}

fn value_to_string(v: &::redis::Value) -> Option<String> {
    match v {
        ::redis::Value::SimpleString(s) => Some(s.clone()),
        ::redis::Value::BulkString(b) => Some(String::from_utf8_lossy(b).to_string()),
        _ => None,
    }
}

fn value_to_i64(v: &::redis::Value) -> Option<i64> {
    match v {
        ::redis::Value::Int(i) => Some(*i),
        ::redis::Value::BulkString(b) => String::from_utf8_lossy(b).parse().ok(),
        _ => None,
    }
}

/// One page of the keyspace, with each key described.
pub async fn scan_keys(
    conn: &mut MultiplexedConnection,
    cursor: &str,
    pattern: Option<&str>,
    type_filter: Option<&str>,
    limit: i64,
    with_memory: bool,
    supports_scan_type: bool,
) -> Result<RedisScanPage, String> {
    let (names, next) = scan_key_names(
        conn,
        cursor,
        pattern,
        type_filter,
        limit,
        supports_scan_type,
    )
    .await?;

    // On a server too old for `SCAN … TYPE`, filter after describing instead.
    let mut keys = describe_keys(conn, names, with_memory).await?;
    if !supports_scan_type {
        if let Some(t) = type_filter {
            keys.retain(|k| k.kind == t);
        }
    }

    Ok(RedisScanPage {
        done: next == "0",
        cursor: next,
        keys,
    })
}

// ---------------------------------------------------------------------------
// Prefix tree
// ---------------------------------------------------------------------------

/// One `prefix:`-shaped group of keys.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PrefixGroup {
    pub prefix: String,
    pub count: u64,
    /// True when the prefix itself is not valid UTF-8 and `prefix` is base64.
    pub binary: bool,
}

/// Group keys by their first `delimiter`-separated segment.
///
/// Keys with no delimiter are their own group, which is what makes a flat
/// keyspace still readable rather than collapsing into one nameless bucket.
/// Groups come back ordered by descending count so the big ones are visible
/// first, with the name as a tiebreak so the order is stable between scans.
pub fn group_by_prefix(keys: &[Vec<u8>], delimiter: u8) -> Vec<PrefixGroup> {
    use std::collections::HashMap;
    let mut counts: HashMap<Vec<u8>, u64> = HashMap::new();
    for key in keys {
        let head = match key.iter().position(|b| *b == delimiter) {
            Some(idx) => &key[..idx],
            None => &key[..],
        };
        *counts.entry(head.to_vec()).or_insert(0) += 1;
    }

    let mut out: Vec<PrefixGroup> = counts
        .into_iter()
        .map(|(raw, count)| match String::from_utf8(raw.clone()) {
            Ok(prefix) => PrefixGroup {
                prefix,
                count,
                binary: false,
            },
            Err(_) => PrefixGroup {
                prefix: base64::engine::general_purpose::STANDARD.encode(&raw),
                count,
                binary: true,
            },
        })
        .collect();
    out.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.prefix.cmp(&b.prefix)));
    out
}

/// The prefix groups of a database, from a bounded scan.
#[derive(Debug, Clone, Serialize)]
pub struct KeyTree {
    pub groups: Vec<PrefixGroup>,
    pub scanned: u64,
    /// True when the scan hit its cap or time budget, so the groups describe
    /// only part of the keyspace. The UI must say so rather than imply the
    /// list is complete.
    pub truncated: bool,
}

/// Build the prefix groups for a database.
///
/// Deliberately capped in both keys and wall-clock: grouping the *whole*
/// keyspace means scanning it, which is what paging exists to avoid. Reporting
/// a partial answer honestly beats either stalling or lying.
pub async fn key_tree(
    conn: &mut MultiplexedConnection,
    pattern: Option<&str>,
    cap: u64,
    budget: Duration,
) -> Result<KeyTree, String> {
    let started = std::time::Instant::now();
    let mut cursor = "0".to_string();
    let mut keys: Vec<Vec<u8>> = Vec::new();
    let mut truncated = false;

    loop {
        let mut cmd = ::redis::cmd("SCAN");
        cmd.arg(&cursor);
        if let Some(p) = pattern {
            cmd.arg("MATCH").arg(p);
        }
        cmd.arg("COUNT").arg(1000);

        let (next, batch): (String, Vec<Vec<u8>>) = cmd
            .query_async(conn)
            .await
            .map_err(|e| format!("SCAN: {}", crate::uri::redact_uris_in_text(&e.to_string())))?;
        keys.extend(batch);
        cursor = next;

        if cursor == "0" {
            break;
        }
        if keys.len() as u64 >= cap || started.elapsed() >= budget {
            truncated = true;
            break;
        }
    }

    Ok(KeyTree {
        groups: group_by_prefix(&keys, b':'),
        scanned: keys.len() as u64,
        truncated,
    })
}

// ---------------------------------------------------------------------------
// Reading one key
// ---------------------------------------------------------------------------

/// One element of a key's value.
///
/// Shapes differ by type, so the unused parts are `None`: a hash fills `field`
/// and `value`, a sorted set fills `value` and `score`, a stream puts its entry
/// id in `field`.
#[derive(Debug, Clone, Serialize)]
pub struct RedisEntry {
    pub field: Option<RedisBytes>,
    pub value: RedisBytes,
    pub score: Option<f64>,
}

/// One page of a key's value.
#[derive(Debug, Clone, Serialize)]
pub struct RedisValuePage {
    pub kind: String,
    pub entries: Vec<RedisEntry>,
    /// Total elements (or bytes, for a string), when the type can say cheaply.
    pub total: Option<u64>,
    pub ttl_ms: i64,
    pub encoding: Option<String>,
    /// Cursor for the `SCAN`-paged types (hash, set); `"0"` when finished.
    pub cursor: String,
    pub done: bool,
}

/// Read one page of a key's value, bounded for every type.
///
/// The command per type is chosen so that no single call can return an
/// unbounded reply: `HSCAN`/`SSCAN` rather than `HGETALL`/`SMEMBERS`, an
/// explicit index window rather than `LRANGE key 0 -1`, and `GETRANGE` rather
/// than `GET`. A 10-million-element set is as cheap to open as an empty one.
pub async fn key_value(
    conn: &mut MultiplexedConnection,
    key: &[u8],
    cursor: &str,
    offset: i64,
    limit: i64,
) -> Result<RedisValuePage, String> {
    let limit = limit.clamp(1, MAX_ELEMENTS_PER_PAGE);
    let offset = offset.max(0);

    let kind: String = ::redis::cmd("TYPE")
        .arg(key)
        .query_async(conn)
        .await
        .map_err(|e| format!("TYPE: {}", crate::uri::redact_uris_in_text(&e.to_string())))?;

    let ttl_ms: i64 = ::redis::cmd("PTTL")
        .arg(key)
        .query_async(conn)
        .await
        .unwrap_or(-1);

    // OBJECT ENCODING is informational and restricted on some providers.
    let encoding: Option<String> = ::redis::cmd("OBJECT")
        .arg("ENCODING")
        .arg(key)
        .query_async(conn)
        .await
        .ok();

    let mut next_cursor = "0".to_string();
    let (entries, total) = match kind.as_str() {
        "string" => {
            let total: u64 = ::redis::cmd("STRLEN")
                .arg(key)
                .query_async(conn)
                .await
                .map_err(|e| {
                    format!(
                        "STRLEN: {}",
                        crate::uri::redact_uris_in_text(&e.to_string())
                    )
                })?;
            let raw: Vec<u8> = ::redis::cmd("GETRANGE")
                .arg(key)
                .arg(0)
                .arg(MAX_VALUE_PREVIEW_BYTES as i64 - 1)
                .query_async(conn)
                .await
                .map_err(|e| {
                    format!(
                        "GETRANGE: {}",
                        crate::uri::redact_uris_in_text(&e.to_string())
                    )
                })?;
            (
                vec![RedisEntry {
                    field: None,
                    value: RedisBytes::partial(&raw, total),
                    score: None,
                }],
                Some(total),
            )
        }
        "hash" => {
            // COUNT is a hint, not a limit -- see MAX_ELEMENTS_PER_PAGE.
            let total: u64 = ::redis::cmd("HLEN")
                .arg(key)
                .query_async(conn)
                .await
                .ok()
                .unwrap_or(0);
            let (next, flat): (String, Vec<Vec<u8>>) = ::redis::cmd("HSCAN")
                .arg(key)
                .arg(cursor)
                .arg("COUNT")
                .arg(limit)
                .query_async(conn)
                .await
                .map_err(|e| {
                    format!("HSCAN: {}", crate::uri::redact_uris_in_text(&e.to_string()))
                })?;
            next_cursor = next;
            let entries = flat
                .chunks(2)
                .filter(|pair| pair.len() == 2)
                .map(|pair| RedisEntry {
                    field: Some(RedisBytes::whole(&pair[0])),
                    value: RedisBytes::whole(&pair[1]),
                    score: None,
                })
                .collect();
            (entries, Some(total))
        }
        "set" => {
            let total: u64 = ::redis::cmd("SCARD")
                .arg(key)
                .query_async(conn)
                .await
                .ok()
                .unwrap_or(0);
            let (next, members): (String, Vec<Vec<u8>>) = ::redis::cmd("SSCAN")
                .arg(key)
                .arg(cursor)
                .arg("COUNT")
                .arg(limit)
                .query_async(conn)
                .await
                .map_err(|e| {
                    format!("SSCAN: {}", crate::uri::redact_uris_in_text(&e.to_string()))
                })?;
            next_cursor = next;
            let entries = members
                .iter()
                .map(|m| RedisEntry {
                    field: None,
                    value: RedisBytes::whole(m),
                    score: None,
                })
                .collect();
            (entries, Some(total))
        }
        "list" => {
            let total: u64 = ::redis::cmd("LLEN")
                .arg(key)
                .query_async(conn)
                .await
                .ok()
                .unwrap_or(0);
            let items: Vec<Vec<u8>> = ::redis::cmd("LRANGE")
                .arg(key)
                .arg(offset)
                .arg(offset + limit - 1)
                .query_async(conn)
                .await
                .map_err(|e| {
                    format!(
                        "LRANGE: {}",
                        crate::uri::redact_uris_in_text(&e.to_string())
                    )
                })?;
            let entries = items
                .iter()
                .enumerate()
                .map(|(i, v)| RedisEntry {
                    field: Some(RedisBytes::whole(
                        (offset + i as i64).to_string().as_bytes(),
                    )),
                    value: RedisBytes::whole(v),
                    score: None,
                })
                .collect();
            (entries, Some(total))
        }
        "zset" => {
            let total: u64 = ::redis::cmd("ZCARD")
                .arg(key)
                .query_async(conn)
                .await
                .ok()
                .unwrap_or(0);
            let flat: Vec<Vec<u8>> = ::redis::cmd("ZRANGE")
                .arg(key)
                .arg(offset)
                .arg(offset + limit - 1)
                .arg("WITHSCORES")
                .query_async(conn)
                .await
                .map_err(|e| {
                    format!(
                        "ZRANGE: {}",
                        crate::uri::redact_uris_in_text(&e.to_string())
                    )
                })?;
            let entries = flat
                .chunks(2)
                .filter(|pair| pair.len() == 2)
                .map(|pair| RedisEntry {
                    field: None,
                    value: RedisBytes::whole(&pair[0]),
                    score: std::str::from_utf8(&pair[1])
                        .ok()
                        .and_then(|s| s.parse().ok()),
                })
                .collect();
            (entries, Some(total))
        }
        "stream" => {
            let total: u64 = ::redis::cmd("XLEN")
                .arg(key)
                .query_async(conn)
                .await
                .ok()
                .unwrap_or(0);
            // Entries are `[id, [field, value, ...]]`; render the field list as
            // readable text rather than inventing a nested wire shape.
            let raw: Vec<(String, Vec<Vec<u8>>)> = ::redis::cmd("XRANGE")
                .arg(key)
                .arg("-")
                .arg("+")
                .arg("COUNT")
                .arg(limit)
                .query_async(conn)
                .await
                .map_err(|e| {
                    format!(
                        "XRANGE: {}",
                        crate::uri::redact_uris_in_text(&e.to_string())
                    )
                })?;
            let entries = raw
                .iter()
                .map(|(id, fields)| {
                    let rendered = fields
                        .chunks(2)
                        .filter(|p| p.len() == 2)
                        .map(|p| {
                            format!(
                                "{}={}",
                                String::from_utf8_lossy(&p[0]),
                                String::from_utf8_lossy(&p[1])
                            )
                        })
                        .collect::<Vec<_>>()
                        .join(" ");
                    RedisEntry {
                        field: Some(RedisBytes::whole(id.as_bytes())),
                        value: RedisBytes::whole(rendered.as_bytes()),
                        score: None,
                    }
                })
                .collect();
            (entries, Some(total))
        }
        // "none" is an expired key; anything else is a module type with no
        // viewer. Both report the type and stop rather than guessing.
        _ => (Vec::new(), None),
    };

    Ok(RedisValuePage {
        kind,
        entries,
        total,
        ttl_ms,
        encoding,
        done: next_cursor == "0",
        cursor: next_cursor,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keyspace_info_is_parsed_from_a_real_reply() {
        // As Redis actually sends it: a comment header and CRLF line endings.
        let info = "# Keyspace\r\ndb0:keys=12481,expires=307,avg_ttl=0\r\ndb3:keys=7,expires=0,avg_ttl=0\r\n";
        assert_eq!(
            parse_keyspace_info(info),
            vec![
                RedisDbInfo {
                    index: 0,
                    keys: 12481,
                    expires: 307
                },
                RedisDbInfo {
                    index: 3,
                    keys: 7,
                    expires: 0
                },
            ]
        );
    }

    #[test]
    fn an_empty_or_missing_keyspace_section_is_not_an_error() {
        // A server with no keys reports the header and nothing else.
        assert!(parse_keyspace_info("# Keyspace\r\n").is_empty());
        assert!(parse_keyspace_info("").is_empty());
        // Lines that merely start with "db" must not be mistaken for entries.
        assert!(parse_keyspace_info("dbfilename:dump.rdb\r\n").is_empty());
    }

    #[test]
    fn info_field_reads_one_value_and_ignores_comments() {
        let info = "# Server\r\nredis_version:7.2.4\r\nredis_mode:standalone\r\n";
        assert_eq!(info_field(info, "redis_version"), Some("7.2.4"));
        assert_eq!(info_field(info, "redis_mode"), Some("standalone"));
        assert_eq!(info_field(info, "absent"), None);
    }

    #[test]
    fn versions_parse_and_compare() {
        assert_eq!(parse_version("7.2.4"), Some((7, 2, 4)));
        assert_eq!(parse_version("6.0.16"), Some((6, 0, 16)));
        // Short and suffixed forms both appear in the wild.
        assert_eq!(parse_version("7"), Some((7, 0, 0)));
        assert_eq!(parse_version("7.4.0-rc1"), Some((7, 4, 0)));
        assert_eq!(parse_version("garbage"), None);

        assert!(version_at_least("7.2.4", 6, 0));
        assert!(version_at_least("6.0.0", 6, 0));
        assert!(!version_at_least("5.0.14", 6, 0));
        // An unreadable version must not enable a feature that may not exist.
        assert!(!version_at_least("garbage", 6, 0));
    }

    #[test]
    fn a_uri_names_its_database() {
        assert_eq!(uri_db_index("redis://h:6379/2"), Some(2));
        assert_eq!(uri_db_index("redis://:pw@h:6379/0?x=1"), Some(0));
        assert_eq!(uri_db_index("redis://h:6379"), None);
        assert_eq!(uri_db_index("redis://h:6379/"), None);
        assert_eq!(uri_db_index("redis://h:6379/notanumber"), None);
    }

    #[test]
    fn retarget_keeps_everything_but_the_host_and_port() {
        // The credential and the database still apply -- it is the same
        // server, reached by a different route.
        let (uri, host) = retarget_uri("redis://:pw@10.0.1.5:6379/2?x=1", 51234).unwrap();
        assert_eq!(uri, "redis://:pw@127.0.0.1:51234/2?x=1");
        assert_eq!(host, "10.0.1.5");

        // No port, no userinfo, no path.
        let (uri, host) = retarget_uri("redis://cache.internal", 4000).unwrap();
        assert_eq!(uri, "redis://127.0.0.1:4000");
        assert_eq!(host, "cache.internal");

        // An IPv6 literal's colons are part of the address.
        let (uri, host) = retarget_uri("redis://:pw@[fd00::1]:6379/0", 5000).unwrap();
        assert_eq!(uri, "redis://:pw@127.0.0.1:5000/0");
        assert_eq!(host, "[fd00::1]");
    }

    #[test]
    fn retarget_refuses_a_uri_with_no_host() {
        assert!(retarget_uri("not-a-uri", 1234).is_err());
        assert!(retarget_uri("redis:///0", 1234).is_err());
    }

    #[test]
    fn the_tunnel_endpoint_comes_from_the_uri() {
        assert_eq!(
            uri_endpoint("redis://:pw@10.0.1.5:6379/0").unwrap(),
            ("10.0.1.5".to_string(), 6379)
        );
        // Redis's default port, when the URI omits it.
        assert_eq!(
            uri_endpoint("redis://cache.internal").unwrap(),
            ("cache.internal".to_string(), 6379)
        );
        // The brackets are stripped: ssh2 wants the bare address.
        assert_eq!(
            uri_endpoint("redis://[fd00::1]:6380/1").unwrap(),
            ("fd00::1".to_string(), 6380)
        );
        assert_eq!(
            uri_endpoint("redis://[fd00::1]").unwrap(),
            ("fd00::1".to_string(), 6379)
        );
        assert!(uri_endpoint("redis://h:notaport").is_err());
        assert!(uri_endpoint("redis://[fd00::1:6379").is_err());
    }

    #[test]
    fn tls_uris_are_recognised() {
        assert!(is_tls_uri("rediss://h:6380"));
        assert!(is_tls_uri("REDISS://h:6380"));
        assert!(!is_tls_uri("redis://h:6379"));
    }

    #[test]
    fn keys_group_by_their_first_segment() {
        let keys: Vec<Vec<u8>> = [
            "user:1",
            "user:2",
            "user:3",
            "session:a",
            "session:b",
            "plain",
        ]
        .iter()
        .map(|k| k.as_bytes().to_vec())
        .collect();

        let groups = group_by_prefix(&keys, b':');
        assert_eq!(
            groups,
            vec![
                PrefixGroup {
                    prefix: "user".into(),
                    count: 3,
                    binary: false
                },
                PrefixGroup {
                    prefix: "session".into(),
                    count: 2,
                    binary: false
                },
                // A key with no delimiter is its own group, not a nameless
                // bucket -- otherwise a flat keyspace shows nothing useful.
                PrefixGroup {
                    prefix: "plain".into(),
                    count: 1,
                    binary: false
                },
            ]
        );
    }

    #[test]
    fn grouping_is_stable_when_counts_tie() {
        let keys: Vec<Vec<u8>> = ["b:1", "a:1", "c:1"]
            .iter()
            .map(|k| k.as_bytes().to_vec())
            .collect();
        let names: Vec<String> = group_by_prefix(&keys, b':')
            .into_iter()
            .map(|g| g.prefix)
            .collect();
        assert_eq!(names, vec!["a", "b", "c"], "ties must break by name");
    }

    #[test]
    fn a_binary_prefix_is_carried_as_base64_not_mangled() {
        let keys = vec![vec![0xff, 0xfe, b':', b'1'], vec![0xff, 0xfe, b':', b'2']];
        let groups = group_by_prefix(&keys, b':');
        assert_eq!(groups.len(), 1);
        assert!(groups[0].binary, "an invalid-UTF-8 prefix must say so");
        assert_eq!(groups[0].count, 2);
        // Decodable back to the original bytes, so the UI can act on it.
        let raw = base64::engine::general_purpose::STANDARD
            .decode(&groups[0].prefix)
            .unwrap();
        assert_eq!(raw, vec![0xff, 0xfe]);
    }

    #[test]
    fn a_key_round_trips_through_base64_even_when_it_is_not_utf8() {
        // The whole point of addressing keys by bytes: this key has no
        // faithful string form, and a lossy one would fetch nothing.
        let raw = vec![0x00, 0xff, b'k', 0xfe];
        let encoded = encode_key(&raw);
        assert_eq!(decode_key(&encoded).unwrap(), raw);

        let rendered = RedisBytes::whole(&raw);
        assert!(rendered.binary);
        assert_eq!(rendered.bytes, 4);
        assert!(!rendered.truncated);
    }

    #[test]
    fn a_truncated_value_reports_its_real_size() {
        // The preview is 4 bytes of a 9000-byte string: the UI must be able to
        // say "showing 4 of 9000", not imply the value is 4 bytes long.
        let rendered = RedisBytes::partial(b"abcd", 9000);
        assert!(rendered.truncated);
        assert_eq!(rendered.bytes, 9000);
        assert_eq!(rendered.text, "abcd");

        // A value that fits is not marked truncated.
        let whole = RedisBytes::partial(b"abcd", 4);
        assert!(!whole.truncated);
    }

    #[test]
    fn decoding_a_malformed_key_is_an_error_not_a_panic() {
        assert!(decode_key("not base64!!").is_err());
    }
}

// ---------------------------------------------------------------------------
// Backup and restore
// ---------------------------------------------------------------------------

use crate::redis_backup::{BackupHeader, BackupReader, BackupRecord, BackupWriter};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tauri::{Emitter, Window};

/// Keys per pipeline. Large enough that the round trip is amortised, small
/// enough that a cancel is noticed promptly and one batch fits in memory.
const BATCH: usize = 100;

/// How often progress reaches the frontend. One event per key would put tens of
/// thousands of messages on the IPC channel.
const PROGRESS_INTERVAL: Duration = Duration::from_millis(250);

/// A key skipped because dumping it would stall the server.
#[derive(Debug, Clone, Serialize)]
pub struct SkippedKey {
    pub key: String,
    pub bytes: u64,
}

/// A key that could not be written, and why.
#[derive(Debug, Clone, Serialize)]
pub struct FailedKey {
    pub key: String,
    pub reason: String,
}

/// What an export or import actually did.
///
/// Returned rather than squeezed into a toast: "restored 40,000 keys, skipped
/// 12 that already existed, could not restore 3" is the whole point of running
/// the operation, and a single line cannot carry it.
#[derive(Debug, Clone, Default, Serialize)]
pub struct RedisOpSummary {
    pub processed: u64,
    pub written: u64,
    pub skipped_existing: u64,
    pub skipped_big: Vec<SkippedKey>,
    /// Keys that expired between being scanned and being read.
    pub vanished: u64,
    pub failed: Vec<FailedKey>,
    pub elapsed_ms: u64,
}

/// Emits `redis-op-progress` and `redis-op-cancelled`.
///
/// Distinct event names from the MongoDB path's, so a Redis backup cannot
/// drive a MongoDB panel's progress bar.
struct Progress<'a> {
    window: &'a Window,
    op_id: &'a str,
    db: i64,
    last: std::time::Instant,
    highest: f64,
}

impl<'a> Progress<'a> {
    fn new(window: &'a Window, op_id: &'a str, db: i64) -> Self {
        Self {
            window,
            op_id,
            db,
            // Zero-time start so the first update always goes out.
            last: std::time::Instant::now() - PROGRESS_INTERVAL,
            highest: 0.0,
        }
    }

    /// Rate-limited, and monotonic: the bar must never walk backwards, which
    /// it otherwise would when the scan's estimate overshoots.
    fn update(&mut self, stage: &str, processed: u64, total: u64, detail: &str) {
        if self.last.elapsed() < PROGRESS_INTERVAL {
            return;
        }
        self.last = std::time::Instant::now();
        let percent = if total > 0 {
            (processed as f64 / total as f64 * 100.0).clamp(0.0, 99.0)
        } else {
            0.0
        };
        self.highest = self.highest.max(percent);
        self.emit(stage, processed, total, self.highest, detail);
    }

    fn done(&mut self, processed: u64) {
        self.emit("done", processed, processed, 100.0, "");
    }

    fn emit(&self, stage: &str, processed: u64, total: u64, percent: f64, detail: &str) {
        let _ = self.window.emit(
            "redis-op-progress",
            serde_json::json!({
                "opId": self.op_id,
                "db": self.db,
                "stage": stage,
                "processed": processed,
                "total": total,
                "percent": percent.round() as u64,
                "detail": detail,
            }),
        );
    }

    fn cancelled(&self) {
        let _ = self.window.emit(
            "redis-op-cancelled",
            serde_json::json!({ "opId": self.op_id, "db": self.db }),
        );
    }
}

/// The error a cancelled operation returns. Load-bearing: the frontend keys off
/// this exact string to tell a cancel from a failure.
const CANCELLED: &str = "cancelled";

fn key_label(key: &[u8]) -> String {
    match std::str::from_utf8(key) {
        Ok(text) => text.to_string(),
        Err(_) => format!("<binary {} bytes>", key.len()),
    }
}

/// Refuse to back up a cluster.
///
/// `SCAN` covers only the node it is issued against, so on a cluster the result
/// would be a silently partial "backup" — the worst kind of wrong for this
/// feature. Browsing one node is still useful and stays allowed.
fn refuse_cluster(mode: &str) -> Result<(), String> {
    if mode == "cluster" {
        return Err(
            "This server is running in cluster mode. A backup would cover only the \
                    node TermDrop is connected to, so it would be silently incomplete."
                .to_string(),
        );
    }
    Ok(())
}

/// `SCAN` + `DUMP` a database into a `.tdredis` file.
#[allow(clippy::too_many_arguments)]
pub async fn export_keys(
    window: &Window,
    cancelled: Arc<AtomicBool>,
    op_id: &str,
    conn: &mut MultiplexedConnection,
    db: i64,
    pattern: Option<&str>,
    output_path: &str,
    max_key_bytes: u64,
    source_uri: &str,
    server_version: &str,
    server_mode: &str,
) -> Result<RedisOpSummary, String> {
    refuse_cluster(server_mode)?;

    let started = std::time::Instant::now();
    let mut progress = Progress::new(window, op_id, db);
    let mut summary = RedisOpSummary::default();

    // The whole keyspace is scanned even with a MATCH, so DBSIZE is the honest
    // denominator for the bar.
    let total: u64 = ::redis::cmd("DBSIZE").query_async(conn).await.unwrap_or(0);

    // Write to a sibling `.part` and rename only on success, so an aborted
    // export never leaves a file that looks complete.
    let part_path = format!("{}.part", output_path);
    let file =
        std::fs::File::create(&part_path).map_err(|e| format!("create {}: {}", part_path, e))?;
    let header = BackupHeader {
        app: format!("TermDrop {}", env!("CARGO_PKG_VERSION")),
        created_at: now_rfc3339(),
        source: crate::uri::redact_uri(source_uri),
        redis_version: server_version.to_string(),
        dbs: vec![u8::try_from(db).unwrap_or(0)],
        pattern: pattern.map(str::to_string),
        consistency: "scan".to_string(),
    };

    let cleanup = |e: String| -> String {
        let _ = std::fs::remove_file(&part_path);
        e
    };

    let mut writer = BackupWriter::new(std::io::BufWriter::new(file), &header).map_err(cleanup)?;
    let mut cursor = "0".to_string();

    loop {
        if cancelled.load(Ordering::Relaxed) {
            progress.cancelled();
            return Err(cleanup(CANCELLED.to_string()));
        }

        let mut cmd = ::redis::cmd("SCAN");
        cmd.arg(&cursor);
        if let Some(p) = pattern {
            cmd.arg("MATCH").arg(p);
        }
        cmd.arg("COUNT").arg(BATCH);

        let (next, batch): (String, Vec<Vec<u8>>) = cmd.query_async(conn).await.map_err(|e| {
            cleanup(format!(
                "SCAN: {}",
                crate::uri::redact_uris_in_text(&e.to_string())
            ))
        })?;
        cursor = next;

        if !batch.is_empty() {
            // Size and TTL first, so an oversized key is never dumped at all:
            // DUMP of a multi-gigabyte key serialises it synchronously and
            // stalls every other client for seconds.
            let mut meta = ::redis::pipe();
            for key in &batch {
                meta.cmd("MEMORY").arg("USAGE").arg(key);
                meta.cmd("PTTL").arg(key);
            }
            let meta: Vec<::redis::Value> = meta.query_async(conn).await.map_err(|e| {
                cleanup(format!(
                    "measure keys: {}",
                    crate::uri::redact_uris_in_text(&e.to_string())
                ))
            })?;

            let mut to_dump: Vec<(&Vec<u8>, i64)> = Vec::new();
            for (i, key) in batch.iter().enumerate() {
                summary.processed += 1;
                let bytes = meta
                    .get(i * 2)
                    .and_then(value_to_i64)
                    .and_then(|v| u64::try_from(v).ok());
                let ttl = meta.get(i * 2 + 1).and_then(value_to_i64).unwrap_or(-1);

                // -2 is "already gone": expired between the scan and here.
                if ttl == -2 {
                    summary.vanished += 1;
                    continue;
                }
                if let Some(bytes) = bytes {
                    if bytes > max_key_bytes {
                        summary.skipped_big.push(SkippedKey {
                            key: key_label(key),
                            bytes,
                        });
                        continue;
                    }
                }
                to_dump.push((key, ttl));
            }

            if !to_dump.is_empty() {
                let mut dump = ::redis::pipe();
                for (key, _) in &to_dump {
                    dump.cmd("DUMP").arg(*key);
                }
                let payloads: Vec<Option<Vec<u8>>> = dump.query_async(conn).await.map_err(|e| {
                    cleanup(format!(
                        "DUMP: {}",
                        crate::uri::redact_uris_in_text(&e.to_string())
                    ))
                })?;

                for ((key, ttl), payload) in to_dump.iter().zip(payloads) {
                    match payload {
                        // A nil payload is a key that vanished mid-batch.
                        None => summary.vanished += 1,
                        Some(payload) => {
                            writer
                                .write_record(&BackupRecord {
                                    db: u8::try_from(db).unwrap_or(0),
                                    key: (*key).clone(),
                                    ttl_ms: *ttl,
                                    payload,
                                })
                                .map_err(cleanup)?;
                            summary.written += 1;
                        }
                    }
                }
            }
        }

        progress.update(
            "exporting",
            summary.processed,
            total,
            &format!("{} keys written", summary.written),
        );

        if cursor == "0" {
            break;
        }
    }

    writer.finish().map_err(cleanup)?;
    std::fs::rename(&part_path, output_path)
        .map_err(|e| cleanup(format!("finish {}: {}", output_path, e)))?;

    progress.done(summary.processed);
    summary.elapsed_ms = started.elapsed().as_millis() as u64;
    Ok(summary)
}

/// Read a backup file's header without restoring it, for the confirm dialog.
pub fn read_backup_header(path: &str) -> Result<BackupHeader, String> {
    let file = std::fs::File::open(path).map_err(|e| format!("open {}: {}", path, e))?;
    Ok(BackupReader::new(std::io::BufReader::new(file))?.header)
}

/// `RESTORE` every record of a `.tdredis` file into one database.
#[allow(clippy::too_many_arguments)]
pub async fn import_keys(
    window: &Window,
    cancelled: Arc<AtomicBool>,
    op_id: &str,
    conn: &mut MultiplexedConnection,
    db: i64,
    input_path: &str,
    replace: bool,
    flush_first: bool,
    server_version: &str,
) -> Result<RedisOpSummary, String> {
    let started = std::time::Instant::now();
    let mut progress = Progress::new(window, op_id, db);
    let mut summary = RedisOpSummary::default();

    let file =
        std::fs::File::open(input_path).map_err(|e| format!("open {}: {}", input_path, e))?;
    let mut reader = BackupReader::new(std::io::BufReader::new(file))?;

    // Check compatibility before writing a single key. RESTORE reports an RDB
    // mismatch as "DUMP payload version or checksum are wrong", which after
    // 40,000 successful keys tells the user nothing about the cause.
    check_restore_compatibility(&reader.header.redis_version, server_version)?;

    if flush_first {
        ::redis::cmd("FLUSHDB")
            .query_async::<()>(conn)
            .await
            .map_err(|e| {
                format!(
                    "FLUSHDB: {}",
                    crate::uri::redact_uris_in_text(&e.to_string())
                )
            })?;
    }

    let mut batch: Vec<BackupRecord> = Vec::with_capacity(BATCH);
    loop {
        if cancelled.load(Ordering::Relaxed) {
            progress.cancelled();
            return Err(CANCELLED.to_string());
        }

        let record = reader.next_record()?;
        let finished = record.is_none();
        if let Some(record) = record {
            batch.push(record);
        }

        if batch.len() >= BATCH || (finished && !batch.is_empty()) {
            restore_batch(conn, &batch, replace, &mut summary).await?;
            summary.processed += batch.len() as u64;
            batch.clear();
            progress.update(
                "restoring",
                summary.processed,
                0,
                &format!("{} keys restored", summary.written),
            );
        }

        if finished {
            break;
        }
    }

    progress.done(summary.processed);
    summary.elapsed_ms = started.elapsed().as_millis() as u64;
    Ok(summary)
}

/// Restore a batch, pipelined, falling back to one key at a time if anything
/// in it fails.
///
/// A pipeline gives up its per-command results on the first error, so the fast
/// path cannot report *which* key failed. Retrying only the failed batch
/// individually keeps the common case at one round trip per hundred keys while
/// still naming the key when something does go wrong.
async fn restore_batch(
    conn: &mut MultiplexedConnection,
    batch: &[BackupRecord],
    replace: bool,
    summary: &mut RedisOpSummary,
) -> Result<(), String> {
    let mut pipe = ::redis::pipe();
    for record in batch {
        let mut cmd = ::redis::cmd("RESTORE");
        cmd.arg(&record.key)
            .arg(restore_ttl(record.ttl_ms))
            .arg(&record.payload);
        if replace {
            cmd.arg("REPLACE");
        }
        pipe.add_command(cmd);
    }

    if pipe.query_async::<()>(conn).await.is_ok() {
        summary.written += batch.len() as u64;
        return Ok(());
    }

    for record in batch {
        let mut cmd = ::redis::cmd("RESTORE");
        cmd.arg(&record.key)
            .arg(restore_ttl(record.ttl_ms))
            .arg(&record.payload);
        if replace {
            cmd.arg("REPLACE");
        }
        match cmd.query_async::<()>(conn).await {
            Ok(()) => summary.written += 1,
            Err(e) => {
                let message = e.to_string();
                // Without REPLACE, a collision is the expected outcome, not a
                // failure: the user asked not to overwrite.
                if message.contains("BUSYKEY") {
                    summary.skipped_existing += 1;
                } else {
                    summary.failed.push(FailedKey {
                        key: key_label(&record.key),
                        reason: restore_reason(&message),
                    });
                }
            }
        }
    }
    Ok(())
}

/// RESTORE takes 0 for "no expiry"; the file stores -1.
fn restore_ttl(ttl_ms: i64) -> i64 {
    if ttl_ms < 0 {
        0
    } else {
        ttl_ms
    }
}

/// Turn RESTORE's terser errors into something actionable.
pub fn restore_reason(message: &str) -> String {
    if message.contains("Bad data format") {
        return "the value needs a Redis module this server does not have".to_string();
    }
    if message.contains("DUMP payload version or checksum are wrong") {
        return "this server is too old to read the value".to_string();
    }
    crate::uri::redact_uris_in_text(message)
}

/// Refuse a restore the target cannot read, before writing anything.
///
/// `DUMP` payloads are portable only forwards: a newer Redis writes an RDB
/// version an older one will not accept. Comparing major.minor is a heuristic
/// -- 7.0 and 7.2 both write RDB 11 -- so the same-major case is a *warning*
/// worded "may not", while an older major is refused outright.
pub fn check_restore_compatibility(source: &str, target: &str) -> Result<(), String> {
    let (Some((s_major, s_minor, _)), Some((t_major, t_minor, _))) =
        (parse_version(source), parse_version(target))
    else {
        // An unreadable version on either side is not grounds to refuse: the
        // server's own RESTORE remains the authority.
        return Ok(());
    };

    if (t_major, t_minor) >= (s_major, s_minor) {
        return Ok(());
    }
    if t_major < s_major {
        return Err(format!(
            "This backup was written by Redis {}, and the target runs {}, which cannot read \
             values from a newer major version. Restore into Redis {} or newer.",
            source, target, s_major
        ));
    }
    Err(format!(
        "This backup was written by Redis {} and the target runs {}. An older minor version \
         may not be able to read every value. Restore into Redis {} or newer to be sure.",
        source, target, source
    ))
}

/// An RFC 3339 timestamp, without pulling in a date library for one field.
fn now_rfc3339() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    // Days since the epoch, converted with the civil-from-days algorithm.
    let days = (secs / 86_400) as i64;
    let time = secs % 86_400;
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        y,
        m,
        d,
        time / 3600,
        (time % 3600) / 60,
        time % 60
    )
}

#[cfg(test)]
mod backup_tests {
    use super::*;

    #[test]
    fn a_cluster_is_refused_before_a_partial_backup_is_written() {
        let err = refuse_cluster("cluster").unwrap_err();
        assert!(err.contains("cluster mode"), "{}", err);
        assert!(err.contains("incomplete"), "{}", err);
        assert!(refuse_cluster("standalone").is_ok());
        assert!(refuse_cluster("sentinel").is_ok());
    }

    #[test]
    fn restoring_forwards_or_sideways_is_allowed() {
        assert!(check_restore_compatibility("7.2.4", "7.2.4").is_ok());
        assert!(check_restore_compatibility("7.0.0", "7.2.4").is_ok());
        assert!(check_restore_compatibility("6.2.7", "7.2.4").is_ok());
    }

    #[test]
    fn restoring_into_an_older_major_is_refused_with_both_versions_named() {
        let err = check_restore_compatibility("7.2.4", "6.2.7").unwrap_err();
        assert!(err.contains("7.2.4"), "{}", err);
        assert!(err.contains("6.2.7"), "{}", err);
        assert!(err.contains("cannot read"), "{}", err);
    }

    #[test]
    fn an_older_minor_is_a_warning_not_a_certainty() {
        // 7.0 and 7.2 both write RDB 11, so the major.minor comparison is a
        // heuristic and must not claim more than it knows.
        let err = check_restore_compatibility("7.2.4", "7.0.0").unwrap_err();
        assert!(err.contains("may not"), "{}", err);
    }

    #[test]
    fn an_unreadable_version_leaves_the_decision_to_the_server() {
        // Refusing on a version we cannot parse would block a restore that
        // would have worked; RESTORE itself is still the authority.
        assert!(check_restore_compatibility("unknown", "7.2.4").is_ok());
        assert!(check_restore_compatibility("7.2.4", "unknown").is_ok());
    }

    #[test]
    fn restore_errors_are_turned_into_something_actionable() {
        assert!(restore_reason("ERR Bad data format").contains("Redis module"));
        assert!(
            restore_reason("ERR DUMP payload version or checksum are wrong").contains("too old")
        );
        // Anything else passes through, redacted.
        assert!(restore_reason("ERR something else").contains("something else"));
    }

    #[test]
    fn no_expiry_is_stored_as_minus_one_and_restored_as_zero() {
        // The file uses -1 like PTTL; RESTORE spells the same thing 0.
        assert_eq!(restore_ttl(-1), 0);
        assert_eq!(restore_ttl(-2), 0);
        assert_eq!(restore_ttl(60_000), 60_000);
    }

    #[test]
    fn a_binary_key_gets_a_readable_label_rather_than_mojibake() {
        assert_eq!(key_label(b"user:1"), "user:1");
        assert_eq!(key_label(&[0xff, 0xfe]), "<binary 2 bytes>");
    }

    #[test]
    fn timestamps_are_rfc3339() {
        let now = now_rfc3339();
        assert_eq!(now.len(), 20, "{}", now);
        assert!(now.ends_with('Z'), "{}", now);
        // Sanity: this code runs well after 2020 and long before 2100.
        let year: i32 = now[..4].parse().unwrap();
        assert!((2020..2100).contains(&year), "{}", now);
    }
}

/// Tests against a real server.
///
/// `#[ignore]`d because CI has no Docker services, exactly as
/// `mongodb::tests::live` is. Run them with the compose stack up:
///
/// ```text
/// docker compose -f docker-compose.redis.yml up -d
/// scripts/seed-redis.sh
/// cargo test -- --ignored --test-threads=1
/// ```
#[cfg(test)]
mod live {
    use super::*;

    const URI: &str = "redis://:devpass@127.0.0.1:6380";

    async fn conn(db: i64) -> MultiplexedConnection {
        build_client(URI, db)
            .await
            .expect("is the compose stack up? docker compose -f docker-compose.redis.yml up -d")
    }

    #[tokio::test]
    #[ignore]
    async fn reports_the_server_and_its_databases() {
        let mut c = conn(0).await;
        let info = server_info(&mut c, "redis://***@127.0.0.1:6380".into(), false)
            .await
            .unwrap();

        assert!(info.version.starts_with('7'), "version: {}", info.version);
        assert_eq!(info.mode, "standalone");
        assert!(info.supports_scan_type);
        // The fixture seeds db0, db1 and db2.
        let indexes: Vec<i64> = info.databases.iter().map(|d| d.index).collect();
        assert!(indexes.contains(&0), "{:?}", indexes);
        assert!(indexes.contains(&1), "{:?}", indexes);
    }

    #[tokio::test]
    #[ignore]
    async fn scans_the_whole_keyspace_without_using_keys() {
        let mut c = conn(0).await;
        let mut cursor = "0".to_string();
        let mut seen = std::collections::HashSet::new();

        loop {
            let page = scan_keys(&mut c, &cursor, None, None, 200, false, true)
                .await
                .unwrap();
            for k in &page.keys {
                seen.insert(k.key_b64.clone());
            }
            cursor = page.cursor.clone();
            if page.done {
                break;
            }
        }

        let dbsize: u64 = ::redis::cmd("DBSIZE").query_async(&mut c).await.unwrap();
        assert_eq!(seen.len() as u64, dbsize, "the scan missed keys");
    }

    #[tokio::test]
    #[ignore]
    async fn a_match_pattern_narrows_the_scan_server_side() {
        let mut c = conn(0).await;
        let page = scan_keys(&mut c, "0", Some("user:*"), None, 200, false, true)
            .await
            .unwrap();
        assert!(!page.keys.is_empty());
        assert!(page.keys.iter().all(|k| k.key.text.starts_with("user:")));
    }

    #[tokio::test]
    #[ignore]
    async fn every_seeded_type_reads_back() {
        let mut c = conn(0).await;
        for (key, expected) in [
            ("user:1:name", "string"),
            ("user:1:profile", "hash"),
            ("queue:jobs", "list"),
            ("tags:set", "set"),
            ("leaderboard", "zset"),
            ("events:stream", "stream"),
        ] {
            let page = key_value(&mut c, key.as_bytes(), "0", 0, 200)
                .await
                .unwrap();
            assert_eq!(page.kind, expected, "{}", key);
            assert!(!page.entries.is_empty(), "{} came back empty", key);
        }
    }

    #[tokio::test]
    #[ignore]
    async fn a_binary_key_round_trips_through_its_base64() {
        let mut c = conn(0).await;
        let page = scan_keys(&mut c, "0", Some("bin:*"), None, 200, false, true)
            .await
            .unwrap();
        let key = page.keys.first().expect("the fixture seeds a binary key");
        assert!(key.key.binary, "the key should be reported as binary");

        // The whole point: the base64 must fetch the same key back.
        let raw = decode_key(&key.key_b64).unwrap();
        let value = key_value(&mut c, &raw, "0", 0, 200).await.unwrap();
        assert_eq!(value.kind, "string");
    }

    #[tokio::test]
    #[ignore]
    async fn a_huge_string_is_truncated_rather_than_streamed_whole() {
        let mut c = conn(0).await;
        let page = key_value(&mut c, b"big:string", "0", 0, 200).await.unwrap();
        let value = &page.entries[0].value;

        assert!(value.truncated, "a 5 MB string must come back truncated");
        assert!(value.bytes > 4 * 1024 * 1024, "real size: {}", value.bytes);
        assert!(
            value.text.len() <= MAX_VALUE_PREVIEW_BYTES,
            "preview was {} bytes",
            value.text.len()
        );
    }

    #[tokio::test]
    #[ignore]
    async fn a_huge_hash_pages_rather_than_arriving_at_once() {
        let mut c = conn(0).await;
        let page = key_value(&mut c, b"big:hash", "0", 0, 200).await.unwrap();

        assert_eq!(page.kind, "hash");
        assert!(page.total.unwrap() > 50_000, "total: {:?}", page.total);
        // The property that matters is that opening a 100k-field hash does not
        // drag 100k fields across the socket -- not that the page is exactly
        // 200. HSCAN's COUNT is a hint and overshoots by a bucket or so.
        assert!(
            page.entries.len() < 1000,
            "HSCAN returned {} fields: that is HGETALL in disguise",
            page.entries.len()
        );
        assert!(!page.done, "a 100k hash cannot finish in one page");
    }

    #[tokio::test]
    #[ignore]
    async fn the_prefix_tree_groups_the_fixture_families() {
        let mut c = conn(0).await;
        let tree = key_tree(&mut c, None, TREE_SCAN_CAP, Duration::from_secs(3))
            .await
            .unwrap();

        let names: Vec<&str> = tree.groups.iter().map(|g| g.prefix.as_str()).collect();
        for family in ["user", "session", "cache"] {
            assert!(names.contains(&family), "missing {}: {:?}", family, names);
        }
        // A key with no delimiter is its own group.
        assert!(names.contains(&"plain"), "{:?}", names);
    }

    #[tokio::test]
    #[ignore]
    async fn ttls_are_reported_with_their_sentinels() {
        let mut c = conn(0).await;
        let no_expiry = key_value(&mut c, b"user:1:name", "0", 0, 10).await.unwrap();
        assert_eq!(no_expiry.ttl_ms, -1);

        let expiring = key_value(&mut c, b"user:2:session", "0", 0, 10)
            .await
            .unwrap();
        assert!(expiring.ttl_ms > 0, "ttl: {}", expiring.ttl_ms);
    }
}

/// Backup and restore against a real server.
///
/// Separate from `live` because these write: they restore into db9, which the
/// fixture leaves empty, and flush it as they go.
#[cfg(test)]
mod live_backup {
    use super::*;

    const URI: &str = "redis://:devpass@127.0.0.1:6380";
    const SCRATCH_DB: i64 = 9;

    async fn conn(db: i64) -> MultiplexedConnection {
        build_client(URI, db)
            .await
            .expect("compose stack must be up")
    }

    fn temp_path(name: &str) -> String {
        std::env::temp_dir()
            .join(format!(
                "termdrop-test-{}-{}.tdredis",
                name,
                std::process::id()
            ))
            .to_string_lossy()
            .to_string()
    }

    /// `export_keys` needs a Window to emit progress on, which a unit test has
    /// no way to build, so these drive the pieces directly instead.
    async fn dump_all(c: &mut MultiplexedConnection, path: &str) -> u64 {
        let header = BackupHeader {
            app: "test".into(),
            created_at: now_rfc3339(),
            source: "redis://***@127.0.0.1:6380".into(),
            redis_version: "7.2.4".into(),
            dbs: vec![0],
            pattern: None,
            consistency: "scan".into(),
        };
        let file = std::fs::File::create(path).unwrap();
        let mut writer = BackupWriter::new(std::io::BufWriter::new(file), &header).unwrap();

        let mut cursor = "0".to_string();
        let mut count = 0u64;
        loop {
            let (next, batch): (String, Vec<Vec<u8>>) = ::redis::cmd("SCAN")
                .arg(&cursor)
                .arg("COUNT")
                .arg(100)
                .query_async(c)
                .await
                .unwrap();
            cursor = next;
            for key in batch {
                let ttl: i64 = ::redis::cmd("PTTL").arg(&key).query_async(c).await.unwrap();
                if ttl == -2 {
                    continue;
                }
                let payload: Option<Vec<u8>> =
                    ::redis::cmd("DUMP").arg(&key).query_async(c).await.unwrap();
                if let Some(payload) = payload {
                    writer
                        .write_record(&BackupRecord {
                            db: 0,
                            key,
                            ttl_ms: ttl,
                            payload,
                        })
                        .unwrap();
                    count += 1;
                }
            }
            if cursor == "0" {
                break;
            }
        }
        writer.finish().unwrap();
        count
    }

    async fn restore_all(
        c: &mut MultiplexedConnection,
        path: &str,
        replace: bool,
    ) -> RedisOpSummary {
        let file = std::fs::File::open(path).unwrap();
        let mut reader = BackupReader::new(std::io::BufReader::new(file)).unwrap();
        let mut summary = RedisOpSummary::default();
        let mut batch = Vec::new();
        while let Some(record) = reader.next_record().unwrap() {
            batch.push(record);
            if batch.len() >= 100 {
                restore_batch(c, &batch, replace, &mut summary)
                    .await
                    .unwrap();
                batch.clear();
            }
        }
        if !batch.is_empty() {
            restore_batch(c, &batch, replace, &mut summary)
                .await
                .unwrap();
        }
        summary
    }

    #[tokio::test]
    #[ignore]
    async fn a_database_survives_a_round_trip_through_a_backup_file() {
        let path = temp_path("roundtrip");
        let mut source = conn(0).await;
        let mut target = conn(SCRATCH_DB).await;

        ::redis::cmd("FLUSHDB")
            .query_async::<()>(&mut target)
            .await
            .unwrap();

        let dumped = dump_all(&mut source, &path).await;
        assert!(dumped > 15, "the fixture should have more keys: {}", dumped);

        let summary = restore_all(&mut target, &path, false).await;
        assert_eq!(summary.written, dumped, "not every key was restored");
        assert!(summary.failed.is_empty(), "failures: {:?}", summary.failed);

        // Spot-check across the types, not just the count.
        let name: String = ::redis::cmd("GET")
            .arg("user:1:name")
            .query_async(&mut target)
            .await
            .unwrap();
        assert_eq!(name, "alice");

        let hlen: u64 = ::redis::cmd("HLEN")
            .arg("big:hash")
            .query_async(&mut target)
            .await
            .unwrap();
        assert_eq!(hlen, 100_000, "the big hash did not survive");

        let members: u64 = ::redis::cmd("ZCARD")
            .arg("leaderboard")
            .query_async(&mut target)
            .await
            .unwrap();
        assert_eq!(members, 3);

        // A binary key must come back byte-identical, not lossily decoded.
        let mut binary_key = b"bin:".to_vec();
        binary_key.extend_from_slice(&[0xff, 0xfe]);
        let exists: bool = ::redis::cmd("EXISTS")
            .arg(&binary_key)
            .query_async(&mut target)
            .await
            .unwrap();
        assert!(exists, "the binary key did not survive the round trip");

        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    #[ignore]
    async fn ttls_survive_and_keys_without_one_stay_permanent() {
        let path = temp_path("ttl");
        let mut source = conn(0).await;
        let mut target = conn(SCRATCH_DB).await;
        ::redis::cmd("FLUSHDB")
            .query_async::<()>(&mut target)
            .await
            .unwrap();

        // Read the source TTL first: the property is that the restored key
        // carries the *same* remaining time, not some absolute window. A
        // hardcoded range only passes while the fixture is freshly seeded.
        let before: i64 = ::redis::cmd("TTL")
            .arg("user:2:session")
            .query_async(&mut source)
            .await
            .unwrap();
        assert!(
            before > 0,
            "the fixture key should still be alive: {}",
            before
        );

        dump_all(&mut source, &path).await;
        restore_all(&mut target, &path, false).await;

        let ttl: i64 = ::redis::cmd("TTL")
            .arg("user:2:session")
            .query_async(&mut target)
            .await
            .unwrap();
        assert!(
            (before - ttl).abs() <= 10,
            "ttl drifted: {} before, {} after",
            before,
            ttl
        );

        // -1 in the file must restore as "no expiry", not as "expire now".
        let permanent: i64 = ::redis::cmd("TTL")
            .arg("user:1:name")
            .query_async(&mut target)
            .await
            .unwrap();
        assert_eq!(permanent, -1, "a permanent key was given an expiry");

        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    #[ignore]
    async fn restoring_twice_without_replace_skips_rather_than_fails() {
        let path = temp_path("busykey");
        let mut source = conn(0).await;
        let mut target = conn(SCRATCH_DB).await;
        ::redis::cmd("FLUSHDB")
            .query_async::<()>(&mut target)
            .await
            .unwrap();

        let dumped = dump_all(&mut source, &path).await;
        restore_all(&mut target, &path, false).await;

        // Second pass over the same keys: BUSYKEY is the expected outcome, not
        // an error, because the user asked not to overwrite.
        let again = restore_all(&mut target, &path, false).await;
        assert_eq!(again.skipped_existing, dumped, "{:?}", again);
        assert_eq!(again.written, 0);
        assert!(again.failed.is_empty(), "failures: {:?}", again.failed);

        // With REPLACE every key is written.
        let replaced = restore_all(&mut target, &path, true).await;
        assert_eq!(replaced.written, dumped);
        assert_eq!(replaced.skipped_existing, 0);

        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    #[ignore]
    async fn a_truncated_backup_is_refused_before_anything_is_written() {
        let path = temp_path("truncated");
        let mut source = conn(0).await;
        let mut target = conn(SCRATCH_DB).await;
        ::redis::cmd("FLUSHDB")
            .query_async::<()>(&mut target)
            .await
            .unwrap();

        dump_all(&mut source, &path).await;
        let bytes = std::fs::read(&path).unwrap();
        std::fs::write(&path, &bytes[..bytes.len() - 100]).unwrap();

        let file = std::fs::File::open(&path).unwrap();
        let mut reader = BackupReader::new(std::io::BufReader::new(file)).unwrap();
        let mut err = None;
        loop {
            match reader.next_record() {
                Ok(Some(_)) => {}
                Ok(None) => break,
                Err(e) => {
                    err = Some(e);
                    break;
                }
            }
        }
        let err = err.expect("a truncated file must be rejected");
        assert!(
            err.contains("truncated") || err.contains("corrupt"),
            "{}",
            err
        );

        let _ = std::fs::remove_file(&path);
    }
}

/// The SSH tunnel path, against the compose bastion.
///
/// The riskiest part of the feature and the one with no unit test: it needs a
/// real SSH server and a Redis that genuinely cannot be reached without it.
/// `redis-private` publishes no ports for exactly that reason — if this passes
/// against a directly reachable Redis, it has proved nothing.
///
/// ```text
/// docker compose -f docker-compose.redis.yml up -d
/// cargo test redis::live_tunnel -- --ignored --test-threads=1
/// ```
#[cfg(test)]
mod live_tunnel {
    use super::*;

    const BASTION_HOST: &str = "127.0.0.1";
    const BASTION_PORT: u16 = 2222;
    const BASTION_USER: &str = "tunnel";
    const BASTION_PASSWORD: &str = "tunnelpass";
    /// Resolved on the bastion, not here: that is the whole point.
    const PRIVATE_REDIS: &str = "redis-private";

    async fn open_tunnel() -> crate::port_forward::Tunnel {
        tokio::task::spawn_blocking(|| {
            crate::port_forward::start_ephemeral_local(
                BASTION_HOST.to_string(),
                BASTION_PORT,
                BASTION_USER.to_string(),
                Some(BASTION_PASSWORD.to_string()),
                None,
                PRIVATE_REDIS.to_string(),
                6379,
            )
        })
        .await
        .unwrap()
        .expect("compose stack must be up: docker compose -f docker-compose.redis.yml up -d")
    }

    #[tokio::test]
    #[ignore]
    async fn a_private_redis_is_reachable_only_through_the_tunnel() {
        let tunnel = open_tunnel().await;
        assert!(tunnel.local_port > 0);

        let (uri, original) =
            retarget_uri("redis://:devpass@redis-private:6379/0", tunnel.local_port).unwrap();
        assert_eq!(original, PRIVATE_REDIS);

        let mut conn = build_client(&uri, 0)
            .await
            .expect("connect through the tunnel");
        let pong: String = ::redis::cmd("PING").query_async(&mut conn).await.unwrap();
        assert_eq!(pong, "PONG");

        // A real command, not just a handshake.
        ::redis::cmd("SET")
            .arg("tunnel:probe")
            .arg("reached the private server")
            .query_async::<()>(&mut conn)
            .await
            .unwrap();
        let value: String = ::redis::cmd("GET")
            .arg("tunnel:probe")
            .query_async(&mut conn)
            .await
            .unwrap();
        assert_eq!(value, "reached the private server");
    }

    #[tokio::test]
    #[ignore]
    async fn the_listener_is_bound_to_loopback_only() {
        // The local end is unauthenticated: anything that can reach it can
        // reach the Redis behind it, so it must not be reachable off-machine.
        let tunnel = open_tunnel().await;
        let from_loopback = std::net::TcpStream::connect(("127.0.0.1", tunnel.local_port));
        assert!(from_loopback.is_ok(), "loopback should connect");

        let local_ip = local_non_loopback_ip();
        if let Some(ip) = local_ip {
            let from_lan = std::net::TcpStream::connect_timeout(
                &std::net::SocketAddr::new(ip, tunnel.local_port),
                Duration::from_millis(500),
            );
            assert!(
                from_lan.is_err(),
                "the tunnel accepted a connection on {}, which is off-machine",
                ip
            );
        }
    }

    #[tokio::test]
    #[ignore]
    async fn dropping_the_handle_closes_the_listener() {
        let port = {
            let tunnel = open_tunnel().await;
            let port = tunnel.local_port;
            assert!(std::net::TcpStream::connect(("127.0.0.1", port)).is_ok());
            port
        };

        // The accept loop polls the flag every 100ms.
        tokio::time::sleep(Duration::from_millis(400)).await;
        let after = std::net::TcpStream::connect_timeout(
            &format!("127.0.0.1:{}", port).parse().unwrap(),
            Duration::from_millis(500),
        );
        assert!(after.is_err(), "the tunnel outlived its handle");
    }

    #[tokio::test]
    #[ignore]
    async fn a_wrong_password_fails_at_open_rather_than_on_the_first_command() {
        // Without the eager auth probe this would surface minutes later as a
        // connection failure against 127.0.0.1, which says nothing useful.
        let result = tokio::task::spawn_blocking(|| {
            crate::port_forward::start_ephemeral_local(
                BASTION_HOST.to_string(),
                BASTION_PORT,
                BASTION_USER.to_string(),
                Some("definitely-not-the-password".to_string()),
                None,
                PRIVATE_REDIS.to_string(),
                6379,
            )
        })
        .await
        .unwrap();

        let err = result.expect_err("a wrong password must not produce a working tunnel");
        assert!(err.contains("auth"), "unhelpful error: {}", err);
    }

    /// This machine's first non-loopback address, if it has one.
    fn local_non_loopback_ip() -> Option<std::net::IpAddr> {
        // Connecting a UDP socket assigns a local address without sending
        // anything, which is the portable way to learn the outbound IP.
        let sock = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
        sock.connect("203.0.113.1:80").ok()?;
        let addr = sock.local_addr().ok()?.ip();
        if addr.is_loopback() {
            None
        } else {
            Some(addr)
        }
    }
}
