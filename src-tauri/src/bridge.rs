//! The Tauri side of `termdrop_core::events`.
//!
//! The core reports typed [`CoreEvent`]s and pushes bytes into a [`ByteSink`].
//! The webview has always received named events with JSON payloads and raw IPC
//! channel messages, and it must keep receiving exactly those: every event name
//! and payload key below is part of the frontend contract, and the tests pin
//! them.

use serde_json::{json, Value};
use std::sync::Arc;
use tauri::ipc::{Channel, InvokeResponseBody};
use tauri::{Emitter, Window};
use termdrop_core::events::{ByteSink, CoreEvent, EventSink, Events, PtyKind};

/// Emits core events on a window, under their historical names.
struct WindowSink(Window);

impl EventSink for WindowSink {
    fn emit(&self, event: CoreEvent) {
        let (name, payload) = wire(event);
        let _ = self.0.emit(name, payload);
    }
}

/// The event sink for a command's window.
pub fn events(window: &Window) -> Events {
    Arc::new(WindowSink(window.clone()))
}

/// A frontend IPC channel as a byte sink.
///
/// Raw, never `Vec<u8>`: Tauri has a blanket `impl<T: Serialize> IpcResponse`,
/// so a `Vec<u8>` would cross as a JSON array of decimal numbers. A Raw body of
/// 1 KB or more takes Tauri's fetch path and arrives as a real `ArrayBuffer`.
struct ChannelSink(Channel<InvokeResponseBody>);

impl ByteSink for ChannelSink {
    fn send(&mut self, bytes: Vec<u8>) -> bool {
        self.0.send(InvokeResponseBody::Raw(bytes)).is_ok()
    }
}

pub fn byte_sink(channel: Channel<InvokeResponseBody>) -> Box<dyn ByteSink> {
    Box::new(ChannelSink(channel))
}

/// The event name and payload the frontend listens for.
fn wire(event: CoreEvent) -> (&'static str, Value) {
    // The shell and the docker exec PTY share one loop and differ only in
    // these names and the payload's id key.
    let pty = |kind: PtyKind, shell: &'static str, exec: &'static str| match kind {
        PtyKind::Shell => (shell, "session_id"),
        PtyKind::ExecPty => (exec, "pty_session_id"),
    };
    match event {
        CoreEvent::PtyConnected { kind, id } => (
            pty(kind, "ssh-connected", "exec-pty-connected").0,
            json!(id),
        ),
        CoreEvent::PtyData { kind, id, data } => {
            let (name, key) = pty(kind, "ssh-data", "exec-pty-data");
            (name, json!({ key: id, "data": data }))
        }
        CoreEvent::PtyDisconnected { kind, id } => (
            pty(kind, "ssh-disconnected", "exec-pty-disconnected").0,
            json!(id),
        ),
        CoreEvent::PtyError { kind, id, error } => {
            let (name, key) = pty(kind, "ssh-error", "exec-pty-error");
            (name, json!({ key: id, "error": error }))
        }
        CoreEvent::SshReconnected { session_id } => ("ssh-reconnected", json!(session_id)),
        CoreEvent::SftpProgress {
            sftp_session_id,
            file,
            bytes_transferred,
            total_bytes,
        } => (
            "sftp-progress",
            json!({
                "sftp_session_id": sftp_session_id,
                "file": file,
                "bytes_transferred": bytes_transferred,
                "total_bytes": total_bytes,
            }),
        ),
        CoreEvent::MongoProgress(p) => {
            let mut payload = json!({
                "opId": p.op_id,
                "db": p.db,
                "collection": p.collection,
                "stage": p.stage,
                "synced": p.synced,
                "total": p.total,
                "percent": p.percent,
            });
            // Only the CLI tools' own progress lines carry one; the pulse and
            // done events never had the key at all.
            if let Some(detail) = p.detail {
                payload["detail"] = json!(detail);
            }
            ("mongodb-sync-progress", payload)
        }
        CoreEvent::MongoCancelled { op_id, db } => {
            ("mongodb-sync-cancelled", json!({ "opId": op_id, "db": db }))
        }
        CoreEvent::RedisProgress(p) => (
            "redis-op-progress",
            json!({
                "opId": p.op_id,
                "db": p.db,
                "stage": p.stage,
                "processed": p.processed,
                "total": p.total,
                "percent": p.percent,
                "detail": p.detail,
            }),
        ),
        CoreEvent::RedisCancelled { op_id, db } => {
            ("redis-op-cancelled", json!({ "opId": op_id, "db": db }))
        }
        CoreEvent::VncBell { session_id } => ("vnc-bell", json!(session_id)),
        CoreEvent::VncClipboard { session_id, text } => (
            "vnc-clipboard",
            json!({ "sessionId": session_id, "text": text }),
        ),
        CoreEvent::VncResize {
            session_id,
            width,
            height,
        } => (
            "vnc-resize",
            json!({ "sessionId": session_id, "width": width, "height": height }),
        ),
        CoreEvent::VncDisconnected { session_id } => ("vnc-disconnected", json!(session_id)),
        CoreEvent::VncError { session_id, error } => (
            "vnc-error",
            json!({ "sessionId": session_id, "error": error }),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use termdrop_core::events::{MongoProgress, RedisProgress};

    fn id() -> String {
        "s1".to_string()
    }

    #[test]
    fn shell_events_keep_their_names_and_payloads() {
        let kind = PtyKind::Shell;
        assert_eq!(
            wire(CoreEvent::PtyConnected { kind, id: id() }),
            ("ssh-connected", json!("s1"))
        );
        assert_eq!(
            wire(CoreEvent::PtyData {
                kind,
                id: id(),
                data: "hi".into()
            }),
            ("ssh-data", json!({ "session_id": "s1", "data": "hi" }))
        );
        assert_eq!(
            wire(CoreEvent::PtyDisconnected { kind, id: id() }),
            ("ssh-disconnected", json!("s1"))
        );
        assert_eq!(
            wire(CoreEvent::PtyError {
                kind,
                id: id(),
                error: "no".into()
            }),
            ("ssh-error", json!({ "session_id": "s1", "error": "no" }))
        );
        assert_eq!(
            wire(CoreEvent::SshReconnected { session_id: id() }),
            ("ssh-reconnected", json!("s1"))
        );
    }

    #[test]
    fn exec_pty_events_keep_their_names_and_id_key() {
        let kind = PtyKind::ExecPty;
        assert_eq!(
            wire(CoreEvent::PtyConnected { kind, id: id() }),
            ("exec-pty-connected", json!("s1"))
        );
        assert_eq!(
            wire(CoreEvent::PtyData {
                kind,
                id: id(),
                data: "hi".into()
            }),
            (
                "exec-pty-data",
                json!({ "pty_session_id": "s1", "data": "hi" })
            )
        );
        assert_eq!(
            wire(CoreEvent::PtyDisconnected { kind, id: id() }),
            ("exec-pty-disconnected", json!("s1"))
        );
        assert_eq!(
            wire(CoreEvent::PtyError {
                kind,
                id: id(),
                error: "no".into()
            }),
            (
                "exec-pty-error",
                json!({ "pty_session_id": "s1", "error": "no" })
            )
        );
    }

    #[test]
    fn sftp_progress_keeps_its_snake_case_payload() {
        assert_eq!(
            wire(CoreEvent::SftpProgress {
                sftp_session_id: id(),
                file: "/f".into(),
                bytes_transferred: 5,
                total_bytes: 10,
            }),
            (
                "sftp-progress",
                json!({
                    "sftp_session_id": "s1",
                    "file": "/f",
                    "bytes_transferred": 5,
                    "total_bytes": 10,
                })
            )
        );
    }

    #[test]
    fn mongo_progress_carries_detail_only_when_the_tool_gave_one() {
        let pulse = MongoProgress {
            op_id: "op".into(),
            db: "d".into(),
            collection: String::new(),
            stage: "dumping".into(),
            synced: 0,
            total: 1,
            percent: 40,
            detail: None,
        };
        assert_eq!(
            wire(CoreEvent::MongoProgress(pulse.clone())),
            (
                "mongodb-sync-progress",
                json!({
                    "opId": "op", "db": "d", "collection": "", "stage": "dumping",
                    "synced": 0, "total": 1, "percent": 40,
                })
            )
        );
        let cli = MongoProgress {
            collection: "c".into(),
            detail: Some("1/2".into()),
            ..pulse
        };
        assert_eq!(wire(CoreEvent::MongoProgress(cli)).1["detail"], "1/2");
        assert_eq!(
            wire(CoreEvent::MongoCancelled {
                op_id: "op".into(),
                db: "d".into()
            }),
            ("mongodb-sync-cancelled", json!({ "opId": "op", "db": "d" }))
        );
    }

    #[test]
    fn redis_progress_keeps_its_own_event_names() {
        let p = RedisProgress {
            op_id: "op".into(),
            db: 3,
            stage: "dumping".into(),
            processed: 1,
            total: 2,
            percent: 50,
            detail: String::new(),
        };
        assert_eq!(
            wire(CoreEvent::RedisProgress(p)),
            (
                "redis-op-progress",
                json!({
                    "opId": "op", "db": 3, "stage": "dumping", "processed": 1,
                    "total": 2, "percent": 50, "detail": "",
                })
            )
        );
        assert_eq!(
            wire(CoreEvent::RedisCancelled {
                op_id: "op".into(),
                db: 3
            }),
            ("redis-op-cancelled", json!({ "opId": "op", "db": 3 }))
        );
    }

    #[test]
    fn vnc_events_keep_their_names_and_camel_case_ids() {
        assert_eq!(
            wire(CoreEvent::VncBell { session_id: id() }),
            ("vnc-bell", json!("s1"))
        );
        assert_eq!(
            wire(CoreEvent::VncDisconnected { session_id: id() }),
            ("vnc-disconnected", json!("s1"))
        );
        assert_eq!(
            wire(CoreEvent::VncError {
                session_id: id(),
                error: "e".into()
            }),
            ("vnc-error", json!({ "sessionId": "s1", "error": "e" }))
        );
        assert_eq!(
            wire(CoreEvent::VncResize {
                session_id: id(),
                width: 800,
                height: 600
            }),
            (
                "vnc-resize",
                json!({ "sessionId": "s1", "width": 800, "height": 600 })
            )
        );
        assert_eq!(
            wire(CoreEvent::VncClipboard {
                session_id: id(),
                text: "t".into()
            }),
            ("vnc-clipboard", json!({ "sessionId": "s1", "text": "t" }))
        );
    }
}
