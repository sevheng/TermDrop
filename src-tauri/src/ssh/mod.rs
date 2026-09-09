pub mod exec;
pub mod io_loop;
pub mod pty;
pub mod session;

pub use session::create_exec_session;

use ssh2::{Channel as SshChannel, Session};
use std::sync::{Arc, Mutex};
use tauri::{ipc::Channel, Emitter, Window};
use tokio::sync::mpsc;

pub struct SshSessionHandle {
    pub host_id: i64,
    pub write_tx: mpsc::UnboundedSender<String>,
    pub disconnect_tx: mpsc::UnboundedSender<()>,
    pub resize_tx: mpsc::UnboundedSender<(u32, u32)>,
    pub data_channel: Arc<Mutex<Option<Channel<Vec<u8>>>>>,
}

pub struct ExecPtyHandle {
    pub host_id: i64,
    pub write_tx: mpsc::UnboundedSender<String>,
    pub disconnect_tx: mpsc::UnboundedSender<()>,
    pub data_channel: Arc<Mutex<Option<Channel<Vec<u8>>>>>,
}

/// Connection parameters shared by the interactive shell and exec-PTY paths.
struct ConnectParams {
    host: String,
    port: u16,
    username: String,
    password: Option<String>,
    key_path: Option<String>,
}

/// Event names and payload id key that distinguish the interactive shell
/// from the exec-PTY (docker exec/logs) channel on the frontend.
struct PtyEvents {
    id_key: &'static str,
    error: &'static str,
    connected: &'static str,
    data: &'static str,
    disconnected: &'static str,
}

const SHELL_EVENTS: PtyEvents = PtyEvents {
    id_key: "session_id",
    error: "ssh-error",
    connected: "ssh-connected",
    data: "ssh-data",
    disconnected: "ssh-disconnected",
};

const EXEC_PTY_EVENTS: PtyEvents = PtyEvents {
    id_key: "pty_session_id",
    error: "exec-pty-error",
    connected: "exec-pty-connected",
    data: "exec-pty-data",
    disconnected: "exec-pty-disconnected",
};

/// Connect, open a channel via `open_channel`, and run the I/O loop on a
/// dedicated thread. Failures before the loop starts are reported through
/// `events.error`; the loop reports data and disconnects through the rest.
#[allow(clippy::too_many_arguments)]
fn spawn_pty_thread(
    window: Window,
    id: String,
    params: ConnectParams,
    events: PtyEvents,
    open_channel: impl FnOnce(&Session) -> Result<SshChannel, String> + Send + 'static,
    write_rx: mpsc::UnboundedReceiver<String>,
    disconnect_rx: mpsc::UnboundedReceiver<()>,
    resize_rx: Option<mpsc::UnboundedReceiver<(u32, u32)>>,
    data_channel: Arc<Mutex<Option<Channel<Vec<u8>>>>>,
) {
    std::thread::spawn(move || {
        let session = match session::create_session(
            &params.host,
            params.port,
            &params.username,
            params.password.as_deref(),
            params.key_path.as_deref(),
        ) {
            Ok(s) => s,
            Err(e) => {
                let payload = serde_json::json!({ events.id_key: &id, "error": e });
                let _ = window.emit(events.error, payload);
                return;
            }
        };

        let channel = match open_channel(&session) {
            Ok(c) => c,
            Err(e) => {
                let payload = serde_json::json!({ events.id_key: &id, "error": e });
                let _ = window.emit(events.error, payload);
                return;
            }
        };

        let _ = window.emit(events.connected, id.clone());

        io_loop::run_loop(
            channel,
            write_rx,
            disconnect_rx,
            resize_rx,
            data_channel,
            |data| {
                let payload = serde_json::json!({ events.id_key: &id, "data": data });
                let _ = window.emit(events.data, payload);
            },
            || {
                let _ = window.emit(events.disconnected, id.clone());
            },
        );
    });
}

/// Connect to an SSH host and spawn an interactive shell.
#[allow(clippy::too_many_arguments)]
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
    let data_channel: Arc<Mutex<Option<Channel<Vec<u8>>>>> = Arc::new(Mutex::new(None));

    spawn_pty_thread(
        window,
        session_id,
        ConnectParams {
            host,
            port,
            username,
            password,
            key_path,
        },
        SHELL_EVENTS,
        move |session| pty::create_pty_channel(session, initial_cols, initial_rows),
        write_rx,
        disconnect_rx,
        Some(resize_rx),
        data_channel.clone(),
    );

    Ok(SshSessionHandle {
        host_id,
        write_tx,
        disconnect_tx,
        resize_tx,
        data_channel,
    })
}

/// Connect to an SSH host and execute a command in a PTY.
#[allow(clippy::too_many_arguments)]
pub fn exec_pty_connect(
    window: Window,
    pty_session_id: String,
    host_id: i64,
    host: String,
    port: u16,
    username: String,
    password: Option<String>,
    key_path: Option<String>,
    command: String,
) -> Result<ExecPtyHandle, String> {
    let (write_tx, write_rx) = mpsc::unbounded_channel::<String>();
    let (disconnect_tx, disconnect_rx) = mpsc::unbounded_channel::<()>();
    let data_channel: Arc<Mutex<Option<Channel<Vec<u8>>>>> = Arc::new(Mutex::new(None));

    spawn_pty_thread(
        window,
        pty_session_id,
        ConnectParams {
            host,
            port,
            username,
            password,
            key_path,
        },
        EXEC_PTY_EVENTS,
        move |session| pty::create_exec_pty_channel(session, &command),
        write_rx,
        disconnect_rx,
        None,
        data_channel.clone(),
    );

    Ok(ExecPtyHandle {
        host_id,
        write_tx,
        disconnect_tx,
        data_channel,
    })
}
