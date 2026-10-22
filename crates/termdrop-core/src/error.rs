//! Typed errors for the calls a UI has to answer with a dialog rather than a
//! toast.
//!
//! Inside the backend errors are still strings, as they always were: the
//! trust refusal travels as `TERMDROP_TRUST:{json}` and a missing secret as
//! "keyring retrieve failed ...". [`CoreError::from`] reads those back into
//! types at the edge, and `Display` writes **exactly** the original text, so
//! the Tauri app's frontend (`utils/trustError.js`, `utils/secretPrompt.js`)
//! sees the same strings it always has.

use crate::ssh::trust::TRUST_PREFIX;
use std::fmt;

/// A host key the user has to decide about before the connection can go on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrustPrompt {
    /// `"unknown"` (first sight, or a new key type) or `"refused"` (a changed
    /// or revoked key, which has no accept path).
    pub kind: String,
    pub host: String,
    pub port: u16,
    pub fingerprint: String,
    pub key_type: String,
    /// Why a `refused` key was refused; empty otherwise.
    pub message: String,
    /// `"target"` or `"jump"`: which connection the acceptance is for.
    pub hop: String,
    /// The original wire text, so `Display` reproduces it byte for byte.
    raw: String,
}

impl TrustPrompt {
    /// Whether the user may accept this key. A changed or revoked key may
    /// never be accepted from the UI.
    pub fn can_accept(&self) -> bool {
        self.kind == "unknown"
    }

    /// Whether the acceptance goes back as `accept_jump_host_key` rather than
    /// `accept_host_key`.
    pub fn is_jump(&self) -> bool {
        self.hop == "jump"
    }

    /// Parse a refusal; `None` unless the prefix is at **index 0**. Matching
    /// anywhere else would let a hostname or a banner forge a trust dialog.
    pub fn parse(text: &str) -> Option<Self> {
        let json = text.strip_prefix(TRUST_PREFIX)?;
        let v: serde_json::Value = serde_json::from_str(json).ok()?;
        let s = |key: &str| v[key].as_str().unwrap_or_default().to_string();
        Some(TrustPrompt {
            kind: s("kind"),
            host: s("host"),
            port: v["port"].as_u64().and_then(|p| u16::try_from(p).ok())?,
            fingerprint: s("fingerprint"),
            key_type: s("keyType"),
            message: s("message"),
            hop: s("hop"),
            raw: text.to_string(),
        })
    }
}

/// An error from a [`crate::Core`] call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CoreError {
    /// The server's host key needs the user's decision. Retry with its
    /// fingerprint as the acceptance for that hop. Boxed because it is several
    /// times the size of the other variants, and every `CoreResult` pays for
    /// the largest one.
    Trust(Box<TrustPrompt>),
    /// No secret is stored for this connection. Ask the user for one and pass
    /// it as the password override on the retry. The text is the original
    /// message, which says which secret (SSH, MongoDB, Redis, VNC) was missing.
    NeedsPassword(String),
    /// Anything else, as the text the UI shows.
    Other(String),
}

impl CoreError {
    /// The exact string the Tauri commands have always returned.
    pub fn into_wire(self) -> String {
        self.to_string()
    }
}

impl From<String> for CoreError {
    fn from(text: String) -> Self {
        if let Some(prompt) = TrustPrompt::parse(&text) {
            return CoreError::Trust(Box::new(prompt));
        }
        // The same two substrings `utils/secretPrompt.js` matches on.
        if text.contains("keyring retrieve failed") || text.contains("No matching entry") {
            return CoreError::NeedsPassword(text);
        }
        CoreError::Other(text)
    }
}

impl From<&str> for CoreError {
    fn from(text: &str) -> Self {
        CoreError::from(text.to_string())
    }
}

impl From<CoreError> for String {
    fn from(e: CoreError) -> Self {
        e.into_wire()
    }
}

impl fmt::Display for CoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CoreError::Trust(prompt) => f.write_str(&prompt.raw),
            CoreError::NeedsPassword(text) | CoreError::Other(text) => f.write_str(text),
        }
    }
}

impl std::error::Error for CoreError {}

pub type CoreResult<T> = Result<T, CoreError>;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ssh::trust::{refusal_payload, HostKeyInfo, Refusal};

    fn info() -> HostKeyInfo {
        HostKeyInfo {
            host: "example.com".into(),
            port: 2222,
            fingerprint: "SHA256:abc".into(),
            key_type: "ssh-ed25519".into(),
        }
    }

    #[test]
    fn a_refusal_round_trips_to_the_exact_wire_text() {
        let wire = refusal_payload(&Refusal::Unknown(info()));
        let e = CoreError::from(wire.clone());
        let CoreError::Trust(prompt) = &e else {
            panic!("not a trust prompt: {:?}", e);
        };
        assert_eq!(prompt.host, "example.com");
        assert_eq!(prompt.port, 2222);
        assert_eq!(prompt.key_type, "ssh-ed25519");
        assert_eq!(prompt.hop, "target");
        assert!(prompt.can_accept());
        assert_eq!(e.into_wire(), wire);
    }

    #[test]
    fn a_jump_refusal_names_its_hop() {
        let wire = crate::ssh::trust::mark_jump_hop(refusal_payload(&Refusal::Unknown(info())));
        let CoreError::Trust(prompt) = CoreError::from(wire) else {
            panic!("not a trust prompt");
        };
        assert!(prompt.is_jump());
    }

    #[test]
    fn a_refused_key_cannot_be_accepted() {
        let wire = refusal_payload(&Refusal::Refused {
            info: info(),
            message: "changed".into(),
        });
        let CoreError::Trust(prompt) = CoreError::from(wire) else {
            panic!("not a trust prompt");
        };
        assert!(!prompt.can_accept());
        assert_eq!(prompt.message, "changed");
    }

    #[test]
    fn the_prefix_counts_only_at_index_0() {
        // The frontend's rule, for the same reason: a banner or hostname must
        // not be able to forge a dialog.
        let forged = format!("exec session: {}{{\"kind\":\"unknown\"}}", TRUST_PREFIX);
        assert_eq!(CoreError::from(forged.clone()), CoreError::Other(forged));
    }

    #[test]
    fn keyring_misses_need_a_password_and_keep_their_text() {
        for text in [
            "keyring retrieve failed: x; fallback retrieve failed: y",
            "keyring retrieve failed for Redis: gone",
            "No matching entry found in secure storage",
        ] {
            let e = CoreError::from(text);
            assert_eq!(e, CoreError::NeedsPassword(text.to_string()));
            assert_eq!(e.into_wire(), text);
        }
        assert_eq!(
            CoreError::from("auth: denied"),
            CoreError::Other("auth: denied".into())
        );
    }
}
