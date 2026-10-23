//! The real backend: `termdrop-core` over the same `termdrop.db`, keyring
//! and `known_hosts` the Tauri app uses, so both apps see one set of hosts,
//! settings and trusted keys during the side-by-side period.
//!
//! Three pieces live here:
//! - [`CoreStore`], the shell's [`HostStore`] over [`Core`] and its host
//!   operations (`termdrop_core::hosts`).
//! - [`Services`], what every session view shares: the core, the tokio
//!   runtime its async calls need, and the event bridge.
//! - [`Bridge`], the core's [`EventSink`]. It carries events to the UI thread
//!   and routes early terminal output (see [`Bridge::attach`]).

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard};

use anyhow::{Result, anyhow};
use termdrop_core::events::FnSink;
use termdrop_core::{
    Core, CoreEvent, EventSink, Events, PtyKind, crypto, db, hosts, ssh_config_parser, state,
};
use termdrop_terminal::TerminalFeed;

use crate::backend::{
    HostRecord, HostStore, HostSummary, ImportEntry, ImportHost, ImportSummary, SecretKind,
    SshConfigEntry,
};

/// The shell's hosts and settings, over the core's database and keyring.
pub struct CoreStore {
    core: Arc<Core>,
    /// For the core's async host operations: an update or delete releases
    /// pooled connections behind tokio locks, in the background.
    rt: tokio::runtime::Handle,
}

impl CoreStore {
    pub fn new(core: Arc<Core>, rt: tokio::runtime::Handle) -> Self {
        Self { core, rt }
    }

    /// Drop a saved or deleted host's pooled connections and tunnels on the
    /// runtime. The pool locks are held across a connect in flight (a tunnel
    /// handshake, a Redis connect of up to its timeout), and they are global,
    /// so waiting for them here froze the window while any tab was
    /// connecting, even when editing an unrelated host. The row is already
    /// written; a connection opened before this runs is dropped when it does.
    fn forget_in_background(&self, host_id: i64) -> tokio::task::JoinHandle<()> {
        let core = self.core.clone();
        self.rt.spawn(async move {
            if let Err(e) = core.forget_host_connections(host_id).await {
                tracing::warn!(host_id, "releasing the host's connections failed: {e}");
            }
        })
    }
}

/// `db::Host` to the shell's row. Ports and the favourite flag are INTEGER
/// columns; anything out of range reads as unset rather than wrapping.
pub fn host_record(h: &db::Host) -> HostRecord {
    HostRecord {
        id: h.id,
        name: h.name.clone(),
        host: h.host.clone(),
        port: u16::try_from(h.port).unwrap_or(0),
        username: h.username.clone(),
        auth_type: h.auth_type.clone(),
        // The column is NOT NULL with '' for "none"; the editors want None.
        key_path: h.key_path.clone().filter(|k| !k.is_empty()),
        group: h.group.clone(),
        favorite: h.favorite != 0,
        mongo_uri: h.mongo_uri.clone(),
        mongo_local_uri: h.mongo_local_uri.clone(),
        redis_uri: h.redis_uri.clone(),
        redis_tunnel_host_id: h.redis_tunnel_host_id,
        jump_host_id: h.jump_host_id,
        vnc_host: h.vnc_host.clone(),
        vnc_port: h.vnc_port.and_then(|p| u16::try_from(p).ok()),
        vnc_username: h.vnc_username.clone(),
        vnc_tunnel_host_id: h.vnc_tunnel_host_id,
        vnc_view_only: h.vnc_view_only != 0,
        last_connected_at: h.last_connected_at.clone(),
    }
}

/// The shell's row to the `db::NewHost` every write takes. `favorite` is
/// always written as a number, which is what `update_host` stores for `None`
/// anyway; `vnc_view_only` the same.
pub fn new_host(h: &HostRecord) -> db::NewHost {
    db::NewHost {
        name: h.name.clone(),
        host: h.host.clone(),
        port: i64::from(h.port),
        username: h.username.clone(),
        auth_type: h.auth_type.clone(),
        key_path: h.key_path.clone(),
        group: h.group.clone(),
        favorite: Some(i64::from(h.favorite)),
        mongo_uri: h.mongo_uri.clone(),
        mongo_local_uri: h.mongo_local_uri.clone(),
        redis_uri: h.redis_uri.clone(),
        redis_tunnel_host_id: h.redis_tunnel_host_id,
        jump_host_id: h.jump_host_id,
        vnc_host: h.vnc_host.clone(),
        vnc_port: h.vnc_port.map(i64::from),
        vnc_username: h.vnc_username.clone(),
        vnc_tunnel_host_id: h.vnc_tunnel_host_id,
        vnc_view_only: Some(i64::from(h.vnc_view_only)),
    }
}

/// An import as the `db::NewHost` the core validates and writes.
fn import_host(h: ImportHost) -> db::NewHost {
    db::NewHost {
        name: h.name,
        host: h.host,
        port: h.port,
        username: h.username,
        auth_type: h.auth_type,
        key_path: h.key_path,
        group: h.group,
        favorite: Some(h.favorite),
        mongo_uri: h.mongo_uri,
        mongo_local_uri: h.mongo_local_uri,
        redis_uri: h.redis_uri,
        redis_tunnel_host_id: None,
        jump_host_id: None,
        vnc_host: h.vnc_host,
        vnc_port: h.vnc_port,
        vnc_username: h.vnc_username,
        vnc_tunnel_host_id: None,
        vnc_view_only: Some(h.vnc_view_only),
    }
}

impl HostStore for CoreStore {
    fn hosts(&self) -> Result<Vec<HostSummary>> {
        let hosts = self.core.get_hosts().map_err(|e| anyhow!(e))?;
        Ok(hosts.iter().map(|h| host_record(h).summary()).collect())
    }

    fn host_record(&self, host_id: i64) -> Result<Option<HostRecord>> {
        let host = self.core.get_host(host_id).map_err(|e| anyhow!(e))?;
        Ok(host.as_ref().map(host_record))
    }

    fn add_host(&self, host: &HostRecord) -> Result<i64> {
        self.core.add_host(&new_host(host)).map_err(|e| anyhow!(e))
    }

    /// `Core::update_existing_host`, with the connection release moved off
    /// the UI thread (see [`CoreStore::forget_in_background`]).
    fn update_host(&self, host_id: i64, host: &HostRecord) -> Result<()> {
        let rows = self
            .core
            .write_host_row(host_id, &new_host(host))
            .map_err(|e| anyhow!(e))?;
        self.forget_in_background(host_id);
        match rows {
            0 => Err(anyhow!("host {host_id} not found")),
            _ => Ok(()),
        }
    }

    /// `Core::delete_host`, likewise.
    fn delete_host(&self, host_id: i64) -> Result<()> {
        self.core.delete_host_row(host_id).map_err(|e| anyhow!(e))?;
        hosts::delete_host_secrets(host_id);
        self.forget_in_background(host_id);
        Ok(())
    }

    fn set_favorite(&self, host_id: i64, favorite: bool) -> Result<()> {
        self.core
            .update_host_favorite(host_id, i64::from(favorite))
            .map_err(|e| anyhow!(e))
    }

    fn set_group(&self, host_id: i64, group: &str) -> Result<()> {
        self.core
            .update_host_group(host_id, group)
            .map_err(|e| anyhow!(e))
    }

    fn rename_group(&self, old: &str, new: &str) -> Result<usize> {
        self.core
            .batch_update_host_group(old, new)
            .map_err(|e| anyhow!(e))
    }

    fn clear_group(&self, group: &str) -> Result<usize> {
        self.core
            .batch_clear_host_group(group)
            .map_err(|e| anyhow!(e))
    }

    fn store_secret(&self, kind: SecretKind, host_id: i64, secret: &str) -> Result<()> {
        match kind {
            SecretKind::SshPassword => hosts::store_password(host_id, secret),
            SecretKind::Mongo => hosts::store_mongo_secret(host_id, secret),
            SecretKind::Redis => hosts::store_redis_secret(host_id, secret),
            SecretKind::Vnc => hosts::store_vnc_secret(host_id, secret),
        }
        .map_err(|e| anyhow!(e))
    }

    fn has_secret(&self, kind: SecretKind, host_id: i64) -> bool {
        match kind {
            SecretKind::SshPassword => crypto::get_password(host_id).is_ok(),
            SecretKind::Mongo => hosts::has_mongo_secret(host_id),
            SecretKind::Redis => hosts::has_redis_secret(host_id),
            SecretKind::Vnc => hosts::has_vnc_secret(host_id),
        }
    }

    fn clear_secret(&self, kind: SecretKind, host_id: i64) -> Result<()> {
        match kind {
            SecretKind::Vnc => hosts::clear_vnc_secret(host_id).map_err(|e| anyhow!(e)),
            SecretKind::SshPassword | SecretKind::Mongo | SecretKind::Redis => Ok(()),
        }
    }

    fn export_hosts(&self) -> Result<String> {
        self.core.export_hosts().map_err(|e| anyhow!(e))
    }

    fn import_hosts(&self, entries: Vec<ImportEntry>) -> Result<ImportSummary> {
        let entries = entries
            .into_iter()
            .map(|e| db::ImportEntry {
                host: import_host(e.host),
                replace_id: e.replace_id,
            })
            .collect();
        let summary = self.core.import_hosts(entries).map_err(|e| anyhow!(e))?;
        Ok(ImportSummary {
            added: summary.added,
            replaced: summary.replaced,
            failed: summary
                .failed
                .into_iter()
                .map(|f| (f.name, f.reason))
                .collect(),
        })
    }

    fn parse_ssh_config(&self) -> Result<Vec<SshConfigEntry>> {
        let hosts = ssh_config_parser::parse_ssh_config().map_err(|e| anyhow!(e))?;
        Ok(hosts
            .into_iter()
            .map(|h| SshConfigEntry {
                name: h.name,
                host: h.host,
                port: h.port,
                username: h.username,
                auth_type: h.auth_type,
                key_path: h.key_path,
                proxy_jump: h.proxy_jump,
            })
            .collect())
    }

    fn get_setting(&self, key: &str) -> Result<Option<String>> {
        self.core.get_setting(key).map_err(|e| anyhow!(e))
    }

    fn set_setting(&self, key: &str, value: &str) -> Result<()> {
        self.core.set_setting(key, value).map_err(|e| anyhow!(e))
    }
}

/// Where a terminal's output goes before and after its view is attached.
enum Slot {
    /// Output that arrived as `PtyData` before the view attached.
    Pending(Vec<u8>),
    Attached(TerminalFeed),
}

/// The core's event sink.
///
/// Events other than terminal output go into a channel the UI thread drains
/// (`EventSink::emit` runs on backend threads and must not block). Terminal
/// output that arrives as `PtyData` -- before a sink is attached, or after an
/// SSH reconnect replaced the session's channel -- is fed straight into the
/// terminal on the emitting thread instead. Sending it through the channel
/// would let it overtake, or be overtaken by, the bytes the attached sink
/// feeds from that same thread.
pub struct Bridge {
    slots: Mutex<Slots>,
    tx: async_channel::Sender<CoreEvent>,
}

/// How many detached session ids are remembered.
const CLOSED_REMEMBERED: usize = 64;

/// One kind of PTY's slots: shells and docker exec PTYs are kept apart, as
/// their ids come from different places (the core's uuids; the caller's own
/// choice for an exec PTY).
#[derive(Default)]
struct SlotTable {
    by_id: HashMap<String, Slot>,
    /// PTYs detached recently. The PTY thread keeps running until the
    /// disconnect lands, and output it sends meanwhile arrives as `PtyData`
    /// (a tab closed mid-connect never had a sink). Without this, that
    /// output would open a `Pending` slot nothing ever detaches again. Kept
    /// to a bounded window, since a session that had already ended sends
    /// nothing after its detach to say it is done.
    closed: VecDeque<String>,
}

impl SlotTable {
    /// Replay what arrived early into `feed` and route later output to it.
    fn attach(&mut self, id: &str, feed: &TerminalFeed) {
        // A reattach (reconnect) brings a detached id back to life.
        self.closed.retain(|c| c != id);
        if let Some(Slot::Pending(early)) = self.by_id.remove(id) {
            feed.feed(&early);
        }
        self.by_id
            .insert(id.to_string(), Slot::Attached(feed.clone()));
    }

    fn detach(&mut self, id: &str) {
        self.by_id.remove(id);
        if !self.closed.iter().any(|c| c == id) {
            if self.closed.len() == CLOSED_REMEMBERED {
                self.closed.pop_front();
            }
            self.closed.push_back(id.to_string());
        }
    }

    /// Output that came as an event: into the attached feed, or held.
    fn data(&mut self, id: &str, data: &[u8]) {
        if self.closed.iter().any(|c| c == id) {
            // The view is gone; this is the PTY's last words before the
            // disconnect lands.
            return;
        }
        match self
            .by_id
            .entry(id.to_string())
            .or_insert_with(|| Slot::Pending(Vec::new()))
        {
            Slot::Pending(buf) => buf.extend_from_slice(data),
            Slot::Attached(feed) => feed.feed(data),
        }
    }
}

#[derive(Default)]
struct Slots {
    shell: SlotTable,
    exec: SlotTable,
}

impl Slots {
    fn table(&mut self, kind: PtyKind) -> &mut SlotTable {
        match kind {
            PtyKind::Shell => &mut self.shell,
            PtyKind::ExecPty => &mut self.exec,
        }
    }
}

fn feed_sink(feed: TerminalFeed) -> Box<FnSink<impl FnMut(Vec<u8>) -> bool + Send + 'static>> {
    Box::new(FnSink(move |bytes: Vec<u8>| {
        feed.feed(&bytes);
        true
    }))
}

impl Bridge {
    pub fn new() -> (Arc<Self>, async_channel::Receiver<CoreEvent>) {
        let (tx, rx) = async_channel::unbounded();
        let bridge = Arc::new(Self {
            slots: Mutex::new(Slots::default()),
            tx,
        });
        (bridge, rx)
    }

    fn slots(&self) -> MutexGuard<'_, Slots> {
        // A panic while holding this lock leaves a plain map behind; nothing
        // in it can be half-updated, so keep going.
        self.slots.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Point terminal `session_id`'s output at `feed`: replay what arrived
    /// early, then attach a byte sink in the core.
    ///
    /// Both happen under the slot lock. A backend thread that found no sink
    /// and is about to emit `PtyData` waits on that lock, then finds the slot
    /// attached and feeds directly, so no chunk is lost or reordered. It does
    /// not deadlock: the core releases its channel lock before it falls back
    /// to the event.
    pub fn attach(&self, core: &Core, session_id: &str, feed: TerminalFeed) -> Result<(), String> {
        let mut slots = self.slots();
        slots.shell.attach(session_id, &feed);
        core.attach_terminal_sink(session_id, feed_sink(feed))
    }

    /// Forget a terminal: its early output and its feed.
    pub fn detach(&self, session_id: &str) {
        self.slots().shell.detach(session_id);
    }

    /// [`Self::attach`] for a docker exec PTY (`Core::exec_pty_connect`):
    /// call it once the connect has returned, with the pane's terminal feed.
    /// Output from before the call is replayed first, so none is lost.
    pub fn attach_exec_pty(
        &self,
        core: &Core,
        pty_session_id: &str,
        feed: TerminalFeed,
    ) -> Result<(), String> {
        let mut slots = self.slots();
        slots.exec.attach(pty_session_id, &feed);
        core.attach_exec_pty_sink(pty_session_id, feed_sink(feed))
    }

    /// [`Self::detach`] for a docker exec PTY. Call it with
    /// `Core::exec_pty_disconnect` when the pane closes.
    pub fn detach_exec_pty(&self, pty_session_id: &str) {
        self.slots().exec.detach(pty_session_id);
    }
}

impl EventSink for Bridge {
    fn emit(&self, event: CoreEvent) {
        // PTY output never goes through the queue: it is fed straight into
        // the terminal on this thread (see the type's comment), for shells
        // and docker exec panes alike.
        if let CoreEvent::PtyData { kind, id, data } = &event {
            self.slots().table(*kind).data(id, data.as_bytes());
            return;
        }
        // Unbounded, so this never blocks; it fails only once the UI is gone.
        let _ = self.tx.try_send(event);
    }
}

/// What the session views share.
pub struct Services {
    pub core: Arc<Core>,
    /// The core uses tokio internally (`spawn_blocking`, tokio mutexes), so
    /// its async calls run here; the UI awaits the join handles, which are
    /// plain futures and need no runtime of their own.
    pub rt: tokio::runtime::Runtime,
    pub bridge: Arc<Bridge>,
}

impl Services {
    pub fn events(&self) -> Events {
        self.bridge.clone()
    }
}

/// Open the core over the Tauri app's database, with a runtime and bridge.
/// Returns the receiving end of the bridge for the UI thread to drain.
pub fn open() -> Result<(Arc<Services>, async_channel::Receiver<CoreEvent>), String> {
    let core = Core::open(&state::default_db_path())?;
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_name("termdrop-core")
        .build()
        .map_err(|e| format!("Failed to start the async runtime: {e}"))?;
    let (bridge, rx) = Bridge::new();
    Ok((
        Arc::new(Services {
            core: Arc::new(core),
            rt,
            bridge,
        }),
        rx,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn host() -> db::Host {
        db::Host {
            id: 7,
            name: "box".into(),
            host: "10.0.0.7".into(),
            port: 2222,
            username: "me".into(),
            auth_type: "password".into(),
            key_path: None,
            group: Some("Lab".into()),
            favorite: 1,
            last_connected_at: None,
            created_at: String::new(),
            mongo_uri: None,
            mongo_local_uri: None,
            redis_uri: None,
            redis_tunnel_host_id: None,
            jump_host_id: None,
            vnc_host: None,
            vnc_port: None,
            vnc_username: None,
            vnc_tunnel_host_id: None,
            vnc_view_only: 0,
        }
    }

    #[test]
    fn an_imported_redis_or_vnc_row_passes_the_cores_checks() {
        let redis = ImportHost {
            name: "cache".into(),
            auth_type: "password".into(),
            redis_uri: Some("redis://@localhost:6380".into()),
            ..Default::default()
        };
        let vnc = ImportHost {
            name: "kiosk".into(),
            auth_type: "password".into(),
            vnc_host: Some("10.0.0.5".into()),
            vnc_port: Some(5902),
            vnc_username: Some("me".into()),
            vnc_view_only: 1,
            ..Default::default()
        };
        let redis = import_host(redis);
        assert_eq!(db::validate_new_host(&redis), Ok(()));
        assert_eq!(redis.redis_uri.as_deref(), Some("redis://@localhost:6380"));
        let vnc = import_host(vnc);
        assert_eq!(db::validate_new_host(&vnc), Ok(()));
        assert_eq!(
            (vnc.vnc_host.as_deref(), vnc.vnc_port, vnc.vnc_view_only),
            (Some("10.0.0.5"), Some(5902), Some(1))
        );
        assert_eq!(vnc.vnc_username.as_deref(), Some("me"));
    }

    /// Saving a host must not wait for the Redis pool locks, which a tab
    /// that is connecting holds across its handshake.
    #[test]
    fn saving_a_host_does_not_wait_for_a_connect_in_flight() {
        let dir = std::env::temp_dir().join(format!("termdrop-store-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let core = Arc::new(Core::open(&dir.join("termdrop.db")).unwrap());
        let rt = tokio::runtime::Runtime::new().unwrap();
        let store = Arc::new(CoreStore::new(core.clone(), rt.handle().clone()));
        let record = host_record(&host());
        let id = store.add_host(&record).unwrap();

        // A tunnel handshake in flight, as `redis_tunnel_port` holds it.
        let held = core.redis_tunnels.blocking_lock();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn({
            let store = store.clone();
            let record = HostRecord {
                name: "renamed".into(),
                ..record.clone()
            };
            move || tx.send(store.update_host(id, &record).map_err(|e| e.to_string()))
        });
        let saved = rx.recv_timeout(std::time::Duration::from_secs(5));
        drop(held);
        assert_eq!(saved.expect("saving blocked on the pool lock"), Ok(()));
        assert_eq!(store.host_record(id).unwrap().unwrap().name, "renamed");
        // A row that is gone is still an error.
        assert!(store.update_host(id + 1, &record).is_err());

        drop(rt);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_host_row_maps_onto_the_shells_record() {
        let r = host_record(&host());
        assert_eq!((r.id, r.port, r.favorite), (7, 2222, true));
        assert_eq!(r.group.as_deref(), Some("Lab"));
    }

    #[test]
    fn a_record_writes_back_as_the_row_it_came_from() {
        let row = db::Host {
            key_path: Some("~/.ssh/id".into()),
            jump_host_id: Some(3),
            vnc_host: Some("127.0.0.1".into()),
            vnc_port: Some(5901),
            vnc_view_only: 1,
            ..host()
        };
        let n = new_host(&host_record(&row));
        assert_eq!(
            (n.name.as_str(), n.port, n.auth_type.as_str()),
            ("box", 2222, "password")
        );
        assert_eq!(n.key_path.as_deref(), Some("~/.ssh/id"));
        assert_eq!((n.jump_host_id, n.vnc_port), (Some(3), Some(5901)));
        assert_eq!((n.favorite, n.vnc_view_only), (Some(1), Some(1)));
        assert_eq!(n.group.as_deref(), Some("Lab"));
        // An empty key path column reads as none.
        assert_eq!(
            host_record(&db::Host {
                key_path: Some(String::new()),
                ..host()
            })
            .key_path,
            None
        );
    }

    #[test]
    fn an_out_of_range_port_reads_as_unset_rather_than_wrapping() {
        let r = host_record(&db::Host {
            port: 70_000,
            vnc_port: Some(-1),
            ..host()
        });
        assert_eq!(r.port, 0);
        assert_eq!(r.vnc_port, None);
    }

    #[test]
    fn early_output_is_held_and_other_events_are_queued() {
        let (bridge, rx) = Bridge::new();
        bridge.emit(CoreEvent::PtyData {
            kind: PtyKind::Shell,
            id: "s".into(),
            data: "hello ".into(),
        });
        bridge.emit(CoreEvent::PtyData {
            kind: PtyKind::Shell,
            id: "s".into(),
            data: "world".into(),
        });
        bridge.emit(CoreEvent::PtyConnected {
            kind: PtyKind::Shell,
            id: "s".into(),
        });
        match bridge.slots().shell.by_id.get("s") {
            Some(Slot::Pending(buf)) => assert_eq!(buf, b"hello world"),
            _ => panic!("early output was not held"),
        }
        assert!(matches!(rx.try_recv(), Ok(CoreEvent::PtyConnected { .. })));
        assert!(
            rx.try_recv().is_err(),
            "output must not go through the queue"
        );
        bridge.detach("s");
        assert!(bridge.slots().shell.by_id.is_empty());
    }

    #[test]
    fn output_after_a_detach_does_not_open_a_new_slot() {
        let (bridge, _rx) = Bridge::new();
        bridge.detach("s");
        bridge.emit(CoreEvent::PtyData {
            kind: PtyKind::Shell,
            id: "s".into(),
            data: "late".into(),
        });
        assert!(
            bridge.slots().shell.by_id.is_empty(),
            "late output leaked a slot"
        );
        // The remembered ids stay bounded however many tabs close.
        for i in 0..1000 {
            bridge.detach(&format!("t{i}"));
        }
        assert_eq!(bridge.slots().shell.closed.len(), CLOSED_REMEMBERED);
    }

    #[test]
    fn exec_pty_output_is_held_apart_from_shells_and_never_queued() {
        let (bridge, rx) = Bridge::new();
        bridge.emit(CoreEvent::PtyData {
            kind: PtyKind::ExecPty,
            id: "s".into(),
            data: "logs".into(),
        });
        bridge.emit(CoreEvent::PtyConnected {
            kind: PtyKind::ExecPty,
            id: "s".into(),
        });
        match bridge.slots().exec.by_id.get("s") {
            Some(Slot::Pending(buf)) => assert_eq!(buf, b"logs"),
            _ => panic!("early exec output was not held"),
        }
        assert!(
            bridge.slots().shell.by_id.is_empty(),
            "a shell id was opened"
        );
        assert!(matches!(
            rx.try_recv(),
            Ok(CoreEvent::PtyConnected {
                kind: PtyKind::ExecPty,
                ..
            })
        ));
        assert!(rx.try_recv().is_err());
        bridge.detach_exec_pty("s");
        bridge.emit(CoreEvent::PtyData {
            kind: PtyKind::ExecPty,
            id: "s".into(),
            data: "late".into(),
        });
        assert!(bridge.slots().exec.by_id.is_empty());
    }
}
