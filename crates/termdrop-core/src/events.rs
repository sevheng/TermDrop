//! How the backend talks to whatever UI is on top of it.
//!
//! Two kinds of traffic leave the backend unasked: **events** (a terminal
//! connected, a backup moved on, a VNC server rang its bell) and **bytes**
//! (terminal output, VNC frames). Each has a trait here, so nothing in this
//! crate names a UI toolkit. The Tauri app maps every [`CoreEvent`] back to the
//! event name and JSON payload the webview has always received; the native app
//! handles them as values.

use std::sync::{Arc, Mutex};

/// Receives the backend's events. Called from worker threads and async tasks,
/// so it must not block for long: a slow sink stalls the terminal or transfer
/// that emitted the event.
pub trait EventSink: Send + Sync + 'static {
    fn emit(&self, event: CoreEvent);
}

/// The shared handle every emitting function takes. Cloned freely, the way a
/// Tauri `Window` used to be.
pub type Events = Arc<dyn EventSink>;

/// Receives a stream of bytes: a terminal's output or a VNC session's frames.
///
/// One message per call, in order. The bytes are already gathered into as few
/// messages as the stream allows (see `ssh::io_loop::OutputSink`), so a sink
/// should hand each one on as it is rather than splitting it.
pub trait ByteSink: Send + 'static {
    /// Returns `false` once the receiver is gone.
    fn send(&mut self, bytes: Vec<u8>) -> bool;
}

/// Where a terminal's or a VNC session's bytes go once the UI has registered a
/// sink. `None` until then, which is what sends terminal output down the JSON
/// fallback and keeps a VNC session from requesting frames.
pub type ByteSlot = Arc<Mutex<Option<Box<dyn ByteSink>>>>;

/// Which kind of PTY an event is about. A shell tab and a docker exec pane use
/// the same thread and loop, and differ only in what they are called.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PtyKind {
    /// An interactive shell, SSH or local, in a terminal tab.
    Shell,
    /// A `docker exec` / `docker logs` PTY in the Docker panel.
    ExecPty,
}

/// Progress through a MongoDB dump or restore.
#[derive(Debug, Clone, PartialEq)]
pub struct MongoProgress {
    pub op_id: String,
    pub db: String,
    /// Empty while a tool runs without reporting a collection.
    pub collection: String,
    pub stage: String,
    pub synced: u64,
    pub total: u64,
    pub percent: u64,
    /// The tool's own "current / total" text, when it printed one.
    pub detail: Option<String>,
}

/// Progress through a Redis backup or restore.
#[derive(Debug, Clone, PartialEq)]
pub struct RedisProgress {
    pub op_id: String,
    pub db: i64,
    pub stage: String,
    pub processed: u64,
    pub total: u64,
    pub percent: u64,
    pub detail: String,
}

/// Everything the backend reports without being asked.
#[derive(Debug, Clone, PartialEq)]
pub enum CoreEvent {
    /// The PTY is open and output will follow.
    PtyConnected {
        kind: PtyKind,
        id: String,
    },
    /// Output that arrived before a [`ByteSink`] was registered, decoded as
    /// UTF-8 with split characters carried over.
    PtyData {
        kind: PtyKind,
        id: String,
        data: String,
    },
    /// The remote end or the shell went away. Not sent for a disconnect the
    /// UI asked for.
    PtyDisconnected {
        kind: PtyKind,
        id: String,
    },
    /// The PTY could not be opened.
    PtyError {
        kind: PtyKind,
        id: String,
        error: String,
    },
    /// A shell tab was reconnected under the same session id.
    SshReconnected {
        session_id: String,
    },
    SftpProgress {
        sftp_session_id: String,
        file: String,
        bytes_transferred: u64,
        total_bytes: u64,
    },
    MongoProgress(MongoProgress),
    MongoCancelled {
        op_id: String,
        db: String,
    },
    RedisProgress(RedisProgress),
    RedisCancelled {
        op_id: String,
        db: i64,
    },
    VncBell {
        session_id: String,
    },
    VncClipboard {
        session_id: String,
        text: String,
    },
    VncResize {
        session_id: String,
        width: u16,
        height: u16,
    },
    /// A clean end: the user closed it, or the server hung up politely.
    VncDisconnected {
        session_id: String,
    },
    VncError {
        session_id: String,
        error: String,
    },
}

/// An [`EventSink`] that drops everything, for callers with nobody to tell.
pub struct NoEvents;

impl EventSink for NoEvents {
    fn emit(&self, _event: CoreEvent) {}
}

/// Collects events in memory. For tests, and for a caller that polls.
#[derive(Default)]
pub struct EventLog(pub Mutex<Vec<CoreEvent>>);

impl EventLog {
    pub fn take(&self) -> Vec<CoreEvent> {
        std::mem::take(&mut *self.0.lock().unwrap_or_else(|e| e.into_inner()))
    }
}

impl EventSink for EventLog {
    fn emit(&self, event: CoreEvent) {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).push(event);
    }
}

/// A [`ByteSink`] over a closure, for tests and simple consumers.
pub struct FnSink<F>(pub F);

impl<F: FnMut(Vec<u8>) -> bool + Send + 'static> ByteSink for FnSink<F> {
    fn send(&mut self, bytes: Vec<u8>) -> bool {
        (self.0)(bytes)
    }
}

/// A [`ByteSink`] over a tokio channel: what a UI that owns an event loop
/// usually wants.
impl ByteSink for tokio::sync::mpsc::UnboundedSender<Vec<u8>> {
    fn send(&mut self, bytes: Vec<u8>) -> bool {
        tokio::sync::mpsc::UnboundedSender::send(self, bytes).is_ok()
    }
}

/// A [`ByteSink`] over a std channel.
impl ByteSink for std::sync::mpsc::Sender<Vec<u8>> {
    fn send(&mut self, bytes: Vec<u8>) -> bool {
        std::sync::mpsc::Sender::send(self, bytes).is_ok()
    }
}
