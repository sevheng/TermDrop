//! The real backend: `termdrop-core` over the same `termdrop.db`, keyring
//! and `known_hosts` the Tauri app uses, so both apps see one set of hosts,
//! settings and trusted keys during the side-by-side period.
//!
//! Three pieces live here:
//! - [`CoreStore`], the shell's [`HostStore`] over [`Core`].
//! - [`Services`], what every session view shares: the core, the tokio
//!   runtime its async calls need, and the event bridge.
//! - [`Bridge`], the core's [`EventSink`]. It carries events to the UI thread
//!   and routes early terminal output (see [`Bridge::attach`]).

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};

use anyhow::{Result, anyhow};
use termdrop_core::events::FnSink;
use termdrop_core::{Core, CoreEvent, EventSink, Events, PtyKind, db, state};
use termdrop_terminal::TerminalFeed;

use crate::backend::{HostRecord, HostStore, HostSummary};

/// The shell's host list and settings, read from the core's database.
pub struct CoreStore {
    core: Arc<Core>,
}

impl CoreStore {
    pub fn new(core: Arc<Core>) -> Self {
        Self { core }
    }
}

/// `db::Host` to the shell's raw row. Ports and the favourite flag are
/// INTEGER columns; anything out of range reads as unset rather than wrapping.
pub fn host_record(h: &db::Host) -> HostRecord {
    HostRecord {
        id: h.id,
        name: h.name.clone(),
        host: h.host.clone(),
        port: u16::try_from(h.port).unwrap_or(0),
        username: h.username.clone(),
        group: h.group.clone(),
        favorite: h.favorite != 0,
        mongo_uri: h.mongo_uri.clone(),
        redis_uri: h.redis_uri.clone(),
        vnc_host: h.vnc_host.clone(),
        vnc_port: h.vnc_port.and_then(|p| u16::try_from(p).ok()),
        last_connected_at: h.last_connected_at.clone(),
    }
}

impl HostStore for CoreStore {
    fn hosts(&self) -> Result<Vec<HostSummary>> {
        let hosts = self.core.get_hosts().map_err(|e| anyhow!(e))?;
        Ok(hosts.iter().map(|h| host_record(h).summary()).collect())
    }

    fn set_favorite(&self, host_id: i64, favorite: bool) -> Result<()> {
        state::with_db(&self.core, |conn| {
            db::update_host_favorite(conn, host_id, i64::from(favorite))
        })
        .map_err(|e| anyhow!(e))
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
    slots: Mutex<HashMap<String, Slot>>,
    tx: async_channel::Sender<CoreEvent>,
}

impl Bridge {
    pub fn new() -> (Arc<Self>, async_channel::Receiver<CoreEvent>) {
        let (tx, rx) = async_channel::unbounded();
        let bridge = Arc::new(Self {
            slots: Mutex::new(HashMap::new()),
            tx,
        });
        (bridge, rx)
    }

    fn slots(&self) -> MutexGuard<'_, HashMap<String, Slot>> {
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
        if let Some(Slot::Pending(early)) = slots.remove(session_id) {
            feed.feed(&early);
        }
        slots.insert(session_id.to_string(), Slot::Attached(feed.clone()));
        core.attach_terminal_sink(
            session_id,
            Box::new(FnSink(move |bytes: Vec<u8>| {
                feed.feed(&bytes);
                true
            })),
        )
    }

    /// Forget a terminal: its early output and its feed.
    pub fn detach(&self, session_id: &str) {
        self.slots().remove(session_id);
    }
}

impl EventSink for Bridge {
    fn emit(&self, event: CoreEvent) {
        if let CoreEvent::PtyData {
            kind: PtyKind::Shell,
            id,
            data,
        } = &event
        {
            let mut slots = self.slots();
            match slots
                .entry(id.clone())
                .or_insert_with(|| Slot::Pending(Vec::new()))
            {
                Slot::Pending(buf) => buf.extend_from_slice(data.as_bytes()),
                Slot::Attached(feed) => feed.feed(data.as_bytes()),
            }
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
    fn a_host_row_maps_onto_the_shells_record() {
        let r = host_record(&host());
        assert_eq!((r.id, r.port, r.favorite), (7, 2222, true));
        assert_eq!(r.group.as_deref(), Some("Lab"));
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
        match bridge.slots().get("s") {
            Some(Slot::Pending(buf)) => assert_eq!(buf, b"hello world"),
            _ => panic!("early output was not held"),
        }
        assert!(matches!(rx.try_recv(), Ok(CoreEvent::PtyConnected { .. })));
        assert!(
            rx.try_recv().is_err(),
            "output must not go through the queue"
        );
        bridge.detach("s");
        assert!(bridge.slots().is_empty());
    }
}
