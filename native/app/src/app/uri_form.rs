//! Connection URIs as the editors' fields see them. Ports of
//! `src/composables/useMongoUri.js` (`parseUriToForm`, `buildMongoUri`),
//! `src/utils/redisUri.js` (`parseRedisUri`, `buildRedisUri`, `isRedisUri`)
//! and `src/utils/vncTarget.js` (`parseVncTarget`).
//!
//! Splitting a password out of a URI is not here: that is
//! `termdrop_core::uri::split_password`, the one copy both apps use.

use super::display::VNC_BASE_PORT;

pub const MONGO_DEFAULT_PORT: u16 = 27017;
pub const MONGO_DEFAULT_AUTH_SOURCE: &str = "admin";

// ---- percent-encoding ---------------------------------------------------

/// `encodeURIComponent`: everything but `A-Z a-z 0-9 - _ . ! ~ * ' ( )`.
pub fn encode_uri_component(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.!~*'()".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// `decodeURIComponent`, lenient: a malformed escape is kept as written
/// rather than failing the whole value.
pub fn decode_uri_component(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let Some(v) = s
                .get(i + 1..i + 3)
                .and_then(|h| u8::from_str_radix(h, 16).ok())
        {
            out.push(v);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8(out).unwrap_or_else(|_| s.to_string())
}

/// `URLSearchParams` parsing: `a=1&b=x+y` into ordered pairs.
fn parse_query(q: &str) -> Vec<(String, String)> {
    q.split('&')
        .filter(|p| !p.is_empty())
        .map(|p| {
            let (k, v) = p.split_once('=').unwrap_or((p, ""));
            let d = |s: &str| decode_uri_component(&s.replace('+', " "));
            (d(k), d(v))
        })
        .collect()
}

/// `URLSearchParams.toString()`: the form-urlencoded serialiser.
fn serialize_query(pairs: &[(String, String)]) -> String {
    let enc = |s: &str| {
        let mut out = String::new();
        for b in s.bytes() {
            match b {
                b' ' => out.push('+'),
                b if b.is_ascii_alphanumeric() || b"*-._".contains(&b) => out.push(b as char),
                b => out.push_str(&format!("%{b:02X}")),
            }
        }
        out
    };
    pairs
        .iter()
        .map(|(k, v)| format!("{}={}", enc(k), enc(v)))
        .collect::<Vec<_>>()
        .join("&")
}

/// `URLSearchParams.set`: replace the first pair with this key and drop the
/// rest, or append.
fn set_param(pairs: &mut Vec<(String, String)>, key: &str, value: &str) {
    if let Some(i) = pairs.iter().position(|(k, _)| k == key) {
        pairs[i].1 = value.to_string();
        let mut seen = false;
        pairs.retain(|(k, _)| {
            if k != key {
                return true;
            }
            let keep = !seen;
            seen = true;
            keep
        });
    } else {
        pairs.push((key.to_string(), value.to_string()));
    }
}

// ---- MongoDB -------------------------------------------------------------

/// The structured fields of a single-host MongoDB URI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MongoFields {
    pub host: String,
    /// As typed; the form validates it.
    pub port: String,
    pub username: String,
    pub password: String,
    pub database: String,
    pub auth_source: String,
    pub options: String,
}

impl Default for MongoFields {
    fn default() -> Self {
        Self {
            host: String::new(),
            port: MONGO_DEFAULT_PORT.to_string(),
            username: String::new(),
            password: String::new(),
            database: String::new(),
            auth_source: MONGO_DEFAULT_AUTH_SOURCE.to_string(),
            options: String::new(),
        }
    }
}

/// An SRV seed list or a multi-host URI: no single host and port to require.
pub fn is_srv_like_mongo_uri(uri: &str) -> bool {
    let t = uri.trim();
    t.starts_with("mongodb+srv://") || is_multi_host(t)
}

fn is_multi_host(uri: &str) -> bool {
    let rest = uri
        .strip_prefix("mongodb://")
        .or_else(|| uri.strip_prefix("mongodb+srv://"));
    rest.is_some_and(|r| r[..r.find('/').unwrap_or(r.len())].contains(','))
}

/// `parseUriToForm`: the fields of a URI, or `None` for one the single-host
/// form cannot represent (a seed list, a malformed authority).
pub fn parse_mongo_uri_to_form(uri: &str) -> Option<MongoFields> {
    let t = uri.trim();
    let rest = t
        .strip_prefix("mongodb://")
        .or_else(|| t.strip_prefix("mongodb+srv://"))?;
    if t.starts_with("mongodb://") && is_multi_host(t) {
        return None;
    }
    let auth_end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let authority = &rest[..auth_end];
    let tail = &rest[auth_end..];
    let (userinfo, hostport) = match authority.rfind('@') {
        Some(at) => (&authority[..at], &authority[at + 1..]),
        None => ("", authority),
    };
    let (user, pass) = userinfo.split_once(':').unwrap_or((userinfo, ""));
    let (host, port) = split_mongo_host_port(hostport)?;

    let (path, query) = {
        let no_frag = &tail[..tail.find('#').unwrap_or(tail.len())];
        match no_frag.split_once('?') {
            Some((p, q)) => (p, q),
            None => (no_frag, ""),
        }
    };
    let mut pairs = parse_query(query);
    let auth_source = pairs
        .iter()
        .find(|(k, _)| k == "authSource")
        .map(|(_, v)| v.clone())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| MONGO_DEFAULT_AUTH_SOURCE.to_string());
    pairs.retain(|(k, _)| k != "authSource");

    Some(MongoFields {
        host: host.to_string(),
        port: port.unwrap_or(MONGO_DEFAULT_PORT).to_string(),
        username: decode_uri_component(user),
        password: decode_uri_component(pass),
        database: decode_uri_component(path.strip_prefix('/').unwrap_or(path)),
        auth_source,
        options: serialize_query(&pairs),
    })
}

/// `host[:port]` as a URL parser reads it; `None` when it would throw.
fn split_mongo_host_port(s: &str) -> Option<(&str, Option<u16>)> {
    if s.starts_with('[') {
        let close = s.find(']')?;
        let host = &s[..=close];
        return match &s[close + 1..] {
            "" => Some((host, None)),
            rest => {
                let p = rest.strip_prefix(':')?;
                Some((
                    host,
                    if p.is_empty() {
                        None
                    } else {
                        Some(p.parse().ok()?)
                    },
                ))
            }
        };
    }
    match s.rsplit_once(':') {
        Some((h, "")) => Some((h, None)),
        Some((h, p)) => Some((h, Some(p.parse().ok()?))),
        None => Some((s, None)),
    }
}

/// `buildMongoUri`. The password goes in only with a username, as the Vue
/// form wrote it.
pub fn build_mongo_uri(f: &MongoFields) -> String {
    let mut uri = String::from("mongodb://");
    let user = f.username.trim();
    let pass = f.password.trim();
    if !user.is_empty() {
        uri.push_str(&encode_uri_component(user));
        if !pass.is_empty() {
            uri.push(':');
            uri.push_str(&encode_uri_component(pass));
        }
        uri.push('@');
    }
    let mut host = f.host.trim();
    if host.starts_with('[') && host.ends_with(']') {
        host = &host[1..host.len() - 1];
    }
    if host.contains(':') {
        uri.push_str(&format!("[{host}]"));
    } else {
        uri.push_str(host);
    }
    if let Ok(port) = f.port.trim().parse::<f64>()
        && port != 0.0
        && port != f64::from(MONGO_DEFAULT_PORT)
        && port.is_finite()
    {
        uri.push_str(&format!(":{port}"));
    }
    let db = f.database.trim();
    if !db.is_empty() {
        uri.push('/');
        uri.push_str(&encode_uri_component(db));
    }
    let mut params = Vec::new();
    let auth = f.auth_source.trim();
    if !auth.is_empty() && auth != MONGO_DEFAULT_AUTH_SOURCE {
        set_param(&mut params, "authSource", auth);
    }
    for (k, v) in parse_query(f.options.trim()) {
        if k != "authSource" {
            set_param(&mut params, &k, &v);
        }
    }
    let query = serialize_query(&params);
    if !query.is_empty() {
        uri.push('?');
        uri.push_str(&query);
    }
    uri
}

/// What the Mongo editor's connection string starts as for a stored URI:
/// rebuilt from its fields when the form can show it, verbatim when not.
pub fn mongo_uri_for_edit(stored: &str) -> (String, Option<MongoFields>) {
    if stored.trim().is_empty() {
        return (String::new(), Some(MongoFields::default()));
    }
    if is_srv_like_mongo_uri(stored) {
        return (stored.to_string(), None);
    }
    match parse_mongo_uri_to_form(stored) {
        Some(f) => (build_mongo_uri(&f), Some(f)),
        None => (stored.to_string(), None),
    }
}

pub fn is_mongo_uri(uri: &str) -> bool {
    let t = uri.trim();
    t.starts_with("mongodb://") || t.starts_with("mongodb+srv://")
}

// ---- Redis ---------------------------------------------------------------

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RedisFields {
    pub tls: bool,
    pub host: String,
    pub port: String,
    pub username: String,
    pub password: String,
    pub database: String,
}

fn redis_scheme_len(uri: &str) -> Option<usize> {
    let lower = uri.get(..9).unwrap_or(uri).to_ascii_lowercase();
    if lower.starts_with("rediss://") {
        Some(9)
    } else if lower.starts_with("redis://") {
        Some(8)
    } else {
        None
    }
}

pub fn is_redis_uri(uri: &str) -> bool {
    redis_scheme_len(uri.trim()).is_some()
}

pub fn is_tls_redis_uri(uri: &str) -> bool {
    redis_scheme_len(uri.trim()) == Some(9)
}

/// `parseRedisUri`. Nothing is decoded: the password stays exactly as it
/// appears, so splitting it back out is a splice.
pub fn parse_redis_uri(uri: &str) -> RedisFields {
    let value = uri.trim();
    let Some(start) = redis_scheme_len(value) else {
        return RedisFields::default();
    };
    let end = value[start..]
        .find(['/', '?', '#'])
        .map_or(value.len(), |i| start + i);
    let authority = &value[start..end];
    let (userinfo, hostport) = match authority.rfind('@') {
        Some(at) => (&authority[..at], &authority[at + 1..]),
        None => ("", authority),
    };
    let (username, password) = userinfo.split_once(':').unwrap_or((userinfo, ""));
    let (host, port) = if hostport.starts_with('[') {
        match hostport.find(']') {
            Some(close) => {
                let tail = &hostport[close + 1..];
                (&hostport[..=close], tail.strip_prefix(':').unwrap_or(""))
            }
            None => (hostport, ""),
        }
    } else {
        hostport.rsplit_once(':').unwrap_or((hostport, ""))
    };
    let path = value[end..].trim_start_matches('/');
    let database = &path[..path.find(['?', '#']).unwrap_or(path.len())];
    RedisFields {
        tls: start == 9,
        host: host.to_string(),
        port: port.to_string(),
        username: username.to_string(),
        password: password.to_string(),
        database: database.to_string(),
    }
}

/// `buildRedisUri`. Defaults are left out rather than spelled, and an empty
/// host builds nothing.
pub fn build_redis_uri(f: &RedisFields) -> String {
    let host = f.host.trim();
    if host.is_empty() {
        return String::new();
    }
    let scheme = if f.tls { "rediss" } else { "redis" };
    let userinfo = if !f.username.is_empty() || !f.password.is_empty() {
        if f.password.is_empty() {
            format!("{}@", f.username)
        } else {
            format!("{}:{}@", f.username, f.password)
        }
    } else {
        String::new()
    };
    let port = f.port.trim();
    let hostport = if !port.is_empty() && port != "6379" {
        format!("{host}:{port}")
    } else {
        host.to_string()
    };
    let db = f.database.trim();
    let path = if !db.is_empty() && db != "0" {
        format!("/{db}")
    } else {
        String::new()
    };
    format!("{scheme}://{userinfo}{hostport}{path}")
}

// ---- VNC -----------------------------------------------------------------

/// The largest value still read as a display number rather than a port.
const MAX_DISPLAY: u32 = 99;

/// `parseVncTarget`: `host`, `host:1` (display 1, port 5901), `host:5901`,
/// `host::5901` (a literal port) and bracketed IPv6.
pub fn parse_vnc_target(text: &str) -> Result<(String, u16), String> {
    let raw = text.trim();
    if raw.is_empty() {
        return Err("An address is required".into());
    }
    if let Some(inner) = raw.strip_prefix('[') {
        let Some(close) = inner.find(']') else {
            return Err("Unclosed [ in the IPv6 address".into());
        };
        let host = &inner[..close];
        let rest = &inner[close + 1..];
        if host.is_empty() {
            return Err("An address is required".into());
        }
        if rest.is_empty() {
            return Ok((host.to_string(), VNC_BASE_PORT));
        }
        let Some(suffix) = rest.strip_prefix(':') else {
            return Err(format!("Unexpected {} after the address", js_quote(rest)));
        };
        return with_suffix(host, suffix);
    }
    let Some(colon) = raw.find(':') else {
        return Ok((raw.to_string(), VNC_BASE_PORT));
    };
    let host = &raw[..colon];
    if host.is_empty() {
        return Err("An address is required".into());
    }
    let rest = &raw[colon + 1..];
    if !rest.starts_with(':') && rest.contains(':') {
        return Err("Put an IPv6 address in square brackets, like [::1]:1".into());
    }
    with_suffix(host, rest)
}

fn with_suffix(host: &str, suffix: &str) -> Result<(String, u16), String> {
    let explicit_port = suffix.starts_with(':');
    let digits = if explicit_port { &suffix[1..] } else { suffix };
    if digits.is_empty() {
        return Err("Add a display number or port after the colon".into());
    }
    if digits.contains(':') {
        return Err("Put an IPv6 address in square brackets, like [::1]:1".into());
    }
    if !digits.bytes().all(|b| b.is_ascii_digit()) {
        return Err(format!(
            "{} is not a display number or port",
            js_quote(digits)
        ));
    }
    // A run of digits too long for u32 is far out of range either way.
    let value: u64 = digits.parse().unwrap_or(u64::MAX);
    let port = if explicit_port || value > u64::from(MAX_DISPLAY) {
        value
    } else {
        u64::from(VNC_BASE_PORT) + value
    };
    if !(1..=65535).contains(&port) {
        return Err(format!("Port {port} is outside 1-65535"));
    }
    Ok((host.to_string(), port as u16))
}

/// `JSON.stringify` of a plain string, for the error messages that quote one.
fn js_quote(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_else(|_| format!("\"{s}\""))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uri_components_encode_as_javascript_does() {
        assert_eq!(encode_uri_component("p@ss:w/rd"), "p%40ss%3Aw%2Frd");
        assert_eq!(encode_uri_component("a-b_c.d!~*'()"), "a-b_c.d!~*'()");
        assert_eq!(decode_uri_component("p%40ss%3A"), "p@ss:");
        assert_eq!(decode_uri_component("100%"), "100%", "a stray % is kept");
    }

    #[test]
    fn a_mongo_uri_parses_into_fields_and_back() {
        let f = parse_mongo_uri_to_form(
            "mongodb://admin:p%40ss@db.internal:27018/orders?authSource=users&retryWrites=true",
        )
        .unwrap();
        assert_eq!(f.host, "db.internal");
        assert_eq!(f.port, "27018");
        assert_eq!(
            (f.username.as_str(), f.password.as_str()),
            ("admin", "p@ss")
        );
        assert_eq!(f.database, "orders");
        assert_eq!(f.auth_source, "users");
        assert_eq!(f.options, "retryWrites=true");
        assert_eq!(
            build_mongo_uri(&f),
            "mongodb://admin:p%40ss@db.internal:27018/orders?authSource=users&retryWrites=true"
        );
    }

    #[test]
    fn defaults_are_left_out_of_a_built_mongo_uri() {
        let f = MongoFields {
            host: "localhost".into(),
            ..Default::default()
        };
        assert_eq!(build_mongo_uri(&f), "mongodb://localhost");
        // A password without a user has nowhere to go.
        let f = MongoFields {
            host: "h".into(),
            password: "pw".into(),
            ..Default::default()
        };
        assert_eq!(build_mongo_uri(&f), "mongodb://h");
        // An IPv6 host is bracketed once.
        let f = MongoFields {
            host: "[::1]".into(),
            ..Default::default()
        };
        assert_eq!(build_mongo_uri(&f), "mongodb://[::1]");
    }

    #[test]
    fn seed_lists_and_srv_stay_verbatim() {
        assert!(is_srv_like_mongo_uri("mongodb+srv://cluster0.example.net"));
        assert!(is_srv_like_mongo_uri("mongodb://a:1,b:2/db"));
        assert!(!is_srv_like_mongo_uri("mongodb://a:1/db"));
        assert_eq!(parse_mongo_uri_to_form("mongodb://a:1,b:2/db"), None);
        assert_eq!(parse_mongo_uri_to_form("postgres://x"), None);
        assert_eq!(parse_mongo_uri_to_form("mongodb://h:notaport"), None);
        let (uri, fields) = mongo_uri_for_edit("mongodb+srv://u@cluster0.example.net/db");
        assert_eq!(uri, "mongodb+srv://u@cluster0.example.net/db");
        assert!(fields.is_none());
    }

    #[test]
    fn a_redis_uri_parses_without_decoding() {
        let f = parse_redis_uri("rediss://user:p%40ss@cache.internal:6380/2");
        assert!(f.tls);
        assert_eq!(
            (f.host.as_str(), f.port.as_str(), f.database.as_str()),
            ("cache.internal", "6380", "2")
        );
        assert_eq!(
            (f.username.as_str(), f.password.as_str()),
            ("user", "p%40ss")
        );
        let f = parse_redis_uri("redis://:pw@[::1]:6379");
        assert_eq!((f.host.as_str(), f.port.as_str()), ("[::1]", "6379"));
        assert_eq!(parse_redis_uri("http://x"), RedisFields::default());
    }

    #[test]
    fn a_built_redis_uri_leaves_defaults_out() {
        let f = RedisFields {
            host: "localhost".into(),
            port: "6379".into(),
            database: "0".into(),
            ..Default::default()
        };
        assert_eq!(build_redis_uri(&f), "redis://localhost");
        let f = RedisFields {
            tls: true,
            host: "h".into(),
            port: "6380".into(),
            password: "pw".into(),
            database: "3".into(),
            ..Default::default()
        };
        assert_eq!(build_redis_uri(&f), "rediss://:pw@h:6380/3");
        assert_eq!(build_redis_uri(&RedisFields::default()), "");
        assert!(is_redis_uri(" REDIS://h"));
        assert!(is_tls_redis_uri("rediss://h"));
        assert!(!is_redis_uri("mongodb://h"));
    }

    #[test]
    fn vnc_targets_read_displays_and_ports() {
        assert_eq!(parse_vnc_target("10.0.0.5"), Ok(("10.0.0.5".into(), 5900)));
        assert_eq!(
            parse_vnc_target("10.0.0.5:1"),
            Ok(("10.0.0.5".into(), 5901))
        );
        assert_eq!(parse_vnc_target("h:5901"), Ok(("h".into(), 5901)));
        assert_eq!(parse_vnc_target("h::5"), Ok(("h".into(), 5)));
        assert_eq!(parse_vnc_target("[::1]:1"), Ok(("::1".into(), 5901)));
        assert_eq!(parse_vnc_target("[::1]"), Ok(("::1".into(), 5900)));
        assert_eq!(parse_vnc_target(" ").unwrap_err(), "An address is required");
        assert_eq!(
            parse_vnc_target("fe80::1:5901").unwrap_err(),
            "Put an IPv6 address in square brackets, like [::1]:1"
        );
        assert_eq!(
            parse_vnc_target("h:x").unwrap_err(),
            "\"x\" is not a display number or port"
        );
        assert_eq!(
            parse_vnc_target("h:").unwrap_err(),
            "Add a display number or port after the colon"
        );
        assert_eq!(
            parse_vnc_target("h:70000").unwrap_err(),
            "Port 70000 is outside 1-65535"
        );
        assert_eq!(
            parse_vnc_target("[::1").unwrap_err(),
            "Unclosed [ in the IPv6 address"
        );
    }
}
