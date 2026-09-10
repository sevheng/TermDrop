use futures_util::stream::TryStreamExt;
use mongodb::{options::ClientOptions, Client};
use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::process::Child;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::{Emitter, Window};

/// Lock through a poisoned mutex instead of panicking.
///
/// These mutexes guard a registry of cancel flags and child handles and a
/// buffer of stderr lines — a panic elsewhere does not make either
/// semantically corrupt. Propagating the poison would disable every later
/// MongoDB operation for the lifetime of the process, which is worse than
/// carrying on.
pub(crate) fn lock_or_recover<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn set_mongo_child(
    mongo_ops: &Arc<Mutex<HashMap<String, crate::MongoOpHandle>>>,
    op_id: &str,
    mut child: Child,
) {
    let mut ops = lock_or_recover(mongo_ops);
    match ops.get_mut(op_id) {
        Some(handle) => handle.child = Some(child),
        None => {
            // The op was unregistered while this child was running. Dropping a
            // Child does not kill it on Unix, so it would keep running with
            // nothing able to cancel it.
            tracing::warn!(
                op_id = op_id,
                "mongo child has no registry entry; killing it"
            );
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn take_mongo_child(
    mongo_ops: &Arc<Mutex<HashMap<String, crate::MongoOpHandle>>>,
    op_id: &str,
) -> Option<Child> {
    let mut ops = lock_or_recover(mongo_ops);
    ops.get_mut(op_id).and_then(|h| h.child.take())
}

fn is_retryable_error(stderr: &str) -> bool {
    let lower = stderr.to_ascii_lowercase();
    [
        "i/o timeout",
        "connection reset",
        "connection refused",
        "network is unreachable",
        "temporary failure in name resolution",
        "server selection timeout",
    ]
    .iter()
    .any(|&s| lower.contains(s))
}

/// Emits the `mongodb-sync-*` events for one operation. Each method
/// reproduces the exact payload shape the frontend expects on that path.
struct ProgressEmitter<'a> {
    window: &'a Window,
    op_id: &'a str,
    db: &'a str,
}

impl ProgressEmitter<'_> {
    /// Pulse while a CLI tool runs: no collection, estimated percent.
    fn pulse(&self, stage: &str, percent: u64) {
        let _ = self.window.emit(
            "mongodb-sync-progress",
            serde_json::json!({
                "opId": self.op_id,
                "db": self.db,
                "collection": "",
                "stage": stage,
                "synced": 0,
                "total": 1,
                "percent": percent,
            }),
        );
    }

    /// Real progress parsed from a CLI tool's stderr. `detail` is the tool's
    /// own "current / total", which is documents for mongodump and byte sizes
    /// for mongorestore, so it is passed through as text.
    fn cli(&self, stage: &str, ns: &str, detail: &str, percent: f64) {
        let collection = ns.split_once('.').map(|(_, c)| c).unwrap_or(ns);
        let _ = self.window.emit(
            "mongodb-sync-progress",
            serde_json::json!({
                "opId": self.op_id,
                "db": self.db,
                "collection": collection,
                "stage": stage,
                "synced": 0,
                "total": 1,
                "percent": percent.round() as u64,
                "detail": detail,
            }),
        );
    }

    /// Terminal event once a CLI tool has finished.
    fn done(&self) {
        let _ = self.window.emit(
            "mongodb-sync-progress",
            serde_json::json!({
                "opId": self.op_id,
                "db": self.db,
                "collection": "",
                "stage": "done",
                "synced": 1,
                "total": 1,
                "percent": 100,
            }),
        );
    }

    /// Per-collection progress from the driver streaming fallback.
    fn collection(&self, collection: &str, stage: &str, synced: u64, total: u64) {
        let _ = self.window.emit(
            "mongodb-sync-progress",
            serde_json::json!({
                "opId": self.op_id,
                "db": self.db,
                "collection": collection,
                "stage": stage,
                "synced": synced,
                "total": total,
            }),
        );
    }

    /// The CLI fast path failed and the driver fallback is about to run, which
    /// copies documents only. The user has to be told before it happens.
    fn degraded(&self, message: &str) {
        let _ = self.window.emit(
            "mongodb-sync-warning",
            serde_json::json!({
                "opId": self.op_id,
                "db": self.db,
                "message": message,
            }),
        );
    }

    fn cancelled(&self) {
        let _ = self.window.emit(
            "mongodb-sync-cancelled",
            serde_json::json!({"opId": self.op_id, "db": self.db}),
        );
    }
}

/// A line of mongodump/mongorestore stderr that carries progress.
///
/// The tools print a progress bar only after the first three seconds, so short
/// operations emit nothing but the `Writing`/`Restoring` and `Done` markers and
/// the caller has to fall back to a time-based estimate.
#[derive(Debug, PartialEq)]
enum CliLine {
    /// `[####....]  db.coll  225947/400000  (56.5%)`
    ///
    /// `current`/`total` are documents for mongodump and human-readable byte
    /// sizes ("2.00MB") for mongorestore, so they stay strings.
    Progress {
        ns: String,
        current: String,
        total: String,
        percent: f64,
    },
    /// A collection started: `writing db.coll to ...` / `restoring db.coll from ...`
    Started {
        ns: String,
    },
    /// A collection finished: `done dumping db.coll (...)` / `finished restoring db.coll (...)`
    Finished {
        ns: String,
    },
    Other,
}

/// Parse one stderr line from mongodump/mongorestore.
///
/// The literal formats these match are pinned by tests using output captured
/// from the bundled binaries; if a tools upgrade changes them, those tests fail
/// rather than the progress bar silently reverting to a time-based estimate.
fn parse_cli_line(line: &str) -> CliLine {
    // Lines are "<RFC3339>\t<message>". If a redrawn bar ever arrives with
    // carriage returns, only the last segment is current.
    let after_ts = line.split_once('\t').map(|(_, m)| m).unwrap_or(line);
    let msg = after_ts.rsplit('\r').next().unwrap_or(after_ts).trim();

    if let Some(rest) = msg.strip_prefix('[') {
        // [bar]  ns  current/total  (pct%)
        let Some((_, tail)) = rest.split_once(']') else {
            return CliLine::Other;
        };
        let mut fields = tail.split_whitespace();
        let (Some(ns), Some(ratio), Some(pct)) = (fields.next(), fields.next(), fields.next())
        else {
            return CliLine::Other;
        };
        let Some((current, total)) = ratio.split_once('/') else {
            return CliLine::Other;
        };
        let percent = pct
            .trim_matches(|c| c == '(' || c == ')' || c == '%')
            .parse::<f64>()
            .unwrap_or(0.0);
        return CliLine::Progress {
            ns: ns.to_string(),
            current: current.to_string(),
            total: total.to_string(),
            percent,
        };
    }

    for (prefix, sep) in [("writing ", " to "), ("restoring ", " from ")] {
        if let Some(rest) = msg.strip_prefix(prefix) {
            let ns = rest.split(sep).next().unwrap_or(rest).trim();
            if !ns.is_empty() {
                return CliLine::Started { ns: ns.to_string() };
            }
        }
    }

    for prefix in ["done dumping ", "finished restoring "] {
        if let Some(rest) = msg.strip_prefix(prefix) {
            let ns = rest.split(" (").next().unwrap_or(rest).trim();
            if !ns.is_empty() {
                return CliLine::Finished { ns: ns.to_string() };
            }
        }
    }

    CliLine::Other
}

/// The most recent real progress seen on a tool's stderr.
#[derive(Clone, Default)]
struct CliProgress {
    ns: String,
    detail: String,
    percent: f64,
    finished: u64,
}

/// Cap on retained stderr lines. Only the last 30 are ever reported, but a long
/// operation with verbose output would otherwise grow this without bound.
const MAX_STDERR_LINES: usize = 200;

/// Scale one collection's percentage across the whole operation.
///
/// The tools report progress per collection, so a dump of four collections
/// would otherwise run 0-100% four times. When the number of collections is not
/// known, the current collection's own percentage is the best available.
fn combined_percent(p: &CliProgress, expected_collections: u64) -> f64 {
    if expected_collections <= 1 {
        return p.percent.clamp(0.0, 100.0);
    }
    let done = p.finished.min(expected_collections) as f64;
    let current = if p.finished >= expected_collections {
        0.0
    } else {
        p.percent.clamp(0.0, 100.0) / 100.0
    };
    (((done + current) / expected_collections as f64) * 100.0).clamp(0.0, 100.0)
}

/// A temporary YAML file holding the connection string for the CLI tools'
/// `--config` flag, so the credential never appears in the process command
/// line where any local user could read it from `ps` or `/proc/<pid>/cmdline`.
///
/// Deleted on drop, so success, failure, cancellation and a panic inside the
/// blocking task all converge on one removal site. Do not unlink it anywhere
/// else: `run_with_retry` rebuilds the command once per attempt, and the file
/// has to outlive every attempt.
struct MongoConfigFile {
    path: std::path::PathBuf,
}

impl MongoConfigFile {
    fn new(uri: &str) -> Result<Self, String> {
        let path =
            std::env::temp_dir().join(format!("termdrop-mongo-{}.yaml", uuid::Uuid::new_v4()));
        let contents = format!("uri: \"{}\"\n", yaml_escape(uri));

        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create_new(true);
        #[cfg(unix)]
        {
            // Set the mode at creation: a later set_permissions would leave a
            // window in which the credential is world-readable.
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        // On Windows there are no mode bits. The per-user %TEMP% ACL already
        // restricts this to the current user; emulating 0600 would mean taking
        // a windows-sys dependency and hand-rolling an ACL for no real gain.

        let mut file = opts
            .open(&path)
            .map_err(|e| format!("create mongo config file: {}", e))?;
        std::io::Write::write_all(&mut file, contents.as_bytes())
            .map_err(|e| format!("write mongo config file: {}", e))?;

        Ok(Self { path })
    }

    fn arg(&self) -> String {
        format!("--config={}", self.path.to_string_lossy())
    }
}

impl Drop for MongoConfigFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// A temp file path removed on drop. Used for the intermediate sync archive,
/// which previously leaked whenever the task returned early or panicked.
struct TempPath {
    path: std::path::PathBuf,
}

impl TempPath {
    fn new(file_name: String) -> Self {
        Self {
            path: std::env::temp_dir().join(file_name),
        }
    }
}

impl Drop for TempPath {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// Escape a value for a YAML double-quoted scalar.
fn yaml_escape(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

/// Remove `termdrop-mongo-*.yaml` files left behind by a previous run that was
/// killed before its guard could drop. Best effort, and only files old enough
/// that no live operation could still be using them.
pub fn sweep_stale_config_files() {
    let Ok(entries) = std::fs::read_dir(std::env::temp_dir()) else {
        return;
    };
    let cutoff = std::time::Duration::from_secs(3600);
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.starts_with("termdrop-mongo-") || !name.ends_with(".yaml") {
            continue;
        }
        let stale = entry
            .metadata()
            .and_then(|m| m.modified())
            .map(|t| t.elapsed().map(|age| age > cutoff).unwrap_or(false))
            .unwrap_or(false);
        if stale {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

/// Normalize a URI for the CLI tools: add authSource when needed and strip
/// the database path, which mongodump/mongorestore reject alongside --db.
fn prepare_cli_uri(uri: &str) -> String {
    strip_mongo_uri_database(&normalize_mongo_uri(uri))
}

/// `--archive=<path>` plus `--gzip` when the path ends in `.gz`.
fn push_archive_args(cmd: &mut std::process::Command, path: &str) {
    cmd.arg(format!("--archive={}", path));
    if path.ends_with(".gz") {
        cmd.arg("--gzip");
    }
}

/// One `--nsInclude=<db>.<collection>` per collection.
fn push_ns_includes(cmd: &mut std::process::Command, db: &str, collections: &[String]) {
    for coll in collections {
        cmd.arg(format!("--nsInclude={}.{}", db, coll));
    }
}

/// How to restrict a mongodump to a subset of a database's collections.
///
/// mongodump 100.9.4 has no `--nsInclude` (only mongorestore does), so a
/// multi-collection subset has to be expressed as its complement: exclude
/// everything that was not selected. A per-collection `-c` loop would be exact
/// but cannot work for `--archive`, where each run truncates the file, so
/// excludes keep folder and archive dumps on one code path.
#[derive(Debug, PartialEq, Eq)]
enum DumpFilter {
    /// Dump the whole database.
    None,
    /// Exactly one collection: `-c`.
    Only(String),
    /// Everything except these.
    Exclude(Vec<String>),
}

/// Roughly the point at which a Windows command line (32767 chars) is at risk.
const MAX_EXCLUDE_ARG_BYTES: usize = 24_000;

/// Choose the narrowest filter expressing `selected` out of `all`.
///
/// Note: a collection created between listing `all` and mongodump starting is
/// not in the exclusion set and will be included. That is strictly better than
/// the previous behaviour, where every unselected collection was included, but
/// there is no way to say "only these" with this mongodump.
fn dump_filter(all: &[String], selected: &[String]) -> Result<DumpFilter, String> {
    if selected.is_empty() {
        return Ok(DumpFilter::None);
    }
    if selected.len() == 1 {
        // Exact, and needs no listing of the database.
        return Ok(DumpFilter::Only(selected[0].clone()));
    }

    let mut excluded: Vec<String> = all
        .iter()
        .filter(|c| !selected.contains(c))
        .cloned()
        .collect();
    excluded.sort();

    if excluded.is_empty() {
        return Ok(DumpFilter::None);
    }

    let bytes: usize = excluded.iter().map(|c| c.len() + 22).sum();
    if bytes > MAX_EXCLUDE_ARG_BYTES {
        return Err(format!(
            "too many collections to filter ({} would have to be excluded). \
Dump the whole database, or select fewer collections.",
            excluded.len()
        ));
    }

    Ok(DumpFilter::Exclude(excluded))
}

/// Build the `mongodump` command line.
///
/// `conn_arg` is the whole connection argument (`--uri=...` or `--config=...`)
/// so the builder stays unaware of how the credential reaches the tool.
/// Split out from the spawning closure so the argument vector is testable.
fn build_dump_cmd(
    tool: std::path::PathBuf,
    conn_arg: &str,
    db: &str,
    filter: &DumpFilter,
    output: &str,
    is_archive: bool,
) -> std::process::Command {
    let mut cmd = std::process::Command::new(tool);
    cmd.arg(conn_arg).arg(format!("--db={}", db));

    if is_archive {
        push_archive_args(&mut cmd, output);
    } else {
        cmd.arg("--gzip").arg(format!("--out={}", output));
    }

    match filter {
        DumpFilter::None => {}
        DumpFilter::Only(coll) => {
            cmd.arg("-c").arg(coll);
        }
        DumpFilter::Exclude(colls) => {
            for coll in colls {
                cmd.arg(format!("--excludeCollection={}", coll));
            }
        }
    }

    cmd
}

/// Build the `mongorestore` command line for a dump folder or archive.
#[allow(clippy::too_many_arguments)]
fn build_restore_cmd(
    tool: std::path::PathBuf,
    conn_arg: &str,
    db: &str,
    collections: &[String],
    input: &str,
    is_archive: bool,
    is_direct_db: bool,
    drop_first: bool,
) -> std::process::Command {
    let mut cmd = std::process::Command::new(tool);
    cmd.arg(conn_arg);
    if drop_first {
        cmd.arg("--drop");
    }

    if is_archive {
        push_archive_args(&mut cmd, input);
    } else {
        // Dump folders produced by this app are gzip-compressed.
        cmd.arg("--gzip").arg(input);
    }

    if is_direct_db {
        // Path is a single DB dump; --db tells mongorestore the target DB.
        cmd.arg(format!("--db={}", db));
        push_ns_includes(&mut cmd, db, collections);
    } else if !db.is_empty() {
        // Path is a dump root; filter with --nsInclude instead of deprecated --db.
        if !collections.is_empty() {
            push_ns_includes(&mut cmd, db, collections);
        } else {
            cmd.arg(format!("--nsInclude={}.*", db));
        }
    }

    cmd
}

/// Build the `mongorestore` command line for a single archive file restoring
/// an explicit `db.collection` namespace list.
fn build_restore_archive_cmd(
    tool: std::path::PathBuf,
    conn_arg: &str,
    includes: &[String],
    input: &str,
    drop_first: bool,
) -> std::process::Command {
    let mut cmd = std::process::Command::new(tool);
    cmd.arg(conn_arg);
    if drop_first {
        cmd.arg("--drop");
    }
    push_archive_args(&mut cmd, input);

    for ns in includes {
        cmd.arg(format!("--nsInclude={}", ns));
    }

    cmd
}

/// Run one CLI tool invocation on the blocking pool with retry, then emit
/// the terminal "done" event. `panic_label` names the task if it panics.
#[allow(clippy::too_many_arguments)]
async fn run_cli_op<F>(
    panic_label: &str,
    tool: &'static str,
    stage: &'static str,
    window: Window,
    cancelled: Arc<AtomicBool>,
    mongo_ops: Arc<Mutex<HashMap<String, crate::MongoOpHandle>>>,
    op_id: String,
    progress_db: String,
    uri: String,
    expected_collections: u64,
    mut build_cmd: F,
) -> Result<(), String>
where
    F: FnMut(&str) -> Result<std::process::Command, String> + Send + 'static,
{
    tokio::task::spawn_blocking(move || {
        // Owned out here, not inside build_cmd: run_with_retry rebuilds the
        // command once per attempt, so creating it there would leak a file per
        // retry. One guard, dropped when this task ends however it ends.
        let config = MongoConfigFile::new(&uri)?;
        let config_arg = config.arg();

        run_with_retry(
            tool,
            stage,
            3,
            expected_collections,
            &window,
            &cancelled,
            &mongo_ops,
            &op_id,
            &progress_db,
            move || build_cmd(&config_arg),
        )?;
        ProgressEmitter {
            window: &window,
            op_id: &op_id,
            db: &progress_db,
        }
        .done();
        Ok(())
    })
    .await
    .map_err(|e| format!("{} task panicked: {}", panic_label, e))?
}

#[allow(clippy::too_many_arguments)]
fn run_with_retry<F>(
    label: &str,
    stage: &str,
    max_retries: u32,
    expected_collections: u64,
    window: &Window,
    cancelled: &Arc<AtomicBool>,
    mongo_ops: &Arc<Mutex<HashMap<String, crate::MongoOpHandle>>>,
    op_id: &str,
    db: &str,
    mut build_cmd: F,
) -> Result<(), String>
where
    F: FnMut() -> Result<std::process::Command, String>,
{
    let start = Instant::now();
    let mut last_emit = Instant::now();
    // The bar must never walk backwards, whether the number came from the tool
    // or from the time-based estimate.
    let mut highest: f64 = 0.0;

    let progress = ProgressEmitter { window, op_id, db };

    progress.pulse(stage, 0);

    'attempt: for attempt in 0..max_retries {
        let mut cmd = build_cmd()?;
        cmd.stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped());

        let mut child = cmd
            .spawn()
            .map_err(|e| format!("{} failed to start: {} (is it installed?)", label, e))?;

        let stderr = child.stderr.take().unwrap();
        let stderr_lines: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let stderr_lines_clone = Arc::clone(&stderr_lines);
        // Last-writer-wins slot for real progress parsed off stderr.
        let latest: Arc<Mutex<Option<CliProgress>>> = Arc::new(Mutex::new(None));
        let latest_clone = Arc::clone(&latest);
        let stderr_thread = std::thread::spawn(move || {
            let reader = BufReader::new(stderr);
            let mut finished: u64 = 0;
            // Stop at the first read error; `flatten()` would spin forever on a
            // persistently failing pipe.
            for line in reader.lines().map_while(Result::ok) {
                // Record real progress in a slot the poll loop reads on its own
                // cadence. Emitting from here would put one IPC message on the
                // channel per line the tools print.
                match parse_cli_line(&line) {
                    CliLine::Progress {
                        ns,
                        current,
                        total,
                        percent,
                    } => {
                        *lock_or_recover(&latest_clone) = Some(CliProgress {
                            ns,
                            detail: format!("{} / {}", current, total),
                            percent,
                            finished,
                        });
                    }
                    CliLine::Finished { ns } => {
                        finished += 1;
                        *lock_or_recover(&latest_clone) = Some(CliProgress {
                            ns,
                            detail: String::new(),
                            percent: 100.0,
                            finished,
                        });
                    }
                    CliLine::Started { .. } | CliLine::Other => {}
                }

                // Only the last 30 lines are ever read back, but this used to
                // grow without bound for the life of the operation.
                let mut lines = lock_or_recover(&stderr_lines_clone);
                if lines.len() >= MAX_STDERR_LINES {
                    lines.remove(0);
                }
                lines.push(line);
            }
        });

        set_mongo_child(mongo_ops, op_id, child);

        loop {
            let mut child = take_mongo_child(mongo_ops, op_id)
                .ok_or_else(|| format!("{} child missing from registry", label))?;

            if cancelled.load(Ordering::Relaxed) {
                let _ = child.kill();
                let _ = child.wait();
                let _ = stderr_thread.join();
                progress.cancelled();
                return Err("cancelled".into());
            }

            match child.try_wait() {
                Ok(Some(status)) => {
                    let _ = stderr_thread.join();
                    if status.success() {
                        let lines = lock_or_recover(&stderr_lines);
                        let recent: Vec<_> = lines
                            .iter()
                            .rev()
                            .take(30)
                            .map(|l| redact_uris_in_text(l))
                            .collect();
                        tracing::debug!(
                            label = label,
                            stage = stage,
                            exit_code = status.code(),
                            stderr = ?recent,
                            "command succeeded"
                        );
                        return Ok(());
                    }
                    let lines = lock_or_recover(&stderr_lines);
                    // Redact here so neither the log, the retry decision, nor the
                    // toast this becomes can carry the connection string.
                    let err = lines
                        .iter()
                        .rev()
                        .take(30)
                        .map(|l| redact_uris_in_text(l))
                        .collect::<Vec<_>>()
                        .join("\n");
                    if is_retryable_error(&err) && attempt < max_retries - 1 {
                        progress.pulse("retrying", 0);
                        let mut cancelled_during_sleep = false;
                        for _ in 0..10 {
                            if cancelled.load(Ordering::Relaxed) {
                                cancelled_during_sleep = true;
                                break;
                            }
                            std::thread::sleep(Duration::from_millis(100));
                        }
                        if cancelled_during_sleep {
                            progress.cancelled();
                            return Err("cancelled".into());
                        }
                        continue 'attempt;
                    }
                    return Err(format!("{} failed: {}", label, err));
                }
                Ok(None) => {
                    set_mongo_child(mongo_ops, op_id, child);
                    if last_emit.elapsed() >= Duration::from_millis(500) {
                        match lock_or_recover(&latest).clone() {
                            // Real progress from the tool itself.
                            Some(p) => {
                                let percent = combined_percent(&p, expected_collections);
                                highest = highest.max(percent);
                                progress.cli(stage, &p.ns, &p.detail, highest);
                            }
                            // The tools print no bar for the first three seconds,
                            // so estimate until one arrives. Never let the estimate
                            // walk back past real progress already shown.
                            None => {
                                let elapsed_ms = start.elapsed().as_millis() as u64;
                                let pulse = std::cmp::min(95, elapsed_ms / 100) as f64;
                                highest = highest.max(pulse);
                                progress.pulse(stage, highest as u64);
                            }
                        }
                        last_emit = Instant::now();
                    }
                    std::thread::sleep(Duration::from_millis(100));
                }
                Err(e) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    let _ = stderr_thread.join();
                    return Err(format!("failed to wait for {}: {}", label, e));
                }
            }
        }
    }

    Ok(())
}

/// Resolve the path to a MongoDB tool binary.
/// Tries bundled binary first, then falls back to PATH.
fn resolve_mongo_tool(name: &str) -> Result<std::path::PathBuf, String> {
    #[cfg(target_os = "windows")]
    let name = format!("{}.exe", name);

    // Try bundled binary next to the executable
    if let Ok(exe_path) = std::env::current_exe() {
        if let Some(exe_dir) = exe_path.parent() {
            // Same directory as executable (Windows, Linux AppImage/standalone,
            // and cargo's target/debug or target/release directories)
            let bundled = exe_dir.join(name);
            if bundled.exists() {
                return Ok(bundled);
            }

            // Cargo sometimes places test/run binaries in target/<profile>/deps/;
            // the profile directory (e.g. target/debug) is the parent.
            if let Some(profile_dir) = exe_dir.parent() {
                let bundled = profile_dir.join(name);
                if bundled.exists() {
                    return Ok(bundled);
                }
            }

            // macOS app bundle: Contents/MacOS/ -> Contents/Resources/
            #[cfg(target_os = "macos")]
            {
                let macos_bundle = exe_dir.join("../Resources").join(&name);
                if macos_bundle.exists() {
                    return Ok(macos_bundle);
                }
            }

            // Linux .deb/AppImage: usr/bin/ -> usr/lib/TermDrop/
            #[cfg(target_os = "linux")]
            {
                let linux_bundle = exe_dir.join("../lib/TermDrop").join(name);
                if linux_bundle.exists() {
                    return Ok(linux_bundle);
                }
            }
        }
    }

    // Fall back to PATH
    Ok(std::path::PathBuf::from(&name))
}

/// Returns true if the MongoDB URI has a path component after the authority.
fn mongo_uri_has_path(uri: &str) -> bool {
    let Some(scheme_end) = uri.find("://") else {
        return false;
    };
    let authority_start = scheme_end + 3;
    uri[authority_start..]
        .find(&['/', '?', '#'][..])
        .map(|idx| uri.as_bytes()[authority_start + idx] == b'/')
        .unwrap_or(false)
}

/// The byte range of a URI's authority (everything between `://` and the
/// first `/`, `?` or `#`). Shared by the userinfo helpers below.
fn authority_range(uri: &str) -> Option<(usize, usize)> {
    let scheme_end = uri.find("://")?;
    let start = scheme_end + 3;
    let end = uri[start..]
        .find(&['/', '?', '#'][..])
        .map(|idx| start + idx)
        .unwrap_or(uri.len());
    Some((start, end))
}

/// The byte range of the userinfo inside the authority, if the URI has one.
///
/// Takes the *last* `@` in the authority: a percent-encoded password cannot
/// contain a literal `@`, but a host list can't either, so the last one is the
/// separator in every valid form.
fn userinfo_range(uri: &str) -> Option<(usize, usize)> {
    let (start, end) = authority_range(uri)?;
    let at = uri[start..end].rfind('@')? + start;
    Some((start, at))
}

/// Split a URI into a password-free URI and its password.
///
/// The password is returned **still percent-encoded**, exactly as it appeared,
/// so putting it back is a pure splice with no encoding decisions to get wrong
/// and a password containing a literal `%40` round-trips unchanged.
pub fn split_mongo_password(uri: &str) -> (String, Option<String>) {
    let Some((start, at)) = userinfo_range(uri) else {
        return (uri.to_string(), None);
    };
    let userinfo = &uri[start..at];
    // The first `:` separates user from password; any later one is inside the
    // password and must be left alone.
    let Some(colon) = userinfo.find(':') else {
        return (uri.to_string(), None);
    };
    let password = &userinfo[colon + 1..];
    if password.is_empty() {
        return (uri.to_string(), None);
    }
    let stripped = format!("{}{}{}", &uri[..start], &userinfo[..colon], &uri[at..]);
    (stripped, Some(password.to_string()))
}

/// Whether a URI names a user, and so expects a password to go with it.
pub fn uri_expects_password(uri: &str) -> bool {
    userinfo_range(uri).is_some_and(|(start, at)| !uri[start..at].is_empty())
}

/// Splice a password back into a URI produced by [`split_mongo_password`].
///
/// If the URI already carries a password it is left alone, and a URI with no
/// username has nowhere to put one.
pub fn with_mongo_password(uri: &str, password: &str) -> String {
    let Some((start, at)) = userinfo_range(uri) else {
        return uri.to_string();
    };
    let userinfo = &uri[start..at];
    if userinfo.contains(':') || userinfo.is_empty() {
        return uri.to_string();
    }
    format!("{}{}:{}{}", &uri[..start], userinfo, password, &uri[at..])
}

/// Replace a URI's credentials with `***` so it can be logged.
///
/// Connection strings reach the log through command arguments and through
/// mongodump/mongorestore stderr, both of which echo them verbatim.
pub fn redact_mongo_uri(uri: &str) -> String {
    let Some((start, at)) = userinfo_range(uri) else {
        return uri.to_string();
    };
    let userinfo = &uri[start..at];
    let masked = match userinfo.find(':') {
        Some(_) => "***:***",
        None => "***",
    };
    format!("{}{}{}", &uri[..start], masked, &uri[at..])
}

/// Redact every MongoDB connection string embedded in free text.
///
/// The CLI tools echo the connection string back in their own error output, so
/// their stderr reaches both the log and the user-facing toast. Scan for the
/// scheme and redact each URI-shaped token in place.
pub fn redact_uris_in_text(text: &str) -> String {
    const SCHEMES: [&str; 2] = ["mongodb+srv://", "mongodb://"];
    let mut out = String::with_capacity(text.len());
    let mut rest = text;

    'outer: loop {
        // Find whichever scheme appears first in what is left.
        let mut best: Option<(usize, &str)> = None;
        for scheme in SCHEMES {
            if let Some(idx) = rest.find(scheme) {
                if best.is_none_or(|(b, _)| idx < b) {
                    best = Some((idx, scheme));
                }
            }
        }
        let Some((idx, _)) = best else { break 'outer };

        // The URI runs to the next character that cannot appear in one.
        let tail = &rest[idx..];
        let end = tail
            .find(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | ',' | ')'))
            .unwrap_or(tail.len());

        out.push_str(&rest[..idx]);
        out.push_str(&redact_mongo_uri(&tail[..end]));
        rest = &tail[end..];
    }

    out.push_str(rest);
    out
}

/// Ensure a MongoDB URI authenticates against the `admin` database when
/// credentials are provided but no authSource is set. Root users created via
/// `MONGO_INITDB_ROOT_USERNAME` live in `admin`, so tools/drivers fail without
/// this when the connection string points at another database.
fn normalize_mongo_uri(uri: &str) -> String {
    // No credentials -> nothing to fix.
    if !uri.contains('@') {
        return uri.to_string();
    }
    // Already has an auth source -> leave it alone.
    if uri.to_ascii_lowercase().contains("authsource=") {
        return uri.to_string();
    }

    // Split base and query; make sure there is a '/' before the query string.
    let (base, query) = match uri.find('?') {
        Some(idx) => (&uri[..idx], Some(&uri[idx + 1..])),
        None => (uri, None),
    };
    let base = if mongo_uri_has_path(base) || base.ends_with('/') {
        base.to_string()
    } else {
        format!("{}/", base)
    };

    let new_query = match query {
        Some(q) if !q.is_empty() => format!("authSource=admin&{}", q),
        _ => "authSource=admin".to_string(),
    };

    format!("{}?{}", base, new_query)
}

/// Strip the default database path from a MongoDB URI so it can be used with
/// `--db` on the mongodump / mongorestore command line. Those tools reject a
/// URI whose database differs from the one passed via `--db`.
fn strip_mongo_uri_database(uri: &str) -> String {
    let Some(scheme_end) = uri.find("://") else {
        return uri.to_string();
    };
    let authority_start = scheme_end + 3;
    let authority_and_rest = &uri[authority_start..];

    let sep_pos = authority_and_rest.find(&['/', '?', '#'][..]);

    // Base includes the scheme and authority, stopping right before the separator.
    let base = sep_pos
        .map(|pos| &uri[..authority_start + pos])
        .unwrap_or(uri);
    let rest = sep_pos.map(|pos| &authority_and_rest[pos..]).unwrap_or("");

    match rest.as_bytes().first() {
        Some(b'/') => {
            // Remove the database path, keeping any query/fragment.
            if let Some(qpos) = rest.find(&['?', '#'][..]) {
                let query = &rest[qpos..];
                format!("{}/{}", base, query)
            } else {
                format!("{}/", base)
            }
        }
        Some(b'?') | Some(b'#') => {
            // No database path, but ensure there is a slash before the query/fragment.
            format!("{}/{}", base, rest)
        }
        _ => {
            // No path or query at all; add a trailing slash for consistency.
            format!("{}/", base)
        }
    }
}

/// How long to wait for a server before giving up. The driver's own default is
/// 30s, which is a long time to stare at a spinner for a host that is simply
/// not there.
const SERVER_SELECTION_TIMEOUT: Duration = Duration::from_secs(8);

/// Build a driver client with explicit timeouts.
///
/// Clients are pooled by the caller: each one owns a connection pool and a
/// background topology monitor, so creating one per call is wasteful.
pub async fn build_client(uri: &str) -> Result<Client, String> {
    let uri = normalize_mongo_uri(uri);
    let mut options = ClientOptions::parse(&uri)
        .await
        .map_err(|e| format!("parse uri: {}", redact_uris_in_text(&e.to_string())))?;
    options.server_selection_timeout = Some(SERVER_SELECTION_TIMEOUT);
    options.connect_timeout = Some(SERVER_SELECTION_TIMEOUT);
    options.app_name = Some("TermDrop".to_string());
    Client::with_options(options)
        .map_err(|e| format!("create client: {}", redact_uris_in_text(&e.to_string())))
}

/// Hard ceiling on documents returned in one page, whatever the caller asks.
pub const MAX_FIND_LIMIT: i64 = 200;
/// Hard ceiling on the serialized size of one page.
const MAX_RESPONSE_BYTES: usize = 8 * 1024 * 1024;

/// One page of documents, each already serialized as canonical extended JSON.
#[derive(serde::Serialize)]
pub struct FindResult {
    /// Pre-serialized so Tauri's own JSON layer cannot re-coerce the BSON
    /// representations (Int64 vs Double, Decimal128) we just preserved.
    pub documents: Vec<String>,
    /// The page was cut short by the size cap.
    pub truncated: bool,
    pub elapsed_ms: u64,
}

#[derive(serde::Serialize)]
pub struct CountResult {
    pub count: u64,
    /// An unfiltered count uses collection metadata and is O(1) but may lag.
    pub estimated: bool,
}

/// Parse a user-typed filter into a BSON document.
///
/// `serde_json::from_str::<Document>` gives true extended-JSON semantics on
/// this bson version — `{"$oid": ...}` becomes an ObjectId rather than a nested
/// document — which is what makes `{"_id": {"$oid": "..."}}` match. That
/// behaviour is pinned by a test.
pub fn parse_filter(text: &str) -> Result<mongodb::bson::Document, String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Ok(mongodb::bson::Document::new());
    }

    let mut doc: mongodb::bson::Document =
        serde_json::from_str(trimmed).map_err(|e| format!("filter is not valid JSON: {}", e))?;

    coerce_id_hex(&mut doc);
    Ok(doc)
}

/// Treat `{"_id": "<24 hex chars>"}` as an ObjectId.
///
/// Typing the bare hex is the single most common mistake, and its failure mode
/// — zero results and no error — is the most confusing one.
fn coerce_id_hex(doc: &mut mongodb::bson::Document) {
    let Some(mongodb::bson::Bson::String(s)) = doc.get("_id") else {
        return;
    };
    if s.len() != 24 || !s.chars().all(|c| c.is_ascii_hexdigit()) {
        return;
    }
    if let Ok(oid) = mongodb::bson::oid::ObjectId::parse_str(s) {
        doc.insert("_id", oid);
    }
}

/// Serialize a document as canonical extended JSON.
///
/// Canonical, not relaxed: relaxed collapses Int32/Int64/Double and renders
/// Decimal128 as a bare JSON number, so the viewer would show a *wrong* value
/// with no indication that it had been changed.
fn to_canonical_json(doc: mongodb::bson::Document) -> String {
    mongodb::bson::Bson::Document(doc)
        .into_canonical_extjson()
        .to_string()
}

/// Read a page of documents from a collection.
pub async fn find_documents(
    client: &Client,
    db: &str,
    collection: &str,
    filter: mongodb::bson::Document,
    sort: Option<mongodb::bson::Document>,
    projection: Option<mongodb::bson::Document>,
    skip: u64,
    limit: i64,
) -> Result<FindResult, String> {
    let started = Instant::now();
    let coll = client
        .database(db)
        .collection::<mongodb::bson::Document>(collection);

    let mut find = coll
        .find(filter)
        .skip(skip)
        .limit(limit.clamp(1, MAX_FIND_LIMIT));
    if let Some(sort) = sort {
        find = find.sort(sort);
    }
    if let Some(projection) = projection {
        find = find.projection(projection);
    }

    let mut cursor = find
        .await
        .map_err(|e| format!("find: {}", redact_uris_in_text(&e.to_string())))?;

    let mut documents = Vec::new();
    let mut bytes = 0usize;
    let mut truncated = false;

    while let Some(doc) = cursor
        .try_next()
        .await
        .map_err(|e| format!("read documents: {}", redact_uris_in_text(&e.to_string())))?
    {
        let json = to_canonical_json(doc);
        bytes += json.len();
        documents.push(json);
        // Stop *after* pushing, so a single oversized document is still
        // returned rather than silently vanishing.
        if bytes >= MAX_RESPONSE_BYTES {
            truncated = true;
            break;
        }
    }

    Ok(FindResult {
        documents,
        truncated,
        elapsed_ms: started.elapsed().as_millis() as u64,
    })
}

/// Count documents, exactly when filtered and by metadata when not.
pub async fn count_documents(
    client: &Client,
    db: &str,
    collection: &str,
    filter: mongodb::bson::Document,
) -> Result<CountResult, String> {
    let coll = client
        .database(db)
        .collection::<mongodb::bson::Document>(collection);

    if filter.is_empty() {
        // O(1) from collection metadata: a real count of a 10M-document
        // collection would scan it on every page render.
        let count = coll
            .estimated_document_count()
            .await
            .map_err(|e| format!("count: {}", redact_uris_in_text(&e.to_string())))?;
        return Ok(CountResult {
            count,
            estimated: true,
        });
    }

    let count = coll
        .count_documents(filter)
        .await
        .map_err(|e| format!("count: {}", redact_uris_in_text(&e.to_string())))?;
    Ok(CountResult {
        count,
        estimated: false,
    })
}

/// A collection's indexes, as canonical extended JSON.
pub async fn list_indexes(
    client: &Client,
    db: &str,
    collection: &str,
) -> Result<Vec<String>, String> {
    let specs = client
        .database(db)
        .collection::<mongodb::bson::Document>(collection)
        .list_indexes()
        .await
        .map_err(|e| format!("list indexes: {}", redact_uris_in_text(&e.to_string())))?
        .try_collect::<Vec<_>>()
        .await
        .map_err(|e| format!("read indexes: {}", redact_uris_in_text(&e.to_string())))?;

    Ok(specs
        .into_iter()
        .map(|ix| {
            let mut doc = mongodb::bson::doc! { "keys": ix.keys };
            if let Some(options) = ix.options {
                if let Ok(mongodb::bson::Bson::Document(o)) = mongodb::bson::to_bson(&options) {
                    doc.insert("options", o);
                }
            }
            to_canonical_json(doc)
        })
        .collect())
}

/// Storage statistics for one collection.
pub async fn collection_stats(
    client: &Client,
    db: &str,
    collection: &str,
) -> Result<String, String> {
    let pipeline = vec![
        mongodb::bson::doc! { "$collStats": { "storageStats": {} } },
        mongodb::bson::doc! { "$project": {
            "count": "$storageStats.count",
            "size": "$storageStats.size",
            "storageSize": "$storageStats.storageSize",
            "avgObjSize": "$storageStats.avgObjSize",
            "nindexes": "$storageStats.nindexes",
            "totalIndexSize": "$storageStats.totalIndexSize",
        }},
    ];

    let mut cursor = client
        .database(db)
        .collection::<mongodb::bson::Document>(collection)
        .aggregate(pipeline)
        .await
        .map_err(|e| format!("collection stats: {}", redact_uris_in_text(&e.to_string())))?;

    let doc = cursor
        .try_next()
        .await
        .map_err(|e| {
            format!(
                "read collection stats: {}",
                redact_uris_in_text(&e.to_string())
            )
        })?
        .ok_or_else(|| "collection stats returned nothing".to_string())?;

    Ok(to_canonical_json(doc))
}

/// List database names on an existing client.
pub async fn list_databases_with(client: &Client) -> Result<Vec<String>, String> {
    client
        .list_database_names()
        .await
        .map_err(|e| format!("list databases: {}", redact_uris_in_text(&e.to_string())))
}

/// List collection names in one database on an existing client.
pub async fn list_collections_with(client: &Client, db: &str) -> Result<Vec<String>, String> {
    client
        .database(db)
        .list_collection_names()
        .await
        .map_err(|e| format!("list collections: {}", redact_uris_in_text(&e.to_string())))
}

pub async fn list_collections(uri: &str, db: &str) -> Result<Vec<String>, String> {
    let uri = normalize_mongo_uri(uri);
    let options = ClientOptions::parse(&uri)
        .await
        .map_err(|e| format!("parse uri: {}", e))?;
    let client = Client::with_options(options).map_err(|e| format!("create client: {}", e))?;
    let collections = client
        .database(db)
        .list_collection_names()
        .await
        .map_err(|e| format!("list collections: {}", e))?;
    Ok(collections)
}

/// Sync collections from remote to local.
/// Tries mongodump+mongorestore first, falls back to driver streaming.
pub async fn sync_collections(
    window: Window,
    cancelled: Arc<AtomicBool>,
    mongo_ops: Arc<Mutex<HashMap<String, crate::MongoOpHandle>>>,
    op_id: String,
    remote_uri: &str,
    local_uri: &str,
    db: &str,
    collections: Vec<String>,
    drop_first: bool,
    allow_driver_fallback: bool,
) -> Result<(), String> {
    let remote_uri = normalize_mongo_uri(remote_uri);
    let local_uri = normalize_mongo_uri(local_uri);

    // CLI tools reject a URI whose database path differs from --db, so strip it.
    let remote_uri_cli = strip_mongo_uri_database(&remote_uri);
    let local_uri_cli = strip_mongo_uri_database(&local_uri);

    // Try CLI fast path first
    match try_cli_sync(
        window.clone(),
        cancelled.clone(),
        mongo_ops.clone(),
        op_id.clone(),
        &remote_uri_cli,
        &local_uri_cli,
        db,
        &collections,
        drop_first,
    )
    .await
    {
        Ok(()) => {
            return Ok(());
        }
        Err(e) => {
            if e == "cancelled" {
                // try_cli_sync already emitted mongodb-sync-cancelled
                return Err(e);
            }
            let reason = redact_uris_in_text(&e);
            tracing::warn!(
                "CLI sync failed ({}), falling back to a document-only driver copy",
                reason
            );

            if !allow_driver_fallback {
                return Err(format!(
                    "mongodump/mongorestore failed and the document-only fallback is \
disabled: {}",
                    reason
                ));
            }

            ProgressEmitter {
                window: &window,
                op_id: &op_id,
                db,
            }
            .degraded(&format!(
                "mongodump/mongorestore could not run ({}). Falling back to a \
document-only copy: indexes, collection options and validators will not be copied.",
                reason
            ));
        }
    }

    // Fallback to driver-based streaming
    driver_sync(
        window,
        cancelled,
        &op_id,
        &remote_uri,
        &local_uri,
        db,
        collections,
        drop_first,
    )
    .await
}

async fn try_cli_sync(
    window: Window,
    cancelled: Arc<AtomicBool>,
    mongo_ops: Arc<Mutex<HashMap<String, crate::MongoOpHandle>>>,
    op_id: String,
    remote_uri: &str,
    local_uri: &str,
    db: &str,
    collections: &[String],
    drop_first: bool,
) -> Result<(), String> {
    let remote_uri = remote_uri.to_string();
    let local_uri = local_uri.to_string();
    let db = db.to_string();
    let collections = collections.to_vec();

    tokio::task::spawn_blocking(move || {
        let archive = TempPath::new(format!("termdrop-sync-{}.gz", uuid::Uuid::new_v4()));
        let archive_path_str = archive.path.to_string_lossy().to_string();

        // One config file per endpoint, both dropped when this task ends.
        let remote_config = MongoConfigFile::new(&remote_uri)?;
        let local_config = MongoConfigFile::new(&local_uri)?;
        let remote_config_arg = remote_config.arg();
        let local_config_arg = local_config.arg();

        // Step 1: mongodump from remote (dump whole DB; mongorestore will filter collections)
        let dump_result = run_with_retry(
            "mongodump",
            "sync",
            3,
            // The dump is unfiltered (mongorestore filters), so the tool walks
            // the whole database and its collection count is not known here.
            0,
            &window,
            &cancelled,
            &mongo_ops,
            &op_id,
            &db,
            || {
                let mut dump_cmd = std::process::Command::new(resolve_mongo_tool("mongodump")?);
                dump_cmd
                    .arg(&remote_config_arg)
                    .arg(format!("--db={}", db))
                    .arg("--gzip")
                    .arg(format!("--archive={}", archive_path_str));
                Ok(dump_cmd)
            },
        );

        dump_result?;

        // Step 2: mongorestore to local
        let restore_result = run_with_retry(
            "mongorestore",
            "sync",
            3,
            collections.len() as u64,
            &window,
            &cancelled,
            &mongo_ops,
            &op_id,
            &db,
            || {
                let mut restore_cmd =
                    std::process::Command::new(resolve_mongo_tool("mongorestore")?);
                restore_cmd
                    .arg(&local_config_arg)
                    .arg("--gzip")
                    .arg(format!("--archive={}", archive_path_str));

                if drop_first {
                    restore_cmd.arg("--drop");
                }

                // Only restore selected collections
                push_ns_includes(&mut restore_cmd, &db, &collections);

                Ok(restore_cmd)
            },
        );

        restore_result
    })
    .await
    .map_err(|e| format!("sync task panicked: {}", e))?
}

async fn driver_sync(
    window: Window,
    cancelled: Arc<AtomicBool>,
    op_id: &str,
    remote_uri: &str,
    local_uri: &str,
    db: &str,
    collections: Vec<String>,
    drop_first: bool,
) -> Result<(), String> {
    let remote_options = ClientOptions::parse(remote_uri)
        .await
        .map_err(|e| format!("parse remote uri: {}", e))?;
    let remote_client =
        Client::with_options(remote_options).map_err(|e| format!("remote client: {}", e))?;

    let local_options = ClientOptions::parse(local_uri)
        .await
        .map_err(|e| format!("parse local uri: {}", e))?;
    let local_client =
        Client::with_options(local_options).map_err(|e| format!("local client: {}", e))?;

    let remote_db = remote_client.database(db);
    let local_db = local_client.database(db);
    let progress = ProgressEmitter {
        window: &window,
        op_id,
        db,
    };

    for collection_name in &collections {
        if cancelled.load(Ordering::Relaxed) {
            progress.cancelled();
            return Err("cancelled".into());
        }

        progress.collection(collection_name, "count", 0, 0);

        let remote_coll = remote_db.collection::<mongodb::bson::Document>(collection_name);
        let local_coll = local_db.collection::<mongodb::bson::Document>(collection_name);

        // Get total count for progress
        let total = remote_coll
            .count_documents(mongodb::bson::doc! {})
            .await
            .map_err(|e| format!("count {}: {}", collection_name, e))?;

        if drop_first {
            let _ = local_coll.drop().await;
        }

        let mut cursor = remote_coll
            .find(mongodb::bson::doc! {})
            .await
            .map_err(|e| format!("find {}: {}", collection_name, e))?;

        let mut batch: Vec<mongodb::bson::Document> = Vec::new();
        const BATCH_SIZE: usize = 1000;
        let mut synced: u64 = 0;
        let mut last_emit = std::time::Instant::now();

        while let Some(doc) = cursor
            .try_next()
            .await
            .map_err(|e| format!("cursor {}: {}", collection_name, e))?
        {
            if cancelled.load(Ordering::Relaxed) {
                progress.cancelled();
                return Err("cancelled".into());
            }

            batch.push(doc);
            synced += 1;

            if batch.len() >= BATCH_SIZE {
                local_coll
                    .insert_many(&batch)
                    .await
                    .map_err(|e| format!("insert {}: {}", collection_name, e))?;
                batch.clear();
            }

            // Emit progress every 500ms or on batch boundary
            if last_emit.elapsed() >= Duration::from_millis(500) {
                progress.collection(collection_name, "copy", synced, total);
                last_emit = std::time::Instant::now();
            }
        }

        if !batch.is_empty() {
            local_coll
                .insert_many(&batch)
                .await
                .map_err(|e| format!("insert {}: {}", collection_name, e))?;
        }

        // mongorestore recreates indexes after the documents; do the same so the
        // fallback is merely slower rather than lossy.
        progress.collection(collection_name, "indexes", synced, total);
        copy_indexes(&remote_coll, &local_coll, collection_name).await?;

        progress.collection(collection_name, "done", synced, total);
    }

    Ok(())
}

/// Recreate the source collection's indexes on the destination.
///
/// The `_id` index always exists on both sides and cannot be created again, so
/// it is skipped. A failure here is reported: a silently un-indexed collection
/// is exactly the problem this function exists to fix.
async fn copy_indexes(
    remote_coll: &mongodb::Collection<mongodb::bson::Document>,
    local_coll: &mongodb::Collection<mongodb::bson::Document>,
    collection_name: &str,
) -> Result<(), String> {
    let specs: Vec<_> = remote_coll
        .list_indexes()
        .await
        .map_err(|e| format!("list indexes for {}: {}", collection_name, e))?
        .try_collect::<Vec<_>>()
        .await
        .map_err(|e| format!("read indexes for {}: {}", collection_name, e))?
        .into_iter()
        .filter(|ix| ix.keys != mongodb::bson::doc! { "_id": 1 })
        .collect();

    if specs.is_empty() {
        return Ok(());
    }

    local_coll
        .create_indexes(specs)
        .await
        .map_err(|e| format!("create indexes for {}: {}", collection_name, e))?;

    Ok(())
}

/// Dump selected collections from remote to a local directory or archive using mongodump.
pub async fn dump_collections(
    window: Window,
    cancelled: Arc<AtomicBool>,
    mongo_ops: Arc<Mutex<HashMap<String, crate::MongoOpHandle>>>,
    op_id: String,
    remote_uri: &str,
    db: &str,
    collections: Vec<String>,
    output_dir: &str,
    is_archive: bool,
) -> Result<(), String> {
    // Resolve the filter before the URI is stripped for the CLI: list_collections
    // normalizes the URI itself and must not receive the database-less form.
    let filter = if collections.len() > 1 {
        let all = list_collections(remote_uri, db).await?;
        dump_filter(&all, &collections)?
    } else {
        dump_filter(&[], &collections)?
    };

    // How many collections the tool will walk, for whole-operation progress.
    let expected_collections = match &filter {
        DumpFilter::Only(_) => 1,
        DumpFilter::Exclude(_) => collections.len() as u64,
        DumpFilter::None => 0,
    };

    let remote_uri = prepare_cli_uri(remote_uri);
    let db = db.to_string();
    let output_dir = output_dir.to_string();
    let cmd_db = db.clone();

    run_cli_op(
        "dump",
        "mongodump",
        "dump",
        window,
        cancelled,
        mongo_ops,
        op_id,
        db,
        remote_uri,
        expected_collections,
        move |config_arg| {
            Ok(build_dump_cmd(
                resolve_mongo_tool("mongodump")?,
                config_arg,
                &cmd_db,
                &filter,
                &output_dir,
                is_archive,
            ))
        },
    )
    .await
}

/// Restore selected collections from a local directory or archive to remote using mongorestore.
pub async fn restore_collections(
    window: Window,
    cancelled: Arc<AtomicBool>,
    mongo_ops: Arc<Mutex<HashMap<String, crate::MongoOpHandle>>>,
    op_id: String,
    remote_uri: &str,
    db: &str,
    collections: Vec<String>,
    input_dir: &str,
    is_archive: bool,
    drop_first: bool,
) -> Result<(), String> {
    let remote_uri = prepare_cli_uri(remote_uri);
    let db = db.to_string();
    let input_dir = input_dir.to_string();
    let has_db = !db.is_empty();
    let has_collections = !collections.is_empty();
    let progress_db = if has_db {
        db.clone()
    } else {
        "all".to_string()
    };

    // A "direct DB folder" contains BSON files directly (e.g. /dump/mydb/*.bson.gz).
    // A "dump root" contains DB subfolders (e.g. /dump/<db>/*.bson.gz).
    let is_direct_db = !is_archive
        && has_db
        && !collect_bson_collections(std::path::Path::new(&input_dir)).is_empty();
    let restore_dir = input_dir.clone();

    tracing::debug!(
        db = %db,
        is_archive = is_archive,
        is_direct_db = is_direct_db,
        has_db = has_db,
        has_collections = has_collections,
        input_dir = %input_dir,
        restore_dir = %restore_dir,
        collections = ?collections,
        "restore_collections called"
    );

    if !is_archive {
        let contents: Vec<String> = std::fs::read_dir(&restore_dir)
            .map(|entries| {
                entries
                    .filter_map(|e| e.ok())
                    .map(|e| e.path().to_string_lossy().to_string())
                    .collect()
            })
            .unwrap_or_default();
        tracing::debug!(restore_dir = %restore_dir, contents = ?contents, "restore folder contents");
    }

    let cmd_db = db.clone();
    run_cli_op(
        "restore",
        "mongorestore",
        "restore",
        window,
        cancelled,
        mongo_ops,
        op_id,
        progress_db,
        remote_uri,
        collections.len() as u64,
        move |config_arg| {
            let cmd = build_restore_cmd(
                resolve_mongo_tool("mongorestore")?,
                config_arg,
                &cmd_db,
                &collections,
                if is_archive { &input_dir } else { &restore_dir },
                is_archive,
                is_direct_db,
                drop_first,
            );

            tracing::debug!(
                program = %cmd.get_program().to_string_lossy(),
                args = ?cmd
                    .get_args()
                    .map(|a| redact_uris_in_text(&a.to_string_lossy()))
                    .collect::<Vec<_>>(),
                "mongorestore command"
            );

            Ok(cmd)
        },
    )
    .await
}

/// Restore selected namespaces from a single archive file to remote using mongorestore.
/// This runs once for the whole archive, filtering with --nsInclude.
pub async fn restore_archive(
    window: Window,
    cancelled: Arc<AtomicBool>,
    mongo_ops: Arc<Mutex<HashMap<String, crate::MongoOpHandle>>>,
    op_id: String,
    remote_uri: &str,
    includes: Vec<String>,
    input_path: &str,
    drop_first: bool,
) -> Result<(), String> {
    let remote_uri = prepare_cli_uri(remote_uri);
    let input_path = input_path.to_string();

    run_cli_op(
        "restore archive",
        "mongorestore",
        "restore",
        window,
        cancelled,
        mongo_ops,
        op_id,
        "archive".to_string(),
        remote_uri,
        includes.len() as u64,
        move |config_arg| {
            Ok(build_restore_archive_cmd(
                resolve_mongo_tool("mongorestore")?,
                config_arg,
                &includes,
                &input_path,
                drop_first,
            ))
        },
    )
    .await
}

#[derive(serde::Serialize, PartialEq, Eq, PartialOrd, Ord)]
pub struct RestoreSourceCollection {
    pub name: String,
}

#[derive(serde::Serialize)]
pub struct RestoreSourceDb {
    pub name: String,
    pub collections: Vec<RestoreSourceCollection>,
}

fn collect_bson_collections(dir: &std::path::Path) -> Vec<RestoreSourceCollection> {
    let mut collections = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if let Some(stem) = name.strip_suffix(".bson.gz") {
                collections.push(RestoreSourceCollection {
                    name: stem.to_string(),
                });
            } else if let Some(stem) = name.strip_suffix(".bson") {
                collections.push(RestoreSourceCollection {
                    name: stem.to_string(),
                });
            }
        }
    }
    collections.sort();
    collections
}

/// Scan a folder for mongodump-style data and return the databases/collections found.
#[tauri::command]
pub fn scan_restore_folder(path: String) -> Result<Vec<RestoreSourceDb>, String> {
    let root = std::path::Path::new(&path);
    if !root.is_dir() {
        return Err("Selected path is not a folder".to_string());
    }

    // If the chosen folder itself contains BSON files, treat it as a single DB folder.
    let direct_collections = collect_bson_collections(root);
    if !direct_collections.is_empty() {
        let db_name = root
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "unknown".to_string());
        return Ok(vec![RestoreSourceDb {
            name: db_name,
            collections: direct_collections,
        }]);
    }

    // Otherwise look for DB subfolders.
    let mut dbs = Vec::new();
    for e in std::fs::read_dir(root)
        .map_err(|e| e.to_string())?
        .flatten()
    {
        if e.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            let collections = collect_bson_collections(&e.path());
            if !collections.is_empty() {
                dbs.push(RestoreSourceDb {
                    name: e.file_name().to_string_lossy().to_string(),
                    collections,
                });
            }
        }
    }

    if dbs.is_empty() {
        return Err("No MongoDB dump data found in the selected folder".to_string());
    }

    dbs.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(dbs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_mongo_tools_are_resolved() {
        let dump = resolve_mongo_tool("mongodump").expect("failed to resolve mongodump");
        assert!(dump.exists(), "mongodump not found at {:?}", dump);
        let restore = resolve_mongo_tool("mongorestore").expect("failed to resolve mongorestore");
        assert!(restore.exists(), "mongorestore not found at {:?}", restore);
    }

    #[test]
    fn normalize_mongo_uri_adds_auth_source_for_root_user() {
        assert_eq!(
            normalize_mongo_uri("mongodb://root:example@localhost:27017"),
            "mongodb://root:example@localhost:27017/?authSource=admin"
        );
        assert_eq!(
            normalize_mongo_uri("mongodb://root:example@localhost:27017/termdrop_test"),
            "mongodb://root:example@localhost:27017/termdrop_test?authSource=admin"
        );
        assert_eq!(
            normalize_mongo_uri("mongodb://root:example@localhost:27017/?retryWrites=true"),
            "mongodb://root:example@localhost:27017/?authSource=admin&retryWrites=true"
        );
        assert_eq!(
            normalize_mongo_uri("mongodb://root:example@localhost:27017/?"),
            "mongodb://root:example@localhost:27017/?authSource=admin"
        );
        assert_eq!(
            normalize_mongo_uri("mongodb://root:example@localhost:27017?retryWrites=true"),
            "mongodb://root:example@localhost:27017/?authSource=admin&retryWrites=true"
        );
    }

    #[test]
    fn normalize_mongo_uri_leaves_uris_without_credentials_or_existing_auth_source_alone() {
        assert_eq!(
            normalize_mongo_uri("mongodb://localhost:27017"),
            "mongodb://localhost:27017"
        );
        assert_eq!(
            normalize_mongo_uri("mongodb://root:example@localhost:27017/?authSource=custom"),
            "mongodb://root:example@localhost:27017/?authSource=custom"
        );
    }

    #[test]
    fn prepare_cli_uri_normalizes_then_strips_database() {
        assert_eq!(
            prepare_cli_uri("mongodb://root:example@localhost:27017/termdrop_test"),
            "mongodb://root:example@localhost:27017/?authSource=admin"
        );
        assert_eq!(
            prepare_cli_uri("mongodb://localhost/db"),
            "mongodb://localhost/"
        );
    }

    fn args(cmd: &std::process::Command) -> Vec<String> {
        cmd.get_args()
            .map(|a| a.to_string_lossy().to_string())
            .collect()
    }

    fn dummy() -> std::path::PathBuf {
        std::path::PathBuf::from("mongodump")
    }

    #[test]
    fn dump_cmd_folder_uses_out_and_gzip() {
        let cmd = build_dump_cmd(
            dummy(),
            "--uri=U",
            "mydb",
            &DumpFilter::None,
            "/tmp/out",
            false,
        );
        assert_eq!(
            args(&cmd),
            vec!["--uri=U", "--db=mydb", "--gzip", "--out=/tmp/out"]
        );
    }

    #[test]
    fn dump_cmd_single_collection_uses_dash_c() {
        let cmd = build_dump_cmd(
            dummy(),
            "--uri=U",
            "mydb",
            &DumpFilter::Only("logs".to_string()),
            "/tmp/out",
            false,
        );
        assert!(args(&cmd).windows(2).any(|w| w == ["-c", "logs"]));
    }

    #[test]
    fn dump_cmd_archive_path_gets_archive_args() {
        let cmd = build_dump_cmd(
            dummy(),
            "--uri=U",
            "mydb",
            &DumpFilter::None,
            "/tmp/d.gz",
            true,
        );
        let a = args(&cmd);
        assert!(a.contains(&"--archive=/tmp/d.gz".to_string()));
        assert!(a.contains(&"--gzip".to_string()));
        assert!(!a.iter().any(|s| s.starts_with("--out=")));
    }

    #[test]
    fn restore_cmd_direct_db_folder_sets_db_and_ns_includes() {
        let cmd = build_restore_cmd(
            dummy(),
            "--uri=U",
            "mydb",
            &["a".to_string(), "b".to_string()],
            "/dump/mydb",
            false,
            true,
            true,
        );
        let a = args(&cmd);
        assert!(a.contains(&"--db=mydb".to_string()));
        assert!(a.contains(&"--nsInclude=mydb.a".to_string()));
        assert!(a.contains(&"--nsInclude=mydb.b".to_string()));
    }

    #[test]
    fn restore_cmd_dump_root_without_collections_includes_whole_db() {
        let cmd = build_restore_cmd(dummy(), "--uri=U", "mydb", &[], "/dump", false, false, true);
        let a = args(&cmd);
        assert!(a.contains(&"--nsInclude=mydb.*".to_string()));
        assert!(!a.contains(&"--db=mydb".to_string()));
    }

    #[test]
    fn config_file_holds_the_uri_and_disappears_on_drop() {
        let uri = "mongodb://admin:p@ss\"w\\rd@localhost:27017/?authSource=admin";
        let path = {
            let cfg = MongoConfigFile::new(uri).expect("create config");
            let text = std::fs::read_to_string(&cfg.path).expect("read config");

            // The tools parse exactly one key; quoting must survive a password
            // containing a quote and a backslash.
            assert!(text.starts_with("uri: \""), "unexpected shape: {}", text);
            assert!(text.contains("admin:p@ss"));
            assert!(cfg.arg().starts_with("--config="));

            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mode = std::fs::metadata(&cfg.path).unwrap().permissions().mode();
                assert_eq!(
                    mode & 0o777,
                    0o600,
                    "config file must not be readable by others"
                );
            }

            cfg.path.clone()
        };
        assert!(!path.exists(), "config file outlived its guard");
    }

    #[test]
    fn yaml_escape_escapes_backslash_before_quote() {
        assert_eq!(yaml_escape(r#"a\b"c"#), r#"a\\b\"c"#);
    }

    #[test]
    fn split_and_rejoin_a_password_round_trips() {
        // Passwords are deliberately distinctive: a single character would
        // appear incidentally in the host name and make the assertion useless.
        let cases = [
            "mongodb://user:hunter2@localhost:27017/db",
            "mongodb+srv://u:swordfish@cluster.example.net/db?retryWrites=true",
            "mongodb://u:swordfish@h1:27017,h2:27017/db",
            "mongodb://u:swordfish@[::1]:27017/db",
            // A percent-encoded password containing an encoded @ and :
            "mongodb://u:p%40ss%3Aw%2Frd@localhost:27017/?authSource=admin",
        ];
        for uri in cases {
            let (stripped, password) = split_mongo_password(uri);
            let password = password.unwrap_or_else(|| panic!("no password found in {}", uri));
            assert!(
                !stripped.contains(&password),
                "password survived in {}",
                stripped
            );
            assert_eq!(with_mongo_password(&stripped, &password), uri);
        }
    }

    #[test]
    fn split_password_stores_the_encoded_form_verbatim() {
        let (_, password) =
            split_mongo_password("mongodb://u:p%40ss%3Aw%2Frd@localhost:27017/?authSource=admin");
        // Not decoded: re-inserting is then a pure splice.
        assert_eq!(password.unwrap(), "p%40ss%3Aw%2Frd");
    }

    #[test]
    fn split_password_leaves_uris_without_one_alone() {
        for uri in [
            "mongodb://localhost:27017/db",
            "mongodb://user@localhost:27017/db",
            "mongodb://user:@localhost:27017/db",
            "not-a-uri",
        ] {
            let (stripped, password) = split_mongo_password(uri);
            assert_eq!(stripped, uri, "{} was modified", uri);
            assert!(password.is_none(), "{} yielded a password", uri);
        }
    }

    #[test]
    fn with_password_does_not_double_up_or_invent_a_user() {
        // Already has one: unchanged.
        assert_eq!(
            with_mongo_password("mongodb://u:existing@h:27017", "new"),
            "mongodb://u:existing@h:27017"
        );
        // No userinfo at all: nowhere to put it.
        assert_eq!(
            with_mongo_password("mongodb://h:27017", "new"),
            "mongodb://h:27017"
        );
    }

    #[test]
    fn redact_covers_every_uri_shape() {
        // credentials -> masked
        assert_eq!(
            redact_mongo_uri("mongodb://user:pass@localhost:27017/db"),
            "mongodb://***:***@localhost:27017/db"
        );
        // username with no password
        assert_eq!(
            redact_mongo_uri("mongodb://user@localhost:27017"),
            "mongodb://***@localhost:27017"
        );
        // no credentials -> untouched
        assert_eq!(
            redact_mongo_uri("mongodb://localhost:27017/db"),
            "mongodb://localhost:27017/db"
        );
        // srv
        assert_eq!(
            redact_mongo_uri("mongodb+srv://u:p@cluster.example.net/db?retryWrites=true"),
            "mongodb+srv://***:***@cluster.example.net/db?retryWrites=true"
        );
        // multi-host seedlist
        assert_eq!(
            redact_mongo_uri("mongodb://u:p@h1:27017,h2:27017/db"),
            "mongodb://***:***@h1:27017,h2:27017/db"
        );
        // IPv6 literal
        assert_eq!(
            redact_mongo_uri("mongodb://u:p@[::1]:27017/db"),
            "mongodb://***:***@[::1]:27017/db"
        );
        // an @ in the host list must not be mistaken for the separator
        assert_eq!(
            redact_mongo_uri("mongodb://localhost:27017/?appName=a@b"),
            "mongodb://localhost:27017/?appName=a@b"
        );
    }

    #[test]
    fn redact_scrubs_uris_out_of_tool_stderr() {
        let stderr = "Failed: can\'t create session: connection() error occurred during \
connection handshake: auth error: sasl conversation error: unable to authenticate \
using mechanism \"SCRAM-SHA-1\": (AuthenticationFailed) Authentication failed., \
uri: mongodb://admin:hunter2@10.0.0.5:27017/?authSource=admin";
        let out = redact_uris_in_text(stderr);
        assert!(!out.contains("hunter2"), "password survived: {}", out);
        assert!(out.contains("mongodb://***:***@10.0.0.5:27017/?authSource=admin"));
    }

    #[test]
    fn redact_leaves_text_without_uris_alone() {
        let plain = "mongorestore failed: no such file or directory";
        assert_eq!(redact_uris_in_text(plain), plain);
    }

    // The literal lines below were captured from the bundled v100.9.4 binaries.
    // If a tools upgrade changes these formats, these tests fail rather than the
    // progress bar quietly falling back to a time-based estimate.

    #[test]
    fn parses_a_mongodump_progress_bar() {
        let line = "2026-09-10T15:35:49.986+0700\t[#############...........]  bigdb.big4  225947/400000  (56.5%)";
        assert_eq!(
            parse_cli_line(line),
            CliLine::Progress {
                ns: "bigdb.big4".to_string(),
                current: "225947".to_string(),
                total: "400000".to_string(),
                percent: 56.5,
            }
        );
    }

    #[test]
    fn parses_a_mongorestore_progress_bar_reporting_bytes() {
        // mongorestore reports byte sizes, not document counts, so the numbers
        // cannot be parsed as integers.
        let line = "2026-09-10T15:36:08.404+0700\t[################........]  restoredb.big  2.00MB/2.98MB  (67.3%)";
        assert_eq!(
            parse_cli_line(line),
            CliLine::Progress {
                ns: "restoredb.big".to_string(),
                current: "2.00MB".to_string(),
                total: "2.98MB".to_string(),
                percent: 67.3,
            }
        );
    }

    #[test]
    fn parses_collection_start_and_finish_markers() {
        assert_eq!(
            parse_cli_line(
                "2026-09-10T15:35:16.836+0700\twriting bigdb.big to bigout/bigdb/big.bson.gz"
            ),
            CliLine::Started {
                ns: "bigdb.big".to_string()
            }
        );
        assert_eq!(
            parse_cli_line("2026-09-10T15:36:05.431+0700\trestoring restoredb.big from bigout/bigdb/big.bson.gz"),
            CliLine::Started { ns: "restoredb.big".to_string() }
        );
        assert_eq!(
            parse_cli_line(
                "2026-09-10T15:35:18.642+0700\tdone dumping bigdb.big (400000 documents)"
            ),
            CliLine::Finished {
                ns: "bigdb.big".to_string()
            }
        );
        assert_eq!(
            parse_cli_line("2026-09-10T15:36:09.871+0700\tfinished restoring restoredb.big (400000 documents, 0 failures)"),
            CliLine::Finished { ns: "restoredb.big".to_string() }
        );
    }

    #[test]
    fn ignores_lines_that_are_not_progress() {
        assert_eq!(parse_cli_line(""), CliLine::Other);
        assert_eq!(
            parse_cli_line("2026-09-10T15:36:05.431+0700\tpreparing collections to restore from"),
            CliLine::Other
        );
        assert_eq!(
            parse_cli_line("Failed: error connecting to db server"),
            CliLine::Other
        );
    }

    #[test]
    fn a_redrawn_bar_reports_its_last_segment() {
        let line = "2026-09-10T15:35:49.986+0700\t[##......]  d.c  1/10  (10.0%)\r[####....]  d.c  5/10  (50.0%)";
        match parse_cli_line(line) {
            CliLine::Progress {
                current, percent, ..
            } => {
                assert_eq!(current, "5");
                assert_eq!(percent, 50.0);
            }
            other => panic!("expected progress, got {:?}", other),
        }
    }

    // These pin the extended-JSON behaviour the browser depends on. Getting it
    // wrong produces filters that silently match nothing, which is the worst
    // possible failure mode for a query box.

    #[test]
    fn filter_parsing_understands_extended_json() {
        use mongodb::bson::Bson;

        let doc = parse_filter(r#"{"_id":{"$oid":"507f1f77bcf86cd799439011"}}"#).unwrap();
        assert!(
            matches!(doc.get("_id"), Some(Bson::ObjectId(_))),
            "$oid must become an ObjectId, not a nested document: {:?}",
            doc.get("_id")
        );

        let doc = parse_filter(r#"{"t":{"$date":"2020-01-01T00:00:00Z"}}"#).unwrap();
        assert!(
            matches!(doc.get("t"), Some(Bson::DateTime(_))),
            "$date must become a DateTime: {:?}",
            doc.get("t")
        );
    }

    #[test]
    fn a_bare_hex_id_is_treated_as_an_object_id() {
        use mongodb::bson::Bson;

        // The most common user mistake, whose failure mode is zero results
        // and no error.
        let doc = parse_filter(r#"{"_id":"507f1f77bcf86cd799439011"}"#).unwrap();
        assert!(matches!(doc.get("_id"), Some(Bson::ObjectId(_))));

        // A string that merely looks id-ish is left alone.
        let doc = parse_filter(r#"{"_id":"not-an-object-id"}"#).unwrap();
        assert!(matches!(doc.get("_id"), Some(Bson::String(_))));

        // 24 characters but not hex.
        let doc = parse_filter(r#"{"_id":"zzzzzzzzzzzzzzzzzzzzzzzz"}"#).unwrap();
        assert!(matches!(doc.get("_id"), Some(Bson::String(_))));
    }

    #[test]
    fn output_is_canonical_so_numbers_keep_their_type() {
        use mongodb::bson::{doc, Bson};

        let out = to_canonical_json(doc! {
            "big": Bson::Int64(9_007_199_254_740_993),
            "small": Bson::Int32(7),
            "dec": Bson::Decimal128("1.10".parse().unwrap()),
        });

        // Relaxed extended JSON would render these as bare numbers and lose the
        // distinction, showing a wrong value with no indication.
        assert!(out.contains("$numberLong"), "{}", out);
        assert!(out.contains("$numberInt"), "{}", out);
        assert!(out.contains("$numberDecimal"), "{}", out);
        assert!(out.contains("9007199254740993"), "{}", out);
    }

    #[test]
    fn an_empty_filter_is_an_empty_document_and_bad_json_is_reported() {
        assert!(parse_filter("").unwrap().is_empty());
        assert!(parse_filter("   ").unwrap().is_empty());

        let err = parse_filter("{not json}").unwrap_err();
        assert!(err.contains("not valid JSON"), "unexpected: {}", err);
    }

    #[test]
    fn combined_percent_spreads_collections_across_the_operation() {
        let at = |percent: f64, finished: u64| CliProgress {
            ns: "d.c".to_string(),
            detail: String::new(),
            percent,
            finished,
        };

        // Unknown count: report the collection's own percentage.
        assert_eq!(combined_percent(&at(40.0, 0), 0), 40.0);
        assert_eq!(combined_percent(&at(40.0, 0), 1), 40.0);

        // Four collections: the first at half way is an eighth of the whole.
        assert_eq!(combined_percent(&at(50.0, 0), 4), 12.5);
        // Two done, third half way.
        assert_eq!(combined_percent(&at(50.0, 2), 4), 62.5);
        // All done stays at 100 rather than overshooting.
        assert_eq!(combined_percent(&at(100.0, 4), 4), 100.0);
    }

    #[test]
    fn a_poisoned_lock_does_not_disable_later_operations() {
        let m = Arc::new(Mutex::new(vec!["before".to_string()]));

        let m2 = Arc::clone(&m);
        let panicked = std::thread::spawn(move || {
            let _guard = m2.lock().unwrap();
            panic!("poison the mutex while holding it");
        })
        .join();
        assert!(panicked.is_err(), "the thread was supposed to panic");
        assert!(m.lock().is_err(), "the mutex was supposed to be poisoned");

        // The recovering lock still hands back usable state.
        let mut guard = lock_or_recover(&m);
        guard.push("after".to_string());
        assert_eq!(guard.len(), 2);
    }

    #[test]
    fn dump_filter_picks_the_narrowest_expression() {
        let all = vec![
            "alpha".to_string(),
            "beta".to_string(),
            "gamma".to_string(),
            "delta".to_string(),
        ];

        // Nothing selected -> whole database.
        assert_eq!(dump_filter(&all, &[]).unwrap(), DumpFilter::None);

        // One selected -> -c, no listing needed.
        assert_eq!(
            dump_filter(&[], &["beta".to_string()]).unwrap(),
            DumpFilter::Only("beta".to_string())
        );

        // A subset -> exclude the complement, sorted for a stable arg vector.
        assert_eq!(
            dump_filter(&all, &["alpha".to_string(), "beta".to_string()]).unwrap(),
            DumpFilter::Exclude(vec!["delta".to_string(), "gamma".to_string()])
        );

        // Everything selected -> nothing to exclude.
        assert_eq!(dump_filter(&all, &all).unwrap(), DumpFilter::None);
    }

    #[test]
    fn dump_filter_refuses_an_oversized_command_line() {
        let all: Vec<String> = (0..3000)
            .map(|i| format!("collection_number_{}", i))
            .collect();
        let selected = vec![all[0].clone(), all[1].clone()];
        let err = dump_filter(&all, &selected).unwrap_err();
        assert!(err.contains("too many collections"), "unexpected: {}", err);
    }

    #[test]
    fn dump_cmd_subset_excludes_the_complement() {
        let cmd = build_dump_cmd(
            dummy(),
            "--uri=U",
            "mydb",
            &DumpFilter::Exclude(vec!["gamma".to_string(), "delta".to_string()]),
            "/tmp/out",
            false,
        );
        let a = args(&cmd);
        assert!(a.contains(&"--excludeCollection=gamma".to_string()));
        assert!(a.contains(&"--excludeCollection=delta".to_string()));
        // The whole-DB dump must not silently come back.
        assert!(!a.iter().any(|s| s == "-c"));
    }

    #[test]
    fn restore_never_drops_unless_asked() {
        // Restoring into a live cluster must not destroy the target unless the
        // caller opted in; --drop used to be hardcoded.
        let folder = build_restore_cmd(
            dummy(),
            "--uri=U",
            "mydb",
            &["a".to_string()],
            "/dump/mydb",
            false,
            true,
            false,
        );
        assert!(!args(&folder).contains(&"--drop".to_string()));

        let archive = build_restore_archive_cmd(
            dummy(),
            "--uri=U",
            &["db1.c1".to_string()],
            "/tmp/a.gz",
            false,
        );
        assert!(!args(&archive).contains(&"--drop".to_string()));
    }

    #[test]
    fn restore_archive_cmd_maps_includes_verbatim() {
        let cmd = build_restore_archive_cmd(
            dummy(),
            "--uri=U",
            &["db1.c1".to_string(), "db2.c2".to_string()],
            "/tmp/a.gz",
            true,
        );
        let a = args(&cmd);
        assert!(a.contains(&"--nsInclude=db1.c1".to_string()));
        assert!(a.contains(&"--nsInclude=db2.c2".to_string()));
        assert!(a.contains(&"--archive=/tmp/a.gz".to_string()));
    }

    #[test]
    fn archive_args_add_gzip_only_for_gz_paths() {
        let mut cmd = std::process::Command::new("x");
        push_archive_args(&mut cmd, "/tmp/dump.gz");
        assert_eq!(args(&cmd), vec!["--archive=/tmp/dump.gz", "--gzip"]);

        let mut cmd = std::process::Command::new("x");
        push_archive_args(&mut cmd, "/tmp/dump.archive");
        assert_eq!(args(&cmd), vec!["--archive=/tmp/dump.archive"]);
    }

    #[test]
    fn ns_includes_one_per_collection() {
        let mut cmd = std::process::Command::new("x");
        push_ns_includes(&mut cmd, "app", &["users".to_string(), "logs".to_string()]);
        assert_eq!(
            args(&cmd),
            vec!["--nsInclude=app.users", "--nsInclude=app.logs"]
        );
        let mut cmd = std::process::Command::new("x");
        push_ns_includes(&mut cmd, "app", &[]);
        assert!(args(&cmd).is_empty());
    }

    #[test]
    fn test_strip_mongo_uri_database() {
        assert_eq!(
            strip_mongo_uri_database(
                "mongodb://root:example@localhost:27017/admin?retryWrites=true"
            ),
            "mongodb://root:example@localhost:27017/?retryWrites=true"
        );
        assert_eq!(
            strip_mongo_uri_database(
                "mongodb+srv://user:pass@cluster.example.com/admin?retryWrites=true"
            ),
            "mongodb+srv://user:pass@cluster.example.com/?retryWrites=true"
        );
        assert_eq!(
            strip_mongo_uri_database(
                "mongodb+srv://user:pass@cluster.example.com/?retryWrites=true"
            ),
            "mongodb+srv://user:pass@cluster.example.com/?retryWrites=true"
        );
        assert_eq!(
            strip_mongo_uri_database("mongodb+srv://user:pass@cluster.example.com"),
            "mongodb+srv://user:pass@cluster.example.com/"
        );
    }
}
