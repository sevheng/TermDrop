use futures_util::stream::TryStreamExt;
use mongodb::{options::ClientOptions, Client};
use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::process::Child;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::{Emitter, Window};

fn set_mongo_child(
    mongo_ops: &Arc<Mutex<HashMap<String, crate::MongoOpHandle>>>,
    op_id: &str,
    child: Child,
) {
    let mut ops = mongo_ops.lock().unwrap();
    if let Some(handle) = ops.get_mut(op_id) {
        handle.child = Some(child);
    }
}

fn take_mongo_child(
    mongo_ops: &Arc<Mutex<HashMap<String, crate::MongoOpHandle>>>,
    op_id: &str,
) -> Option<Child> {
    let mut ops = mongo_ops.lock().unwrap();
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

    fn cancelled(&self) {
        let _ = self.window.emit(
            "mongodb-sync-cancelled",
            serde_json::json!({"opId": self.op_id, "db": self.db}),
        );
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

/// Build the `mongodump` command line.
///
/// `conn_arg` is the whole connection argument (`--uri=...` or `--config=...`)
/// so the builder stays unaware of how the credential reaches the tool.
/// Split out from the spawning closure so the argument vector is testable.
fn build_dump_cmd(
    tool: std::path::PathBuf,
    conn_arg: &str,
    db: &str,
    collections: &[String],
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

    // mongodump v100.9.4 doesn't support --nsInclude; use -c for single collection
    if collections.len() == 1 {
        cmd.arg("-c").arg(&collections[0]);
    }
    // For multiple collections, dump the whole DB (mongorestore will filter)

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
) -> std::process::Command {
    let mut cmd = std::process::Command::new(tool);
    cmd.arg(conn_arg).arg("--drop");

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
) -> std::process::Command {
    let mut cmd = std::process::Command::new(tool);
    cmd.arg(conn_arg).arg("--drop");
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
    build_cmd: F,
) -> Result<(), String>
where
    F: FnMut() -> Result<std::process::Command, String> + Send + 'static,
{
    tokio::task::spawn_blocking(move || {
        run_with_retry(
            tool,
            stage,
            3,
            &window,
            &cancelled,
            &mongo_ops,
            &op_id,
            &progress_db,
            build_cmd,
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

fn run_with_retry<F>(
    label: &str,
    stage: &str,
    max_retries: u32,
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
        let stderr_thread = std::thread::spawn(move || {
            let reader = BufReader::new(stderr);
            // Stop at the first read error; `flatten()` would spin forever on a
            // persistently failing pipe.
            for line in reader.lines().map_while(Result::ok) {
                stderr_lines_clone.lock().unwrap().push(line);
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
                        let lines = stderr_lines.lock().unwrap();
                        let recent: Vec<_> = lines.iter().rev().take(30).cloned().collect();
                        tracing::debug!(
                            label = label,
                            stage = stage,
                            exit_code = status.code(),
                            stderr = ?recent,
                            "command succeeded"
                        );
                        return Ok(());
                    }
                    let lines = stderr_lines.lock().unwrap();
                    let err = lines
                        .iter()
                        .rev()
                        .take(30)
                        .cloned()
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
                        let elapsed_ms = start.elapsed().as_millis() as u64;
                        // Monotonic pulse: grows toward 95% so the bar never loops back.
                        let pulse = std::cmp::min(95, elapsed_ms / 100);
                        progress.pulse(stage, pulse);
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

pub async fn list_databases(uri: &str) -> Result<Vec<String>, String> {
    let uri = normalize_mongo_uri(uri);
    let options = ClientOptions::parse(&uri)
        .await
        .map_err(|e| format!("parse uri: {}", e))?;
    let client = Client::with_options(options).map_err(|e| format!("create client: {}", e))?;
    let dbs = client
        .list_database_names()
        .await
        .map_err(|e| format!("list databases: {}", e))?;
    Ok(dbs)
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
            tracing::info!("CLI sync failed ({}), falling back to driver", e);
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
        let archive_path =
            std::env::temp_dir().join(format!("termdrop-sync-{}.gz", uuid::Uuid::new_v4()));
        let archive_path_str = archive_path.to_string_lossy().to_string();

        // Step 1: mongodump from remote (dump whole DB; mongorestore will filter collections)
        let dump_result = run_with_retry(
            "mongodump",
            "sync",
            3,
            &window,
            &cancelled,
            &mongo_ops,
            &op_id,
            &db,
            || {
                let mut dump_cmd = std::process::Command::new(resolve_mongo_tool("mongodump")?);
                dump_cmd
                    .arg(format!("--uri={}", remote_uri))
                    .arg(format!("--db={}", db))
                    .arg("--gzip")
                    .arg(format!("--archive={}", archive_path_str));
                Ok(dump_cmd)
            },
        );

        if let Err(e) = dump_result {
            let _ = std::fs::remove_file(&archive_path);
            return Err(e);
        }

        // Step 2: mongorestore to local
        let restore_result = run_with_retry(
            "mongorestore",
            "sync",
            3,
            &window,
            &cancelled,
            &mongo_ops,
            &op_id,
            &db,
            || {
                let mut restore_cmd =
                    std::process::Command::new(resolve_mongo_tool("mongorestore")?);
                restore_cmd
                    .arg(format!("--uri={}", local_uri))
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

        let _ = std::fs::remove_file(&archive_path);
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

        progress.collection(collection_name, "done", synced, total);
    }

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
        move || {
            Ok(build_dump_cmd(
                resolve_mongo_tool("mongodump")?,
                &format!("--uri={}", &remote_uri),
                &cmd_db,
                &collections,
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
        move || {
            let cmd = build_restore_cmd(
                resolve_mongo_tool("mongorestore")?,
                &format!("--uri={}", &remote_uri),
                &cmd_db,
                &collections,
                if is_archive { &input_dir } else { &restore_dir },
                is_archive,
                is_direct_db,
            );

            tracing::debug!(
                program = %cmd.get_program().to_string_lossy(),
                args = ?cmd.get_args().map(|a| a.to_string_lossy().to_string()).collect::<Vec<_>>(),
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
        move || {
            Ok(build_restore_archive_cmd(
                resolve_mongo_tool("mongorestore")?,
                &format!("--uri={}", &remote_uri),
                &includes,
                &input_path,
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
        let cmd = build_dump_cmd(dummy(), "--uri=U", "mydb", &[], "/tmp/out", false);
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
            &["logs".to_string()],
            "/tmp/out",
            false,
        );
        assert!(args(&cmd).windows(2).any(|w| w == ["-c", "logs"]));
    }

    #[test]
    fn dump_cmd_archive_path_gets_archive_args() {
        let cmd = build_dump_cmd(dummy(), "--uri=U", "mydb", &[], "/tmp/d.gz", true);
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
        );
        let a = args(&cmd);
        assert!(a.contains(&"--db=mydb".to_string()));
        assert!(a.contains(&"--nsInclude=mydb.a".to_string()));
        assert!(a.contains(&"--nsInclude=mydb.b".to_string()));
    }

    #[test]
    fn restore_cmd_dump_root_without_collections_includes_whole_db() {
        let cmd = build_restore_cmd(dummy(), "--uri=U", "mydb", &[], "/dump", false, false);
        let a = args(&cmd);
        assert!(a.contains(&"--nsInclude=mydb.*".to_string()));
        assert!(!a.contains(&"--db=mydb".to_string()));
    }

    #[test]
    fn restore_archive_cmd_maps_includes_verbatim() {
        let cmd = build_restore_archive_cmd(
            dummy(),
            "--uri=U",
            &["db1.c1".to_string(), "db2.c2".to_string()],
            "/tmp/a.gz",
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
