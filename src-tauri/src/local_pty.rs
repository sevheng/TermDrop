//! A shell on this machine, in an ordinary terminal tab.
//!
//! It shares `SshSessionHandle` and the `ssh-*` events with a remote shell, so
//! `ssh_write`, `ssh_resize`, `open_data_channel` and `ssh_disconnect` serve
//! both. The handle's `host_id` is `None`: a local tab belongs to no saved host.
//!
//! Unlike SSH it does not run `io_loop::run_loop`. ssh2 has nothing to wait
//! on, so that loop polls with a 10 ms sleep; a local PTY can be waited on, so
//! `run_local_loop` sleeps until output, a keystroke, a resize, a disconnect or
//! the shell exiting actually happens.

use crate::local_shells::LocalShell;
use crate::ssh::io_loop::{self, OutputSink, COALESCE_MAX_AGE, COALESCE_MAX_BYTES};
use crate::ssh::{SshSessionHandle, SHELL_EVENTS};
use portable_pty::{native_pty_system, ChildKiller, CommandBuilder, MasterPty, PtySize};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use tauri::{Emitter, Window};
use tokio::sync::{mpsc, oneshot};
use tokio::time::sleep_until;

/// After the child exits, how long to wait for each further chunk before
/// calling it EOF. The reader thread can lag the exit by a moment, and the
/// last lines a command prints are the ones the user wants to see.
const EXIT_DRAIN: Duration = Duration::from_millis(100);

/// How long the loop waits for the frontend's data channel before starting.
/// A local shell prints its prompt within milliseconds -- well before the tab
/// has mounted -- and output emitted as an event before the tab registers its
/// handler is simply lost. The PTY buffers in the meantime.
const CHANNEL_WAIT: Duration = Duration::from_secs(2);

/// How long fallback text may sit before it is emitted, matching the batch
/// interval `OutputSink` applies on that path.
const FALLBACK_INTERVAL: Duration = Duration::from_millis(16);

fn size(cols: u32, rows: u32) -> PtySize {
    PtySize {
        rows: rows.clamp(1, u16::MAX as u32) as u16,
        cols: cols.clamp(1, u16::MAX as u32) as u16,
        pixel_width: 0,
        pixel_height: 0,
    }
}

/// A spawned shell, taken apart into what the event loop waits on.
///
/// portable-pty's reader blocks and has no async mode, so a thread reads it
/// into `output`. The child is owned by a waiter thread that reports its exit
/// on `exited`; `killer` ends it from here.
pub struct LocalPty {
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    killer: Box<dyn ChildKiller + Send + Sync>,
    output: mpsc::UnboundedReceiver<Vec<u8>>,
    exited: oneshot::Receiver<()>,
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

        let mut child = pair
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
        let killer = child.clone_killer();

        let (out_tx, output) = mpsc::unbounded_channel();
        std::thread::spawn(move || {
            let mut buf = vec![0u8; 16384];
            loop {
                match reader.read(&mut buf) {
                    // Linux reports the slave closing as EIO, not as 0.
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        if out_tx.send(buf[..n].to_vec()).is_err() {
                            break;
                        }
                    }
                }
            }
        });

        let (exit_tx, exited) = oneshot::channel();
        std::thread::spawn(move || {
            let _ = child.wait();
            let _ = exit_tx.send(());
        });

        Ok(LocalPty {
            master: pair.master,
            writer,
            killer,
            output,
            exited,
        })
    }
}

/// Drive a local shell until it exits or the tab disconnects.
///
/// Nothing here sleeps on a timer except to send gathered output. Output that
/// arrives after a quiet spell is sent at once, so an echo is never delayed;
/// output arriving within `COALESCE_MAX_AGE` of the last send is gathered and
/// sent when that window closes, which keeps a fast producer's messages large
/// enough to cross as binary (see `OutputSink`).
pub fn run_local_loop(
    mut pty: LocalPty,
    mut write_rx: mpsc::UnboundedReceiver<String>,
    mut disconnect_rx: mpsc::UnboundedReceiver<()>,
    mut resize_rx: mpsc::UnboundedReceiver<(u32, u32)>,
    data_channel: io_loop::TerminalChannel,
    on_data_fallback: impl FnMut(&str),
    mut on_disconnect: impl FnMut(),
) {
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            tracing::error!("local terminal runtime failed to start: {}", e);
            on_disconnect();
            return;
        }
    };
    let mut sink = OutputSink::new(io_loop::channel_sender(data_channel), on_data_fallback);

    let intentional = runtime.block_on(async {
        let mut last_send = Instant::now() - COALESCE_MAX_AGE;
        let mut deadline: Option<tokio::time::Instant> = None;

        loop {
            tokio::select! {
                chunk = pty.output.recv() => match chunk {
                    Some(bytes) => {
                        sink.append(&bytes);
                        // Take everything already queued in one go.
                        while let Ok(more) = pty.output.try_recv() {
                            sink.append(&more);
                        }
                        let quiet = last_send.elapsed() >= COALESCE_MAX_AGE;
                        if quiet || sink.pending_len() >= COALESCE_MAX_BYTES {
                            sink.flush();
                            last_send = Instant::now();
                            deadline = (!sink.is_empty()).then(|| tokio::time::Instant::now() + FALLBACK_INTERVAL);
                        } else if deadline.is_none() {
                            deadline = Some((last_send + COALESCE_MAX_AGE).into());
                        }
                    }
                    // The reader ended: the slave side closed.
                    None => break false,
                },
                _ = async { sleep_until(deadline.unwrap()).await }, if deadline.is_some() => {
                    sink.idle();
                    last_send = Instant::now();
                    deadline = (!sink.is_empty()).then(|| tokio::time::Instant::now() + FALLBACK_INTERVAL);
                }
                Some(data) = write_rx.recv() => {
                    // The reader thread drains output independently, so a
                    // blocking write here cannot deadlock against a shell
                    // that is busy echoing.
                    let _ = pty.writer.write_all(data.as_bytes());
                    let _ = pty.writer.flush();
                }
                Some((cols, rows)) = resize_rx.recv() => {
                    let _ = pty.master.resize(size(cols, rows));
                }
                _ = disconnect_rx.recv() => break true,
                // ConPTY keeps its output pipe open after the child exits, so
                // on Windows the reader never ends by itself; the exit is the
                // signal. Drain what is still in flight, then stop.
                _ = &mut pty.exited => {
                    while let Ok(Some(bytes)) = tokio::time::timeout(EXIT_DRAIN, pty.output.recv()).await {
                        sink.append(&bytes);
                    }
                    break false;
                }
            }
        }
    });

    if intentional {
        let _ = pty.killer.kill();
    } else {
        sink.finish();
        let _ = pty.killer.kill();
        on_disconnect();
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
        run_local_loop(
            pty,
            write_rx,
            disconnect_rx,
            resize_rx,
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
    use std::sync::{Arc, Mutex};
    use tauri::ipc::{Channel, InvokeResponseBody};

    #[test]
    fn sizes_are_clamped_into_range() {
        let s = size(0, 100_000);
        assert_eq!((s.cols, s.rows), (1, u16::MAX));
    }

    /// What reached the data channel, with when it arrived.
    type Sent = Arc<Mutex<Vec<(Instant, Vec<u8>)>>>;

    fn channel() -> (io_loop::TerminalChannel, Sent) {
        let sent: Sent = Arc::new(Mutex::new(Vec::new()));
        let sink = sent.clone();
        let ch = Channel::new(move |body| {
            if let InvokeResponseBody::Raw(bytes) = body {
                sink.lock().unwrap().push((Instant::now(), bytes));
            }
            Ok(())
        });
        (Arc::new(Mutex::new(Some(ch))), sent)
    }

    struct Session {
        write_tx: mpsc::UnboundedSender<String>,
        disconnect_tx: mpsc::UnboundedSender<()>,
        sent: Sent,
        disconnected: Arc<Mutex<bool>>,
        done: std::thread::JoinHandle<()>,
    }

    fn start(program: &str, args: &[&str]) -> Session {
        let args: Vec<String> = args.iter().map(|a| a.to_string()).collect();
        let pty = LocalPty::spawn(program, &args, None, 80, 24).expect("spawn");
        let (data_channel, sent) = channel();
        let (write_tx, write_rx) = mpsc::unbounded_channel();
        let (disconnect_tx, disconnect_rx) = mpsc::unbounded_channel();
        let (_resize_tx, resize_rx) = mpsc::unbounded_channel();
        let disconnected = Arc::new(Mutex::new(false));
        let flag = disconnected.clone();
        let done = std::thread::spawn(move || {
            run_local_loop(
                pty,
                write_rx,
                disconnect_rx,
                resize_rx,
                data_channel,
                |_| {},
                move || *flag.lock().unwrap() = true,
            )
        });
        Session {
            write_tx,
            disconnect_tx,
            sent,
            disconnected,
            done,
        }
    }

    fn output(sent: &Sent) -> String {
        String::from_utf8_lossy(
            &sent
                .lock()
                .unwrap()
                .iter()
                .flat_map(|(_, b)| b.clone())
                .collect::<Vec<_>>(),
        )
        .into_owned()
    }

    fn wait_for(sent: &Sent, needle: &str) -> Instant {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some((t, _)) = {
                let s = sent.lock().unwrap();
                let mut seen = Vec::new();
                s.iter()
                    .find(|(_, b)| {
                        seen.extend_from_slice(b);
                        String::from_utf8_lossy(&seen).contains(needle)
                    })
                    .cloned()
            } {
                return t;
            }
            assert!(
                Instant::now() < deadline,
                "never saw {:?}; got {:?}",
                needle,
                output(sent)
            );
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    #[test]
    #[cfg(unix)]
    fn a_shell_that_exits_delivers_its_output_and_reports_the_disconnect() {
        let s = start("/bin/sh", &["-c", "printf hi"]);
        s.done.join().unwrap();
        assert_eq!(output(&s.sent), "hi");
        assert!(*s.disconnected.lock().unwrap());
    }

    #[test]
    #[cfg(unix)]
    fn an_echo_arrives_without_waiting_on_a_poll_interval() {
        // The point of the event loop: a keystroke wakes it at once. The old
        // polling loop could add up to 10 ms each way.
        let s = start("/bin/cat", &[]);
        std::thread::sleep(Duration::from_millis(200));
        let mut worst = Duration::ZERO;
        for i in 0..20 {
            let word = format!("w{}\n", i);
            let sent_at = Instant::now();
            s.write_tx.send(word.clone()).unwrap();
            let arrived = wait_for(&s.sent, &format!("w{}", i));
            worst = worst.max(arrived.saturating_duration_since(sent_at));
        }
        assert!(
            worst < Duration::from_millis(20),
            "slowest echo took {:?}",
            worst
        );
        s.disconnect_tx.send(()).unwrap();
        s.done.join().unwrap();
        assert!(
            !*s.disconnected.lock().unwrap(),
            "an intentional disconnect is not reported"
        );
    }

    #[test]
    #[cfg(unix)]
    fn a_fast_producer_is_coalesced_and_nothing_is_lost() {
        let s = start("/bin/sh", &["-c", "seq 1 100000"]);
        s.done.join().unwrap();
        let expected: String = (1..=100000).map(|i| format!("{}\r\n", i)).collect();
        assert_eq!(output(&s.sent), expected);
        let messages = s.sent.lock().unwrap().len();
        assert!(messages < 400, "{} messages for 590 KB", messages);
    }
}
