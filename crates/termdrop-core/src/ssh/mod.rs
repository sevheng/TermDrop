pub mod exec;
pub mod io_loop;
pub mod jump;
pub mod known_hosts;
pub mod pty;
pub mod session;
pub mod trust;

pub use session::{create_exec_session, Accept, SshTarget};

use crate::events::{CoreEvent, Events, PtyKind};
use ssh2::{Channel as SshChannel, Session};
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;

pub struct SshSessionHandle {
    /// `None` for a local shell, which belongs to no saved host.
    pub host_id: Option<i64>,
    /// What a local tab is running, so a reconnect starts the same shell.
    pub local_shell: Option<crate::local_shells::LocalShell>,
    pub write_tx: mpsc::UnboundedSender<Vec<u8>>,
    pub disconnect_tx: mpsc::UnboundedSender<()>,
    pub resize_tx: mpsc::UnboundedSender<(u32, u32)>,
    pub data_channel: io_loop::TerminalChannel,
}

pub struct ExecPtyHandle {
    pub host_id: i64,
    pub write_tx: mpsc::UnboundedSender<Vec<u8>>,
    pub disconnect_tx: mpsc::UnboundedSender<()>,
    pub data_channel: io_loop::TerminalChannel,
}

/// Connect, open a channel via `open_channel`, and run the I/O loop on a
/// dedicated thread. Failures before the loop starts are reported as
/// `PtyError`; the loop reports data and disconnects through the rest. `kind`
/// tells the shell's events from a docker exec PTY's.
#[allow(clippy::too_many_arguments)]
fn spawn_pty_thread(
    events: Events,
    id: String,
    target: SshTarget,
    kind: PtyKind,
    open_channel: impl FnOnce(&Session) -> Result<SshChannel, String> + Send + 'static,
    write_rx: mpsc::UnboundedReceiver<Vec<u8>>,
    disconnect_rx: mpsc::UnboundedReceiver<()>,
    resize_rx: Option<mpsc::UnboundedReceiver<(u32, u32)>>,
    data_channel: io_loop::TerminalChannel,
) {
    std::thread::spawn(move || {
        // Never prompts. The terminal opens after the exec session, which is
        // the one that carries the user's acceptance, so a key the user just
        // accepted is on file by now; a docker-exec PTY reuses a host that
        // already has a terminal.
        let session = match session::create_session(&target, &Accept::none()) {
            Ok(s) => s,
            Err(error) => {
                events.emit(CoreEvent::PtyError { kind, id, error });
                return;
            }
        };

        let channel = match open_channel(&session) {
            Ok(c) => c,
            Err(error) => {
                events.emit(CoreEvent::PtyError { kind, id, error });
                return;
            }
        };

        events.emit(CoreEvent::PtyConnected {
            kind,
            id: id.clone(),
        });

        io_loop::run_loop(
            channel,
            write_rx,
            disconnect_rx,
            resize_rx,
            data_channel,
            |data| {
                events.emit(CoreEvent::PtyData {
                    kind,
                    id: id.clone(),
                    data: data.to_string(),
                })
            },
            || {
                events.emit(CoreEvent::PtyDisconnected {
                    kind,
                    id: id.clone(),
                })
            },
        );
    });
}

/// Connect to an SSH host and spawn an interactive shell.
#[allow(clippy::too_many_arguments)]
pub fn connect(
    events: Events,
    session_id: String,
    host_id: i64,
    target: SshTarget,
    initial_cols: u32,
    initial_rows: u32,
) -> Result<SshSessionHandle, String> {
    let (write_tx, write_rx) = mpsc::unbounded_channel::<Vec<u8>>();
    let (disconnect_tx, disconnect_rx) = mpsc::unbounded_channel::<()>();
    let (resize_tx, resize_rx) = mpsc::unbounded_channel::<(u32, u32)>();
    let data_channel: io_loop::TerminalChannel = Arc::new(Mutex::new(None));

    spawn_pty_thread(
        events,
        session_id,
        target,
        PtyKind::Shell,
        move |session| pty::create_pty_channel(session, initial_cols, initial_rows),
        write_rx,
        disconnect_rx,
        Some(resize_rx),
        data_channel.clone(),
    );

    Ok(SshSessionHandle {
        host_id: Some(host_id),
        local_shell: None,
        write_tx,
        disconnect_tx,
        resize_tx,
        data_channel,
    })
}

/// Connect to an SSH host and execute a command in a PTY.
#[allow(clippy::too_many_arguments)]
pub fn exec_pty_connect(
    events: Events,
    pty_session_id: String,
    host_id: i64,
    target: SshTarget,
    command: String,
) -> Result<ExecPtyHandle, String> {
    let (write_tx, write_rx) = mpsc::unbounded_channel::<Vec<u8>>();
    let (disconnect_tx, disconnect_rx) = mpsc::unbounded_channel::<()>();
    let data_channel: io_loop::TerminalChannel = Arc::new(Mutex::new(None));

    spawn_pty_thread(
        events,
        pty_session_id,
        target,
        PtyKind::ExecPty,
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
