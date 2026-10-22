//! Finding URLs in a row of terminal text, for Ctrl/Cmd+click.

use std::ops::Range;
use std::sync::OnceLock;

use regex::Regex;

fn url_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        // http(s) only, as in xterm's web-links addon and the Tauri app's
        // opener scope. The text comes from a remote host, and a click hands
        // it to the OS: `file://host/share/x.exe` is a UNC path that Windows
        // fetches (leaking NTLM credentials) and runs, and xdg-open launches
        // a local `.desktop` file. Only a browser launch is safe to offer.
        Regex::new(r#"https?://[^\s<>"'`{}|\\^\x00-\x1f]+"#).expect("valid URL regex")
    })
}

/// Trims what is almost always sentence punctuation rather than part of the
/// URL: a trailing `.`/`,`/`:` and so on, and closing brackets that have no
/// opening partner inside the URL (so Wikipedia's `Foo_(bar)` survives but
/// `(see https://x.y)` loses its `)`).
fn trim_url(s: &str) -> &str {
    let mut end = s.len();
    loop {
        let t = &s[..end];
        let Some(last) = t.chars().last() else { break };
        let trim = match last {
            '.' | ',' | ':' | ';' | '!' | '?' => true,
            ')' => t.matches('(').count() < t.matches(')').count(),
            ']' => t.matches('[').count() < t.matches(']').count(),
            _ => false,
        };
        if !trim {
            break;
        }
        end -= last.len_utf8();
    }
    &s[..end]
}

/// Whether `url` may be handed to the OS opener. Checked again at the click,
/// so a later change to the regex cannot widen what remote text can launch.
pub fn is_openable(url: &str) -> bool {
    let lower = url.get(..8).unwrap_or(url).to_ascii_lowercase();
    lower.starts_with("https://") || lower.starts_with("http://")
}

/// Every URL in `text`, as byte ranges.
pub fn find_urls(text: &str) -> Vec<Range<usize>> {
    url_regex()
        .find_iter(text)
        .filter_map(|m| {
            let trimmed = trim_url(m.as_str());
            // A bare scheme ("https://") is not a link.
            let has_host = trimmed
                .split_once("://")
                .is_some_and(|(_, rest)| !rest.is_empty());
            has_host.then(|| m.start()..m.start() + trimmed.len())
        })
        .collect()
}

/// The URL covering the character at `char_index`, with its character range.
pub fn url_at(text: &str, char_index: usize) -> Option<(String, Range<usize>)> {
    let byte = text.char_indices().nth(char_index).map(|(i, _)| i)?;
    find_urls(text)
        .into_iter()
        .find(|r| r.contains(&byte))
        .map(|r| {
            let start = text[..r.start].chars().count();
            let len = text[r.clone()].chars().count();
            (text[r].to_string(), start..start + len)
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn urls(text: &str) -> Vec<&str> {
        find_urls(text).into_iter().map(|r| &text[r]).collect()
    }

    #[test]
    fn finds_plain_urls() {
        assert_eq!(
            urls("see https://example.com/a?b=c#d and http://x.io"),
            ["https://example.com/a?b=c#d", "http://x.io"]
        );
    }

    #[test]
    fn only_web_urls_are_links() {
        // Remote output must never produce anything but a browser launch.
        assert!(urls("file:///tmp/x.log").is_empty());
        assert!(urls("file://attacker.example/share/report.exe").is_empty());
        assert!(urls("ftp://host/x").is_empty());
        assert!(is_openable("https://a.b"));
        assert!(is_openable("HTTP://a.b"));
        assert!(!is_openable("file:///etc/passwd"));
        assert!(!is_openable("ftp://a.b"));
        assert!(!is_openable("javascript:alert(1)"));
    }

    #[test]
    fn trims_sentence_punctuation() {
        assert_eq!(urls("Go to https://example.com."), ["https://example.com"]);
        assert_eq!(
            urls("(see https://example.com/x)"),
            ["https://example.com/x"]
        );
        assert_eq!(
            urls("https://en.wikipedia.org/wiki/Rust_(language),"),
            ["https://en.wikipedia.org/wiki/Rust_(language)"]
        );
    }

    #[test]
    fn stops_at_quotes_and_brackets() {
        assert_eq!(urls(r#"href="https://a.b/c">"#), ["https://a.b/c"]);
        assert_eq!(urls("<https://a.b/c>"), ["https://a.b/c"]);
    }

    #[test]
    fn ignores_non_urls() {
        assert!(urls("no links here, just text: and https://").is_empty());
        assert!(urls("mailto:someone").is_empty());
    }

    #[test]
    fn url_at_maps_characters() {
        let line = "→ open https://a.b/c now";
        let (url, range) = url_at(line, 9).unwrap();
        assert_eq!(url, "https://a.b/c");
        assert_eq!(range, 7..20);
        assert!(url_at(line, 2).is_none());
        assert!(url_at(line, 21).is_none());
    }
}
