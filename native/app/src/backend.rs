//! What the shell needs from the rest of TermDrop, as two small seams.
//!
//! The shell is built in parallel with `termdrop-core` and
//! `termdrop-terminal` (docs/gpui-migration-plan.md, "Parallel execution"),
//! so it depends on neither. It codes against [`HostStore`] for hosts and
//! settings, and against [`TabContentFactory`] for what fills a tab. The
//! integration step implements the first over `termdrop-core` and the second
//! with a `TerminalView`; until then [`FakeBackend`] and
//! [`PlaceholderContent`] stand in.

use std::collections::HashMap;
use std::sync::Mutex;

use anyhow::{Result, anyhow};
use gpui_kit::{AnyView, App, Entity, Window};

use crate::app::AppModel;

use crate::app::display::{mongo_display_uri, redis_display_uri, vnc_display_target};
use crate::app::host_kind::{HostKind, KindFields, has_vnc};
use crate::app::tabs::Tab;

/// One row of the host list, already safe to display.
///
/// It deliberately carries no URI: `address` is the display form with any
/// credentials masked, so nothing the sidebar renders or searches can contain
/// a stored password.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostSummary {
    pub id: i64,
    pub name: String,
    /// Empty for ungrouped, as an empty or NULL `group` column is.
    pub group: String,
    pub kind: HostKind,
    /// A capability, not a kind: true of a VNC-only row and of an SSH host
    /// that also has a screen.
    pub has_vnc: bool,
    pub favorite: bool,
    /// The SSH host. Empty for a datastore or VNC-only row.
    pub host: String,
    pub port: u16,
    pub username: String,
    /// What the row's subtitle shows and the search matches for a row whose
    /// `host` is empty: the masked datastore URI or the VNC `host:port`.
    pub address: String,
    /// `hosts.last_connected_at` as SQLite wrote it, for the row's age label.
    pub last_connected_at: Option<String>,
}

/// The raw columns of a `hosts` row that a summary is computed from. The core
/// adapter fills this from its own row type and calls [`HostRecord::summary`],
/// so the kind and masking rules live in one place.
#[derive(Debug, Clone, Default)]
pub struct HostRecord {
    pub id: i64,
    pub name: String,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub group: Option<String>,
    pub favorite: bool,
    pub mongo_uri: Option<String>,
    pub redis_uri: Option<String>,
    pub vnc_host: Option<String>,
    pub vnc_port: Option<u16>,
    pub last_connected_at: Option<String>,
}

impl HostRecord {
    pub fn summary(&self) -> HostSummary {
        let kind = HostKind::classify(KindFields {
            host: Some(&self.host),
            redis_uri: self.redis_uri.as_deref(),
            mongo_uri: self.mongo_uri.as_deref(),
            vnc_host: self.vnc_host.as_deref(),
        });
        // Every display form a row can be found by, joined: the Vue search
        // matched all three, so a Redis URI on an SSH host is findable too.
        let address = [
            self.mongo_uri.as_deref().map(mongo_display_uri),
            self.redis_uri.as_deref().map(redis_display_uri),
            Some(vnc_display_target(self.vnc_host.as_deref(), self.vnc_port)),
        ]
        .into_iter()
        .flatten()
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
        HostSummary {
            id: self.id,
            name: self.name.clone(),
            group: self.group.clone().unwrap_or_default(),
            kind,
            has_vnc: has_vnc(self.vnc_host.as_deref()),
            favorite: self.favorite,
            host: self.host.clone(),
            port: self.port,
            username: self.username.clone(),
            address,
            last_connected_at: self.last_connected_at.clone(),
        }
    }
}

/// Hosts and the `settings` table.
///
/// Settings are the same keys and value encodings the Tauri app writes
/// (`src/composables/usePrefs.js`, `stores/connection.js`), so both apps read
/// one database during the side-by-side period.
pub trait HostStore: Send + Sync {
    fn hosts(&self) -> Result<Vec<HostSummary>>;
    fn set_favorite(&self, host_id: i64, favorite: bool) -> Result<()>;
    /// `None` when the key was never written.
    fn get_setting(&self, key: &str) -> Result<Option<String>>;
    fn set_setting(&self, key: &str, value: &str) -> Result<()>;
}

/// What fills a tab's content area.
///
/// The shell owns the tab strip and the tab's lifetime; the factory owns what
/// is drawn inside. It is called once per tab, when the tab opens, and the
/// view it returns is kept until the tab closes, so a terminal's state
/// survives switching tabs. The integration step supplies one that returns a
/// `TerminalView` for terminal tabs.
pub trait TabContentFactory: 'static {
    /// Called once, after the model exists and before the window opens, so
    /// a factory with sessions can keep the model and start background work
    /// (detecting local shells, routing backend events).
    fn start(&self, _model: &Entity<AppModel>, _cx: &mut App) {}

    fn build(&self, tab: &Tab, window: &mut Window, cx: &mut App) -> AnyView;

    /// The tab left the strip, after the user confirmed. Where a session is
    /// torn down: the view is dropped right after, and a dropped view cannot
    /// be relied on to say goodbye.
    fn tab_closed(&self, _tab: &Tab, _cx: &mut App) {}
}

/// The default factory: a labelled placeholder per tab kind.
pub struct PlaceholderContent;

impl TabContentFactory for PlaceholderContent {
    fn build(&self, tab: &Tab, _window: &mut Window, cx: &mut App) -> AnyView {
        crate::shell::placeholder::TabPlaceholder::view(tab, cx)
    }
}

/// An in-memory store with fictional hosts, mirroring
/// `scripts/screenshots/demo-backend.js`. Every address is from a
/// documentation range or a private network; nothing reads the real machine.
pub struct FakeBackend {
    hosts: Mutex<Vec<HostRecord>>,
    settings: Mutex<HashMap<String, String>>,
}

impl Default for FakeBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl FakeBackend {
    pub fn new() -> Self {
        Self::with_hosts(demo_hosts())
    }

    pub fn with_hosts(hosts: Vec<HostRecord>) -> Self {
        Self {
            hosts: Mutex::new(hosts),
            settings: Mutex::new(HashMap::new()),
        }
    }
}

impl HostStore for FakeBackend {
    fn hosts(&self) -> Result<Vec<HostSummary>> {
        let hosts = self
            .hosts
            .lock()
            .map_err(|_| anyhow!("host list poisoned"))?;
        Ok(hosts.iter().map(HostRecord::summary).collect())
    }

    fn set_favorite(&self, host_id: i64, favorite: bool) -> Result<()> {
        let mut hosts = self
            .hosts
            .lock()
            .map_err(|_| anyhow!("host list poisoned"))?;
        let host = hosts
            .iter_mut()
            .find(|h| h.id == host_id)
            .ok_or_else(|| anyhow!("no host with id {host_id}"))?;
        host.favorite = favorite;
        Ok(())
    }

    fn get_setting(&self, key: &str) -> Result<Option<String>> {
        let settings = self
            .settings
            .lock()
            .map_err(|_| anyhow!("settings poisoned"))?;
        Ok(settings.get(key).cloned())
    }

    fn set_setting(&self, key: &str, value: &str) -> Result<()> {
        let mut settings = self
            .settings
            .lock()
            .map_err(|_| anyhow!("settings poisoned"))?;
        settings.insert(key.to_string(), value.to_string());
        Ok(())
    }
}

fn demo_hosts() -> Vec<HostRecord> {
    let mut next = 0;
    let mut host = |name: &str, addr: &str, group: &str| {
        next += 1;
        HostRecord {
            id: next,
            name: name.into(),
            host: addr.into(),
            port: 22,
            username: "deploy".into(),
            group: Some(group.into()),
            ..Default::default()
        }
    };
    let mut hosts = vec![
        HostRecord {
            favorite: true,
            last_connected_at: Some("2026-10-22 08:00:00".into()),
            ..host("api-prod-01", "10.20.0.11", "Production")
        },
        host("api-prod-02", "10.20.0.12", "Production"),
        host("worker-prod", "10.20.0.21", "Production"),
        HostRecord {
            favorite: true,
            last_connected_at: Some("2026-10-21 09:00:00".into()),
            ..host("bastion", "bastion.example.com", "Production")
        },
        host("staging", "staging.example.com", "Staging"),
        HostRecord {
            username: "ci".into(),
            ..host("ci-runner", "203.0.113.40", "Staging")
        },
        HostRecord {
            port: 0,
            username: String::new(),
            mongo_uri: Some("mongodb+srv://cluster0.example.net".into()),
            ..host("Orders DB", "", "Data")
        },
        HostRecord {
            port: 0,
            username: String::new(),
            redis_uri: Some("redis://cache.internal:6379".into()),
            ..host("Session cache", "", "Data")
        },
        HostRecord {
            port: 0,
            username: String::new(),
            vnc_host: Some("192.168.1.20".into()),
            vnc_port: Some(5900),
            ..host("Design iMac", "", "Office")
        },
        HostRecord {
            username: "pi".into(),
            vnc_host: Some("127.0.0.1".into()),
            vnc_port: Some(5901),
            ..host("homelab", "192.168.1.10", "Office")
        },
    ];
    hosts.push(HostRecord {
        group: None,
        ..host("scratch-vm", "192.168.56.10", "")
    });
    hosts
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fake_backend_round_trips_settings() {
        let b = FakeBackend::new();
        assert_eq!(b.get_setting("sidebar-width").unwrap(), None);
        b.set_setting("sidebar-width", "240").unwrap();
        assert_eq!(
            b.get_setting("sidebar-width").unwrap().as_deref(),
            Some("240")
        );
    }

    #[test]
    fn fake_backend_has_every_kind() {
        let hosts = FakeBackend::new().hosts().unwrap();
        for kind in [
            HostKind::Ssh,
            HostKind::Mongodb,
            HostKind::Redis,
            HostKind::Vnc,
        ] {
            assert!(hosts.iter().any(|h| h.kind == kind), "{kind:?} missing");
        }
        // An SSH host with a screen stays SSH but reports the capability.
        let homelab = hosts.iter().find(|h| h.name == "homelab").unwrap();
        assert_eq!(homelab.kind, HostKind::Ssh);
        assert!(homelab.has_vnc);
    }

    #[test]
    fn summaries_never_carry_a_password() {
        let rec = HostRecord {
            name: "x".into(),
            redis_uri: Some("redis://:hunter2@h:6379".into()),
            mongo_uri: Some("mongodb://u:hunter3@h:27017".into()),
            ..Default::default()
        };
        let s = rec.summary();
        assert!(!format!("{s:?}").contains("hunter2"));
        assert!(!format!("{s:?}").contains("hunter3"));
    }

    #[test]
    fn set_favorite_updates_the_row() {
        let b = FakeBackend::new();
        let id = b.hosts().unwrap()[1].id;
        b.set_favorite(id, true).unwrap();
        assert!(b.hosts().unwrap()[1].favorite);
        assert!(b.set_favorite(9999, true).is_err());
    }
}
