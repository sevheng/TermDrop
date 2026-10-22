//! Backend timestamps for the terminal benchmark (`src/dev/termPerf.js`).
//!
//! The benchmark times a keystroke's echo from the webview, which cannot see
//! where inside the backend the time goes. `mark` records a named stage with
//! the wall-clock time; the benchmark turns recording on around its echo
//! samples, collects the marks through `dev_perf_marks`, and lines them up
//! with its own `performance.timeOrigin`-based times (one machine, one clock).
//!
//! `mark` does nothing unless recording is on, and nothing at all in release
//! builds, where `dev_perf_marks` is not registered.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

static RECORDING: AtomicBool = AtomicBool::new(false);
static MARKS: Mutex<Vec<(&'static str, f64)>> = Mutex::new(Vec::new());

/// A cap, so a benchmark that dies with recording on cannot grow this forever.
const MAX_MARKS: usize = 10_000;

/// Record that `stage` happened now, in milliseconds since the Unix epoch.
#[inline]
pub fn mark(stage: &'static str) {
    if !cfg!(debug_assertions) || !RECORDING.load(Ordering::Relaxed) {
        return;
    }
    let ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64() * 1000.0)
        .unwrap_or(0.0);
    if let Ok(mut marks) = MARKS.lock() {
        if marks.len() < MAX_MARKS {
            marks.push((stage, ms));
        }
    }
}

/// Set whether marks are recorded, and hand back (and clear) those so far.
#[cfg(debug_assertions)]
pub fn take(recording: bool) -> Vec<(&'static str, f64)> {
    RECORDING.store(recording, Ordering::Relaxed);
    MARKS
        .lock()
        .map(|mut m| std::mem::take(&mut *m))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marks_are_kept_only_while_recording() {
        take(false);
        mark("ignored");
        take(true);
        mark("a");
        mark("b");
        // Terminal tests running alongside may add their own stages.
        let ours = |marks: Vec<(&'static str, f64)>| {
            let names = ["a", "b", "ignored"];
            marks
                .into_iter()
                .filter(|m| names.contains(&m.0))
                .collect::<Vec<_>>()
        };
        let marks = ours(take(false));
        mark("ignored");
        assert_eq!(marks.iter().map(|m| m.0).collect::<Vec<_>>(), ["a", "b"]);
        assert!(marks[0].1 <= marks[1].1);
        assert!(ours(take(false)).is_empty());
    }
}
