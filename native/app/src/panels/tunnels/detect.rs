//! "Forward <host>:<port>" from a terminal selection: a port of
//! `detectForwardInfo` in `TerminalTab.vue`, which the terminal's context
//! menu calls to decide whether to offer the entry and what to prefill.
//!
//! The Vue function tried `new URL(text)` first, then `host:port`, then a
//! bare port. The URL step is reproduced for the shapes a terminal prints
//! (`scheme://host[:port][/...]`), with the WHATWG rules that decide its
//! outcome: a special scheme's default port reads as no port (so
//! `https://x:443` is "Forward x:443" through the https fallback, and
//! `ftp://x:21` matches nothing), the host of a special scheme is
//! lowercased, and a port over 65535 is not a URL at all.

use super::logic::Prefill;

/// What the context menu offers for a selection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForwardHint {
    /// The port to forward on both ends. From `host:port` it is not
    /// range-checked (Vue did not either); the dialog's validation catches
    /// it.
    pub port: u32,
    /// The menu entry: "Forward localhost:3000" or "Forward port 3000".
    pub label: String,
}

impl ForwardHint {
    /// What `createForwardFromSelection` prefilled the dialog with.
    pub fn prefill(&self) -> Prefill {
        Prefill::forward_port(self.port)
    }
}

/// `detectForwardInfo(selection)`.
pub fn detect_forward(selection: &str) -> Option<ForwardHint> {
    let trimmed = selection.trim();
    if trimmed.is_empty() {
        return None;
    }

    if let Some(url) = parse_url(trimmed) {
        if let Some(port) = url.port {
            return Some(ForwardHint {
                port,
                label: format!("Forward {}:{}", url.host, port),
            });
        }
        match url.scheme.as_str() {
            "http" => {
                return Some(ForwardHint {
                    port: 80,
                    label: format!("Forward {}:80", url.host),
                });
            }
            "https" => {
                return Some(ForwardHint {
                    port: 443,
                    label: format!("Forward {}:443", url.host),
                });
            }
            _ => {}
        }
    }

    // host:port, /^([a-zA-Z0-9._-]+):(\d{2,5})$/
    if let Some((host, port)) = trimmed.rsplit_once(':')
        && !host.is_empty()
        && host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
        && is_digits(port, 2, 5)
    {
        let port: u32 = port.parse().ok()?;
        return Some(ForwardHint {
            port,
            label: format!("Forward {host}:{port}"),
        });
    }

    // :3000 or 3000, /^:?(\d{2,5})$/
    let bare = trimmed.strip_prefix(':').unwrap_or(trimmed);
    if is_digits(bare, 2, 5) {
        let port: u32 = bare.parse().ok()?;
        if (1..=65535).contains(&port) {
            return Some(ForwardHint {
                port,
                label: format!("Forward port {port}"),
            });
        }
    }
    None
}

fn is_digits(s: &str, min: usize, max: usize) -> bool {
    (min..=max).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_digit())
}

struct Url {
    scheme: String,
    host: String,
    /// `url.port`: `None` when absent or the scheme's default.
    port: Option<u32>,
}

fn default_port(scheme: &str) -> Option<u32> {
    match scheme {
        "http" | "ws" => Some(80),
        "https" | "wss" => Some(443),
        "ftp" => Some(21),
        _ => None,
    }
}

fn is_special(scheme: &str) -> bool {
    default_port(scheme).is_some() || scheme == "file"
}

/// Just enough of the WHATWG URL parser for `detectForwardInfo`: `None`
/// where `new URL` would throw, or where the result has neither a port nor
/// an http(s) scheme (both lead to the same next step in the Vue code).
fn parse_url(text: &str) -> Option<Url> {
    let (scheme, rest) = text.split_once(':')?;
    let mut chars = scheme.chars();
    if !chars.next()?.is_ascii_alphabetic()
        || !chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
    {
        return None;
    }
    let scheme = scheme.to_ascii_lowercase();
    if text.chars().any(char::is_whitespace) {
        return None;
    }
    let special = is_special(&scheme);
    let after = if special {
        // A special scheme takes any run of slashes (or backslashes) as the
        // start of the authority: `http:host:1` is `http://host:1`.
        rest.trim_start_matches(['/', '\\'])
    } else {
        // Otherwise only `//` starts an authority; `localhost:3000` is the
        // scheme "localhost" with an opaque path and no port.
        rest.strip_prefix("//")?
    };
    let end = after
        .find(|c: char| matches!(c, '/' | '?' | '#') || (special && c == '\\'))
        .unwrap_or(after.len());
    let authority = &after[..end];
    let host_port = authority.rsplit_once('@').map_or(authority, |(_, h)| h);

    let (host, port) = if let Some(v6) = host_port.strip_prefix('[') {
        let close = v6.find(']')?;
        let host = &host_port[..close + 2];
        match &v6[close + 1..] {
            "" => (host, None),
            p => (host, Some(p.strip_prefix(':')?)),
        }
    } else {
        match host_port.rsplit_once(':') {
            Some((h, p)) => (h, Some(p)),
            None => (host_port, None),
        }
    };
    if host.is_empty() && special && scheme != "file" {
        return None;
    }
    if host
        .chars()
        .any(|c| matches!(c, '<' | '>' | '^' | '|' | '%' | '"'))
    {
        return None;
    }

    let port = match port {
        None | Some("") => None,
        Some(p) => {
            if !p.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            let digits = p.trim_start_matches('0');
            if digits.len() > 5 {
                return None;
            }
            let n: u32 = digits.parse().unwrap_or(0);
            if n > 65535 {
                return None;
            }
            (default_port(&scheme) != Some(n)).then_some(n)
        }
    };
    let host = if special {
        host.to_ascii_lowercase()
    } else {
        host.to_string()
    };
    Some(Url { scheme, host, port })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn label(s: &str) -> Option<String> {
        detect_forward(s).map(|h| h.label)
    }

    fn port(s: &str) -> Option<u32> {
        detect_forward(s).map(|h| h.port)
    }

    #[test]
    fn urls_with_a_port() {
        assert_eq!(
            label("http://localhost:3000"),
            Some("Forward localhost:3000".into())
        );
        assert_eq!(port("https://127.0.0.1:8080/path?q=1"), Some(8080));
        assert_eq!(
            label("  https://127.0.0.1:8080/path \n"),
            Some("Forward 127.0.0.1:8080".into())
        );
        assert_eq!(
            label("HTTP://Example.COM:5000"),
            Some("Forward example.com:5000".into())
        );
        assert_eq!(
            label("http://user:pw@host:9000/"),
            Some("Forward host:9000".into())
        );
        assert_eq!(
            label("http://[::1]:8000"),
            Some("Forward [::1]:8000".into())
        );
        assert_eq!(
            label("postgres://db:5432/app"),
            Some("Forward db:5432".into())
        );
    }

    #[test]
    fn http_and_https_default_ports() {
        assert_eq!(
            label("http://example.com"),
            Some("Forward example.com:80".into())
        );
        assert_eq!(
            label("https://example.com/x"),
            Some("Forward example.com:443".into())
        );
        // The default port reads as none, so it comes from the fallback.
        assert_eq!(
            label("https://example.com:443"),
            Some("Forward example.com:443".into())
        );
        assert_eq!(
            label("http://example.com:80"),
            Some("Forward example.com:80".into())
        );
    }

    #[test]
    fn other_schemes_without_a_port_offer_nothing() {
        assert_eq!(detect_forward("ftp://files.example.com"), None);
        assert_eq!(detect_forward("ftp://files.example.com:21"), None);
        assert_eq!(detect_forward("mailto:root@example.com"), None);
    }

    #[test]
    fn a_url_port_out_of_range_is_not_a_url() {
        assert_eq!(detect_forward("http://example.com:70000"), None);
    }

    #[test]
    fn host_and_port() {
        assert_eq!(
            label("localhost:3000"),
            Some("Forward localhost:3000".into())
        );
        assert_eq!(
            label("127.0.0.1:8080"),
            Some("Forward 127.0.0.1:8080".into())
        );
        assert_eq!(label("db_1:5432"), Some("Forward db_1:5432".into()));
        // Two to five digits, unchecked against the range, as in Vue.
        assert_eq!(port("localhost:99999"), Some(99999));
        assert_eq!(detect_forward("localhost:1"), None);
        assert_eq!(detect_forward("localhost:123456"), None);
        assert_eq!(detect_forward("my host:3000"), None);
    }

    #[test]
    fn bare_ports() {
        assert_eq!(label("3000"), Some("Forward port 3000".into()));
        assert_eq!(label(":8080"), Some("Forward port 8080".into()));
        assert_eq!(label("65535"), Some("Forward port 65535".into()));
        assert_eq!(detect_forward("70000"), None);
        assert_eq!(detect_forward("7"), None);
        assert_eq!(detect_forward("00"), None);
        assert_eq!(label("080"), Some("Forward port 80".into()));
    }

    #[test]
    fn nothing_useful() {
        assert_eq!(detect_forward(""), None);
        assert_eq!(detect_forward("   "), None);
        assert_eq!(detect_forward("hello world"), None);
        assert_eq!(detect_forward("ls -la"), None);
    }

    #[test]
    fn prefill_is_what_create_forward_from_selection_sent() {
        let p = detect_forward("http://localhost:3000").unwrap().prefill();
        assert_eq!(p.name, "Forward 3000");
        assert_eq!(p.kind, super::super::logic::ForwardKind::Local);
        assert_eq!(p.local_host, "127.0.0.1");
        assert_eq!(p.local_port, Some(3000));
        assert_eq!(p.remote_host, "localhost");
        assert_eq!(p.remote_port, Some(3000));
    }
}
