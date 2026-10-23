//! The transfer list under the file browser. Port of
//! `composables/useSftpTransfers.js`: rows keyed by remote path (or
//! `folder:<path>` for a folder download), updated from `SftpProgress`
//! events, and removed [`LINGER`] after they finish.
//!
//! Time is passed in, so the speed arithmetic is testable; the view
//! schedules the removal when a call reports that a row just finished.

use std::time::{Duration, Instant};

use crate::panels::common::{base_name, display_name};

/// How long a finished row stays on screen.
pub const LINGER: Duration = Duration::from_millis(3000);

#[derive(Debug, Clone, PartialEq)]
pub struct Transfer {
    /// The key: the remote path, or `folder:<path>`.
    pub key: String,
    /// What the row says.
    pub label: String,
    /// A folder download: an archive with no byte counts until it is done.
    pub folder: bool,
    pub bytes: u64,
    pub total: u64,
    /// Bytes per second since the previous event.
    pub speed: f64,
    pub done: bool,
    pub failed: bool,
    last_update: Instant,
}

impl Transfer {
    /// Still waiting for the first byte count of a folder archive: the row
    /// shows a spinner and "Preparing archive...".
    pub fn preparing(&self) -> bool {
        self.total == 0 && !self.done
    }

    /// 0..=100 for the bar and the label.
    pub fn percent(&self) -> u8 {
        crate::panels::common::percent(self.bytes, self.total)
    }
}

#[derive(Debug, Default)]
pub struct Transfers {
    rows: Vec<Transfer>,
}

/// What a change needs from the view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Finished {
    /// Nothing to schedule.
    No,
    /// This row just finished: remove it after [`LINGER`].
    Remove(String),
}

impl Transfers {
    pub fn rows(&self) -> &[Transfer] {
        &self.rows
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// Apply one progress event. `handleProgress`.
    pub fn progress(&mut self, file: &str, bytes: u64, total: u64, now: Instant) -> Finished {
        if let Some(row) = self.rows.iter_mut().find(|t| t.key == file) {
            let dt = now.saturating_duration_since(row.last_update).as_secs_f64();
            if dt > 0.0 {
                row.speed = (bytes as f64 - row.bytes as f64) / dt;
            }
            row.bytes = bytes;
            row.total = total;
            row.last_update = now;
            if row.bytes >= row.total && !row.done {
                row.done = true;
                return Finished::Remove(file.to_string());
            }
            return Finished::No;
        }
        let done = total == 0 || bytes >= total;
        let name = base_name(file);
        self.rows.push(Transfer {
            key: file.to_string(),
            label: display_name(if name.is_empty() { file } else { name }),
            folder: false,
            bytes,
            total,
            speed: 0.0,
            done,
            failed: false,
            last_update: now,
        });
        if done {
            Finished::Remove(file.to_string())
        } else {
            Finished::No
        }
    }

    /// A transfer whose call failed: its row says so and goes away like a
    /// finished one. The Vue list left such a row at its last percentage
    /// forever; nothing else ever finished it.
    pub fn fail(&mut self, file: &str) -> Finished {
        match self.rows.iter_mut().find(|t| t.key == file && !t.done) {
            Some(row) => {
                row.done = true;
                row.failed = true;
                Finished::Remove(file.to_string())
            }
            None => Finished::No,
        }
    }

    /// Add the indeterminate row for a folder download; returns its key.
    /// `beginFolderTransfer`.
    pub fn begin_folder(&mut self, path: &str, name: &str, now: Instant) -> String {
        let key = format!("folder:{path}");
        self.rows.push(Transfer {
            key: key.clone(),
            label: display_name(name),
            folder: true,
            bytes: 0,
            total: 0,
            speed: 0.0,
            done: false,
            failed: false,
            last_update: now,
        });
        key
    }

    /// Finish a folder row as saved or failed. `finishFolderTransfer`.
    pub fn finish_folder(&mut self, key: &str, name: &str, ok: bool) -> Finished {
        match self.rows.iter_mut().find(|t| t.key == key) {
            Some(row) => {
                row.done = true;
                row.failed = !ok;
                row.label = format!(
                    "{} {}",
                    display_name(name),
                    if ok { "(saved)" } else { "(failed)" }
                );
                Finished::Remove(key.to_string())
            }
            None => Finished::No,
        }
    }

    /// Drop a finished row (the [`LINGER`] timer). A row with the same key
    /// that started again since is kept.
    pub fn remove(&mut self, key: &str) {
        self.rows.retain(|t| !(t.key == key && t.done));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_creates_updates_and_finishes_a_row() {
        let t0 = Instant::now();
        let mut list = Transfers::default();
        assert_eq!(list.progress("/d/a.bin", 0, 1000, t0), Finished::No);
        assert_eq!(list.rows()[0].label, "a.bin");
        let t1 = t0 + Duration::from_millis(500);
        assert_eq!(list.progress("/d/a.bin", 500, 1000, t1), Finished::No);
        assert!((list.rows()[0].speed - 1000.0).abs() < 1e-6);
        assert_eq!(list.rows()[0].percent(), 50);
        let t2 = t1 + Duration::from_millis(500);
        assert_eq!(
            list.progress("/d/a.bin", 1000, 1000, t2),
            Finished::Remove("/d/a.bin".into())
        );
        // A repeated final event does not schedule a second removal.
        assert_eq!(list.progress("/d/a.bin", 1000, 1000, t2), Finished::No);
        list.remove("/d/a.bin");
        assert!(list.is_empty());
    }

    #[test]
    fn an_empty_file_is_done_at_once() {
        let mut list = Transfers::default();
        assert_eq!(
            list.progress("/e", 0, 0, Instant::now()),
            Finished::Remove("/e".into())
        );
        assert!(list.rows()[0].done);
        assert_eq!(list.rows()[0].percent(), 100);
    }

    #[test]
    fn a_failure_finishes_the_row() {
        let mut list = Transfers::default();
        list.progress("/x", 10, 100, Instant::now());
        assert_eq!(list.fail("/x"), Finished::Remove("/x".into()));
        assert!(list.rows()[0].failed);
        assert_eq!(list.fail("/x"), Finished::No);
        assert_eq!(list.fail("/never"), Finished::No);
    }

    #[test]
    fn folder_rows_prepare_then_finish() {
        let mut list = Transfers::default();
        let key = list.begin_folder("/srv/www", "www", Instant::now());
        assert_eq!(key, "folder:/srv/www");
        assert!(list.rows()[0].preparing());
        assert_eq!(
            list.finish_folder(&key, "www", true),
            Finished::Remove(key.clone())
        );
        assert_eq!(list.rows()[0].label, "www (saved)");
        assert!(!list.rows()[0].failed);
        let k2 = list.begin_folder("/x", "x", Instant::now());
        list.finish_folder(&k2, "x", false);
        assert_eq!(list.rows()[1].label, "x (failed)");
        assert!(list.rows()[1].failed);
    }

    #[test]
    fn labels_keep_a_control_character_off_the_row() {
        let mut list = Transfers::default();
        list.progress("/t/a\nb.txt", 0, 9, Instant::now());
        assert_eq!(list.rows()[0].label, "a?b.txt");
        assert_eq!(list.rows()[0].key, "/t/a\nb.txt");
        let key = list.begin_folder("/t/d\u{1b}", "d\u{1b}", Instant::now());
        assert_eq!(list.rows()[1].label, "d?");
        list.finish_folder(&key, "d\u{1b}", true);
        assert_eq!(list.rows()[1].label, "d? (saved)");
    }

    #[test]
    fn removal_spares_a_row_that_restarted() {
        let now = Instant::now();
        let mut list = Transfers::default();
        list.progress("/a", 5, 5, now);
        list.remove("/a");
        list.progress("/a", 1, 5, now);
        list.remove("/a");
        assert_eq!(list.rows().len(), 1);
    }
}
