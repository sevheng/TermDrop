//! Rendering connection addresses for display, never with credentials.
//!
//! Ports of `mongoDisplayUri` (`src/utils/mongoDisplay.js`),
//! `redisDisplayUri` (`src/utils/redisUri.js`) and `vncDisplayTarget`
//! (`src/utils/vncTarget.js`). The host list shows and searches these forms,
//! so a stored password can never be matched against, nor confirmed by a
//! match.

/// The first VNC display's port. Display N listens on `BASE_PORT + N`.
pub const VNC_BASE_PORT: u16 = 5900;

/// `mongodb://host:port` (or `mongodb+srv://host`) for a connection string,
/// or a truncated credential-free form when it cannot be parsed.
pub fn mongo_display_uri(uri: &str) -> String {
    let value = uri.trim();
    if value.is_empty() {
        return String::new();
    }
    let (is_srv, rest) = if let Some(r) = value.strip_prefix("mongodb+srv://") {
        (true, r)
    } else if let Some(r) = value.strip_prefix("mongodb://") {
        (false, r)
    } else {
        return strip_credentials(value);
    };

    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let authority = &rest[..end];
    // Any userinfo is dropped rather than rendered.
    let hostport = match authority.rfind('@') {
        Some(at) => &authority[at + 1..],
        None => authority,
    };
    match split_host_port(hostport) {
        Some((host, port)) if !host.is_empty() => {
            let host = host.to_ascii_lowercase();
            if is_srv {
                format!("mongodb+srv://{host}")
            } else {
                match port {
                    Some(p) => format!("mongodb://{host}:{p}"),
                    None => format!("mongodb://{host}"),
                }
            }
        }
        // A seed list (`a:1,b:2`) or anything else a URL parser rejects.
        _ => strip_credentials(value),
    }
}

/// `host[:port]`, or `None` when it is not a single well-formed host.
fn split_host_port(s: &str) -> Option<(&str, Option<u16>)> {
    if s.contains(',') {
        return None;
    }
    if let Some(inner) = s.strip_prefix('[') {
        let close = inner.find(']')?;
        let host = &s[..close + 2];
        let rest = &inner[close + 1..];
        return match rest.strip_prefix(':') {
            Some(p) if !p.is_empty() => Some((host, Some(p.parse().ok()?))),
            Some(_) => Some((host, None)),
            None if rest.is_empty() => Some((host, None)),
            None => None,
        };
    }
    match s.split_once(':') {
        Some((h, "")) => Some((h, None)),
        Some((h, p)) => Some((h, Some(p.parse().ok()?))),
        None => Some((s, None)),
    }
}

/// Remove any `user:password@` from a URI that could not be parsed, so the
/// fallback path cannot render a credential either.
pub fn strip_credentials(uri: &str) -> String {
    let without = ["mongodb+srv://", "mongodb://"]
        .iter()
        .find_map(|scheme| {
            let rest = uri.strip_prefix(scheme)?;
            let userinfo_end = rest.find(['/', '@'])?;
            (rest.as_bytes()[userinfo_end] == b'@')
                .then(|| format!("{scheme}{}", &rest[userinfo_end + 1..]))
        })
        .unwrap_or_else(|| uri.to_string());
    if without.chars().count() > 35 {
        let mut s: String = without.chars().take(35).collect();
        s.push('…');
        s
    } else {
        without
    }
}

/// The byte range of a `redis://` / `rediss://` URI's authority.
fn redis_authority(uri: &str) -> Option<(usize, usize)> {
    let lower = uri.get(..9).unwrap_or(uri).to_ascii_lowercase();
    let start = if lower.starts_with("rediss://") {
        9
    } else if lower.starts_with("redis://") {
        8
    } else {
        return None;
    };
    let end = uri[start..]
        .find(['/', '?', '#'])
        .map_or(uri.len(), |i| start + i);
    Some((start, end))
}

/// A Redis URI with its userinfo masked. `redis://:pw@h` is Redis's usual
/// form, with no username, and it still masks: `redis://***@h`.
pub fn redis_display_uri(uri: &str) -> String {
    let Some((start, end)) = redis_authority(uri) else {
        return uri.to_string();
    };
    let authority = &uri[start..end];
    let Some(at) = authority.rfind('@') else {
        return uri.to_string();
    };
    let masked = if authority[..at].contains(':') {
        "***:***"
    } else {
        "***"
    };
    format!("{}{}{}", &uri[..start], masked, &uri[start + at..])
}

/// A VNC address as `host:port`, never as a display number: the stored value
/// is a port, and showing `:1` for 5901 but `:5999` for 5999 would be a
/// needless puzzle. IPv6 literals are bracketed.
pub fn vnc_display_target(vnc_host: Option<&str>, vnc_port: Option<u16>) -> String {
    let Some(host) = vnc_host.filter(|h| !h.is_empty()) else {
        return String::new();
    };
    let port = vnc_port.filter(|p| *p != 0).unwrap_or(VNC_BASE_PORT);
    if host.contains(':') {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mongo_shows_host_and_port_without_credentials() {
        assert_eq!(
            mongo_display_uri("mongodb://admin:secret@db.internal:27017/app?authSource=admin"),
            "mongodb://db.internal:27017"
        );
        assert_eq!(
            mongo_display_uri("mongodb+srv://u:p@cluster0.example.net/db"),
            "mongodb+srv://cluster0.example.net"
        );
        assert_eq!(mongo_display_uri(""), "");
    }

    #[test]
    fn mongo_seed_lists_fall_back_without_credentials() {
        let shown = mongo_display_uri("mongodb://u:hunter2@a:1,b:2/db");
        assert!(!shown.contains("hunter2"));
        assert!(shown.starts_with("mongodb://a:1,b:2"));
    }

    #[test]
    fn redis_masks_userinfo() {
        assert_eq!(
            redis_display_uri("redis://:hunter2@h:6379"),
            "redis://***:***@h:6379"
        );
        assert_eq!(
            redis_display_uri("redis://@10.0.9.9:6380/0"),
            "redis://***@10.0.9.9:6380/0"
        );
        assert_eq!(redis_display_uri("redis://h:6379"), "redis://h:6379");
    }

    #[test]
    fn vnc_targets_are_host_and_port() {
        assert_eq!(
            vnc_display_target(Some("10.0.9.9"), Some(5901)),
            "10.0.9.9:5901"
        );
        assert_eq!(vnc_display_target(Some("::1"), None), "[::1]:5900");
        assert_eq!(vnc_display_target(None, Some(5901)), "");
    }
}
