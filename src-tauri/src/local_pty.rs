//! A shell on this machine, in an ordinary terminal tab.
//!
//! It plugs into the same `io_loop::run_loop`, `SshSessionHandle` and
//! `ssh-*` events as a remote shell, so `ssh_write`, `ssh_resize`,
//! `open_data_channel` and `ssh_disconnect` serve both. The handle's
//! `host_id` is `None`: a local tab belongs to no saved host.

use crate::local_shells::LocalShell;
use crate::ssh::io_loop::{self, PtyIo};
use crate::ssh::{SshSessionHandle, SHELL_EVENTS};
use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self as std_mpsc, RecvTimeoutError, TryRecvError};
use std::time::{Duration, Instant};
use tauri::{Emitter, Window};
use tokio::sync::mpsc;

/// After the child exits, how long to wait for each further chunk before
/// calling it EOF. The reader thread can lag the exit by a moment, and the
/// last lines a command prints are the ones the user wants to see.
const EXIT_DRAIN: Duration = Duration::from_millis(100);

/// How long the loop waits for the frontend's data channel before starting.
/// A local shell prints its prompt within milliseconds -- well before the tab
/// has mounted -- and output emitted as an event before the tab registers its
/// handler is simply lost. The PTY buffers in the meantime.
const CHANNEL_WAIT: Duration = Duration::from_secs(2);

fn size(cols: u32, rows: u32) -> PtySize {
    PtySize {
        rows: rows.clamp(1, u16::MAX as u32) as u16,
        cols: cols.clamp(1, u16::MAX as u32) as u16,
        pixel_width: 0,
        pixel_height: 0,
    }
}

/// A local PTY behind the non-blocking contract `run_loop` expects.
///
/// portable-pty's reader blocks and has no non-blocking mode, so a thread
/// reads it into a channel and `read` drains that channel, answering
/// `WouldBlock` when it is empty.
pub struct LocalPty {
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    child: Box<dyn Child + Send + Sync>,
    rx: std_mpsc::Receiver<Vec<u8>>,
    pending: Vec<u8>,
    exited: bool,
}

impl LocalPty {
    pub fn spawn(
        program: &str,
        args: &[String],
        cwd: Option<&Path>,
        cols: u32,
        rows: u32,
    ) -> Result<Self, String> {
        let pair = native_pty_system()
            .openpty(size(cols, rows))
            .map_err(|e| format!("Failed to open a PTY: {}", e))?;

        let mut cmd = CommandBuilder::new(program);
        cmd.args(args);
        if let Some(dir) = cwd {
            cmd.cwd(dir);
        }
        cmd.env("TERM", "xterm-256color");
        cmd.env("COLORTERM", "truecolor");
        cmd.env("TERM_PROGRAM", "TermDrop");

        let child = pair
            .slave
            .spawn_command(cmd)
            .map_err(|e| format!("Failed to start {}: {}", program, e))?;
        // Our copy of the slave would keep the PTY open after the child exits,
        // and the reader would never see the end.
        drop(pair.slave);

        let mut reader = pair
            .master
            .try_clone_reader()
            .map_err(|e| format!("Failed to read the PTY: {}", e))?;
        let writer = pair
            .master
            .take_writer()
            .map_err(|e| format!("Failed to write the PTY: {}", e))?;

        let (tx, rx) = std_mpsc::channel();
        std::thread::spawn(move || {
            let mut buf = vec![0u8; 16384];
            loop {
                match reader.read(&mut buf) {
                    // Linux reports the slave closing as EIO, not as 0.
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        if tx.send(buf[..n].to_vec()).is_err() {
                            break;
                        }
                    }
                }
            }
        });

        Ok(LocalPty {
            master: pair.master,
            writer,
            child,
            rx,
            pending: Vec::new(),
            exited: false,
        })
    }

    fn child_exited(&mut self) -> bool {
        if !self.exited {
            self.exited = matches!(self.child.try_wait(), Ok(Some(_)));
        }
        self.exited
    }

    fn deliver(&mut self, chunk: Vec<u8>, buf: &mut [u8]) -> usize {
        self.pending = chunk;
        let n = self.pending.len().min(buf.len());
        buf[..n].copy_from_slice(&self.pending[..n]);
        self.pending.drain(..n);
        n
    }
}

impl Read for LocalPty {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if !self.pending.is_empty() {
            let chunk = std::mem::take(&mut self.pending);
            return Ok(self.deliver(chunk, buf));
        }
        match self.rx.try_recv() {
            Ok(chunk) => Ok(self.deliver(chunk, buf)),
            Err(TryRecvError::Disconnected) => Ok(0),
            // ConPTY keeps its output pipe open after the child exits, so on
            // Windows the reader never ends by itself; the exit is the signal.
            Err(TryRecvError::Empty) if self.child_exited() => {
                match self.rx.recv_timeout(EXIT_DRAIN) {
                    Ok(chunk) => Ok(self.deliver(chunk, buf)),
                    Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => Ok(0),
                }
            }
            Err(TryRecvError::Empty) => Err(std::io::ErrorKind::WouldBlock.into()),
        }
    }
}

impl Write for LocalPty {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.writer.write(buf)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.writer.flush()
    }
}

impl PtyIo for LocalPty {
    fn resize(&mut self, cols: u32, rows: u32) {
        let _ = self.master.resize(size(cols, rows));
    }

    fn shutdown(&mut self) {
        if !self.child_exited() {
            let _ = self.child.kill();
        }
        let _ = self.child.wait();
    }
}

/// Start `shell` and run it on a dedicated thread.
///
/// The process is spawned before this returns, so a shell that cannot start
/// is the command's error rather than an event into a tab that is not there.
/// `data_channel` is passed in so a reconnect can keep the tab's channel.
pub fn connect(
    window: Window,
    session_id: String,
    shell: LocalShell,
    cols: u32,
    rows: u32,
    data_channel: io_loop::TerminalChannel,
) -> Result<SshSessionHandle, String> {
    let home: Option<PathBuf> = dirs::home_dir();
    let pty = LocalPty::spawn(&shell.program, &shell.args, home.as_deref(), cols, rows)?;

    let (write_tx, write_rx) = mpsc::unbounded_channel::<String>();
    let (disconnect_tx, disconnect_rx) = mpsc::unbounded_channel::<()>();
    let (resize_tx, resize_rx) = mpsc::unbounded_channel::<(u32, u32)>();

    let loop_channel = data_channel.clone();
    std::thread::spawn(move || {
        let deadline = Instant::now() + CHANNEL_WAIT;
        while Instant::now() < deadline && loop_channel.lock().map(|c| c.is_none()).unwrap_or(false)
        {
            std::thread::sleep(Duration::from_millis(10));
        }

        let events = &SHELL_EVENTS;
        let _ = window.emit(events.connected, session_id.clone());
        io_loop::run_loop(
            pty,
            write_rx,
            disconnect_rx,
            Some(resize_rx),
            loop_channel,
            |data| {
                let payload = serde_json::json!({ events.id_key: &session_id, "data": data });
                let _ = window.emit(events.data, payload);
            },
            || {
                let _ = window.emit(events.disconnected, session_id.clone());
            },
        );
    });

    Ok(SshSessionHandle {
        host_id: None,
        local_shell: Some(shell),
        write_tx,
        disconnect_tx,
        resize_tx,
        data_channel,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_are_clamped_into_range() {
        let s = size(0, 100_000);
        assert_eq!((s.cols, s.rows), (1, u16::MAX));
    }

    /// Spawns a real process through the same trait `run_loop` drives, and
    /// checks the output arrives whole and the exit reads as EOF.
    #[test]
    #[cfg(unix)]
    fn local_pty_reads_output_then_eof() {
        let args = vec!["-c".to_string(), "printf hi".to_string()];
        let mut pty = LocalPty::spawn("/bin/sh", &args, None, 80, 24).expect("spawn");
        let mut out = Vec::new();
        let mut buf = [0u8; 64];
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            assert!(Instant::now() < deadline, "no EOF; got {:?}", out);
            match pty.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => out.extend_from_slice(&buf[..n]),
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(5));
                }
                Err(e) => panic!("read failed: {}", e),
            }
        }
        assert_eq!(String::from_utf8_lossy(&out), "hi");
        pty.shutdown();
    }

    #[test]
    #[cfg(unix)]
    fn local_pty_takes_input_and_shutdown_kills_the_child() {
        let mut pty = LocalPty::spawn("/bin/cat", &[], None, 80, 24).expect("spawn");
        pty.write_all(b"ping\n").unwrap();
        let mut out = Vec::new();
        let mut buf = [0u8; 64];
        let deadline = Instant::now() + Duration::from_secs(5);
        while !String::from_utf8_lossy(&out).contains("ping") {
            assert!(Instant::now() < deadline, "echo never came back: {:?}", out);
            match pty.read(&mut buf) {
                Ok(n) => out.extend_from_slice(&buf[..n]),
                Err(_) => std::thread::sleep(Duration::from_millis(5)),
            }
        }
        // cat never exits by itself; shutdown must not hang.
        pty.shutdown();
        assert!(pty.child_exited());
    }
}
