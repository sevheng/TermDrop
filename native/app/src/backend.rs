//! What the shell needs from the rest of TermDrop, as two small seams.
//!
//! The shell is built in parallel with `termdrop-core` and
//! `termdrop-terminal` (docs/gpui-migration-plan.md, "Parallel execution"),
//! so it depends on neither. It codes against [`HostStore`] for hosts and
//! settings, and against [`TabContentFactory`] for what fills a tab. The
//! integration step implements the first over `termdrop-core` and the second
//! with a `TerminalView`; until then [`FakeBackend`] and
//! [`PlaceholderContent`] stand in.

use std::collections::{HashMap, HashSet};
use std::sync::{Mutex, MutexGuard};

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
    /// The bastion this host connects through, for the jump-host picker,
    /// which offers only hosts without one of their own.
    pub jump_host_id: Option<i64>,
    /// `hosts.last_connected_at` as SQLite wrote it, for the row's age label.
    pub last_connected_at: Option<String>,
}

/// Every column of a `hosts` row the app reads or writes. The core adapter
/// fills this from its own row type and calls [`HostRecord::summary`], so the
/// kind and masking rules live in one place; the host editors read it to fill
/// their fields and write it back whole, because an update rewrites every
/// column and anything left out would be reset.
///
/// URIs here never carry a password the app put there: passwords live in the
/// keyring. A row written by hand or imported from an old export may still
/// have one, which is why nothing renders these fields directly.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HostRecord {
    pub id: i64,
    pub name: String,
    pub host: String,
    pub port: u16,
    pub username: String,
    /// `"password"` or `"key"`.
    pub auth_type: String,
    pub key_path: Option<String>,
    pub group: Option<String>,
    pub favorite: bool,
    pub mongo_uri: Option<String>,
    /// Legacy: nothing reads it, and the launch migration moves any value to
    /// its own host. Carried only so an import of an old export is lossless.
    pub mongo_local_uri: Option<String>,
    pub redis_uri: Option<String>,
    pub redis_tunnel_host_id: Option<i64>,
    pub jump_host_id: Option<i64>,
    pub vnc_host: Option<String>,
    pub vnc_port: Option<u16>,
    pub vnc_username: Option<String>,
    pub vnc_tunnel_host_id: Option<i64>,
    pub vnc_view_only: bool,
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
            jump_host_id: self.jump_host_id,
            last_connected_at: self.last_connected_at.clone(),
        }
    }
}

/// Which keyring entry a secret goes to. Each kind has its own account, so
/// an SSH password and a Redis password on hosts with the same id never meet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SecretKind {
    /// An SSH host's login password.
    SshPassword,
    Mongo,
    Redis,
    Vnc,
}

/// One host to import, shaped as `db::NewHost`. Its own type rather than a
/// [`HostRecord`] because an import is not yet validated: a port of 70000 or
/// 0 must reach the backend's check and be reported by name, not wrap.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ImportHost {
    pub name: String,
    pub host: String,
    pub port: i64,
    pub username: String,
    pub auth_type: String,
    pub key_path: Option<String>,
    pub group: Option<String>,
    pub favorite: i64,
    pub mongo_uri: Option<String>,
    pub mongo_local_uri: Option<String>,
}

/// One host to import, as `db::ImportEntry`: `replace_id` names an existing
/// row to update in place rather than inserting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportEntry {
    pub host: ImportHost,
    pub replace_id: Option<i64>,
}

/// What an import did, as `db::ImportSummary`. `failed` is `(name, reason)`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ImportSummary {
    pub added: usize,
    pub replaced: usize,
    pub failed: Vec<(String, String)>,
}

/// One `Host` block from `~/.ssh/config`, as `ssh_config_parser::SshConfigHost`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SshConfigEntry {
    pub name: String,
    pub host: String,
    /// As the parser read it, unclamped: a `Port 70000` must reach the
    /// backend's check and be refused by name, not saved on another port.
    pub port: i64,
    pub username: String,
    pub auth_type: String,
    pub key_path: Option<String>,
    /// The first hop of `ProxyJump`, as written.
    pub proxy_jump: Option<String>,
}

/// Hosts and the `settings` table.
///
/// Settings are the same keys and value encodings the Tauri app writes
/// (`src/composables/usePrefs.js`, `stores/connection.js`), so both apps read
/// one database during the side-by-side period. Host writes go through the
/// same core operations as the Tauri commands, so a delete drops the same
/// keyring entries and an edit releases the same cached connections.
pub trait HostStore: Send + Sync {
    fn hosts(&self) -> Result<Vec<HostSummary>>;
    /// Every column of one row, for an editor. `None` when it is gone.
    fn host_record(&self, host_id: i64) -> Result<Option<HostRecord>>;
    /// Insert a row; `host.id` is ignored. Returns the new id.
    fn add_host(&self, host: &HostRecord) -> Result<i64>;
    /// Rewrite every column of row `host_id`. A row that is gone (deleted by
    /// the other app meanwhile) is an error, never a silent success.
    fn update_host(&self, host_id: i64, host: &HostRecord) -> Result<()>;
    /// Delete a row and every secret stored under its id.
    fn delete_host(&self, host_id: i64) -> Result<()>;
    fn set_favorite(&self, host_id: i64, favorite: bool) -> Result<()>;
    fn set_group(&self, host_id: i64, group: &str) -> Result<()>;
    /// Move every host in `old` to `new`. Returns how many moved.
    fn rename_group(&self, old: &str, new: &str) -> Result<usize>;
    /// Move every host in `group` to Ungrouped. Returns how many moved.
    fn clear_group(&self, group: &str) -> Result<usize>;
    /// Store a secret. Never called with an empty one: an empty field means
    /// "keep what is stored".
    fn store_secret(&self, kind: SecretKind, host_id: i64, secret: &str) -> Result<()>;
    /// Whether a secret is stored, so an editor can say so without showing it.
    fn has_secret(&self, kind: SecretKind, host_id: i64) -> bool;
    /// Every host as the JSON array an export file holds.
    fn export_hosts(&self) -> Result<String>;
    fn import_hosts(&self, entries: Vec<ImportEntry>) -> Result<ImportSummary>;
    /// The hosts in `~/.ssh/config`.
    fn parse_ssh_config(&self) -> Result<Vec<SshConfigEntry>>;
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
/// documentation range or a private network; nothing reads the real machine,
/// the database or the keyring. Secrets are remembered only as "one is
/// stored", never their value.
pub struct FakeBackend {
    hosts: Mutex<Vec<HostRecord>>,
    settings: Mutex<HashMap<String, String>>,
    secrets: Mutex<HashSet<(SecretKind, i64)>>,
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
            secrets: Mutex::new(HashSet::new()),
        }
    }

    fn rows(&self) -> Result<MutexGuard<'_, Vec<HostRecord>>> {
        self.hosts.lock().map_err(|_| anyhow!("host list poisoned"))
    }

    fn row<T>(&self, host_id: i64, f: impl FnOnce(&mut HostRecord) -> T) -> Result<T> {
        let mut hosts = self.rows()?;
        let host = hosts
            .iter_mut()
            .find(|h| h.id == host_id)
            .ok_or_else(|| anyhow!("no host with id {host_id}"))?;
        Ok(f(host))
    }

    fn insert(hosts: &mut Vec<HostRecord>, host: &HostRecord) -> i64 {
        let id = hosts.iter().map(|h| h.id).max().unwrap_or(0) + 1;
        hosts.push(HostRecord {
            id,
            last_connected_at: None,
            ..host.clone()
        });
        id
    }
}

/// Columns an update writes; `id` and `last_connected_at` are the row's own.
fn overwrite(row: &mut HostRecord, host: &HostRecord) {
    *row = HostRecord {
        id: row.id,
        last_connected_at: row.last_connected_at.take(),
        ..host.clone()
    };
}

impl HostStore for FakeBackend {
    fn hosts(&self) -> Result<Vec<HostSummary>> {
        Ok(self.rows()?.iter().map(HostRecord::summary).collect())
    }

    fn host_record(&self, host_id: i64) -> Result<Option<HostRecord>> {
        Ok(self.rows()?.iter().find(|h| h.id == host_id).cloned())
    }

    fn add_host(&self, host: &HostRecord) -> Result<i64> {
        Ok(Self::insert(&mut *self.rows()?, host))
    }

    fn update_host(&self, host_id: i64, host: &HostRecord) -> Result<()> {
        // As `Core::update_existing_host`: a row that is gone is an error,
        // so an editor does not go on to store secrets under a dead id.
        let mut hosts = self.rows()?;
        let row = hosts
            .iter_mut()
            .find(|h| h.id == host_id)
            .ok_or_else(|| anyhow!("host {host_id} not found"))?;
        overwrite(row, host);
        Ok(())
    }

    fn delete_host(&self, host_id: i64) -> Result<()> {
        self.rows()?.retain(|h| h.id != host_id);
        if let Ok(mut secrets) = self.secrets.lock() {
            secrets.retain(|(_, id)| *id != host_id);
        }
        Ok(())
    }

    fn set_favorite(&self, host_id: i64, favorite: bool) -> Result<()> {
        self.row(host_id, |h| h.favorite = favorite)
    }

    fn set_group(&self, host_id: i64, group: &str) -> Result<()> {
        // `db::update_host_group` updates nothing for a missing row.
        let mut hosts = self.rows()?;
        if let Some(h) = hosts.iter_mut().find(|h| h.id == host_id) {
            h.group = Some(group.to_string());
        }
        Ok(())
    }

    fn rename_group(&self, old: &str, new: &str) -> Result<usize> {
        let mut moved = 0;
        for h in self.rows()?.iter_mut() {
            if h.group.as_deref() == Some(old) {
                h.group = Some(new.to_string());
                moved += 1;
            }
        }
        Ok(moved)
    }

    fn clear_group(&self, group: &str) -> Result<usize> {
        self.rename_group(group, "")
    }

    fn store_secret(&self, kind: SecretKind, host_id: i64, secret: &str) -> Result<()> {
        if secret.is_empty() {
            return Err(anyhow!("refusing to store an empty password"));
        }
        self.secrets
            .lock()
            .map_err(|_| anyhow!("secrets poisoned"))?
            .insert((kind, host_id));
        Ok(())
    }

    fn has_secret(&self, kind: SecretKind, host_id: i64) -> bool {
        self.secrets
            .lock()
            .is_ok_and(|s| s.contains(&(kind, host_id)))
    }

    fn export_hosts(&self) -> Result<String> {
        let rows: Vec<serde_json::Value> = self.rows()?.iter().map(export_json).collect();
        Ok(serde_json::to_string(&rows)?)
    }

    fn import_hosts(&self, entries: Vec<ImportEntry>) -> Result<ImportSummary> {
        let mut summary = ImportSummary::default();
        let mut hosts = self.rows()?;
        for entry in entries {
            if let Err(reason) = validate_import(&entry.host) {
                summary.failed.push((entry.host.name.clone(), reason));
                continue;
            }
            let record = import_record(&entry.host);
            let replaced = match entry.replace_id {
                Some(id) => match hosts.iter_mut().find(|h| h.id == id) {
                    Some(row) => {
                        overwrite(row, &record);
                        true
                    }
                    None => false,
                },
                None => false,
            };
            if replaced {
                summary.replaced += 1;
            } else {
                Self::insert(&mut hosts, &record);
                summary.added += 1;
            }
        }
        Ok(summary)
    }

    fn parse_ssh_config(&self) -> Result<Vec<SshConfigEntry>> {
        // Fictional, like everything else here: the demo must never read the
        // real ~/.ssh/config. "bastion" is already saved, to show the skip.
        let entry = |name: &str, host: &str, user: &str, jump: Option<&str>| SshConfigEntry {
            name: name.into(),
            host: host.into(),
            port: 22,
            username: user.into(),
            auth_type: "key".into(),
            key_path: Some("~/.ssh/id_ed25519".into()),
            proxy_jump: jump.map(String::from),
        };
        Ok(vec![
            entry("bastion", "bastion.example.com", "deploy", None),
            entry("build-01", "198.51.100.21", "ci", None),
            entry("db-private", "10.20.0.31", "deploy", Some("bastion")),
        ])
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

/// `db::validate_new_host`, for the in-memory store. The real store runs the
/// core's own copy.
fn validate_import(host: &ImportHost) -> std::result::Result<(), String> {
    let waives_ssh = host.mongo_uri.is_some();
    if host.name.trim().is_empty() {
        return Err("Name is required".to_string());
    }
    if !(1..=65535).contains(&host.port) && !waives_ssh {
        return Err(format!(
            "Port must be between 1 and 65535, got {}",
            host.port
        ));
    }
    if host.auth_type != "password" && host.auth_type != "key" {
        return Err(format!(
            "Auth type must be \"password\" or \"key\", got \"{}\"",
            host.auth_type
        ));
    }
    if host.host.trim().is_empty() && !waives_ssh {
        return Err("Host is required".to_string());
    }
    Ok(())
}

/// A validated import as a row. A datastore row's port of 0 stays 0.
fn import_record(h: &ImportHost) -> HostRecord {
    HostRecord {
        name: h.name.clone(),
        host: h.host.clone(),
        port: u16::try_from(h.port).unwrap_or(0),
        username: h.username.clone(),
        auth_type: h.auth_type.clone(),
        key_path: h.key_path.clone().filter(|k| !k.is_empty()),
        group: h.group.clone(),
        favorite: h.favorite != 0,
        mongo_uri: h.mongo_uri.clone(),
        mongo_local_uri: h.mongo_local_uri.clone(),
        ..Default::default()
    }
}

/// One row as `db::ExportHost` serialises it.
fn export_json(h: &HostRecord) -> serde_json::Value {
    serde_json::json!({
        "name": h.name,
        "host": h.host,
        "port": h.port,
        "username": h.username,
        "auth_type": h.auth_type,
        "key_path": h.key_path,
        "group": h.group,
        "favorite": i64::from(h.favorite),
        "mongo_uri": h.mongo_uri,
        "mongo_local_uri": h.mongo_local_uri,
        "redis_uri": h.redis_uri,
        "vnc_host": h.vnc_host,
        "vnc_port": h.vnc_port,
        "vnc_username": h.vnc_username,
        "vnc_view_only": i64::from(h.vnc_view_only),
    })
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
            auth_type: "password".into(),
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
        HostRecord {
            jump_host_id: Some(4),
            ..host("worker-prod", "10.20.0.21", "Production")
        },
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
    fn the_fake_store_adds_updates_and_deletes_rows() {
        let b = FakeBackend::new();
        let before = b.hosts().unwrap().len();
        let id = b
            .add_host(&HostRecord {
                id: 999,
                name: "new".into(),
                host: "192.0.2.1".into(),
                port: 22,
                auth_type: "password".into(),
                last_connected_at: Some("ignored".into()),
                ..Default::default()
            })
            .unwrap();
        assert_ne!(id, 999, "the id is the store's to give");
        let rec = b.host_record(id).unwrap().unwrap();
        assert_eq!(rec.last_connected_at, None);

        b.update_host(
            id,
            &HostRecord {
                name: "renamed".into(),
                ..rec.clone()
            },
        )
        .unwrap();
        assert_eq!(b.host_record(id).unwrap().unwrap().name, "renamed");

        b.store_secret(SecretKind::Redis, id, "pw").unwrap();
        assert!(b.has_secret(SecretKind::Redis, id));
        assert!(!b.has_secret(SecretKind::Mongo, id));
        assert!(b.store_secret(SecretKind::Redis, id, "").is_err());

        b.delete_host(id).unwrap();
        assert_eq!(b.hosts().unwrap().len(), before);
        assert!(
            b.update_host(id, &rec).is_err(),
            "saving over a deleted row must fail, not store secrets under it"
        );
        assert!(
            !b.has_secret(SecretKind::Redis, id),
            "a delete drops its secrets"
        );
    }

    #[test]
    fn the_fake_store_renames_and_clears_groups() {
        let b = FakeBackend::new();
        assert_eq!(b.rename_group("Staging", "QA").unwrap(), 2);
        assert_eq!(b.clear_group("QA").unwrap(), 2);
        let hosts = b.hosts().unwrap();
        assert!(
            hosts
                .iter()
                .all(|h| h.group != "Staging" && h.group != "QA")
        );
        let id = hosts[0].id;
        b.set_group(id, "Lab").unwrap();
        assert_eq!(
            b.host_record(id).unwrap().unwrap().group.as_deref(),
            Some("Lab")
        );
    }

    #[test]
    fn the_fake_import_validates_like_the_core() {
        let b = FakeBackend::new();
        let host = |name: &str, port: i64| ImportHost {
            name: name.into(),
            host: "192.0.2.5".into(),
            port,
            auth_type: "password".into(),
            ..Default::default()
        };
        let summary = b
            .import_hosts(vec![
                ImportEntry {
                    host: host("ok", 22),
                    replace_id: None,
                },
                ImportEntry {
                    host: host("bad", 70000),
                    replace_id: None,
                },
                ImportEntry {
                    host: host("replaced", 2222),
                    replace_id: Some(1),
                },
            ])
            .unwrap();
        assert_eq!((summary.added, summary.replaced), (1, 1));
        assert_eq!(
            summary.failed,
            [(
                "bad".to_string(),
                "Port must be between 1 and 65535, got 70000".to_string()
            )]
        );
        assert_eq!(b.host_record(1).unwrap().unwrap().name, "replaced");
        // The export is the same shape the core writes, and carries no secret.
        let json: serde_json::Value = serde_json::from_str(&b.export_hosts().unwrap()).unwrap();
        assert!(json.as_array().unwrap()[0].get("auth_type").is_some());
        assert!(json.as_array().unwrap()[0].get("id").is_none());
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
