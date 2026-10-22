//! Reading and deciding on `known_hosts` entries.
//!
//! **This does not use `ssh2::KnownHosts`, deliberately.** That wrapper is
//! unusable for the job in three separate ways, each verified in the vendored
//! libssh2 source:
//!
//! - `check_port` hardcodes `TYPE_PLAIN | KEYENC_RAW` with **no key-type bit**,
//!   and libssh2's own rule is "if key_type is zero, match always". So it
//!   compares a stored ed25519 line against an offered ecdsa key and reports a
//!   mismatch. libssh2 negotiates ecdsa-nistp256 first while OpenSSH writes
//!   ed25519, so that is the *common* case — it would raise a full "the host
//!   key has changed" alarm on a host the user has trusted for years. A
//!   mismatch warning that is wrong is worse than no warning at all, because
//!   it teaches people to click through the one screen that must never be
//!   clicked through.
//! - `libssh2_knownhost_readfile` returns an error on the **first** line it
//!   cannot parse, so one odd line in a long file means trusting nothing.
//! - `libssh2_knownhost_writefile` opens with `FOPEN_WRITETEXT` — truncate —
//!   and regenerates the whole file from memory. Pointing that at a user's
//!   `~/.ssh/known_hosts` would drop every comment and race any concurrent
//!   `ssh`.
//!
//! So the parsing and the decision live here, as pure functions over text, and
//! every branch is testable without a socket.

use base64::Engine as _;
use hmac::{Hmac, Mac};
use sha1::Sha1;

/// A marker prefix on a `known_hosts` line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Marker {
    None,
    /// The key signs host certificates. libssh2 cannot verify those, so an
    /// entry carrying this is recorded and reported, never silently treated as
    /// an ordinary key.
    CertAuthority,
    /// The user has declared this key compromised.
    Revoked,
}

/// How one name in a line's name-list is written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Pattern {
    /// A literal name or a `*`/`?` glob.
    Plain(String),
    /// A `!`-prefixed exclusion, which suppresses any other match on the line.
    Negated(String),
    /// A `|1|salt|hash` entry: HMAC-SHA1 of the name, keyed by the salt.
    Hashed { salt: Vec<u8>, hash: Vec<u8> },
}

/// One parsed line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostEntry {
    pub marker: Marker,
    pub patterns: Vec<Pattern>,
    pub key_type: String,
    pub key: Vec<u8>,
    /// Where this came from, so a mismatch can name the file and line the user
    /// has to edit. Without it they are told something is wrong and not where.
    pub source: String,
}

/// What to do about the key a server presented.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Trust {
    /// A stored key of this type matches.
    Match,
    /// Nothing on file for this host.
    Unknown,
    /// Entries exist for this host, but none of the type offered.
    ///
    /// **Deliberately not a mismatch.** A host with ed25519 on file that
    /// offers ecdsa has not changed its identity; we simply have not seen this
    /// key. Conflating the two is the exact bug that makes the warning
    /// untrustworthy.
    UnknownForThisType,
    /// A stored key of this type is different. Refuse.
    Mismatch { expected: String, source: String },
    /// The user has declared this key compromised.
    Revoked { source: String },
}

/// The lookup name OpenSSH uses. Port 22 is bare; anything else is bracketed.
pub fn lookup_name(host: &str, port: u16) -> String {
    if port == 22 {
        host.to_string()
    } else {
        format!("[{}]:{}", host, port)
    }
}

/// Parse a whole file, skipping what cannot be understood.
///
/// A malformed line is dropped with a warning rather than failing the read.
/// One stray line must never mean trusting nothing — which is what libssh2's
/// own reader does.
pub fn parse_file(text: &str, source_name: &str) -> Vec<HostEntry> {
    text.lines()
        .enumerate()
        .filter_map(|(i, line)| {
            let source = format!("{}:{}", source_name, i + 1);
            match parse_line(line, &source) {
                Ok(Some(entry)) => Some(entry),
                Ok(None) => None,
                Err(reason) => {
                    tracing::warn!(source = %source, "skipping known_hosts line: {}", reason);
                    None
                }
            }
        })
        .collect()
}

/// Parse one line. `Ok(None)` is a comment or blank.
pub fn parse_line(line: &str, source: &str) -> Result<Option<HostEntry>, String> {
    // Trailing \r matters: a file edited on Windows would otherwise put a
    // carriage return inside the base64 and fail every key.
    let line = line.trim_end_matches(['\r', '\n']).trim();
    if line.is_empty() || line.starts_with('#') {
        return Ok(None);
    }

    let mut fields = line.split_whitespace();
    let mut first = fields.next().ok_or("empty line")?;

    let marker = match first {
        "@cert-authority" => {
            first = fields.next().ok_or("marker with no host list")?;
            Marker::CertAuthority
        }
        "@revoked" => {
            first = fields.next().ok_or("marker with no host list")?;
            Marker::Revoked
        }
        _ if first.starts_with('@') => return Err(format!("unknown marker {}", first)),
        _ => Marker::None,
    };

    let patterns = parse_patterns(first)?;
    let key_type = fields.next().ok_or("no key type")?.to_string();
    let key_b64 = fields.next().ok_or("no key")?;
    let key = base64::engine::general_purpose::STANDARD
        .decode(key_b64)
        .map_err(|e| format!("key is not base64: {}", e))?;

    Ok(Some(HostEntry {
        marker,
        patterns,
        key_type,
        key,
        source: source.to_string(),
    }))
}

fn parse_patterns(field: &str) -> Result<Vec<Pattern>, String> {
    if let Some(rest) = field.strip_prefix("|1|") {
        let (salt, hash) = rest.split_once('|').ok_or("malformed hashed host")?;
        let engine = base64::engine::general_purpose::STANDARD;
        return Ok(vec![Pattern::Hashed {
            salt: engine.decode(salt).map_err(|_| "bad hash salt")?,
            hash: engine.decode(hash).map_err(|_| "bad host hash")?,
        }]);
    }
    if field.starts_with('|') {
        return Err("unsupported hash type".to_string());
    }

    Ok(field
        .split(',')
        .filter(|p| !p.is_empty())
        .map(|p| match p.strip_prefix('!') {
            Some(negated) => Pattern::Negated(negated.to_string()),
            None => Pattern::Plain(p.to_string()),
        })
        .collect())
}

/// Whether an entry covers this name.
pub fn entry_matches(entry: &HostEntry, name: &str) -> bool {
    // A negation suppresses the whole line, even if a glob on it would match.
    if entry.patterns.iter().any(|p| match p {
        Pattern::Negated(pattern) => glob_matches(pattern, name),
        _ => false,
    }) {
        return false;
    }

    entry.patterns.iter().any(|p| match p {
        Pattern::Plain(pattern) => glob_matches(pattern, name),
        Pattern::Hashed { salt, hash } => hashed_matches(salt, hash, name),
        Pattern::Negated(_) => false,
    })
}

/// OpenSSH's `*` and `?` globbing. No character classes — OpenSSH has none here.
fn glob_matches(pattern: &str, name: &str) -> bool {
    if !pattern.contains(['*', '?']) {
        return pattern.eq_ignore_ascii_case(name);
    }
    glob_inner(pattern.as_bytes(), name.to_ascii_lowercase().as_bytes())
}

fn glob_inner(pattern: &[u8], name: &[u8]) -> bool {
    match pattern.first() {
        None => name.is_empty(),
        Some(b'*') => {
            // Try every split point; patterns here are short enough that the
            // simple recursion is not worth avoiding.
            (0..=name.len()).any(|i| glob_inner(&pattern[1..], &name[i..]))
        }
        Some(b'?') => !name.is_empty() && glob_inner(&pattern[1..], &name[1..]),
        Some(c) => match name.first() {
            Some(n) if c.eq_ignore_ascii_case(n) => glob_inner(&pattern[1..], &name[1..]),
            _ => false,
        },
    }
}

fn hashed_matches(salt: &[u8], hash: &[u8], name: &str) -> bool {
    let Ok(mut mac) = Hmac::<Sha1>::new_from_slice(salt) else {
        return false;
    };
    mac.update(name.as_bytes());
    mac.finalize().into_bytes().as_slice() == hash
}

/// Decide what to do about the key a server just presented.
///
/// Pure, and the whole security decision. Every branch has a test.
pub fn decide(entries: &[HostEntry], name: &str, key_type: &str, key: &[u8]) -> Trust {
    let applicable: Vec<&HostEntry> = entries.iter().filter(|e| entry_matches(e, name)).collect();

    // A revocation wins over everything, including a match elsewhere in the
    // file: the user has said in writing that this key is compromised.
    if applicable
        .iter()
        .any(|e| e.marker == Marker::Revoked && e.key == key)
    {
        let source = applicable
            .iter()
            .find(|e| e.marker == Marker::Revoked && e.key == key)
            .map(|e| e.source.clone())
            .unwrap_or_default();
        return Trust::Revoked { source };
    }

    // Certificate authorities are recorded but cannot be honoured: libssh2
    // does not verify host certificates. Ignoring them silently would be worse
    // than treating the host as unseen.
    let usable: Vec<&&HostEntry> = applicable
        .iter()
        .filter(|e| e.marker == Marker::None)
        .collect();

    if usable.is_empty() {
        return Trust::Unknown;
    }

    let same_type: Vec<&&&HostEntry> = usable
        .iter()
        .filter(|e| e.key_type.eq_ignore_ascii_case(key_type))
        .collect();

    if same_type.is_empty() {
        // Entries exist, but we have never seen this key type for this host.
        return Trust::UnknownForThisType;
    }
    if same_type.iter().any(|e| e.key == key) {
        return Trust::Match;
    }

    let offender = same_type[0];
    Trust::Mismatch {
        expected: fingerprint(&offender.key),
        source: offender.source.clone(),
    }
}

/// The `SHA256:…` form `ssh-keygen -lf` prints: unpadded base64, no colons.
pub fn fingerprint(key: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(key);
    format!(
        "SHA256:{}",
        base64::engine::general_purpose::STANDARD_NO_PAD.encode(digest)
    )
}

/// One `known_hosts` line for a key we are recording.
pub fn format_entry(name: &str, key_type: &str, key: &[u8], comment: &str) -> String {
    format!(
        "{} {} {} {}\n",
        name,
        key_type,
        base64::engine::general_purpose::STANDARD.encode(key),
        comment
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two distinct key blobs; the bytes are arbitrary but must differ.
    const KEY_A: &[u8] = b"AAAAC3NzaC1lZDI1NTE5AAAAIkey-a";
    const KEY_B: &[u8] = b"AAAAC3NzaC1lZDI1NTE5AAAAIkey-b";

    fn entry(line: &str) -> HostEntry {
        parse_line(line, "test:1").unwrap().unwrap()
    }

    fn ed25519_line(names: &str, key: &[u8]) -> String {
        format!(
            "{} ssh-ed25519 {}",
            names,
            base64::engine::general_purpose::STANDARD.encode(key)
        )
    }

    // -- the decision, which is the whole feature -------------------------

    #[test]
    fn a_stored_key_of_the_same_type_matches() {
        let entries = vec![entry(&ed25519_line("example.com", KEY_A))];
        assert_eq!(
            decide(&entries, "example.com", "ssh-ed25519", KEY_A),
            Trust::Match
        );
    }

    #[test]
    fn a_different_key_of_the_same_type_is_a_mismatch() {
        let entries = vec![entry(&ed25519_line("example.com", KEY_A))];
        match decide(&entries, "example.com", "ssh-ed25519", KEY_B) {
            Trust::Mismatch { expected, source } => {
                assert_eq!(expected, fingerprint(KEY_A));
                assert_eq!(source, "test:1", "the user must be told which line");
            }
            other => panic!("expected a mismatch, got {:?}", other),
        }
    }

    #[test]
    fn a_different_key_type_is_not_a_mismatch() {
        // The bug that would have made this feature worse than useless.
        // libssh2 negotiates ecdsa-nistp256 first; OpenSSH writes ed25519. So
        // a host trusted for years, whose ed25519 line is on file, offers
        // ecdsa -- and calling that "the host key has changed" teaches people
        // to click through the one screen that must never be clicked through.
        let entries = vec![entry(&ed25519_line("example.com", KEY_A))];
        assert_eq!(
            decide(&entries, "example.com", "ecdsa-sha2-nistp256", KEY_B),
            Trust::UnknownForThisType
        );
    }

    #[test]
    fn an_unseen_host_is_unknown() {
        let entries = vec![entry(&ed25519_line("other.com", KEY_A))];
        assert_eq!(
            decide(&entries, "example.com", "ssh-ed25519", KEY_A),
            Trust::Unknown
        );
    }

    #[test]
    fn a_revoked_key_beats_a_match_elsewhere_in_the_file() {
        // The user has said in writing that this key is compromised. A
        // matching line further down must not rescue it.
        let entries = vec![
            entry(&ed25519_line("example.com", KEY_A)),
            entry(&format!("@revoked {}", ed25519_line("example.com", KEY_A))),
        ];
        assert!(matches!(
            decide(&entries, "example.com", "ssh-ed25519", KEY_A),
            Trust::Revoked { .. }
        ));
    }

    #[test]
    fn a_certificate_authority_entry_is_not_treated_as_an_ordinary_key() {
        // libssh2 cannot verify host certificates, so honouring a CA line
        // would be a lie. Treating the host as unseen is the honest answer.
        let entries = vec![entry(&format!(
            "@cert-authority {}",
            ed25519_line("*.example.com", KEY_A)
        ))];
        assert_eq!(
            decide(&entries, "host.example.com", "ssh-ed25519", KEY_A),
            Trust::Unknown
        );
    }

    // -- name matching ----------------------------------------------------

    #[test]
    fn the_lookup_name_follows_openssh_port_rules() {
        // A bare name for 22 and a bracketed one otherwise. Diverging here is
        // how you manufacture surprises against the user's own file.
        assert_eq!(lookup_name("example.com", 22), "example.com");
        assert_eq!(lookup_name("example.com", 2222), "[example.com]:2222");
    }

    #[test]
    fn a_port_specific_entry_does_not_cover_the_default_port() {
        let entries = vec![entry(&ed25519_line("[example.com]:2222", KEY_A))];
        assert_eq!(
            decide(&entries, "example.com", "ssh-ed25519", KEY_A),
            Trust::Unknown
        );
        assert_eq!(
            decide(&entries, "[example.com]:2222", "ssh-ed25519", KEY_A),
            Trust::Match
        );
    }

    #[test]
    fn a_comma_separated_name_list_covers_every_name() {
        let entries = vec![entry(&ed25519_line(
            "a.example.com,b.example.com,10.0.0.1",
            KEY_A,
        ))];
        for name in ["a.example.com", "b.example.com", "10.0.0.1"] {
            assert_eq!(
                decide(&entries, name, "ssh-ed25519", KEY_A),
                Trust::Match,
                "{} should match",
                name
            );
        }
    }

    #[test]
    fn globs_match_and_negations_suppress_the_whole_line() {
        let entries = vec![entry(&ed25519_line("!secret.internal,*.internal", KEY_A))];
        assert_eq!(
            decide(&entries, "web.internal", "ssh-ed25519", KEY_A),
            Trust::Match
        );
        assert_eq!(
            decide(&entries, "secret.internal", "ssh-ed25519", KEY_A),
            Trust::Unknown,
            "a negation must suppress the glob on the same line"
        );
    }

    #[test]
    fn a_hashed_entry_matches_its_own_host_and_nothing_else() {
        // Computed here rather than pasted, so the test proves the HMAC
        // construction rather than a fixture someone may have mistyped.
        let salt = b"0123456789abcdefffff";
        let mut mac = Hmac::<Sha1>::new_from_slice(salt).unwrap();
        mac.update(b"example.com");
        let engine = base64::engine::general_purpose::STANDARD;
        let line = ed25519_line(
            &format!(
                "|1|{}|{}",
                engine.encode(salt),
                engine.encode(mac.finalize().into_bytes())
            ),
            KEY_A,
        );

        let entries = vec![entry(&line)];
        assert_eq!(
            decide(&entries, "example.com", "ssh-ed25519", KEY_A),
            Trust::Match
        );
        assert_eq!(
            decide(&entries, "example.org", "ssh-ed25519", KEY_A),
            Trust::Unknown
        );
    }

    // -- parsing ----------------------------------------------------------

    #[test]
    fn one_bad_line_does_not_discard_the_rest_of_the_file() {
        // libssh2's own reader gives up on the first line it cannot parse,
        // which on a real file means trusting nothing at all.
        let text = format!(
            "# a comment\n\n{}\nthis is not a known_hosts line\n{}\n",
            ed25519_line("good1.example.com", KEY_A),
            ed25519_line("good2.example.com", KEY_B),
        );
        let entries = parse_file(&text, "known_hosts");
        assert_eq!(entries.len(), 2, "both good lines should survive");
        assert_eq!(entries[0].source, "known_hosts:3");
        assert_eq!(entries[1].source, "known_hosts:5");
    }

    #[test]
    fn crlf_line_endings_do_not_corrupt_the_key() {
        // A stray carriage return inside the base64 fails every key on a file
        // that was edited on Windows.
        let text = format!("{}\r\n", ed25519_line("example.com", KEY_A));
        let entries = parse_file(&text, "known_hosts");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].key, KEY_A);
    }

    #[test]
    fn a_trailing_comment_is_ignored() {
        let line = format!(
            "{} added by someone 2024",
            ed25519_line("example.com", KEY_A)
        );
        assert_eq!(entry(&line).key, KEY_A);
    }

    #[test]
    fn markers_are_recognised_rather_than_read_as_host_names() {
        // libssh2 takes "@revoked" as the hostname and the pattern as the key
        // type, so the actually-revoked key is never recorded -- and the user
        // gets invited to trust it.
        assert_eq!(
            entry(&format!("@revoked {}", ed25519_line("h", KEY_A))).marker,
            Marker::Revoked
        );
        assert_eq!(
            entry(&format!("@cert-authority {}", ed25519_line("h", KEY_A))).marker,
            Marker::CertAuthority
        );
        assert_eq!(entry(&ed25519_line("h", KEY_A)).marker, Marker::None);
    }

    #[test]
    fn unparseable_lines_are_rejected_rather_than_half_understood() {
        for bad in [
            "example.com ssh-ed25519",             // no key
            "example.com",                         // no key type
            "example.com ssh-ed25519 not!base64!", // undecodable
            "@nonsense example.com ssh-ed25519 AAAA",
            "|2|salt|hash ssh-ed25519 AAAA", // unknown hash type
        ] {
            assert!(parse_line(bad, "t:1").is_err(), "should reject {:?}", bad);
        }
        assert_eq!(parse_line("", "t:1").unwrap(), None);
        assert_eq!(parse_line("   # comment", "t:1").unwrap(), None);
    }

    // -- formatting -------------------------------------------------------

    #[test]
    fn the_fingerprint_matches_what_ssh_keygen_prints() {
        // Unpadded base64 after `SHA256:` -- users compare this against
        // `ssh-keygen -lf` character by character, which is the whole point of
        // showing it to them.
        let printed = fingerprint(KEY_A);
        assert!(printed.starts_with("SHA256:"));
        assert!(!printed.ends_with('='), "OpenSSH prints it unpadded");
        assert_eq!(printed.len(), "SHA256:".len() + 43);
    }

    #[test]
    fn a_written_entry_parses_back_as_itself() {
        // The store has to stay valid input to ssh-keygen -F and -R, which is
        // what the mismatch dialog tells the user to run.
        let line = format_entry("example.com", "ssh-ed25519", KEY_A, "added by TermDrop");
        let parsed = parse_line(line.trim_end(), "store:1").unwrap().unwrap();
        assert_eq!(parsed.key, KEY_A);
        assert_eq!(parsed.key_type, "ssh-ed25519");
        assert_eq!(
            decide(&[parsed], "example.com", "ssh-ed25519", KEY_A),
            Trust::Match
        );
    }
}

/// Parsing a real `known_hosts` file.
///
/// `#[ignore]`d because it needs a file to point at, but it is the check that
/// matters for the interoperability claim: our own parser has to understand
/// what OpenSSH actually wrote, not what the format documentation says.
///
///   TERMDROP_KNOWN_HOSTS=~/.ssh/known_hosts cargo test real_known_hosts -- --ignored --nocapture
#[cfg(test)]
mod real_file {
    use super::*;

    #[test]
    #[ignore]
    fn a_real_known_hosts_file_parses_without_losing_lines() {
        let path = std::env::var("TERMDROP_KNOWN_HOSTS")
            .expect("set TERMDROP_KNOWN_HOSTS to a known_hosts file");
        let text = std::fs::read_to_string(&path).expect("could not read it");

        let meaningful = text
            .lines()
            .filter(|l| {
                let l = l.trim();
                !l.is_empty() && !l.starts_with('#')
            })
            .count();
        let entries = parse_file(&text, &path);

        println!(
            "{} meaningful lines -> {} entries",
            meaningful,
            entries.len()
        );
        for e in entries.iter().take(3) {
            println!("  {} {} ({:?})", e.source, e.key_type, e.marker);
        }
        assert_eq!(
            entries.len(),
            meaningful,
            "every non-comment line should have parsed"
        );
    }
}
