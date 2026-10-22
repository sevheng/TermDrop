//! What a host row represents. Port of `src/utils/hostKind.js`.
//!
//! There is no type column: a row with a datastore URI and no SSH host *is*
//! that datastore. The Vue app wrote the predicate inline in four places until
//! a third kind arrived; here it is one enum, so a new kind is a compile error
//! at every `match` rather than a silently wrong `!=`.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum HostKind {
    #[default]
    Ssh,
    Mongodb,
    Redis,
    Vnc,
    /// A shell on this machine. Never a saved row: it exists so a summary can
    /// describe the target of a local tab with the same type as everything
    /// else, and so `kind: local` from the task's `HostSummary` has a home.
    Local,
}

/// The raw columns that decide a kind, borrowed from whatever row shape the
/// caller has. `None` and `Some("")` mean the same thing, as falsy values did
/// in JS.
#[derive(Debug, Clone, Copy, Default)]
pub struct KindFields<'a> {
    pub host: Option<&'a str>,
    pub redis_uri: Option<&'a str>,
    pub mongo_uri: Option<&'a str>,
    pub vnc_host: Option<&'a str>,
}

fn present(v: Option<&str>) -> bool {
    v.is_some_and(|s| !s.is_empty())
}

/// The colour a host's icon takes. A token name, not a colour, so the theme
/// decides what it looks like.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IconTone {
    Ink3,
    Redis,
    Mongo,
    Vnc,
}

impl HostKind {
    /// A row's kind. Redis is checked first so a row that somehow carries both
    /// URIs resolves the same way every time rather than depending on order.
    /// A row with an SSH host is SSH even when it carries a URI: the URI
    /// belongs to the machine, not instead of it.
    pub fn classify(fields: KindFields<'_>) -> Self {
        if !present(fields.host) {
            if present(fields.redis_uri) {
                return Self::Redis;
            }
            if present(fields.mongo_uri) {
                return Self::Mongodb;
            }
            if present(fields.vnc_host) {
                return Self::Vnc;
            }
        }
        Self::Ssh
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ssh => "ssh",
            Self::Mongodb => "mongodb",
            Self::Redis => "redis",
            Self::Vnc => "vnc",
            Self::Local => "local",
        }
    }

    /// A datastore (or screen) host opens a panel tab rather than a shell.
    pub fn is_datastore(self) -> bool {
        matches!(self, Self::Mongodb | Self::Redis | Self::Vnc)
    }

    /// The verb for the context menu and the double-click action.
    pub fn activate_verb(self) -> &'static str {
        if self.is_datastore() {
            "Open"
        } else {
            "Connect"
        }
    }

    /// The row drew every kind in ink-3 until a Redis host and an SSH host
    /// looked identical. An SSH host with a screen keeps the SSH look.
    pub fn icon_tone(self) -> IconTone {
        match self {
            Self::Redis => IconTone::Redis,
            Self::Mongodb => IconTone::Mongo,
            Self::Vnc => IconTone::Vnc,
            Self::Ssh | Self::Local => IconTone::Ink3,
        }
    }
}

/// Whether a row can open a VNC tab at all.
///
/// Deliberately *not* a kind, and true of two of them. A screen is a
/// capability a machine has, not a thing it is: an SSH host with a VNC server
/// on it is still an SSH host, and double-clicking it still opens a shell.
pub fn has_vnc(vnc_host: Option<&str>) -> bool {
    present(vnc_host)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ssh() -> KindFields<'static> {
        KindFields {
            host: Some("10.0.0.1"),
            ..Default::default()
        }
    }
    fn mongo() -> KindFields<'static> {
        KindFields {
            host: Some(""),
            mongo_uri: Some("mongodb://h/db"),
            ..Default::default()
        }
    }
    fn redis() -> KindFields<'static> {
        KindFields {
            host: Some(""),
            redis_uri: Some("redis://h:6379/0"),
            ..Default::default()
        }
    }
    fn vnc() -> KindFields<'static> {
        KindFields {
            host: Some(""),
            vnc_host: Some("10.0.0.5"),
            ..Default::default()
        }
    }
    fn ssh_with_screen() -> KindFields<'static> {
        KindFields {
            host: Some("10.0.0.1"),
            vnc_host: Some("127.0.0.1"),
            ..Default::default()
        }
    }

    #[test]
    fn names_each_kind() {
        assert_eq!(HostKind::classify(ssh()), HostKind::Ssh);
        assert_eq!(HostKind::classify(mongo()), HostKind::Mongodb);
        assert_eq!(HostKind::classify(redis()), HostKind::Redis);
        assert_eq!(HostKind::classify(vnc()), HostKind::Vnc);
    }

    #[test]
    fn leaves_an_ssh_host_that_has_a_screen_as_an_ssh_host() {
        // The consequential case: double-clicking this row must still open a
        // shell, and the VNC tab is reached from the context menu instead.
        let kind = HostKind::classify(ssh_with_screen());
        assert_eq!(kind, HostKind::Ssh);
        assert!(!kind.is_datastore());
        assert_eq!(kind.activate_verb(), "Connect");
    }

    #[test]
    fn reports_vnc_capability_separately_from_kind() {
        // True of two different kinds, which is exactly why it is not one.
        assert!(has_vnc(vnc().vnc_host));
        assert!(has_vnc(ssh_with_screen().vnc_host));
        assert!(!has_vnc(ssh().vnc_host));
        assert!(!has_vnc(redis().vnc_host));
        assert!(!has_vnc(None));
    }

    #[test]
    fn gives_a_vnc_only_row_its_own_icon_colour() {
        assert_eq!(HostKind::classify(vnc()).icon_tone(), IconTone::Vnc);
        // ...but an SSH host with a screen keeps the SSH look.
        assert_eq!(
            HostKind::classify(ssh_with_screen()).icon_tone(),
            IconTone::Ink3
        );
    }

    #[test]
    fn treats_a_row_with_an_ssh_host_as_ssh_even_when_it_carries_a_uri() {
        let with_redis = KindFields {
            host: Some("10.0.0.1"),
            redis_uri: Some("redis://h:6379"),
            ..Default::default()
        };
        let with_mongo = KindFields {
            host: Some("10.0.0.1"),
            mongo_uri: Some("mongodb://h"),
            ..Default::default()
        };
        assert_eq!(HostKind::classify(with_redis), HostKind::Ssh);
        assert_eq!(HostKind::classify(with_mongo), HostKind::Ssh);
    }

    #[test]
    fn resolves_a_row_carrying_both_uris_deterministically() {
        let both = KindFields {
            host: Some(""),
            mongo_uri: Some("mongodb://h"),
            redis_uri: Some("redis://h:6379"),
            ..Default::default()
        };
        assert_eq!(HostKind::classify(both), HostKind::Redis);
    }

    #[test]
    fn does_not_throw_on_a_missing_host() {
        assert_eq!(HostKind::classify(KindFields::default()), HostKind::Ssh);
    }

    #[test]
    fn predicates_are_mutually_exclusive() {
        let m = HostKind::classify(mongo());
        let r = HostKind::classify(redis());
        assert!(m == HostKind::Mongodb && r != HostKind::Mongodb);
        assert!(r == HostKind::Redis && m != HostKind::Redis);
        assert!(!HostKind::classify(ssh()).is_datastore());
        assert!(r.is_datastore());
    }

    #[test]
    fn says_connect_for_a_machine_and_open_for_a_datastore() {
        assert_eq!(HostKind::classify(ssh()).activate_verb(), "Connect");
        assert_eq!(HostKind::classify(redis()).activate_verb(), "Open");
        assert_eq!(HostKind::classify(mongo()).activate_verb(), "Open");
    }
}
