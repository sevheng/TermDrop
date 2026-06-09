pub mod session;
pub mod pty;
pub mod io_loop;

pub use session::create_exec_session;

use ssh2::Session;
use std::io::Read;
use tauri::{Emitter, Window};
use tokio::sync::mpsc;

pub struct SshSessionHandle {
    pub host_id: i64,
    pub write_tx: mpsc::UnboundedSender<String>,
    pub disconnect_tx: mpsc::UnboundedSender<()>,
    pub resize_tx: mpsc::UnboundedSender<(u32, u32)>,
}

pub struct ExecPtyHandle {
    pub write_tx: mpsc::UnboundedSender<String>,
    pub disconnect_tx: mpsc::UnboundedSender<()>,
}

/// Run a command on an existing SSH session (reuses connection).
pub fn exec_with_session(
    session: &Session,
    command: &str,
) -> Result<String, String> {
    let mut channel = session.channel_session()
        .map_err(|e| format!("channel: {}", e))?;
    channel.exec(command)
        .map_err(|e| format!("exec: {}", e))?;

    let mut output = String::new();
    channel.read_to_string(&mut output)
        .map_err(|e| format!("read: {}", e))?;

    channel.wait_close().ok();
    Ok(output.trim().to_string())
}

/// Connect to an SSH host and spawn an interactive shell.
pub fn connect(
    window: Window,
    session_id: String,
    host_id: i64,
    host: String,
    port: u16,
    username: String,
    password: Option<String>,
    key_path: Option<String>,
    initial_cols: u32,
    initial_rows: u32,
) -> Result<SshSessionHandle, String> {
    let (write_tx, write_rx) = mpsc::unbounded_channel::<String>();
    let (disconnect_tx, disconnect_rx) = mpsc::unbounded_channel::<()>();
    let (resize_tx, resize_rx) = mpsc::unbounded_channel::<(u32, u32)>();

    std::thread::spawn(move || {
        let session = match session::create_session(&host, port, &username, password.as_deref(), key_path.as_deref()) {
            Ok(s) => s,
            Err(e) => {
                let payload = serde_json::json!({"session_id": &session_id, "error": e});
                let _ = window.emit("ssh-error", payload);
                return;
            }
        };

        let channel = match pty::create_pty_channel(&session, initial_cols, initial_rows) {
            Ok(c) => c,
            Err(e) => {
                let payload = serde_json::json!({"session_id": &session_id, "error": e});
                let _ = window.emit("ssh-error", payload);
                return;
            }
        };

        let _ = window.emit("ssh-connected", session_id.clone());

        io_loop::run_io_loop(
            channel,
            write_rx,
            disconnect_rx,
            resize_rx,
            |data| {
                let payload = serde_json::json!({
                    "session_id": &session_id,
                    "data": data,
                });
                let _ = window.emit("ssh-data", payload);
            },
            || {
                let _ = window.emit("ssh-disconnected", session_id.clone());
            },
        );
    });

    Ok(SshSessionHandle { host_id, write_tx, disconnect_tx, resize_tx })
}

/// Connect to an SSH host and execute a command in a PTY.
pub fn exec_pty_connect(
    window: Window,
    pty_session_id: String,
    host: String,
    port: u16,
    username: String,
    password: Option<String>,
    key_path: Option<String>,
    command: String,
) -> Result<ExecPtyHandle, String> {
    let (write_tx, write_rx) = mpsc::unbounded_channel::<String>();
    let (disconnect_tx, disconnect_rx) = mpsc::unbounded_channel::<()>();

    std::thread::spawn(move || {
        let session = match session::create_session(&host, port, &username, password.as_deref(), key_path.as_deref()) {
            Ok(s) => s,
            Err(e) => {
                let payload = serde_json::json!({"pty_session_id": &pty_session_id, "error": e});
                let _ = window.emit("exec-pty-error", payload);
                return;
            }
        };

        let channel = match pty::create_exec_pty_channel(&session, &command) {
            Ok(c) => c,
            Err(e) => {
                let payload = serde_json::json!({"pty_session_id": &pty_session_id, "error": e});
                let _ = window.emit("exec-pty-error", payload);
                return;
            }
        };

        let _ = window.emit("exec-pty-connected", pty_session_id.clone());

        io_loop::run_exec_pty_loop(
            channel,
            write_rx,
            disconnect_rx,
            |data| {
                let payload = serde_json::json!({
                    "pty_session_id": &pty_session_id,
                    "data": data,
                });
                let _ = window.emit("exec-pty-data", payload);
            },
            || {
                let _ = window.emit("exec-pty-disconnected", pty_session_id.clone());
            },
        );
    });

    Ok(ExecPtyHandle { write_tx, disconnect_tx })
}
