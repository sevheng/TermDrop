//! Credential surgery on connection URIs.
//!
//! MongoDB and Redis both store a password-free URI in the database and splice
//! the secret back in from the keyring at connect time, and both have to keep
//! that secret out of logs and toasts. The rules are the same for either
//! scheme — everything here works on the generic
//! `scheme://userinfo@authority/path?query` shape and never looks at the
//! scheme itself, except in [`redact_uris_in_text`], which has to know what to
//! search for.
//!
//! Splitting a password is security-sensitive in both directions: a bug that
//! fails to strip leaks a credential into SQLite or an export file, and a bug
//! that fails to rejoin locks the user out. That is why there is one copy.

/// The byte range of a URI's authority (everything between `://` and the
/// first `/`, `?` or `#`). Shared by the userinfo helpers below.
fn authority_range(uri: &str) -> Option<(usize, usize)> {
    let scheme_end = uri.find("://")?;
    let start = scheme_end + 3;
    let end = uri[start..]
        .find(&['/', '?', '#'][..])
        .map(|idx| start + idx)
        .unwrap_or(uri.len());
    Some((start, end))
}

/// The byte range of the userinfo inside the authority, if the URI has one.
///
/// Takes the *last* `@` in the authority: a percent-encoded password cannot
/// contain a literal `@`, but a host list can't either, so the last one is the
/// separator in every valid form.
fn userinfo_range(uri: &str) -> Option<(usize, usize)> {
    let (start, end) = authority_range(uri)?;
    let at = uri[start..end].rfind('@')? + start;
    Some((start, at))
}

/// Split a URI into a password-free URI and its password.
///
/// The password is returned **still percent-encoded**, exactly as it appeared,
/// so putting it back is a pure splice with no encoding decisions to get wrong
/// and a password containing a literal `%40` round-trips unchanged.
pub fn split_password(uri: &str) -> (String, Option<String>) {
    let Some((start, at)) = userinfo_range(uri) else {
        return (uri.to_string(), None);
    };
    let userinfo = &uri[start..at];
    // The first `:` separates user from password; any later one is inside the
    // password and must be left alone.
    let Some(colon) = userinfo.find(':') else {
        return (uri.to_string(), None);
    };
    let password = &userinfo[colon + 1..];
    if password.is_empty() {
        return (uri.to_string(), None);
    }
    let stripped = format!("{}{}{}", &uri[..start], &userinfo[..colon], &uri[at..]);
    (stripped, Some(password.to_string()))
}

/// Whether a URI carries credentials, and so expects a password to go with them.
///
/// The test is the presence of an `@`, not a non-empty username. Redis's usual
/// form is `redis://:password@host` with no username at all, and stripping that
/// leaves `redis://@host` — still a URI that expects a password. Requiring a
/// username here would report "no password needed" for exactly the connections
/// that need one most.
pub fn expects_password(uri: &str) -> bool {
    userinfo_range(uri).is_some()
}

/// Splice a password back into a URI produced by [`split_password`].
///
/// A URI that already carries a non-empty password is left alone, and one with
/// no `@` has nowhere to put one. An *empty* userinfo is not that second case:
/// `redis://@host` is what stripping `redis://:password@host` produces, and
/// splicing into it restores the original.
pub fn with_password(uri: &str, password: &str) -> String {
    let Some((start, at)) = userinfo_range(uri) else {
        return uri.to_string();
    };
    let userinfo = &uri[start..at];
    // A colon with something after it is an existing password; a colon with
    // nothing after it is an empty slot to fill.
    if userinfo
        .split_once(':')
        .is_some_and(|(_, pw)| !pw.is_empty())
    {
        return uri.to_string();
    }
    let user = userinfo.split_once(':').map_or(userinfo, |(u, _)| u);
    format!("{}{}:{}{}", &uri[..start], user, password, &uri[at..])
}

/// Replace a URI's credentials with `***` so it can be logged.
pub fn redact_uri(uri: &str) -> String {
    let Some((start, at)) = userinfo_range(uri) else {
        return uri.to_string();
    };
    let userinfo = &uri[start..at];
    let masked = match userinfo.find(':') {
        Some(_) => "***:***",
        None => "***",
    };
    format!("{}{}{}", &uri[..start], masked, &uri[at..])
}

/// Redact every connection string embedded in free text.
///
/// Driver errors and CLI stderr echo the connection string back verbatim, and
/// that text reaches both the log and the user-facing toast. Scan for each
/// known scheme and redact the URI-shaped token in place.
///
/// `rediss://` is listed before `redis://` so the longer scheme wins when both
/// match at the same offset.
pub fn redact_uris_in_text(text: &str) -> String {
    const SCHEMES: [&str; 4] = ["mongodb+srv://", "mongodb://", "rediss://", "redis://"];
    let mut out = String::with_capacity(text.len());
    let mut rest = text;

    'outer: loop {
        // Find whichever scheme appears first in what is left.
        let mut best: Option<(usize, &str)> = None;
        for scheme in SCHEMES {
            if let Some(idx) = rest.find(scheme) {
                if best.is_none_or(|(b, _)| idx < b) {
                    best = Some((idx, scheme));
                }
            }
        }
        let Some((idx, _)) = best else { break 'outer };

        // The URI runs to the next character that cannot appear in one.
        let tail = &rest[idx..];
        let end = tail
            .find(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | ',' | ')'))
            .unwrap_or(tail.len());

        out.push_str(&rest[..idx]);
        out.push_str(&redact_uri(&tail[..end]));
        rest = &tail[end..];
    }

    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_and_rejoin_a_password_round_trips() {
        // Passwords are deliberately distinctive: a single character would
        // appear incidentally in the host name and make the assertion useless.
        let cases = [
            "mongodb://user:hunter2@localhost:27017/db",
            "mongodb+srv://u:swordfish@cluster.example.net/db?retryWrites=true",
            "mongodb://u:swordfish@h1:27017,h2:27017/db",
            "mongodb://u:swordfish@[::1]:27017/db",
            // A percent-encoded password containing an encoded @ and :
            "mongodb://u:p%40ss%3Aw%2Frd@localhost:27017/?authSource=admin",
        ];
        for uri in cases {
            let (stripped, password) = split_password(uri);
            let password = password.unwrap_or_else(|| panic!("no password found in {}", uri));
            assert!(
                !stripped.contains(&password),
                "password survived in {}",
                stripped
            );
            assert_eq!(with_password(&stripped, &password), uri);
        }
    }

    #[test]
    fn split_password_stores_the_encoded_form_verbatim() {
        let (_, password) =
            split_password("mongodb://u:p%40ss%3Aw%2Frd@localhost:27017/?authSource=admin");
        // Not decoded: re-inserting is then a pure splice.
        assert_eq!(password.unwrap(), "p%40ss%3Aw%2Frd");
    }

    #[test]
    fn split_password_leaves_uris_without_one_alone() {
        for uri in [
            "mongodb://localhost:27017/db",
            "mongodb://user@localhost:27017/db",
            "mongodb://user:@localhost:27017/db",
            "not-a-uri",
        ] {
            let (stripped, password) = split_password(uri);
            assert_eq!(stripped, uri, "{} was modified", uri);
            assert!(password.is_none(), "{} yielded a password", uri);
        }
    }

    #[test]
    fn with_password_does_not_double_up_or_invent_a_user() {
        // Already has one: unchanged.
        assert_eq!(
            with_password("mongodb://u:existing@h:27017", "new"),
            "mongodb://u:existing@h:27017"
        );
        // No userinfo at all: nowhere to put it.
        assert_eq!(
            with_password("mongodb://h:27017", "new"),
            "mongodb://h:27017"
        );
    }

    #[test]
    fn redact_covers_every_uri_shape() {
        // credentials -> masked
        assert_eq!(
            redact_uri("mongodb://user:pass@localhost:27017/db"),
            "mongodb://***:***@localhost:27017/db"
        );
        // username with no password
        assert_eq!(
            redact_uri("mongodb://user@localhost:27017"),
            "mongodb://***@localhost:27017"
        );
        // no credentials -> untouched
        assert_eq!(
            redact_uri("mongodb://localhost:27017/db"),
            "mongodb://localhost:27017/db"
        );
        // srv
        assert_eq!(
            redact_uri("mongodb+srv://u:p@cluster.example.net/db?retryWrites=true"),
            "mongodb+srv://***:***@cluster.example.net/db?retryWrites=true"
        );
        // multi-host seedlist
        assert_eq!(
            redact_uri("mongodb://u:p@h1:27017,h2:27017/db"),
            "mongodb://***:***@h1:27017,h2:27017/db"
        );
        // IPv6 literal
        assert_eq!(
            redact_uri("mongodb://u:p@[::1]:27017/db"),
            "mongodb://***:***@[::1]:27017/db"
        );
        // an @ in the host list must not be mistaken for the separator
        assert_eq!(
            redact_uri("mongodb://localhost:27017/?appName=a@b"),
            "mongodb://localhost:27017/?appName=a@b"
        );
    }

    #[test]
    fn redact_scrubs_uris_out_of_tool_stderr() {
        let stderr = "Failed: can\'t create session: connection() error occurred during \
connection handshake: auth error: sasl conversation error: unable to authenticate \
using mechanism \"SCRAM-SHA-1\": (AuthenticationFailed) Authentication failed., \
uri: mongodb://admin:hunter2@10.0.0.5:27017/?authSource=admin";
        let out = redact_uris_in_text(stderr);
        assert!(!out.contains("hunter2"), "password survived: {}", out);
        assert!(out.contains("mongodb://***:***@10.0.0.5:27017/?authSource=admin"));
    }

    #[test]
    fn redact_leaves_text_without_uris_alone() {
        let plain = "mongorestore failed: no such file or directory";
        assert_eq!(redact_uris_in_text(plain), plain);
    }

    #[test]
    fn redis_uris_split_and_rejoin_like_mongo_ones() {
        // Redis puts the password in the same place, but usually with no
        // username at all ("redis://:pass@host"), which is the shape most
        // likely to be mishandled.
        let cases = [
            "redis://:hunter2@localhost:6379/0",
            "redis://default:swordfish@10.0.1.5:6379/2?timeout=5",
            "rediss://user:p%40ssword@cache.example.net:6380",
            "redis://u:swordfish@[::1]:6379/1",
        ];
        for uri in cases {
            let (stripped, password) = split_password(uri);
            let password = password.unwrap_or_else(|| panic!("no password found in {}", uri));
            assert!(
                !stripped.contains(&password),
                "password survived in {}",
                stripped
            );
            assert_eq!(with_password(&stripped, &password), uri);
        }
    }

    #[test]
    fn a_stripped_password_only_uri_still_expects_a_password() {
        // Redis's usual form has no username, so stripping "redis://:pw@h"
        // leaves "redis://@h". Reading that as "no credentials" would connect
        // unauthenticated instead of prompting for the missing secret.
        assert!(expects_password("redis://@localhost:6379"));
        assert!(expects_password("redis://default@localhost:6379"));
        assert!(expects_password("mongodb://user@localhost:27017"));
        assert!(!expects_password("redis://localhost:6379"));
        assert!(!expects_password("mongodb://localhost:27017/db"));
    }

    #[test]
    fn with_password_fills_an_empty_slot_but_never_overwrites_a_real_one() {
        // The empty-username form, which is what Redis stripping produces.
        assert_eq!(
            with_password("redis://@h:6379/0", "hunter2"),
            "redis://:hunter2@h:6379/0"
        );
        // An explicit empty password slot is also a slot.
        assert_eq!(
            with_password("redis://:@h:6379", "hunter2"),
            "redis://:hunter2@h:6379"
        );
        // A real password is never replaced.
        assert_eq!(
            with_password("redis://:existing@h:6379", "new"),
            "redis://:existing@h:6379"
        );
    }

    #[test]
    fn redact_covers_redis_uri_shapes() {
        assert_eq!(
            redact_uri("redis://:pass@localhost:6379/0"),
            "redis://***:***@localhost:6379/0"
        );
        assert_eq!(
            redact_uri("rediss://user:pass@cache.example.net:6380"),
            "rediss://***:***@cache.example.net:6380"
        );
        assert_eq!(
            redact_uri("redis://localhost:6379/0"),
            "redis://localhost:6379/0"
        );
    }

    #[test]
    fn redact_in_text_handles_both_families_and_prefers_the_longer_scheme() {
        // "rediss://" starts with "redis" + "s://" -- listing redis:// first
        // would redact from the wrong offset and leave "s://..." behind.
        let text = "connect rediss://u:hunter2@a:6380 failed; \
also mongodb+srv://m:swordfish@c.example.net/db and redis://:topsecret@b:6379";
        let out = redact_uris_in_text(text);
        for secret in ["hunter2", "swordfish", "topsecret"] {
            assert!(!out.contains(secret), "{} survived: {}", secret, out);
        }
        assert!(out.contains("rediss://***:***@a:6380"), "{}", out);
        assert!(
            out.contains("mongodb+srv://***:***@c.example.net/db"),
            "{}",
            out
        );
        assert!(out.contains("redis://***:***@b:6379"), "{}", out);
    }
}
