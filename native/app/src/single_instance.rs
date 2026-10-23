//! One running copy of the app per user. A second launch hands over to the
//! first, which brings its window forward, and exits; this is what the Tauri
//! app gets from `tauri-plugin-single-instance` (its callback focuses the
//! main window).
//!
//! Two parts:
//!
//! - **The lock** is the `single-instance` crate: a named mutex on Windows,
//!   an abstract Unix socket on Linux, `flock` on a file on macOS. The OS
//!   drops it when the process ends, crash included, so a stale lock cannot
//!   keep the app from starting.
//! - **The hand-over** is a loopback TCP listener the first copy opens on an
//!   ephemeral port. Its port and a random token are written to a file in the
//!   data directory; the second copy connects and sends `focus <token>`. The
//!   token keeps another local process from poking the window with a guess.
//!
//! The key is [`crate::IDENTIFIER`], never the Tauri app's `com.termdrop`, so
//! the two apps can run side by side; they share the database, and the core
//! sets a busy timeout so neither fails on the other's lock. The key also
//! carries the data directory, so a run with `XDG_DATA_HOME` pointed at a
//! scratch directory is a separate instance from the everyday one.

use std::hash::{Hash, Hasher};
use std::io::{BufRead, BufReader, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::time::Duration;

use single_instance::SingleInstance;

/// How long the second copy waits for the first to answer.
const HANDOVER_TIMEOUT: Duration = Duration::from_secs(2);
/// Tries at reaching it, 250 ms apart.
const HANDOVER_ATTEMPTS: usize = 12;

pub enum Launch {
    /// This is the only copy. Keep the value alive for the life of the app;
    /// each focus request arrives on `requests`.
    Primary(Primary),
    /// Another copy is running and has been asked to show itself.
    Handed,
    /// Another copy holds the lock but could not be reached.
    Unreachable(String),
}

pub struct Primary {
    _lock: Option<SingleInstance>,
    pub requests: async_channel::Receiver<()>,
}

/// Take the lock for `data_dir`, or hand over to the copy that has it. Any
/// failure to set up the lock itself is logged and treated as "only copy":
/// a broken lock must never stop the app from starting.
pub fn acquire(identifier: &str, data_dir: &Path) -> Launch {
    let rendezvous = rendezvous_file(identifier, data_dir);
    let lock = match SingleInstance::new(&lock_name(identifier, data_dir)) {
        Ok(lock) => lock,
        Err(e) => {
            tracing::warn!("Single-instance lock unavailable ({e}); continuing without it");
            return Launch::Primary(Primary {
                _lock: None,
                requests: listen(&rendezvous),
            });
        }
    };
    if lock.is_single() {
        return Launch::Primary(Primary {
            _lock: Some(lock),
            requests: listen(&rendezvous),
        });
    }
    // The first copy may hold the lock but not have published its address
    // yet (both launched at once), or the file may still be a crashed run's:
    // try for a few seconds before giving up.
    let mut last = String::new();
    for _ in 0..HANDOVER_ATTEMPTS {
        match hand_over(&rendezvous) {
            Ok(()) => return Launch::Handed,
            Err(e) => last = e,
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    Launch::Unreachable(last)
}

/// A short, per-user, per-data-directory name for the lock.
fn lock_name(identifier: &str, data_dir: &Path) -> String {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    data_dir.hash(&mut h);
    let name = format!("{identifier}-{:016x}", h.finish());
    if cfg!(target_os = "macos") {
        // The crate flocks a file at this path on macOS.
        lock_dir(data_dir)
            .join(format!("{name}.lock"))
            .to_string_lossy()
            .into_owned()
    } else {
        name
    }
}

fn lock_dir(data_dir: &Path) -> PathBuf {
    let dir = data_dir.join("termdrop");
    let _ = std::fs::create_dir_all(&dir);
    dir
}

fn rendezvous_file(identifier: &str, data_dir: &Path) -> PathBuf {
    lock_dir(data_dir).join(format!("{identifier}.instance"))
}

/// Open the listener and publish its address. On failure the app still runs;
/// it just cannot be focused by a second launch.
fn listen(rendezvous: &Path) -> async_channel::Receiver<()> {
    let (tx, rx) = async_channel::unbounded();
    let listener = match TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, 0))) {
        Ok(l) => l,
        Err(e) => {
            tracing::warn!("Single-instance listener failed: {e}");
            return rx;
        }
    };
    let port = match listener.local_addr() {
        Ok(a) => a.port(),
        Err(e) => {
            tracing::warn!("Single-instance listener has no address: {e}");
            return rx;
        }
    };
    let token = uuid::Uuid::new_v4().simple().to_string();
    if let Err(e) = write_atomically(rendezvous, &format!("{port} {token}\n")) {
        tracing::warn!("Could not write {}: {e}", rendezvous.display());
        return rx;
    }
    let expected = format!("focus {token}");
    std::thread::Builder::new()
        .name("single-instance".into())
        .spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { continue };
                let _ = stream.set_read_timeout(Some(HANDOVER_TIMEOUT));
                let mut line = String::new();
                // At most one short line: a stray client cannot make it
                // buffer without limit.
                let mut reader = BufReader::new(std::io::Read::take(&stream, 128));
                if reader.read_line(&mut line).is_ok() && line.trim_end() == expected {
                    if tx.send_blocking(()).is_err() {
                        break;
                    }
                    let _ = (&stream).write_all(b"ok\n");
                }
            }
        })
        .map_err(|e| tracing::warn!("Single-instance thread failed: {e}"))
        .ok();
    rx
}

fn write_atomically(path: &Path, contents: &str) -> std::io::Result<()> {
    let tmp = path.with_extension("instance.tmp");
    std::fs::write(&tmp, contents)?;
    std::fs::rename(&tmp, path)
}

/// Ask the running copy to show its window.
fn hand_over(rendezvous: &Path) -> Result<(), String> {
    let raw = std::fs::read_to_string(rendezvous)
        .map_err(|e| format!("cannot read {}: {e}", rendezvous.display()))?;
    let (port, token) = raw
        .trim()
        .split_once(' ')
        .ok_or_else(|| format!("{} is malformed", rendezvous.display()))?;
    let port: u16 = port
        .parse()
        .map_err(|_| format!("{} is malformed", rendezvous.display()))?;
    let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    let mut stream = TcpStream::connect_timeout(&addr, HANDOVER_TIMEOUT)
        .map_err(|e| format!("cannot reach the running copy: {e}"))?;
    let _ = stream.set_read_timeout(Some(HANDOVER_TIMEOUT));
    stream
        .write_all(format!("focus {token}\n").as_bytes())
        .map_err(|e| format!("cannot reach the running copy: {e}"))?;
    let mut reply = String::new();
    BufReader::new(stream)
        .read_line(&mut reply)
        .map_err(|e| format!("the running copy did not answer: {e}"))?;
    if reply.trim_end() == "ok" {
        Ok(())
    } else {
        Err("the running copy did not answer".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "termdrop-single-instance-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_second_launch_hands_over_and_the_first_is_asked_to_focus() {
        let dir = scratch();
        let id = "com.termdrop.test-single-instance";
        let Launch::Primary(first) = acquire(id, &dir) else {
            panic!("the first launch must own the lock");
        };
        assert!(matches!(acquire(id, &dir), Launch::Handed));
        assert_eq!(first.requests.recv_blocking(), Ok(()));
        drop(first);
        // The lock goes with the process (here, the value): a later launch
        // is the only copy again.
        assert!(matches!(acquire(id, &dir), Launch::Primary(_)));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn another_identifier_or_data_dir_is_another_instance() {
        let dir = scratch();
        let other_dir = scratch();
        let Launch::Primary(_a) = acquire("com.termdrop.test-si-a", &dir) else {
            panic!()
        };
        // The Tauri app's key is not ours, and a scratch data dir is separate.
        assert!(matches!(
            acquire("com.termdrop.test-si-b", &dir),
            Launch::Primary(_)
        ));
        assert!(matches!(
            acquire("com.termdrop.test-si-a", &other_dir),
            Launch::Primary(_)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&other_dir);
    }

    #[test]
    fn a_wrong_token_is_ignored() {
        let dir = scratch();
        let id = "com.termdrop.test-si-token";
        let Launch::Primary(first) = acquire(id, &dir) else {
            panic!()
        };
        let raw = std::fs::read_to_string(rendezvous_file(id, &dir)).unwrap();
        let port: u16 = raw.split(' ').next().unwrap().parse().unwrap();
        let mut s = TcpStream::connect((Ipv4Addr::LOCALHOST, port)).unwrap();
        s.write_all(b"focus guess\n").unwrap();
        drop(s);
        std::thread::sleep(Duration::from_millis(100));
        assert!(first.requests.try_recv().is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
