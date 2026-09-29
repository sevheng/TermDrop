//! The trust store: which host keys we have seen, and what to do about one.
//!
//! **We read the user's OpenSSH files and never write them.** Their
//! `~/.ssh/known_hosts` supplies the history, so a host already trusted in a
//! terminal produces no prompt here on the first run — that is the half of
//! interoperability worth having, and it is the half with no risk. Our own
//! acceptances go to `<data_dir>/known_hosts`, appended one line at a time.
//!
//! Owning the user's file has no partial-credit version: the moment we write
//! it we own its corruption modes, its locking against a concurrent `ssh`, and
//! its behaviour when it is a symlink into a synced folder. For a feature
//! whose whole value is being trustworthy, that is the wrong first move.
//!
//! The store is OpenSSH format rather than a SQLite table on purpose: the
//! mismatch dialog tells the user to run `ssh-keygen -R`, and that has to
//! actually work against the file we point them at.

use super::known_hosts::{self, HostEntry, Trust};
use ssh2::{HashType, Session};
use std::io::Write;
use std::path::PathBuf;

/// What a user is shown and asked about.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct HostKeyInfo {
    pub host: String,
    pub port: u16,
    pub fingerprint: String,
    pub key_type: String,
}

/// Why a connection was refused, before any credential was sent.
#[derive(Debug)]
pub enum Refusal {
    /// Nothing on file. The user has to be asked, once.
    Unknown(HostKeyInfo),
    /// Entries exist but not for this key type. Also a first-trust decision,
    /// and emphatically **not** a mismatch.
    UnknownType(HostKeyInfo),
    /// A stored key of this type is different, or the key is revoked, or the
    /// store could not be read. No path through.
    Refused { info: HostKeyInfo, message: String },
}

impl Refusal {
    pub fn info(&self) -> &HostKeyInfo {
        match self {
            Self::Unknown(i) | Self::UnknownType(i) => i,
            Self::Refused { info, .. } => info,
        }
    }

    fn kind(&self) -> &'static str {
        match self {
            Self::Unknown(_) | Self::UnknownType(_) => "unknown",
            Self::Refused { .. } => "refused",
        }
    }
}

/// Marks an error the frontend must answer with a dialog rather than a toast.
///
/// `ssh_connect` returns `Result<String, String>`, so structured data travels
/// inside the error text — the same shape the keyring retry already uses. The
/// frontend must require this at **index 0**, never `includes`: a hostname or
/// a server banner could otherwise forge a trust dialog.
pub const TRUST_PREFIX: &str = "TERMDROP_TRUST:";

/// Encode a refusal for the frontend.
pub fn refusal_payload(refusal: &Refusal) -> String {
    let info = refusal.info();
    let message = match refusal {
        Refusal::Refused { message, .. } => message.clone(),
        _ => String::new(),
    };
    format!(
        "{}{}",
        TRUST_PREFIX,
        serde_json::json!({
            "kind": refusal.kind(),
            "host": info.host,
            "port": info.port,
            "fingerprint": info.fingerprint,
            "keyType": info.key_type,
            "message": message,
        })
    )
}

/// Our own store, the only file we write.
pub fn store_path() -> PathBuf {
    if let Ok(path) = std::env::var("TERMDROP_KNOWN_HOSTS") {
        return PathBuf::from(path);
    }
    dirs::data_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("termdrop")
        .join("known_hosts")
}

/// The OpenSSH files we read but never touch.
fn openssh_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Some(home) = dirs::home_dir() {
        paths.push(home.join(".ssh").join("known_hosts"));
        paths.push(home.join(".ssh").join("known_hosts2"));
    }
    paths.push(PathBuf::from("/etc/ssh/ssh_known_hosts"));
    paths
}

/// Every entry we can see, ours first.
pub fn load_entries() -> Vec<HostEntry> {
    let mut entries = Vec::new();
    let mut sources = vec![store_path()];
    sources.extend(openssh_paths());

    for path in sources {
        // A missing file is not an error: on a first run every one of these is
        // absent, which simply means every host is unknown.
        if let Ok(text) = std::fs::read_to_string(&path) {
            entries.extend(known_hosts::parse_file(&text, &path.display().to_string()));
        }
    }
    entries
}

/// The key the session just negotiated.
fn presented(session: &Session, host: &str, port: u16) -> Result<(Vec<u8>, HostKeyInfo), String> {
    let (key, key_type) = session
        .host_key()
        .ok_or_else(|| "the server presented no host key".to_string())?;
    let hash = session
        .host_key_hash(HashType::Sha256)
        .ok_or_else(|| "could not fingerprint the server's host key".to_string())?;

    Ok((
        key.to_vec(),
        HostKeyInfo {
            host: host.to_string(),
            port,
            fingerprint: known_hosts::fingerprint(hash),
            key_type: key_type_name(key_type).to_string(),
        },
    ))
}

fn key_type_name(key_type: ssh2::HostKeyType) -> &'static str {
    match key_type {
        ssh2::HostKeyType::Rsa => "ssh-rsa",
        ssh2::HostKeyType::Dss => "ssh-dss",
        ssh2::HostKeyType::Ecdsa256 => "ecdsa-sha2-nistp256",
        ssh2::HostKeyType::Ecdsa384 => "ecdsa-sha2-nistp384",
        ssh2::HostKeyType::Ecdsa521 => "ecdsa-sha2-nistp521",
        ssh2::HostKeyType::Ed25519 => "ssh-ed25519",
        _ => "unknown",
    }
}

/// Check the server's identity, before a single credential leaves this machine.
///
/// Called between `handshake()` and any `userauth_*`. The ordering *is* the
/// feature: verifying afterwards means the password has already gone to
/// whoever answered.
///
/// `accept` is the fingerprint the user explicitly agreed to, compared against
/// the key actually presented — so an acceptance can never admit a different
/// server than the one shown.
pub fn verify(
    session: &Session,
    host: &str,
    port: u16,
    accept: Option<&str>,
) -> Result<(), Refusal> {
    let (key, info) = match presented(session, host, port) {
        Ok(pair) => pair,
        Err(message) => {
            return Err(Refusal::Refused {
                info: HostKeyInfo {
                    host: host.to_string(),
                    port,
                    fingerprint: String::new(),
                    key_type: String::new(),
                },
                message,
            })
        }
    };

    let name = known_hosts::lookup_name(host, port);
    let entries = load_entries();

    match known_hosts::decide(&entries, &name, &info.key_type, &key) {
        Trust::Match => Ok(()),

        Trust::Unknown | Trust::UnknownForThisType => {
            // Only the exact fingerprint the user was shown gets through.
            if accept == Some(info.fingerprint.as_str()) {
                if let Err(e) = remember(&name, &info.key_type, &key) {
                    tracing::warn!("could not record the host key: {}", e);
                }
                return Ok(());
            }
            Err(
                match known_hosts::decide(&entries, &name, &info.key_type, &key) {
                    Trust::UnknownForThisType => Refusal::UnknownType(info),
                    _ => Refusal::Unknown(info),
                },
            )
        }

        Trust::Mismatch { expected, source } => {
            let message = mismatch_message(&info, &expected, &source);
            Err(Refusal::Refused { info, message })
        }

        Trust::Revoked { source } => {
            let message = format!(
                "The host key for {}:{} is listed as revoked in {}.\n\n\
                 Someone recorded this key as compromised. TermDrop will not \
                 connect to a server presenting it.",
                info.host, info.port, source
            );
            Err(Refusal::Refused { info, message })
        }
    }
}

/// What the user is told when a known host answers with a different key.
///
/// There is deliberately no way through from here. A rebuilt server and an
/// intercepted connection are indistinguishable from this side, so the remedy
/// is a detour — removing the old entry — not a button beside the warning.
/// Naming the exact file and line is what lets a competent user resolve it in
/// thirty seconds instead of hunting.
pub fn mismatch_message(info: &HostKeyInfo, expected: &str, source: &str) -> String {
    format!(
        "The host key for {}:{} has changed.\n\n\
         Expected {}\n  from {}\n\
         Offered  {} ({})\n\n\
         This happens when a server is reinstalled. It is also what an \
         intercepted connection looks like, and TermDrop cannot tell the two \
         apart — so it has not sent your credentials and will not connect.\n\n\
         If you know this server was rebuilt, remove the old entry and you \
         will be asked to verify the new key:\n\
         ssh-keygen -R {} -f {}",
        info.host,
        info.port,
        expected,
        source,
        info.fingerprint,
        info.key_type,
        info.host,
        source.split(':').next().unwrap_or(source),
    )
}

/// Append an accepted key to our own store. Never touches the user's file.
fn remember(name: &str, key_type: &str, key: &[u8]) -> Result<(), String> {
    let path = store_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("could not create {}: {}", parent.display(), e))?;
    }

    // Append, never rewrite: the file stays valid for `ssh-keygen -F` and `-R`,
    // and nothing already in it can be lost by a bug here.
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|e| format!("could not open {}: {}", path.display(), e))?;

    let line = known_hosts::format_entry(name, key_type, key, "added by TermDrop");
    file.write_all(line.as_bytes())
        .map_err(|e| format!("could not write {}: {}", path.display(), e))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info() -> HostKeyInfo {
        HostKeyInfo {
            host: "db.internal".to_string(),
            port: 22,
            fingerprint: "SHA256:offered".to_string(),
            key_type: "ssh-ed25519".to_string(),
        }
    }

    #[test]
    fn the_payload_is_machine_readable_and_prefixed_at_the_start() {
        // The frontend must match on index 0, not `includes`, so a hostname
        // containing the sentinel cannot forge a trust dialog. That only works
        // if the prefix really is first.
        let payload = refusal_payload(&Refusal::Unknown(info()));
        assert!(payload.starts_with(TRUST_PREFIX));

        let json: serde_json::Value =
            serde_json::from_str(payload.trim_start_matches(TRUST_PREFIX)).unwrap();
        assert_eq!(json["kind"], "unknown");
        assert_eq!(json["host"], "db.internal");
        assert_eq!(json["fingerprint"], "SHA256:offered");
    }

    #[test]
    fn an_unknown_key_type_is_reported_as_unknown_not_refused() {
        // It is a first-trust decision, and the dialog the user sees must be
        // the ordinary one rather than the alarm.
        let payload = refusal_payload(&Refusal::UnknownType(info()));
        let json: serde_json::Value =
            serde_json::from_str(payload.trim_start_matches(TRUST_PREFIX)).unwrap();
        assert_eq!(json["kind"], "unknown");
    }

    #[test]
    fn a_refusal_carries_the_explanation_and_an_unknown_does_not() {
        let unknown: serde_json::Value = serde_json::from_str(
            refusal_payload(&Refusal::Unknown(info())).trim_start_matches(TRUST_PREFIX),
        )
        .unwrap();
        assert_eq!(unknown["message"], "");

        let refused = Refusal::Refused {
            info: info(),
            message: "it changed".to_string(),
        };
        let json: serde_json::Value =
            serde_json::from_str(refusal_payload(&refused).trim_start_matches(TRUST_PREFIX))
                .unwrap();
        assert_eq!(json["kind"], "refused");
        assert_eq!(json["message"], "it changed");
    }

    #[test]
    fn the_mismatch_message_names_the_file_and_offers_no_shortcut() {
        let message = mismatch_message(&info(), "SHA256:expected", "/home/u/.ssh/known_hosts:412");
        assert!(
            message.contains("/home/u/.ssh/known_hosts:412"),
            "must name where"
        );
        assert!(
            message.contains("SHA256:expected"),
            "must show what was expected"
        );
        assert!(
            message.contains("SHA256:offered"),
            "must show what was offered"
        );
        assert!(
            message.contains("ssh-keygen -R db.internal"),
            "must give the remedy"
        );
        assert!(
            !message.to_lowercase().contains("anyway"),
            "the dangerous path must not be offered"
        );
    }

    #[test]
    fn the_store_is_ours_and_separate_from_the_users_file() {
        // We read ~/.ssh/known_hosts and never write it; our acceptances go
        // somewhere we own.
        let previous = std::env::var("TERMDROP_KNOWN_HOSTS").ok();
        std::env::remove_var("TERMDROP_KNOWN_HOSTS");
        let store = store_path();
        assert!(
            !store.starts_with(dirs::home_dir().unwrap_or_default().join(".ssh")),
            "the store must not live in ~/.ssh: {}",
            store.display()
        );
        if let Some(value) = previous {
            std::env::set_var("TERMDROP_KNOWN_HOSTS", value);
        }
    }
}
