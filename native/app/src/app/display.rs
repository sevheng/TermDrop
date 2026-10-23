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

// ---- the host row's second line (HostRow.vue) ---------------------------

/// An SSH row's subtitle: `user@host:port`. With no username the `@` is
/// left out (Vue rendered `@host:22`); the editors require one, so only an
/// imported or hand-written row can lack it.
pub fn ssh_subtitle(username: &str, host: &str, port: u16) -> String {
    if username.is_empty() {
        format!("{host}:{port}")
    } else {
        format!("{username}@{host}:{port}")
    }
}

/// A Redis or VNC row's subtitle: the address, plus ` · tunnelled` when it
/// goes through an SSH host.
pub fn with_tunnel_suffix(address: String, tunnelled: bool) -> String {
    if tunnelled {
        format!("{address} · tunnelled")
    } else {
        address
    }
}

/// Whether an SSH host reads as a Mac, for the Apple icon: the Vue test
/// `/\b(mac|darwin|osx)\b/` over the lower-cased name and address. `\b` is
/// ASCII-only there, so `mac-mini` and `my.mac` match and `imac` or
/// `macbook` do not.
pub fn looks_like_mac(name: &str, host: &str) -> bool {
    let combined = format!("{name} {host}").to_lowercase();
    let bytes = combined.as_bytes();
    let is_word = |b: u8| b.is_ascii_alphanumeric() || b == b'_';
    ["mac", "darwin", "osx"].iter().any(|word| {
        combined.match_indices(word).any(|(i, _)| {
            let end = i + word.len();
            (i == 0 || !is_word(bytes[i - 1])) && (end == bytes.len() || !is_word(bytes[end]))
        })
    })
}

/// The jump-host tooltip: `Via <bastion>`, or a warning when the bastion
/// row has since been deleted (the host is left pointing at nothing on
/// purpose, so it never quietly connects direct). `None` without a jump host.
pub fn jump_label(jump_host_id: Option<i64>, jump_name: Option<&str>) -> Option<String> {
    jump_host_id?;
    Some(match jump_name {
        Some(name) => format!("Via {name}"),
        None => "Via a jump host that no longer exists".to_string(),
    })
}

/// The subtitle's tooltip: the subtitle itself, then the jump label.
pub fn subtitle_title(subtitle: &str, jump_label: Option<&str>) -> String {
    match jump_label {
        Some(label) => format!("{subtitle} — {label}"),
        None => subtitle.to_string(),
    }
}

/// The connected rail's tooltip.
pub fn connected_rail_title(tab_count: usize) -> String {
    if tab_count > 1 {
        format!("{tab_count} open tabs")
    } else {
        "Connected".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ssh_subtitle_carries_the_port() {
        assert_eq!(ssh_subtitle("deploy", "10.0.1.4", 22), "deploy@10.0.1.4:22");
        assert_eq!(ssh_subtitle("", "10.0.1.4", 2222), "10.0.1.4:2222");
    }

    #[test]
    fn tunnelled_rows_say_so() {
        assert_eq!(
            with_tunnel_suffix("redis://h:6379".into(), true),
            "redis://h:6379 · tunnelled"
        );
        assert_eq!(with_tunnel_suffix("h:5901".into(), false), "h:5901");
    }

    #[test]
    fn macs_are_recognised_by_whole_word() {
        assert!(looks_like_mac("Mac mini", "10.0.0.2"));
        assert!(looks_like_mac("build", "mac-builder.local"));
        assert!(looks_like_mac("Darwin box", ""));
        assert!(looks_like_mac("x", "osx"));
        assert!(!looks_like_mac("MacMini", ""));
        assert!(!looks_like_mac("imac", "macbook.lan"));
        assert!(!looks_like_mac("web-01", "10.0.1.4"));
    }

    #[test]
    fn jump_labels_name_the_bastion_or_its_absence() {
        assert_eq!(jump_label(None, Some("b")), None);
        assert_eq!(
            jump_label(Some(3), Some("bastion")).as_deref(),
            Some("Via bastion")
        );
        assert_eq!(
            jump_label(Some(3), None).as_deref(),
            Some("Via a jump host that no longer exists")
        );
        assert_eq!(
            subtitle_title("u@h:22", Some("Via bastion")),
            "u@h:22 — Via bastion"
        );
        assert_eq!(subtitle_title("u@h:22", None), "u@h:22");
    }

    #[test]
    fn connected_rail_counts_tabs() {
        assert_eq!(connected_rail_title(1), "Connected");
        assert_eq!(connected_rail_title(3), "3 open tabs");
    }

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
