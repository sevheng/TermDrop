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

    let emit_progress = |st: &str, percent: u64| {
        let _ = window.emit(
            "mongodb-sync-progress",
            serde_json::json!({
                "opId": op_id,
                "db": db,
                "collection": "",
                "stage": st,
                "synced": 0,
                "total": 1,
                "percent": percent,
            }),
        );
    };

    emit_progress(stage, 0);

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
            for line in reader.lines().flatten() {
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
                let _ = window.emit("mongodb-sync-cancelled", serde_json::json!({"opId": op_id, "db": db}));
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
                        emit_progress("retrying", 0);
                        let mut cancelled_during_sleep = false;
                        for _ in 0..10 {
                            if cancelled.load(Ordering::Relaxed) {
                                cancelled_during_sleep = true;
                                break;
                            }
                            std::thread::sleep(Duration::from_millis(100));
                        }
                        if cancelled_during_sleep {
                            let _ = window.emit("mongodb-sync-cancelled", serde_json::json!({"opId": op_id, "db": db}));
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
                        emit_progress(stage, pulse);
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
            let bundled = exe_dir.join(&name);
            if bundled.exists() {
                return Ok(bundled);
            }

            // Cargo sometimes places test/run binaries in target/<profile>/deps/;
            // the profile directory (e.g. target/debug) is the parent.
            if let Some(profile_dir) = exe_dir.parent() {
                let bundled = profile_dir.join(&name);
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
                let linux_bundle = exe_dir.join("../lib/TermDrop").join(&name);
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
            let _ = window.emit("mongodb-sync-done", serde_json::json!({"opId": &op_id, "db": db}));
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
                for coll in &collections {
                    restore_cmd.arg(format!("--nsInclude={}.{}", db, coll));
                }

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

    for collection_name in &collections {
        if cancelled.load(Ordering::Relaxed) {
            let _ = window.emit("mongodb-sync-cancelled", serde_json::json!({"opId": op_id, "db": db}));
            return Err("cancelled".into());
        }

        let _ = window.emit(
            "mongodb-sync-progress",
            serde_json::json!({
                "opId": op_id,
                "db": db,
                "collection": collection_name,
                "stage": "count",
                "synced": 0,
                "total": 0,
            }),
        );

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
                let _ = window.emit("mongodb-sync-cancelled", serde_json::json!({"opId": op_id, "db": db}));
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
                let _ = window.emit(
                    "mongodb-sync-progress",
                    serde_json::json!({
                        "opId": op_id,
                        "db": db,
                        "collection": collection_name,
                        "stage": "copy",
                        "synced": synced,
                        "total": total,
                    }),
                );
                last_emit = std::time::Instant::now();
            }
        }

        if !batch.is_empty() {
            local_coll
                .insert_many(&batch)
                .await
                .map_err(|e| format!("insert {}: {}", collection_name, e))?;
        }

        let _ = window.emit(
            "mongodb-sync-progress",
            serde_json::json!({
                "opId": op_id,
                "db": db,
                "collection": collection_name,
                "stage": "done",
                "synced": synced,
                "total": total,
            }),
        );
    }

    let _ = window.emit("mongodb-sync-done", serde_json::json!({"opId": op_id, "db": db}));
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
    let remote_uri = normalize_mongo_uri(remote_uri);
    let remote_uri = strip_mongo_uri_database(&remote_uri);
    let db = db.to_string();
    let output_dir = output_dir.to_string();

    tokio::task::spawn_blocking(move || {
        run_with_retry(
            "mongodump",
            "dump",
            3,
            &window,
            &cancelled,
            &mongo_ops,
            &op_id,
            &db,
            || {
                let mut cmd = std::process::Command::new(resolve_mongo_tool("mongodump")?);
                cmd.arg(format!("--uri={}", &remote_uri))
                    .arg(format!("--db={}", db));

                if is_archive {
                    cmd.arg(format!("--archive={}", output_dir));
                    if output_dir.ends_with(".gz") {
                        cmd.arg("--gzip");
                    }
                } else {
                    cmd.arg("--gzip").arg(format!("--out={}", output_dir));
                }

                // mongodump v100.9.4 doesn't support --nsInclude; use -c for single collection
                if collections.len() == 1 {
                    cmd.arg("-c").arg(&collections[0]);
                }
                // For multiple collections, dump the whole DB (mongorestore will filter)

                Ok(cmd)
            },
        )?;

        let _ = window.emit(
            "mongodb-sync-progress",
            serde_json::json!({
                "opId": &op_id,
                "db": &db,
                "collection": "",
                "stage": "done",
                "synced": 1,
                "total": 1,
                "percent": 100,
            }),
        );

        Ok(())
    })
    .await
    .map_err(|e| format!("dump task panicked: {}", e))?
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
    let remote_uri = normalize_mongo_uri(remote_uri);
    let remote_uri = strip_mongo_uri_database(&remote_uri);
    let db = db.to_string();
    let input_dir = input_dir.to_string();
    let has_db = !db.is_empty();
    let has_collections = !collections.is_empty();
    let progress_db = if has_db { db.clone() } else { "all".to_string() };

    // A "direct DB folder" contains BSON files directly (e.g. /dump/mydb/*.bson.gz).
    // A "dump root" contains DB subfolders (e.g. /dump/<db>/*.bson.gz).
    let is_direct_db = !is_archive && has_db && !collect_bson_collections(std::path::Path::new(&input_dir)).is_empty();
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

    tokio::task::spawn_blocking(move || {
        run_with_retry(
            "mongorestore",
            "restore",
            3,
            &window,
            &cancelled,
            &mongo_ops,
            &op_id,
            &progress_db,
            || {
                let mut cmd = std::process::Command::new(resolve_mongo_tool("mongorestore")?);
                cmd.arg(format!("--uri={}", &remote_uri)).arg("--drop");

                if is_archive {
                    cmd.arg(format!("--archive={}", &input_dir));
                    if input_dir.ends_with(".gz") {
                        cmd.arg("--gzip");
                    }
                } else {
                    // Dump folders produced by this app are gzip-compressed.
                    cmd.arg("--gzip").arg(&restore_dir);
                }

                if is_direct_db {
                    // Path is a single DB dump; --db tells mongorestore the target DB.
                    cmd.arg(format!("--db={}", db));
                    for coll in &collections {
                        cmd.arg(format!("--nsInclude={}.{}", db, coll));
                    }
                } else if has_db {
                    // Path is a dump root; filter with --nsInclude instead of deprecated --db.
                    if has_collections {
                        for coll in &collections {
                            cmd.arg(format!("--nsInclude={}.{}", db, coll));
                        }
                    } else {
                        cmd.arg(format!("--nsInclude={}.*", db));
                    }
                }

                tracing::debug!(
                    program = %cmd.get_program().to_string_lossy(),
                    args = ?cmd.get_args().map(|a| a.to_string_lossy().to_string()).collect::<Vec<_>>(),
                    "mongorestore command"
                );

                Ok(cmd)
            },
        )?;

        let _ = window.emit(
            "mongodb-sync-progress",
            serde_json::json!({
                "opId": &op_id,
                "db": &progress_db,
                "collection": "",
                "stage": "done",
                "synced": 1,
                "total": 1,
                "percent": 100,
            }),
        );

        Ok(())
    })
    .await
    .map_err(|e| format!("restore task panicked: {}", e))?
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
    let remote_uri = normalize_mongo_uri(remote_uri);
    let remote_uri = strip_mongo_uri_database(&remote_uri);
    let input_path = input_path.to_string();
    let includes = includes.clone();

    tokio::task::spawn_blocking(move || {
        run_with_retry(
            "mongorestore",
            "restore",
            3,
            &window,
            &cancelled,
            &mongo_ops,
            &op_id,
            "archive",
            || {
                let mut cmd = std::process::Command::new(resolve_mongo_tool("mongorestore")?);
                cmd.arg(format!("--uri={}", &remote_uri))
                    .arg("--drop")
                    .arg(format!("--archive={}", &input_path));

                if input_path.ends_with(".gz") {
                    cmd.arg("--gzip");
                }

                for ns in &includes {
                    cmd.arg(format!("--nsInclude={}", ns));
                }

                Ok(cmd)
            },
        )?;

        let _ = window.emit(
            "mongodb-sync-progress",
            serde_json::json!({
                "opId": &op_id,
                "db": "archive",
                "collection": "",
                "stage": "done",
                "synced": 1,
                "total": 1,
                "percent": 100,
            }),
        );

        Ok(())
    })
    .await
    .map_err(|e| format!("restore archive task panicked: {}", e))?
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
    for e in std::fs::read_dir(root).map_err(|e| e.to_string())?.flatten() {
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
    fn test_strip_mongo_uri_database() {
        assert_eq!(
            strip_mongo_uri_database("mongodb://root:example@localhost:27017/admin?retryWrites=true"),
            "mongodb://root:example@localhost:27017/?retryWrites=true"
        );
        assert_eq!(
            strip_mongo_uri_database("mongodb+srv://user:pass@cluster.example.com/admin?retryWrites=true"),
            "mongodb+srv://user:pass@cluster.example.com/?retryWrites=true"
        );
        assert_eq!(
            strip_mongo_uri_database("mongodb+srv://user:pass@cluster.example.com/?retryWrites=true"),
            "mongodb+srv://user:pass@cluster.example.com/?retryWrites=true"
        );
        assert_eq!(
            strip_mongo_uri_database("mongodb+srv://user:pass@cluster.example.com"),
            "mongodb+srv://user:pass@cluster.example.com/"
        );
    }
}
