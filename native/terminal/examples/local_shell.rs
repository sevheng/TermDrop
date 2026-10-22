//! One window, one terminal, the user's `$SHELL` in a local PTY.
//!
//! `--bench` instead drives the view through three workloads and prints the
//! numbers as JSON on stdout:
//! - `seq`: `seq 1 1000000` through a real PTY (~6.9 MB of short lines, the
//!   shape that froze the webview before output coalescing);
//! - `burst`: 17 MB of coloured log lines fed straight into the view, so the
//!   PTY is out of the picture and parse + paint is all that is measured;
//! - `latency`: synthetic keystrokes dispatched through the window into `cat`
//!   on a PTY, timed until the end of the first paint that shows the echo.
//!   That stops at the end of our paint, before the compositor presents it,
//!   so it is the app's share of key-to-screen latency, not all of it.

use std::io::{Read, Write};
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui_kit::*;
use parking_lot::Mutex;
use portable_pty::{native_pty_system, CommandBuilder, MasterPty, PtySize};
use termdrop_terminal::{TerminalEvent, TerminalFeed, TerminalOptions, TerminalView};

type Writer = Arc<Mutex<Option<Box<dyn Write + Send>>>>;
type Master = Arc<Mutex<Option<Box<dyn MasterPty + Send>>>>;

/// Where the view's input and resizes go. Swappable so the benchmark can move
/// one view from PTY to PTY.
#[derive(Clone, Default)]
struct PtySlot {
    writer: Writer,
    master: Master,
    size: Arc<Mutex<(u16, u16)>>,
}

impl PtySlot {
    fn input(&self) -> Box<dyn Fn(Vec<u8>) + Send + Sync> {
        let writer = self.writer.clone();
        Box::new(move |bytes| {
            if let Some(w) = writer.lock().as_mut() {
                let _ = w.write_all(&bytes);
                let _ = w.flush();
            }
        })
    }

    fn on_resize(&self) -> Box<dyn Fn(u16, u16) + Send + Sync> {
        let master = self.master.clone();
        let size = self.size.clone();
        Box::new(move |cols, rows| {
            *size.lock() = (cols, rows);
            if let Some(m) = master.lock().as_ref() {
                let _ = m.resize(pty_size(cols, rows));
            }
        })
    }

    /// Starts `cmd` on a new PTY sized like the view, wires it into the slot,
    /// and pumps its output into `feed` on a reader thread. `done` receives
    /// the byte count when the output ends.
    fn spawn(
        &self,
        mut cmd: CommandBuilder,
        feed: TerminalFeed,
        done: Option<async_channel::Sender<u64>>,
    ) -> Box<dyn portable_pty::Child + Send + Sync> {
        let (cols, rows) = *self.size.lock();
        let pair = native_pty_system()
            .openpty(pty_size(cols.max(2), rows.max(1)))
            .expect("open a PTY");
        cmd.env("TERM", "xterm-256color");
        cmd.env("COLORTERM", "truecolor");
        if let Ok(cwd) = std::env::current_dir() {
            cmd.cwd(cwd);
        }
        let child = pair.slave.spawn_command(cmd).expect("spawn the command");
        drop(pair.slave);
        let mut reader = pair.master.try_clone_reader().expect("PTY reader");
        *self.writer.lock() = Some(pair.master.take_writer().expect("PTY writer"));
        *self.master.lock() = Some(pair.master);

        std::thread::spawn(move || {
            let mut buf = vec![0u8; 64 * 1024];
            let mut total = 0u64;
            loop {
                match reader.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        total += n as u64;
                        feed.feed(&buf[..n]);
                    }
                }
            }
            match done {
                Some(done) => {
                    let _ = done.try_send(total);
                }
                None => feed.exit(),
            }
        });
        child
    }

    fn close(&self) {
        *self.writer.lock() = None;
        *self.master.lock() = None;
    }
}

fn pty_size(cols: u16, rows: u16) -> PtySize {
    PtySize {
        rows,
        cols,
        pixel_width: 0,
        pixel_height: 0,
    }
}

struct Shell {
    terminal: Entity<TerminalView>,
    _children: Vec<Box<dyn portable_pty::Child + Send + Sync>>,
    _subscription: Subscription,
    _bench: Option<Task<()>>,
}

impl Shell {
    fn new(bench: bool, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let slot = PtySlot::default();
        let terminal = TerminalView::new(
            window,
            cx,
            TerminalOptions::default(),
            slot.input(),
            slot.on_resize(),
        );
        window.focus(&terminal.focus_handle(cx), cx);

        let subscription =
            cx.subscribe_in(&terminal, window, |_, _, event, window, cx| match event {
                TerminalEvent::TitleChanged(title) => window.set_window_title(title),
                TerminalEvent::ClipboardStore(text) => {
                    cx.write_to_clipboard(ClipboardItem::new_string(text.clone()))
                }
                TerminalEvent::Bell => {}
                TerminalEvent::Exited => cx.quit(),
            });

        let feed = terminal.read(cx).feed();
        let mut children = Vec::new();
        let bench_task = if bench {
            let handle = window.window_handle();
            Some(cx.spawn(async move |_, cx: &mut AsyncApp| {
                let report = run_bench(slot, feed, handle, cx).await;
                println!("{report}");
                cx.update(|cx| cx.quit());
            }))
        } else {
            let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into());
            children.push(slot.spawn(CommandBuilder::new(shell), feed, None));
            None
        };

        Self {
            terminal,
            _children: children,
            _subscription: subscription,
            _bench: bench_task,
        }
    }
}

impl Render for Shell {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .p_1()
            .bg(rgb(0x0F1116))
            .child(self.terminal.clone())
    }
}

async fn sleep(cx: &AsyncApp, d: Duration) {
    cx.background_executor().timer(d).await;
}

/// Waits until a painted frame shows at least `bytes` fed bytes; returns when
/// that paint ended.
async fn wait_for_paint(feed: &TerminalFeed, bytes: u64, cx: &AsyncApp) -> Instant {
    loop {
        match feed.stats().last_paint {
            Some((at, shown)) if shown >= bytes => return at,
            _ => {}
        }
        sleep(cx, Duration::from_micros(500)).await;
    }
}

struct Throughput {
    bytes: u64,
    secs: f64,
    frames: u64,
    longest_paint_ms: f64,
    longest_stall_ms: f64,
}

impl Throughput {
    fn json(&self) -> String {
        format!(
            r#"{{"bytes":{},"seconds":{:.3},"mb_per_s":{:.2},"frames":{},"fps":{:.1},"longest_paint_ms":{:.2},"longest_stall_ms":{:.2}}}"#,
            self.bytes,
            self.secs,
            self.bytes as f64 / self.secs / 1e6,
            self.frames,
            self.frames as f64 / self.secs,
            self.longest_paint_ms,
            self.longest_stall_ms,
        )
    }
}

fn finish(feed: &TerminalFeed, start: Instant, end: Instant, bytes: u64) -> Throughput {
    let stats = feed.stats();
    Throughput {
        bytes,
        secs: end.duration_since(start).as_secs_f64(),
        frames: stats.frames,
        longest_paint_ms: stats.longest_paint.as_secs_f64() * 1e3,
        longest_stall_ms: stats.longest_gap.as_secs_f64() * 1e3,
    }
}

/// 17 MB of log-shaped lines, every fourth one coloured, with a fresh
/// timestamp-like prefix so rows do not repeat.
fn burst_payload() -> Vec<u8> {
    const TARGET: usize = 17 * 1024 * 1024;
    let mut out = Vec::with_capacity(TARGET + 256);
    let mut i = 0u64;
    while out.len() < TARGET {
        let line = match i % 4 {
            0 => format!(
                "\x1b[32m2026-10-22T12:{:02}:{:02}.{:03}Z\x1b[0m \x1b[1;34mINFO\x1b[0m [worker-{}] GET /api/v1/items/{} status=200 dur={}ms\r\n",
                (i / 60) % 60, i % 60, i % 1000, i % 8, i, i % 97
            ),
            _ => format!(
                "2026-10-22T12:{:02}:{:02}.{:03}Z DEBUG [worker-{}] cache lookup key=item:{} hit={} size={}B\r\n",
                (i / 60) % 60, i % 60, i % 1000, i % 8, i, i.is_multiple_of(3), i * 37 % 4096
            ),
        };
        out.extend_from_slice(line.as_bytes());
        i += 1;
    }
    out
}

async fn run_bench(
    slot: PtySlot,
    feed: TerminalFeed,
    window: AnyWindowHandle,
    cx: &mut AsyncApp,
) -> String {
    // Let the first layout size the grid before anything is spawned.
    while *slot.size.lock() == (0, 0) {
        sleep(cx, Duration::from_millis(10)).await;
    }
    sleep(cx, Duration::from_millis(200)).await;
    let (cols, rows) = *slot.size.lock();

    // 1. seq through a PTY.
    let (tx, rx) = async_channel::bounded(1);
    let base = feed.stats().bytes_fed;
    feed.reset_stats();
    let start = Instant::now();
    let mut cmd = CommandBuilder::new("seq");
    cmd.args(["1", "1000000"]);
    let mut child = slot.spawn(cmd, feed.clone(), Some(tx));
    let bytes = rx.recv().await.unwrap_or(0);
    let end = wait_for_paint(&feed, base + bytes, cx).await;
    let seq = finish(&feed, start, end, bytes);
    let _ = child.wait();
    slot.close();
    sleep(cx, Duration::from_millis(200)).await;

    // 2. A 17 MB burst fed from a thread, no PTY.
    let payload = burst_payload();
    let bytes = payload.len() as u64;
    let base = feed.stats().bytes_fed;
    feed.reset_stats();
    let start = Instant::now();
    let (tx, rx) = async_channel::bounded(1);
    let producer = feed.clone();
    std::thread::spawn(move || {
        for chunk in payload.chunks(64 * 1024) {
            producer.feed(chunk);
        }
        let _ = tx.try_send(());
    });
    let _ = rx.recv().await;
    let end = wait_for_paint(&feed, base + bytes, cx).await;
    let burst = finish(&feed, start, end, bytes);
    feed.feed(b"\x1b[0m\x1b[2J\x1b[H");
    sleep(cx, Duration::from_millis(200)).await;

    // 3. Key-to-paint latency against `cat`, whose PTY echoes each key.
    let (tx, cat_done) = async_channel::bounded(1);
    let mut child = slot.spawn(CommandBuilder::new("cat"), feed.clone(), Some(tx));
    sleep(cx, Duration::from_millis(300)).await;
    const SAMPLES: usize = 200;
    let mut latencies = Vec::with_capacity(SAMPLES);
    for i in 0..SAMPLES {
        // Keep the line short: Enter every 60 keys.
        let key = if i % 60 == 59 { "enter" } else { "a" };
        let before = feed.stats().bytes_fed;
        let sent = Instant::now();
        let _ = window.update(cx, |_, window, cx| {
            window.dispatch_keystroke(Keystroke::parse(key).expect("valid key"), cx);
        });
        let deadline = sent + Duration::from_secs(2);
        loop {
            let stats = feed.stats();
            match stats.last_paint {
                Some((at, shown)) if shown > before && at > sent => {
                    latencies.push(at.duration_since(sent).as_secs_f64() * 1e3);
                    break;
                }
                _ => {}
            }
            if Instant::now() > deadline {
                break;
            }
            sleep(cx, Duration::from_micros(200)).await;
        }
        // Space the samples out so one echo never queues behind another.
        sleep(cx, Duration::from_millis(15)).await;
    }
    // Ctrl+C must reach the program as SIGINT rather than run GPUI Kit's
    // root Copy binding (see `termdrop_terminal::init`): `cat` ends if it did.
    let _ = window.update(cx, |_, window, cx| {
        window.dispatch_keystroke(Keystroke::parse("ctrl-c").expect("valid key"), cx);
    });
    let mut interrupted = false;
    for _ in 0..200 {
        if cat_done.try_recv().is_ok() {
            interrupted = true;
            break;
        }
        sleep(cx, Duration::from_millis(10)).await;
    }
    let _ = child.kill();
    slot.close();

    latencies.sort_by(|a, b| a.total_cmp(b));
    let pct = |p: f64| -> f64 {
        if latencies.is_empty() {
            return f64::NAN;
        }
        latencies[((latencies.len() - 1) as f64 * p).round() as usize]
    };
    let latency = format!(
        r#"{{"samples":{},"timeouts":{},"median_ms":{:.2},"p95_ms":{:.2},"max_ms":{:.2},"min_ms":{:.2}}}"#,
        latencies.len(),
        SAMPLES - latencies.len(),
        pct(0.5),
        pct(0.95),
        pct(1.0),
        pct(0.0),
    );

    format!(
        r#"{{"grid":[{cols},{rows}],"profile":"{}","ctrl_c_reached_program":{interrupted},"seq":{},"burst":{},"latency":{}}}"#,
        if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        },
        seq.json(),
        burst.json(),
        latency
    )
}

fn main() {
    let bench = std::env::args().any(|a| a == "--bench");
    application().with_assets(assets::Assets).run(move |cx| {
        init(cx);
        termdrop_terminal::init(cx);
        let bounds = Bounds::centered(None, size(px(1000.0), px(640.0)), cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            ..Default::default()
        };
        open_window(options, cx, |window, cx| {
            cx.new(|cx| Shell::new(bench, window, cx))
        })
        .expect("failed to open window");
    });
}
