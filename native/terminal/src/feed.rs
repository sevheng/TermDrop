//! The state shared between the view and whoever produces output, and the
//! handle that producer feeds bytes through.
//!
//! Output is parsed on the producer's thread, so the UI thread never runs the
//! VT parser. The view is woken at most once per painted frame: the first
//! feed after a paint raises `dirty` and sends one wake-up, later feeds only
//! update the grid, and the paint lowers `dirty` again once it has taken its
//! snapshot. A burst of thousands of chunks therefore costs the UI one wake-up
//! per frame, never one per chunk.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use alacritty_terminal::event::{Event, EventListener};
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::{Config, Term};
use alacritty_terminal::vte::ansi::{Processor, StdSyncHandler};
use parking_lot::Mutex;

/// Bytes parsed per lock hold, so a 17 MB write cannot keep the renderer from
/// the grid for the whole burst.
const PARSE_SLICE: usize = 64 * 1024;

/// What the producer side tells the view.
pub(crate) enum Msg {
    /// The grid changed since the last paint.
    Wake,
    /// Something the terminal itself raised (title, bell, OSC 52, replies).
    Term(Event),
    /// The producer has no more output: the shell or channel ended.
    Exited,
    /// A synchronized update (DECSET 2026) is holding output back until this
    /// deadline. If the program never ends it, nothing else would release
    /// it, so the view sets a timer.
    SyncDeadline(Instant),
}

/// The terminal's event sink. alacritty calls it while the grid lock is held,
/// so it only queues; the view handles each event on the UI thread.
#[derive(Clone)]
pub(crate) struct Listener(pub(crate) async_channel::Sender<Msg>);

impl EventListener for Listener {
    fn send_event(&self, event: Event) {
        let _ = self.0.try_send(Msg::Term(event));
    }
}

pub(crate) struct TermState {
    pub(crate) term: Term<Listener>,
    parser: Processor<StdSyncHandler>,
}

/// The grid size handed to alacritty.
pub(crate) struct GridSize {
    pub(crate) cols: usize,
    pub(crate) lines: usize,
}

impl Dimensions for GridSize {
    fn total_lines(&self) -> usize {
        self.lines
    }
    fn screen_lines(&self) -> usize {
        self.lines
    }
    fn columns(&self) -> usize {
        self.cols
    }
}

pub(crate) struct Shared {
    pub(crate) state: FairMutex<TermState>,
    dirty: AtomicBool,
    /// A `SyncDeadline` has been sent and its timer has not fired yet.
    sync_watched: AtomicBool,
    tx: async_channel::Sender<Msg>,
    pub(crate) stats: Stats,
}

impl Shared {
    pub(crate) fn new(scrollback: usize, tx: async_channel::Sender<Msg>) -> Arc<Self> {
        let config = Config {
            scrolling_history: scrollback,
            ..Config::default()
        };
        let term = Term::new(
            config,
            &GridSize {
                cols: 80,
                lines: 24,
            },
            Listener(tx.clone()),
        );
        Arc::new(Self {
            state: FairMutex::new(TermState {
                term,
                parser: Processor::new(),
            }),
            dirty: AtomicBool::new(false),
            sync_watched: AtomicBool::new(false),
            tx,
            stats: Stats::default(),
        })
    }

    /// Marks the grid changed and wakes the view unless a wake-up is already
    /// waiting for the next paint.
    pub(crate) fn mark_dirty(&self) {
        if !self.dirty.swap(true, Ordering::AcqRel) {
            let _ = self.tx.try_send(Msg::Wake);
        }
    }

    /// Releases a synchronized update whose deadline has passed. Returns the
    /// deadline still pending, if any, so the caller can wait again.
    pub(crate) fn flush_expired_sync(&self) -> Option<Instant> {
        let mut state = self.state.lock();
        let TermState { term, parser } = &mut *state;
        let pending = match parser.sync_timeout().sync_timeout() {
            Some(deadline) if deadline <= Instant::now() => {
                parser.stop_sync(term);
                None
            }
            other => other,
        };
        drop(state);
        if pending.is_none() {
            self.sync_watched.store(false, Ordering::Release);
            self.mark_dirty();
        }
        pending
    }

    /// Called by the paint once it has its snapshot.
    pub(crate) fn clear_dirty(&self) {
        self.dirty.store(false, Ordering::Release);
    }
}

/// Counters for the benchmark and for anyone diagnosing a slow terminal.
#[derive(Default)]
pub(crate) struct Stats {
    bytes_fed: AtomicU64,
    frames: AtomicU64,
    longest_paint_ns: AtomicU64,
    longest_gap_ns: AtomicU64,
    /// When the last frame was painted, and how many bytes it showed.
    last_paint: Mutex<Option<(Instant, u64)>>,
}

impl Stats {
    pub(crate) fn record_paint(&self, started: Instant, bytes_shown: u64) {
        let now = Instant::now();
        self.frames.fetch_add(1, Ordering::Relaxed);
        let took = now.duration_since(started).as_nanos() as u64;
        self.longest_paint_ns.fetch_max(took, Ordering::Relaxed);
        let mut last = self.last_paint.lock();
        if let Some((prev, _)) = *last {
            let gap = started.duration_since(prev).as_nanos() as u64;
            self.longest_gap_ns.fetch_max(gap, Ordering::Relaxed);
        }
        *last = Some((now, bytes_shown));
    }

    fn snapshot(&self) -> FrameStats {
        FrameStats {
            bytes_fed: self.bytes_fed.load(Ordering::Relaxed),
            frames: self.frames.load(Ordering::Relaxed),
            longest_paint: Duration::from_nanos(self.longest_paint_ns.load(Ordering::Relaxed)),
            longest_gap: Duration::from_nanos(self.longest_gap_ns.load(Ordering::Relaxed)),
            last_paint: *self.last_paint.lock(),
        }
    }

    fn reset(&self) {
        self.frames.store(0, Ordering::Relaxed);
        self.longest_paint_ns.store(0, Ordering::Relaxed);
        self.longest_gap_ns.store(0, Ordering::Relaxed);
        *self.last_paint.lock() = None;
    }
}

/// A point-in-time copy of a terminal's counters.
#[derive(Clone, Copy, Debug)]
pub struct FrameStats {
    /// Every byte ever fed (not reset by [`TerminalFeed::reset_stats`]).
    pub bytes_fed: u64,
    /// Frames painted since the last reset.
    pub frames: u64,
    /// The slowest prepaint + paint of the terminal element since the reset.
    pub longest_paint: Duration,
    /// The longest time between the end of one paint and the start of the
    /// next since the reset. While output is streaming this is the stall a
    /// user would see.
    pub longest_gap: Duration,
    /// When the last frame finished painting, and `bytes_fed` as of its
    /// snapshot: a frame shows a byte once its count reaches it.
    pub last_paint: Option<(Instant, u64)>,
}

/// A cheap, cloneable handle for pushing output into a terminal from any
/// thread. Parsing happens on the calling thread.
#[derive(Clone)]
pub struct TerminalFeed(pub(crate) Arc<Shared>);

impl TerminalFeed {
    /// Parses `bytes` into the terminal and schedules a repaint (at most one
    /// per frame, however often this is called).
    pub fn feed(&self, bytes: &[u8]) {
        for slice in bytes.chunks(PARSE_SLICE) {
            let mut state = self.0.state.lock();
            let TermState { term, parser } = &mut *state;
            parser.advance(term, slice);
            let sync_deadline = parser.sync_timeout().sync_timeout();
            drop(state);
            if let Some(deadline) = sync_deadline {
                if !self.0.sync_watched.swap(true, Ordering::AcqRel) {
                    let _ = self.0.tx.try_send(Msg::SyncDeadline(deadline));
                }
            }
            self.0
                .stats
                .bytes_fed
                .fetch_add(slice.len() as u64, Ordering::Relaxed);
            self.0.mark_dirty();
        }
    }

    /// Tells the view the output has ended (the shell exited or the channel
    /// closed). The view emits [`crate::TerminalEvent::Exited`].
    pub fn exit(&self) {
        let _ = self.0.tx.try_send(Msg::Exited);
    }

    /// The terminal's counters.
    pub fn stats(&self) -> FrameStats {
        self.0.stats.snapshot()
    }

    /// Zeroes the frame counters (not `bytes_fed`).
    pub fn reset_stats(&self) {
        self.0.stats.reset();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alacritty_terminal::index::{Column, Line};

    fn first_row(shared: &Shared) -> String {
        let state = shared.state.lock();
        let row = &state.term.grid()[Line(0)];
        (0..5).map(|c| row[Column(c)].c).collect()
    }

    #[test]
    fn many_feeds_wake_the_view_once() {
        let (tx, rx) = async_channel::unbounded();
        let shared = Shared::new(100, tx);
        let feed = TerminalFeed(shared.clone());
        for _ in 0..1000 {
            feed.feed(b"x");
        }
        let wakes = std::iter::from_fn(|| rx.try_recv().ok())
            .filter(|m| matches!(m, Msg::Wake))
            .count();
        assert_eq!(wakes, 1);
        // After a paint, the next feed wakes it again.
        shared.clear_dirty();
        feed.feed(b"y");
        assert!(matches!(rx.try_recv(), Ok(Msg::Wake)));
        assert_eq!(feed.stats().bytes_fed, 1001);
    }

    #[test]
    fn an_unfinished_synchronized_update_is_released_at_its_deadline() {
        let (tx, rx) = async_channel::unbounded();
        let shared = Shared::new(100, tx);
        let feed = TerminalFeed(shared.clone());
        feed.feed(b"\x1b[?2026hhello");
        assert_eq!(first_row(&shared), "     ", "held back during the update");
        let deadline = std::iter::from_fn(|| rx.try_recv().ok())
            .find_map(|m| match m {
                Msg::SyncDeadline(d) => Some(d),
                _ => None,
            })
            .expect("a deadline is reported");
        std::thread::sleep(deadline.saturating_duration_since(Instant::now()));
        assert_eq!(shared.flush_expired_sync(), None);
        assert_eq!(first_row(&shared), "hello");
    }
}
