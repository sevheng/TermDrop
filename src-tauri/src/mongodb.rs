use futures_util::stream::TryStreamExt;
use mongodb::{options::ClientOptions, Client};
use std::time::Duration;
use tauri::{Emitter, Window};

/// Resolve the path to a MongoDB tool binary.
/// Tries bundled binary first, then falls back to PATH.
fn resolve_mongo_tool(name: &str) -> Result<std::path::PathBuf, String> {
    #[cfg(target_os = "windows")]
    let name = format!("{}.exe", name);

    // Try bundled binary next to the executable
    if let Ok(exe_path) = std::env::current_exe() {
        if let Some(exe_dir) = exe_path.parent() {
            // Same directory as executable (Windows, Linux AppImage/standalone)
            let bundled = exe_dir.join(&name);
            if bundled.exists() {
                return Ok(bundled);
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

pub async fn list_databases(uri: &str) -> Result<Vec<String>, String> {
    let options = ClientOptions::parse(uri)
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
    let options = ClientOptions::parse(uri)
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
    remote_uri: &str,
    local_uri: &str,
    db: &str,
    collections: Vec<String>,
    drop_first: bool,
) -> Result<(), String> {
    // Try CLI fast path first
    match try_cli_sync(remote_uri, local_uri, db, &collections, drop_first).await {
        Ok(()) => {
            let _ = window.emit("mongodb-sync-done", serde_json::json!({"db": db}));
            return Ok(());
        }
        Err(e) => {
            tracing::info!("CLI sync failed ({}), falling back to driver", e);
        }
    }

    // Fallback to driver-based streaming
    driver_sync(window, remote_uri, local_uri, db, collections, drop_first).await
}

async fn try_cli_sync(
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
        let mut dump_cmd = std::process::Command::new(resolve_mongo_tool("mongodump")?);
        dump_cmd
            .arg(format!("--uri={}", remote_uri))
            .arg(format!("--db={}", db))
            .arg("--gzip")
            .arg(format!("--archive={}", archive_path_str));

        let output = dump_cmd
            .output()
            .map_err(|e| format!("mongodump failed to start: {} (is it installed?)", e))?;
        if !output.status.success() {
            let err = String::from_utf8_lossy(&output.stderr);
            return Err(format!("mongodump failed: {}", err));
        }

        // Step 2: mongorestore to local
        let mut restore_cmd = std::process::Command::new(resolve_mongo_tool("mongorestore")?);
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

        let output = restore_cmd
            .output()
            .map_err(|e| format!("mongorestore failed to start: {} (is it installed?)", e))?;
        if !output.status.success() {
            let err = String::from_utf8_lossy(&output.stderr);
            return Err(format!("mongorestore failed: {}", err));
        }

        // Cleanup
        let _ = std::fs::remove_file(&archive_path);

        Ok(())
    })
    .await
    .map_err(|e| format!("sync task panicked: {}", e))?
}

async fn driver_sync(
    window: Window,
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
        let _ = window.emit(
            "mongodb-sync-progress",
            serde_json::json!({
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
                "db": db,
                "collection": collection_name,
                "stage": "done",
                "synced": synced,
                "total": total,
            }),
        );
    }

    let _ = window.emit("mongodb-sync-done", serde_json::json!({"db": db}));
    Ok(())
}

/// Dump selected collections from remote to a local directory using mongodump.
pub async fn dump_collections(
    window: Window,
    remote_uri: &str,
    db: &str,
    collections: Vec<String>,
    output_dir: &str,
) -> Result<(), String> {
    let remote_uri = remote_uri.to_string();
    let db = db.to_string();
    let collections = collections.to_vec();
    let output_dir = output_dir.to_string();

    tokio::task::spawn_blocking(move || {
        let _ = window.emit(
            "mongodb-sync-progress",
            serde_json::json!({
                "db": &db,
                "collection": "",
                "stage": "dump",
                "synced": 0,
                "total": collections.len(),
            }),
        );

        let mut cmd = std::process::Command::new(resolve_mongo_tool("mongodump")?);
        cmd.arg(format!("--uri={}", remote_uri))
            .arg(format!("--db={}", db))
            .arg("--gzip")
            .arg(format!("--out={}", output_dir));

        // mongodump v100.9.4 doesn't support --nsInclude; use -c for single collection
        if collections.len() == 1 {
            cmd.arg("-c").arg(&collections[0]);
        }
        // For multiple collections, dump the whole DB (mongorestore will filter)

        let output = cmd
            .output()
            .map_err(|e| format!("mongodump failed to start: {} (is it installed?)", e))?;
        if !output.status.success() {
            let err = String::from_utf8_lossy(&output.stderr);
            return Err(format!("mongodump failed: {}", err));
        }

        let _ = window.emit(
            "mongodb-sync-progress",
            serde_json::json!({
                "db": &db,
                "collection": "",
                "stage": "done",
                "synced": collections.len(),
                "total": collections.len(),
            }),
        );

        Ok(())
    })
    .await
    .map_err(|e| format!("dump task panicked: {}", e))?
}

/// Restore selected collections from a local directory to remote using mongorestore.
pub async fn restore_collections(
    window: Window,
    remote_uri: &str,
    db: &str,
    collections: Vec<String>,
    input_dir: &str,
) -> Result<(), String> {
    let remote_uri = remote_uri.to_string();
    let db = db.to_string();
    let collections = collections.to_vec();
    let input_dir = input_dir.to_string();

    tokio::task::spawn_blocking(move || {
        let _ = window.emit(
            "mongodb-sync-progress",
            serde_json::json!({
                "db": &db,
                "collection": "",
                "stage": "restore",
                "synced": 0,
                "total": collections.len(),
            }),
        );

        let mut cmd = std::process::Command::new(resolve_mongo_tool("mongorestore")?);
        cmd.arg(format!("--uri={}", remote_uri))
            .arg(format!("--db={}", db))
            .arg("--drop")
            .arg(&input_dir);

        for coll in &collections {
            cmd.arg(format!("--nsInclude={}.{}", db, coll));
        }

        let output = cmd
            .output()
            .map_err(|e| format!("mongorestore failed to start: {} (is it installed?)", e))?;
        if !output.status.success() {
            let err = String::from_utf8_lossy(&output.stderr);
            return Err(format!("mongorestore failed: {}", err));
        }

        let _ = window.emit(
            "mongodb-sync-progress",
            serde_json::json!({
                "db": &db,
                "collection": "",
                "stage": "done",
                "synced": collections.len(),
                "total": collections.len(),
            }),
        );

        Ok(())
    })
    .await
    .map_err(|e| format!("restore task panicked: {}", e))?
}
