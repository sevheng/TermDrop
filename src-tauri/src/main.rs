#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
// Tauri command signatures mirror the frontend payloads one argument per
// field; grouping them into structs would change the IPC contract.
#![allow(clippy::too_many_arguments)]

use r2d2::Pool;
use r2d2_sqlite::SqliteConnectionManager;
use std::collections::HashMap;
use std::path::Path;
use std::process::Child;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::{Emitter, Manager, State, Window};
use tracing::{info, instrument};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

mod crypto;
mod db;
mod docker;
mod mongodb;
mod port_forward;
mod redis;
mod redis_backup;
mod security;
mod sftp;
mod ssh;
mod ssh_config_parser;
mod system;
mod uri;

/// Per-host coalescing locks so concurrent requests share one fetch.
type FetchLocks = Arc<Mutex<HashMap<i64, Arc<tokio::sync::Mutex<()>>>>>;

/// A per-host cached value with its fetch time.
pub struct Cached<T> {
    value: T,
    cached_at: std::time::Instant,
}

pub struct MongoOpHandle {
    pub cancelled: Arc<AtomicBool>,
    pub child: Option<Child>,
}

pub struct AppState {
    db: Pool<SqliteConnectionManager>,
    sessions: Mutex<HashMap<String, ssh::SshSessionHandle>>,
    exec_sessions: Mutex<HashMap<i64, Arc<Mutex<ssh2::Session>>>>,
    sftp_sessions: Mutex<HashMap<String, Arc<sftp::SftpSessionHandle>>>,
    exec_pty_sessions: Mutex<HashMap<String, ssh::ExecPtyHandle>>,
    docker_cache: Arc<Mutex<HashMap<i64, Cached<Vec<docker::Container>>>>>,
    docker_ps_fetching: FetchLocks,
    security_report_cache: Arc<Mutex<HashMap<i64, Cached<security::SecurityReport>>>>,
    security_report_fetching: FetchLocks,
    forward_manager: port_forward::ForwardManager,
    pub mongo_ops: Arc<Mutex<HashMap<String, MongoOpHandle>>>,
    /// One driver client per host, so browsing a database does not open a fresh
    /// connection pool and topology monitor per call.
    ///
    /// Keyed by host id, never by URI: a URI key would put a credential into a
    /// long-lived map and into any Debug output.
    mongo_clients: Arc<tokio::sync::Mutex<HashMap<i64, ::mongodb::Client>>>,
    /// One pooled connection per `(host_id, database)`.
    ///
    /// Keyed by database as well as host because `SELECT` is *connection*
    /// state and `MultiplexedConnection` pipelines commands from every task
    /// over one socket: a single per-host connection would let the key list's
    /// `SELECT 1` land between another pane's `TYPE` and its `HSCAN`. Baking
    /// the index into the connection removes the race rather than locking
    /// around it.
    redis_conns: Arc<tokio::sync::Mutex<HashMap<(i64, i64), ::redis::aio::MultiplexedConnection>>>,
    /// One SSH tunnel per Redis host, alive while any of its tabs is open.
    redis_tunnels: Arc<tokio::sync::Mutex<HashMap<i64, port_forward::Tunnel>>>,
    /// Cancel flags for in-flight Redis exports and imports, keyed by op id.
    ///
    /// Deliberately not the MongoDB registry: that one also carries a child
    /// process to kill, and a Redis operation has none -- it is a loop in this
    /// process, so a flag is the whole mechanism.
    redis_ops: Arc<Mutex<HashMap<String, Arc<AtomicBool>>>>,
}

fn db_err(e: r2d2::Error) -> String {
    e.to_string()
}

async fn with_timeout<F, R>(f: F, secs: u64) -> Result<R, String>
where
    F: FnOnce() -> Result<R, String> + Send + 'static,
    R: Send + 'static,
{
    tokio::time::timeout(Duration::from_secs(secs), tokio::task::spawn_blocking(f))
        .await
        .map_err(|_| format!("Operation timed out after {} seconds", secs))?
        .map_err(|e| e.to_string())?
}

fn register_mongo_op(state: &State<'_, AppState>, op_id: String) -> Arc<AtomicBool> {
    let cancelled = Arc::new(AtomicBool::new(false));
    let handle = MongoOpHandle {
        cancelled: cancelled.clone(),
        child: None,
    };
    mongodb::lock_or_recover(&state.mongo_ops).insert(op_id, handle);
    cancelled
}

fn unregister_mongo_op(state: &State<'_, AppState>, op_id: &str) {
    mongodb::lock_or_recover(&state.mongo_ops).remove(op_id);
}

/// The keyring account holding a host's MongoDB password.
///
/// Non-numeric by construction, so it cannot collide with an SSH host password
/// in the shared fallback file. The `-remote-` segment is historical, from when
/// a host had two connections: renaming it would mean migrating every stored
/// secret, and a keyring failure mid-migration would lose passwords for no
/// user-visible gain.
fn mongo_account(host_id: i64) -> String {
    format!("mongo-remote-{}", host_id)
}

/// The full connection URI for a MongoDB host.
///
/// The stored URI carries no password; the secret is spliced in here so the
/// credential never crosses the IPC boundary and the frontend never holds it.
///
/// When the URI names a user but no secret is stored, this reports the same
/// "keyring retrieve failed" wording the SSH path uses, so one frontend
/// detector can drive the prompt-and-retry for both.
fn load_mongo_uri(state: &State<'_, AppState>, host_id: i64) -> Result<String, String> {
    let host = with_db(state, |conn| db::get_host_by_id(conn, host_id))?
        .ok_or_else(|| format!("host {} not found", host_id))?;

    let uri = host
        .mongo_uri
        .filter(|u| !u.trim().is_empty())
        .ok_or_else(|| "no MongoDB connection configured".to_string())?;

    // Already carries a password (a row the migration could not rewrite).
    if mongodb::split_mongo_password(&uri).1.is_some() {
        return Ok(uri);
    }

    match crypto::get_secret(&mongo_account(host_id)) {
        Ok(password) => Ok(mongodb::with_mongo_password(&uri, &password)),
        Err(e) if mongodb::uri_expects_password(&uri) => {
            Err(format!("keyring retrieve failed for MongoDB: {}", e))
        }
        // No user in the URI, so no password is expected.
        Err(_) => Ok(uri),
    }
}

/// A pooled driver client for a host, created on first use.
///
/// `Client` is internally reference-counted, so callers get a cheap clone and
/// the pool outlives any single command.
async fn mongo_client(
    state: &State<'_, AppState>,
    host_id: i64,
) -> Result<::mongodb::Client, String> {
    if let Some(client) = state.mongo_clients.lock().await.get(&host_id) {
        return Ok(client.clone());
    }

    // Resolving outside the lock is tempting, but two tabs opening at once
    // would then build two clients; holding it keeps exactly one per host.
    let uri = load_mongo_uri(state, host_id)?;
    let mut clients = state.mongo_clients.lock().await;
    if let Some(client) = clients.get(&host_id) {
        return Ok(client.clone());
    }
    let client = mongodb::build_client(&uri).await?;
    clients.insert(host_id, client.clone());
    Ok(client)
}

/// Drop the pooled client for a host, closing its connection pool.
///
/// Called when the MongoDB tab closes and when the host is edited — without the
/// latter, a changed URI would stay invisible until the app restarted.
async fn forget_mongo_clients(state: &State<'_, AppState>, host_id: i64) {
    state.mongo_clients.lock().await.remove(&host_id);
}

/// Await a future with a timeout, for the async driver calls that
/// `with_timeout` (which wraps a blocking closure) cannot cover.
///
/// Deliberately not applied to dump or restore: those legitimately run for hours.
async fn with_async_timeout<T>(
    fut: impl std::future::Future<Output = Result<T, String>>,
    secs: u64,
) -> Result<T, String> {
    tokio::time::timeout(Duration::from_secs(secs), fut)
        .await
        .map_err(|_| format!("MongoDB operation timed out after {} seconds", secs))?
}

/// Move MongoDB passwords out of the database and into the keyring.
///
/// Runs on every launch and is idempotent: a URI with no credentials is
/// skipped, so once migrated a row is never touched again.
///
/// The secret is stored *before* the row is rewritten. If the keyring is
/// unavailable and the encrypted fallback also fails, the plaintext row is left
/// intact and the migration retries next launch — losing the password would be
/// far worse than leaving it where it already is.
fn migrate_mongo_credentials(conn: &rusqlite::Connection) {
    migrate_mongo_credentials_with(conn, |account, secret| {
        crypto::store_secret(account, secret)
    });
}

/// Give every second MongoDB connection its own host.
///
/// A host used to carry a "remote" and a "local" URI so the two could be synced.
/// Sync is gone and a connection is now one URI, but a configured local
/// connection is still a real connection the user set up — so move it to its own
/// host rather than discarding it.
///
/// Runs before the credential migration, so the moved URI is then treated like
/// any other and has its password lifted out in the same pass. Idempotent: the
/// old column is cleared once the new host exists.
fn split_local_mongo_hosts(conn: &rusqlite::Connection) -> usize {
    let rows = match db::hosts_with_local_mongo_uri(conn) {
        Ok(rows) => rows,
        Err(e) => {
            tracing::warn!("could not read hosts to split local MongoDB URIs: {}", e);
            return 0;
        }
    };

    let mut split = 0usize;
    for (id, name, local_uri) in rows {
        let new_host = db::NewHost {
            name: format!("{} (local)", name),
            host: String::new(),
            port: 0,
            username: String::new(),
            auth_type: "password".to_string(),
            key_path: None,
            group: None,
            favorite: None,
            mongo_uri: Some(local_uri),
            mongo_local_uri: None,
            redis_uri: None,
            redis_tunnel_host_id: None,
        };

        let new_id = match db::add_host(conn, &new_host) {
            Ok(new_id) => new_id,
            Err(e) => {
                tracing::warn!(host_id = id, "could not split local MongoDB URI: {}", e);
                continue;
            }
        };

        // Carry the stored password across, storing before deleting so a keyring
        // failure leaves the old entry rather than losing the secret.
        let old_account = format!("mongo-local-{}", id);
        if let Ok(secret) = crypto::get_secret(&old_account) {
            match crypto::store_secret(&mongo_account(new_id), &secret) {
                Ok(()) => {
                    let _ = crypto::delete_secret(&old_account);
                }
                Err(e) => {
                    tracing::warn!(
                        host_id = id,
                        "could not move the local MongoDB password; leaving it in place: {}",
                        e
                    );
                }
            }
        }

        if let Err(e) = db::clear_mongo_local_uri(conn, id) {
            tracing::warn!(host_id = id, "could not clear the local MongoDB URI: {}", e);
            continue;
        }
        split += 1;
    }

    if split > 0 {
        tracing::info!(
            "moved {} local MongoDB connection(s) to their own host",
            split
        );
    }
    split
}

/// The migration proper, with secret storage injected so the ordering guarantee
/// can be tested without touching the real keyring.
fn migrate_mongo_credentials_with(
    conn: &rusqlite::Connection,
    mut store: impl FnMut(&str, &str) -> Result<(), String>,
) -> usize {
    let rows = match db::all_mongo_uris(conn) {
        Ok(rows) => rows,
        Err(e) => {
            tracing::warn!(
                "could not read hosts for MongoDB credential migration: {}",
                e
            );
            return 0;
        }
    };

    let mut migrated = 0usize;
    for (id, uri) in rows {
        let Some(uri) = uri else { continue };
        let (stripped, Some(password)) = mongodb::split_mongo_password(&uri) else {
            continue;
        };

        if let Err(e) = store(&mongo_account(id), &password) {
            tracing::warn!(
                host_id = id,
                "could not store MongoDB password, leaving it in the database: {}",
                e
            );
            continue;
        }
        if let Err(e) = db::set_mongo_uri(conn, id, &stripped) {
            tracing::warn!(host_id = id, "could not rewrite MongoDB URI: {}", e);
            continue;
        }
        migrated += 1;
    }

    if migrated > 0 {
        // An UPDATE leaves the old plaintext in free pages; without this the
        // secrets would still be recoverable from the file.
        tracing::info!(
            "migrated {} MongoDB password(s) out of the database",
            migrated
        );
        if let Err(e) = db::vacuum(conn) {
            tracing::warn!("could not vacuum after migration: {}", e);
        }
    }

    migrated
}

/// Register `op_id` so mongodb_cancel can reach it, run `f`, then unregister.
async fn with_mongo_op<Fut>(
    state: &State<'_, AppState>,
    op_id: &str,
    f: impl FnOnce(Arc<AtomicBool>, Arc<Mutex<HashMap<String, MongoOpHandle>>>) -> Fut,
) -> Result<(), String>
where
    Fut: std::future::Future<Output = Result<(), String>>,
{
    let cancelled = register_mongo_op(state, op_id.to_string());
    let result = f(cancelled, state.mongo_ops.clone()).await;
    unregister_mongo_op(state, op_id);
    result
}

/// Load a host row or fail with "Host not found".
fn load_host(state: &State<'_, AppState>, host_id: i64) -> Result<db::Host, String> {
    with_db(state, |conn| db::get_host_by_id(conn, host_id))?
        .ok_or_else(|| "Host not found".to_string())
}

/// Resolve `(password, key_path)` for a host. Password hosts use
/// `password_override` when given and the keyring otherwise; key hosts use
/// the stored key path. Unknown auth types pass the override through.
fn resolve_auth(
    host: &db::Host,
    password_override: Option<String>,
) -> Result<(Option<String>, Option<String>), String> {
    match host.auth_type.as_str() {
        "password" => {
            let pw = match password_override {
                Some(p) => Some(p),
                None => Some(crypto::get_password(host.id)?),
            };
            Ok((pw, None))
        }
        "key" => Ok((None, host.key_path.clone())),
        _ => Ok((password_override, host.key_path.clone())),
    }
}

/// The shared non-PTY session for a host, used for exec-style commands.
fn exec_session(
    state: &State<'_, AppState>,
    host_id: i64,
    missing: &'static str,
) -> Result<Arc<Mutex<ssh2::Session>>, String> {
    let exec_sessions = state.exec_sessions.lock().map_err(|e| e.to_string())?;
    exec_sessions
        .get(&host_id)
        .cloned()
        .ok_or_else(|| missing.to_string())
}

fn sftp_handle(
    state: &State<'_, AppState>,
    sftp_session_id: &str,
) -> Result<Arc<sftp::SftpSessionHandle>, String> {
    let sftp_sessions = state.sftp_sessions.lock().map_err(|e| e.to_string())?;
    sftp_sessions
        .get(sftp_session_id)
        .cloned()
        .ok_or_else(|| "SFTP session not found".to_string())
}

/// Run a query on a pooled connection, stringifying either error.
fn with_db<T>(
    state: &State<'_, AppState>,
    f: impl FnOnce(&rusqlite::Connection) -> rusqlite::Result<T>,
) -> Result<T, String> {
    let conn = state.db.get().map_err(db_err)?;
    f(&conn).map_err(|e| e.to_string())
}

/// Run a blocking SFTP operation on the named session off the async thread.
async fn sftp_blocking<T, F>(
    state: &State<'_, AppState>,
    sftp_session_id: &str,
    f: F,
) -> Result<T, String>
where
    F: FnOnce(&sftp::SftpSessionHandle) -> Result<T, String> + Send + 'static,
    T: Send + 'static,
{
    let handle = sftp_handle(state, sftp_session_id)?;
    tokio::task::spawn_blocking(move || f(&handle))
        .await
        .map_err(|e| e.to_string())?
}

/// Run a command on the host's shared exec session with a timeout. The
/// session mutex is held for the duration of `f`.
async fn with_exec_session<T, F>(
    state: &State<'_, AppState>,
    host_id: i64,
    secs: u64,
    f: F,
) -> Result<T, String>
where
    F: FnOnce(&ssh2::Session) -> Result<T, String> + Send + 'static,
    T: Send + 'static,
{
    let session_arc = exec_session(state, host_id, "No active session for this host")?;
    with_timeout(
        move || {
            let session = session_arc.lock().map_err(|e| e.to_string())?;
            f(&session)
        },
        secs,
    )
    .await
}

/// Per-host cache with request coalescing, shared by docker_ps and the
/// security audit. Returns the cached value while it is younger than
/// `fresh_secs`; while younger than `stale_secs` returns it and refreshes in
/// the background; otherwise blocks on `fetch`. `force` skips both reads.
#[allow(clippy::too_many_arguments)]
async fn cached_per_host<T, F>(
    state: &State<'_, AppState>,
    cache: &Arc<Mutex<HashMap<i64, Cached<T>>>>,
    fetching: &FetchLocks,
    host_id: i64,
    fresh_secs: u64,
    stale_secs: u64,
    force: bool,
    fetch: F,
) -> Result<T, String>
where
    T: Clone + Send + 'static,
    F: Fn(&ssh2::Session) -> Result<T, String> + Clone + Send + 'static,
{
    // Fast path: fresh cache
    if !force {
        let cached = cache.lock().map_err(|e| e.to_string())?;
        if let Some(c) = cached.get(&host_id) {
            if c.cached_at.elapsed().as_secs() < fresh_secs {
                return Ok(c.value.clone());
            }
        }
    }

    // Serialize fetches per host (request coalescing)
    let fetch_lock = {
        let mut fetching = fetching.lock().map_err(|e| e.to_string())?;
        fetching
            .entry(host_id)
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
            .clone()
    };

    let _guard = fetch_lock.lock().await;

    // Re-check cache after acquiring lock (another request may have updated it)
    if !force {
        let cached = cache.lock().map_err(|e| e.to_string())?;
        if let Some(c) = cached.get(&host_id) {
            let elapsed = c.cached_at.elapsed().as_secs();
            if elapsed < fresh_secs {
                return Ok(c.value.clone());
            }
            if elapsed < stale_secs {
                // Return stale immediately, refresh in background
                let cache_clone = cache.clone();
                let session_arc = exec_session(state, host_id, "No active session for this host")?;
                let fetch = fetch.clone();
                tokio::task::spawn(async move {
                    let result = with_timeout(
                        move || {
                            let session = session_arc.lock().map_err(|e| e.to_string())?;
                            fetch(&session)
                        },
                        60,
                    )
                    .await;
                    if let Ok(value) = result {
                        let mut cache = cache_clone.lock().unwrap();
                        cache.insert(
                            host_id,
                            Cached {
                                value,
                                cached_at: std::time::Instant::now(),
                            },
                        );
                    }
                });
                return Ok(c.value.clone());
            }
        }
    }

    // Cache is empty or very stale — block and fetch
    let value = with_exec_session(state, host_id, 60, move |session| fetch(session)).await?;

    {
        let mut cached = cache.lock().map_err(|e| e.to_string())?;
        cached.insert(
            host_id,
            Cached {
                value: value.clone(),
                cached_at: std::time::Instant::now(),
            },
        );
    }

    Ok(value)
}

#[tauri::command]
fn get_hosts(state: State<'_, AppState>) -> Result<Vec<db::Host>, String> {
    with_db(&state, db::get_hosts)
}

#[tauri::command]
fn add_host(state: State<'_, AppState>, host: db::NewHost) -> Result<i64, String> {
    with_db(&state, |conn| db::add_host(conn, &host))
}

#[tauri::command]
async fn update_host(state: State<'_, AppState>, id: i64, host: db::NewHost) -> Result<(), String> {
    with_db(&state, |conn| db::update_host(conn, id, &host))?;
    // The URI may have changed; a cached client would keep using the old one.
    forget_mongo_clients(&state, id).await;
    Ok(())
}

#[tauri::command]
async fn delete_host(state: State<'_, AppState>, id: i64) -> Result<(), String> {
    with_db(&state, |conn| db::delete_host(conn, id))?;
    forget_mongo_clients(&state, id).await;
    crypto::delete_password(id).ok();
    crypto::delete_secret(&mongo_account(id)).ok();
    Ok(())
}

#[tauri::command]
fn get_host_by_id(state: State<'_, AppState>, id: i64) -> Result<Option<db::Host>, String> {
    with_db(&state, |conn| db::get_host_by_id(conn, id))
}

#[tauri::command]
fn store_password(host_id: i64, password: String) -> Result<(), String> {
    crypto::store_password(host_id, &password)
}

#[tauri::command]
#[instrument(skip(window, state), fields(host_id))]
async fn ssh_connect(
    window: Window,
    state: State<'_, AppState>,
    host_id: i64,
    password: Option<String>,
    cols: u32,
    rows: u32,
) -> Result<String, String> {
    let host = load_host(&state, host_id)?;

    let (password, key_path) = resolve_auth(&host, password)?;

    let session_id = uuid::Uuid::new_v4().to_string();
    let handle = ssh::connect(
        window.clone(),
        session_id.clone(),
        host_id,
        host.host.clone(),
        host.port as u16,
        host.username.clone(),
        password.clone(),
        key_path.clone(),
        cols,
        rows,
    )?;

    // Create a persistent exec session off the async thread to avoid UI freeze
    let host_clone = host.host.clone();
    let port = host.port as u16;
    let username = host.username.clone();
    let password_clone = password.clone();
    let key_path_clone = key_path.clone();
    let exec_session = tokio::task::spawn_blocking(move || {
        ssh::create_exec_session(
            &host_clone,
            port,
            &username,
            password_clone.as_deref(),
            key_path_clone.as_deref(),
        )
    })
    .await
    .map_err(|e| format!("exec session task failed: {}", e))?
    .map_err(|e| format!("exec session: {}", e))?;

    {
        let mut sessions = state.sessions.lock().map_err(|e| e.to_string())?;
        sessions.insert(session_id.clone(), handle);
    }
    {
        let mut exec_sessions = state.exec_sessions.lock().map_err(|e| e.to_string())?;
        exec_sessions.insert(host_id, Arc::new(Mutex::new(exec_session)));
    }

    info!(session_id = %session_id, host_id = host_id, "SSH connected");
    Ok(session_id)
}

#[tauri::command]
fn ssh_write(state: State<'_, AppState>, session_id: String, data: String) -> Result<(), String> {
    let sessions = state.sessions.lock().map_err(|e| e.to_string())?;
    let session = sessions.get(&session_id).ok_or("Session not found")?;
    session.write_tx.send(data).map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
fn ssh_resize(
    state: State<'_, AppState>,
    session_id: String,
    cols: u32,
    rows: u32,
) -> Result<(), String> {
    let sessions = state.sessions.lock().map_err(|e| e.to_string())?;
    let session = sessions.get(&session_id).ok_or("Session not found")?;
    session
        .resize_tx
        .send((cols, rows))
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
fn open_data_channel(
    state: State<'_, AppState>,
    session_id: String,
    channel: tauri::ipc::Channel<Vec<u8>>,
) -> Result<(), String> {
    let mut sessions = state.sessions.lock().map_err(|e| e.to_string())?;
    let session = sessions.get_mut(&session_id).ok_or("Session not found")?;
    *session.data_channel.lock().map_err(|e| e.to_string())? = Some(channel);
    Ok(())
}

#[tauri::command]
fn open_exec_pty_data_channel(
    state: State<'_, AppState>,
    pty_session_id: String,
    channel: tauri::ipc::Channel<Vec<u8>>,
) -> Result<(), String> {
    let mut sessions = state.exec_pty_sessions.lock().map_err(|e| e.to_string())?;
    let session = sessions
        .get_mut(&pty_session_id)
        .ok_or("PTY session not found")?;
    *session.data_channel.lock().map_err(|e| e.to_string())? = Some(channel);
    Ok(())
}

#[tauri::command]
fn parse_ssh_config() -> Result<Vec<ssh_config_parser::SshConfigHost>, String> {
    ssh_config_parser::parse_ssh_config()
}

#[tauri::command]
fn ssh_disconnect(state: State<'_, AppState>, session_id: String) -> Result<(), String> {
    let host_id = {
        let mut sessions = state.sessions.lock().map_err(|e| e.to_string())?;
        if let Some(session) = sessions.remove(&session_id) {
            let _ = session.disconnect_tx.send(());
            session.host_id
        } else {
            return Ok(());
        }
    };

    // Only remove exec session if no other tabs use this host
    let sessions = state.sessions.lock().map_err(|e| e.to_string())?;
    let still_connected = sessions.values().any(|s| s.host_id == host_id);
    drop(sessions);

    if !still_connected {
        let mut exec_sessions = state.exec_sessions.lock().map_err(|e| e.to_string())?;
        exec_sessions.remove(&host_id);
        drop(exec_sessions);
        forget_host_state(&state, host_id)?;
    }

    Ok(())
}

/// Drop everything cached or running for a host once its last tab closes:
/// docker and security caches, their coalescing locks, and any docker exec
/// PTY sessions still attached to it.
fn forget_host_state(state: &State<'_, AppState>, host_id: i64) -> Result<(), String> {
    state
        .docker_cache
        .lock()
        .map_err(|e| e.to_string())?
        .remove(&host_id);
    state
        .docker_ps_fetching
        .lock()
        .map_err(|e| e.to_string())?
        .remove(&host_id);
    state
        .security_report_cache
        .lock()
        .map_err(|e| e.to_string())?
        .remove(&host_id);
    state
        .security_report_fetching
        .lock()
        .map_err(|e| e.to_string())?
        .remove(&host_id);

    let mut pty_sessions = state.exec_pty_sessions.lock().map_err(|e| e.to_string())?;
    let ids: Vec<String> = pty_sessions
        .iter()
        .filter(|(_, h)| h.host_id == host_id)
        .map(|(id, _)| id.clone())
        .collect();
    for id in ids {
        if let Some(session) = pty_sessions.remove(&id) {
            let _ = session.disconnect_tx.send(());
        }
    }
    Ok(())
}

#[tauri::command]
#[instrument(skip(window, state), fields(session_id))]
async fn ssh_reconnect(
    window: Window,
    state: State<'_, AppState>,
    session_id: String,
    sftp_session_id: Option<String>,
    password: Option<String>,
) -> Result<(), String> {
    let host_id = {
        let sessions = state.sessions.lock().map_err(|e| e.to_string())?;
        let session = sessions.get(&session_id).ok_or("Session not found")?;
        session.host_id
    };

    let host = load_host(&state, host_id)?;

    let (password, key_path) = resolve_auth(&host, password)?;

    // Remove old session
    {
        let mut sessions = state.sessions.lock().map_err(|e| e.to_string())?;
        if let Some(old) = sessions.remove(&session_id) {
            let _ = old.disconnect_tx.send(());
        }
    }

    // Remove old exec session and create new one off the async thread
    {
        let mut exec_sessions = state.exec_sessions.lock().map_err(|e| e.to_string())?;
        exec_sessions.remove(&host_id);
    }
    let host_clone = host.host.clone();
    let port = host.port as u16;
    let username = host.username.clone();
    let password_clone = password.clone();
    let key_path_clone = key_path.clone();
    let exec_session = tokio::task::spawn_blocking(move || {
        ssh::create_exec_session(
            &host_clone,
            port,
            &username,
            password_clone.as_deref(),
            key_path_clone.as_deref(),
        )
    })
    .await
    .map_err(|e| format!("exec session task failed: {}", e))?
    .map_err(|e| format!("exec session: {}", e))?;

    // Open new connection with same session_id
    let handle = ssh::connect(
        window.clone(),
        session_id.clone(),
        host_id,
        host.host.clone(),
        host.port as u16,
        host.username.clone(),
        password.clone(),
        key_path.clone(),
        80,
        24,
    )?;

    {
        let mut sessions = state.sessions.lock().map_err(|e| e.to_string())?;
        sessions.insert(session_id.clone(), handle);
    }
    {
        let mut exec_sessions = state.exec_sessions.lock().map_err(|e| e.to_string())?;
        exec_sessions.insert(host_id, Arc::new(Mutex::new(exec_session)));
    }

    // The tab's SFTP handle died with the connection. Replace it under the
    // same id so the panel keeps working instead of holding a dead session
    // until the tab is closed. A failure here is not fatal to the shell.
    if let Some(sftp_id) = sftp_session_id {
        let sftp_host = host.host.clone();
        let sftp_port = host.port as u16;
        let sftp_user = host.username.clone();
        let sftp_password = password.clone();
        let sftp_key = key_path.clone();
        match tokio::task::spawn_blocking(move || {
            sftp::sftp_connect(
                sftp_host,
                sftp_port,
                sftp_user,
                sftp_password,
                sftp_key,
                host_id,
            )
        })
        .await
        {
            Ok(Ok(handle)) => {
                let mut sftp_sessions = state.sftp_sessions.lock().map_err(|e| e.to_string())?;
                sftp_sessions.insert(sftp_id, Arc::new(handle));
            }
            Ok(Err(e)) => tracing::warn!("SFTP reconnect failed: {}", e),
            Err(e) => tracing::warn!("SFTP reconnect task failed: {}", e),
        }
    }

    let _ = window.emit("ssh-reconnected", session_id);
    Ok(())
}

#[tauri::command]
#[instrument(skip(state), fields(host_id))]
async fn sftp_connect(
    state: State<'_, AppState>,
    host_id: i64,
    password: Option<String>,
) -> Result<String, String> {
    let host = load_host(&state, host_id)?;

    let (password, key_path) = resolve_auth(&host, password)?;

    let sftp_id = uuid::Uuid::new_v4().to_string();
    let sftp_id_clone = sftp_id.clone();
    let host_host = host.host.clone();
    let port = host.port as u16;
    let username = host.username.clone();

    // Run blocking SFTP connect off the async thread
    let handle = tokio::task::spawn_blocking(move || {
        sftp::sftp_connect(host_host, port, username, password, key_path, host_id)
    })
    .await
    .map_err(|e| format!("sftp connect task failed: {}", e))?
    .map_err(|e| format!("sftp connect: {}", e))?;

    let mut sftp_sessions = state.sftp_sessions.lock().map_err(|e| e.to_string())?;
    sftp_sessions.insert(sftp_id.clone(), Arc::new(handle));

    info!(sftp_id = %sftp_id_clone, host_id = host_id, "SFTP connected");
    Ok(sftp_id)
}

#[tauri::command]
async fn sftp_list(
    state: State<'_, AppState>,
    sftp_session_id: String,
    path: String,
) -> Result<Vec<sftp::SftpFile>, String> {
    sftp_blocking(&state, &sftp_session_id, move |handle| {
        sftp::sftp_list(handle, &path)
    })
    .await
}

#[tauri::command]
async fn sftp_upload(
    window: Window,
    state: State<'_, AppState>,
    sftp_session_id: String,
    local_path: String,
    remote_path: String,
) -> Result<(), String> {
    let session_id = sftp_session_id.clone();
    sftp_blocking(&state, &sftp_session_id, move |handle| {
        sftp::sftp_upload(window, &session_id, handle, &local_path, &remote_path)
    })
    .await
}

#[tauri::command]
async fn sftp_download(
    window: Window,
    state: State<'_, AppState>,
    sftp_session_id: String,
    remote_path: String,
) -> Result<String, String> {
    let file_name = Path::new(&remote_path)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "download".to_string());
    let download_dir = dirs::download_dir()
        .or_else(|| dirs::home_dir().map(|h| h.join("Downloads")))
        .unwrap_or_else(std::env::temp_dir);
    let local_path = download_dir.join(&file_name);
    let local_path_str = local_path.to_string_lossy().to_string();
    let local_path_str_for_dl = local_path_str.clone();
    let session_id = sftp_session_id.clone();
    sftp_blocking(&state, &sftp_session_id, move |handle| {
        sftp::sftp_download(
            window,
            &session_id,
            handle,
            &remote_path,
            &local_path_str_for_dl,
        )
    })
    .await?;
    Ok(local_path_str)
}

fn shell_escape(s: &str) -> String {
    if s.is_empty() {
        return "''".to_string();
    }
    if s.chars()
        .all(|c| c.is_alphanumeric() || "_-./:@".contains(c))
    {
        return s.to_string();
    }
    format!("'{}'", s.replace('\'', "'\"'\"'"))
}

#[tauri::command]
async fn sftp_download_dir(
    _window: Window,
    state: State<'_, AppState>,
    sftp_session_id: String,
    remote_path: String,
) -> Result<String, String> {
    // Get SFTP handle and host_id
    let handle = sftp_handle(&state, &sftp_session_id)?;
    let host_id = handle.host_id;

    // Get exec session for tar command
    let exec_session = exec_session(&state, host_id, "No exec session for this host")?;

    let folder_name = Path::new(&remote_path)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "archive".to_string());
    let parent_path = Path::new(&remote_path)
        .parent()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|| "/".to_string());

    let remote_temp = format!("/tmp/termdrop-{}.tar.gz", uuid::Uuid::new_v4());

    // Create tar.gz on remote
    let exec_session_clone = exec_session.clone();
    let remote_temp_clone = remote_temp.clone();
    let parent_path_clone = parent_path.clone();
    let folder_name_clone = folder_name.clone();
    tokio::task::spawn_blocking(move || {
        let session = exec_session_clone.lock().map_err(|e| e.to_string())?;
        let mut channel = session.channel_session().map_err(|e| e.to_string())?;
        let cmd = format!(
            "tar -czf {} -C {} {}",
            shell_escape(&remote_temp_clone),
            shell_escape(&parent_path_clone),
            shell_escape(&folder_name_clone)
        );
        channel.exec(&cmd).map_err(|e| e.to_string())?;
        let mut stdout = String::new();
        use std::io::Read;
        channel
            .read_to_string(&mut stdout)
            .map_err(|e| e.to_string())?;
        let mut stderr = String::new();
        channel
            .stderr()
            .read_to_string(&mut stderr)
            .map_err(|e| e.to_string())?;
        channel.wait_close().ok();
        let status = channel.exit_status().unwrap_or(0);
        if status != 0 {
            return Err(format!(
                "tar failed: {}",
                if stderr.is_empty() { stdout } else { stderr }
            ));
        }
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())??;

    // Download the archive (blocking I/O off the async thread)
    let download_dir = dirs::download_dir()
        .or_else(|| dirs::home_dir().map(|h| h.join("Downloads")))
        .unwrap_or_else(std::env::temp_dir);
    let archive_name = format!("{}.tar.gz", folder_name);
    let local_archive = download_dir.join(&archive_name);
    let local_archive_str = local_archive.to_string_lossy().to_string();
    let remote_temp_clone = remote_temp.clone();
    let handle_clone = handle.clone();
    let local_archive_str_for_dl = local_archive_str.clone();

    tokio::task::spawn_blocking(move || {
        sftp::sftp_download_simple(&handle_clone, &remote_temp_clone, &local_archive_str_for_dl)
    })
    .await
    .map_err(|e| format!("download task failed: {}", e))?
    .map_err(|e| format!("download failed: {}", e))?;

    // Extract locally
    let extract_dir = download_dir.join(&folder_name);
    let extract_dir_str = extract_dir.to_string_lossy().to_string();
    let local_archive_str_clone = local_archive_str.clone();
    let extract_dir_str_clone = extract_dir_str.clone();
    tokio::task::spawn_blocking(move || {
        std::fs::create_dir_all(&extract_dir_str_clone)
            .map_err(|e| format!("create dir: {}", e))?;
        let output = std::process::Command::new("tar")
            .args([
                "-xzf",
                &local_archive_str_clone,
                "-C",
                &extract_dir_str_clone,
                "--strip-components=1",
            ])
            .output()
            .map_err(|e| format!("extract: {}", e))?;
        if !output.status.success() {
            let err = String::from_utf8_lossy(&output.stderr);
            return Err(format!("extract failed: {}", err));
        }
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())??;

    // Clean up local archive and remote temp (best effort)
    let local_archive_clone = local_archive.clone();
    let remote_temp_clone = remote_temp.clone();
    let handle_clone = handle.clone();
    let _ = tokio::task::spawn_blocking(move || {
        let _ = std::fs::remove_file(&local_archive_clone);
        let _ = sftp::sftp_delete(&handle_clone, &remote_temp_clone);
    })
    .await;

    Ok(extract_dir_str)
}

#[tauri::command]
async fn sftp_write_file(
    state: State<'_, AppState>,
    sftp_session_id: String,
    remote_path: String,
    content: String,
) -> Result<(), String> {
    sftp_blocking(&state, &sftp_session_id, move |handle| {
        sftp::sftp_write_file(handle, &remote_path, &content)
    })
    .await
}

/// Size and mtime of a remote file, used to detect that it changed while an
/// editor had it open.
#[tauri::command]
async fn sftp_stat_file(
    state: State<'_, AppState>,
    sftp_session_id: String,
    remote_path: String,
) -> Result<sftp::SftpStat, String> {
    sftp_blocking(&state, &sftp_session_id, move |handle| {
        sftp::sftp_stat_file(handle, &remote_path)
    })
    .await
}

#[tauri::command]
async fn sftp_realpath(
    state: State<'_, AppState>,
    sftp_session_id: String,
    remote_path: String,
) -> Result<String, String> {
    sftp_blocking(&state, &sftp_session_id, move |handle| {
        sftp::sftp_realpath(handle, &remote_path)
    })
    .await
}

#[tauri::command]
async fn sftp_delete(
    state: State<'_, AppState>,
    sftp_session_id: String,
    remote_path: String,
) -> Result<(), String> {
    sftp_blocking(&state, &sftp_session_id, move |handle| {
        sftp::sftp_delete(handle, &remote_path)
    })
    .await
}

#[tauri::command]
async fn sftp_rename(
    state: State<'_, AppState>,
    sftp_session_id: String,
    old_path: String,
    new_path: String,
) -> Result<(), String> {
    sftp_blocking(&state, &sftp_session_id, move |handle| {
        sftp::sftp_rename(handle, &old_path, &new_path)
    })
    .await
}

#[tauri::command]
async fn sftp_mkdir(
    state: State<'_, AppState>,
    sftp_session_id: String,
    remote_path: String,
) -> Result<(), String> {
    sftp_blocking(&state, &sftp_session_id, move |handle| {
        sftp::sftp_mkdir(handle, &remote_path)
    })
    .await
}

#[tauri::command]
async fn sftp_rmdir(
    state: State<'_, AppState>,
    sftp_session_id: String,
    remote_path: String,
) -> Result<(), String> {
    sftp_blocking(&state, &sftp_session_id, move |handle| {
        sftp::sftp_rmdir(handle, &remote_path)
    })
    .await
}

#[tauri::command]
async fn sftp_read_file(
    state: State<'_, AppState>,
    sftp_session_id: String,
    remote_path: String,
) -> Result<String, String> {
    sftp_blocking(&state, &sftp_session_id, move |handle| {
        sftp::sftp_read_file(handle, &remote_path)
    })
    .await
}

#[tauri::command]
async fn sftp_read_file_base64(
    state: State<'_, AppState>,
    sftp_session_id: String,
    remote_path: String,
) -> Result<String, String> {
    sftp_blocking(&state, &sftp_session_id, move |handle| {
        sftp::sftp_read_file_base64(handle, &remote_path)
    })
    .await
}

#[tauri::command]
fn sftp_disconnect(state: State<'_, AppState>, sftp_session_id: String) -> Result<(), String> {
    let mut sftp_sessions = state.sftp_sessions.lock().map_err(|e| e.to_string())?;
    sftp_sessions.remove(&sftp_session_id);
    Ok(())
}

#[tauri::command]
async fn exec_pty_connect(
    window: Window,
    state: State<'_, AppState>,
    host_id: i64,
    pty_session_id: String,
    command: String,
) -> Result<String, String> {
    let host = load_host(&state, host_id)?;

    let (password, key_path) = resolve_auth(&host, None)?;

    let handle = ssh::exec_pty_connect(
        window,
        pty_session_id.clone(),
        host_id,
        host.host.clone(),
        host.port as u16,
        host.username.clone(),
        password.clone(),
        key_path.clone(),
        command,
    )?;

    {
        let mut sessions = state.exec_pty_sessions.lock().map_err(|e| e.to_string())?;
        sessions.insert(pty_session_id.clone(), handle);
    }

    Ok(pty_session_id)
}

#[tauri::command]
fn exec_pty_write(
    state: State<'_, AppState>,
    pty_session_id: String,
    data: String,
) -> Result<(), String> {
    let sessions = state.exec_pty_sessions.lock().map_err(|e| e.to_string())?;
    let session = sessions
        .get(&pty_session_id)
        .ok_or("PTY session not found")?;
    session.write_tx.send(data).map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
fn exec_pty_disconnect(state: State<'_, AppState>, pty_session_id: String) -> Result<(), String> {
    let mut sessions = state.exec_pty_sessions.lock().map_err(|e| e.to_string())?;
    if let Some(session) = sessions.remove(&pty_session_id) {
        let _ = session.disconnect_tx.send(());
    }
    Ok(())
}

#[tauri::command]
fn update_host_group(state: State<'_, AppState>, id: i64, group: String) -> Result<(), String> {
    with_db(&state, |conn| db::update_host_group(conn, id, &group))
}

#[tauri::command]
fn batch_update_host_group(
    state: State<'_, AppState>,
    old_group: String,
    new_group: String,
) -> Result<usize, String> {
    with_db(&state, |conn| {
        db::update_hosts_group_by_name(conn, &old_group, &new_group)
    })
}

#[tauri::command]
fn batch_clear_host_group(state: State<'_, AppState>, group: String) -> Result<usize, String> {
    with_db(&state, |conn| db::clear_hosts_group_by_name(conn, &group))
}

#[tauri::command]
fn update_host_favorite(state: State<'_, AppState>, id: i64, favorite: i64) -> Result<(), String> {
    with_db(&state, |conn| db::update_host_favorite(conn, id, favorite))
}

#[tauri::command]
fn update_host_last_connected(state: State<'_, AppState>, id: i64) -> Result<(), String> {
    with_db(&state, |conn| db::update_host_last_connected(conn, id))
}

#[tauri::command]
fn export_hosts(state: State<'_, AppState>) -> Result<String, String> {
    let hosts = with_db(&state, db::export_hosts)?;
    serde_json::to_string(&hosts).map_err(|e| e.to_string())
}

#[tauri::command]
/// Import hosts from either source. Rows that fail validation or insertion
/// are reported individually rather than aborting the import, so the caller
/// can tell the user exactly what landed and what did not.
fn import_hosts(
    state: State<'_, AppState>,
    entries: Vec<db::ImportEntry>,
) -> Result<db::ImportSummary, String> {
    let mut conn = state.db.get().map_err(db_err)?;
    // One transaction instead of one fsync per row.
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    let summary = db::import_entries(&tx, entries);
    tx.commit().map_err(|e| e.to_string())?;
    Ok(summary)
}

#[tauri::command]
fn get_port_forwards(
    state: State<'_, AppState>,
    host_id: i64,
) -> Result<Vec<db::PortForward>, String> {
    with_db(&state, |conn| db::get_port_forwards(conn, host_id))
}

#[tauri::command]
fn add_port_forward(
    state: State<'_, AppState>,
    forward: db::NewPortForward,
) -> Result<i64, String> {
    with_db(&state, |conn| db::add_port_forward(conn, &forward))
}

#[tauri::command]
fn update_port_forward(
    state: State<'_, AppState>,
    id: i64,
    forward: db::NewPortForward,
) -> Result<(), String> {
    with_db(&state, |conn| db::update_port_forward(conn, id, &forward))
}

#[tauri::command]
fn delete_port_forward(state: State<'_, AppState>, id: i64) -> Result<(), String> {
    state.forward_manager.stop(id);
    with_db(&state, |conn| db::delete_port_forward(conn, id))
}

#[tauri::command]
fn start_port_forward(state: State<'_, AppState>, rule_id: i64) -> Result<(), String> {
    let (host, forward) = {
        let conn = state.db.get().map_err(db_err)?;
        let forward = db::get_port_forward_by_id(&conn, rule_id)
            .map_err(|e| e.to_string())?
            .ok_or("Port forward rule not found")?;
        let host = db::get_host_by_id(&conn, forward.host_id)
            .map_err(|e| e.to_string())?
            .ok_or("Host not found")?;
        (host, forward)
    };

    let (password, key_path) = resolve_auth(&host, None)?;

    match forward.kind.as_str() {
        "local" => {
            let remote_host = forward.remote_host.ok_or("Remote host not set")?;
            let remote_port = forward.remote_port.ok_or("Remote port not set")? as u16;
            state.forward_manager.start_local(
                rule_id,
                host.host,
                host.port as u16,
                host.username,
                password,
                key_path,
                forward.local_host,
                forward.local_port as u16,
                remote_host,
                remote_port,
            )?;
        }
        "dynamic" => {
            state.forward_manager.start_dynamic(
                rule_id,
                host.host,
                host.port as u16,
                host.username,
                password,
                key_path,
                forward.local_host,
                forward.local_port as u16,
            )?;
        }
        _ => return Err(format!("Unsupported forward kind: {}", forward.kind)),
    }

    Ok(())
}

#[tauri::command]
fn stop_port_forward(state: State<'_, AppState>, rule_id: i64) -> Result<(), String> {
    state.forward_manager.stop(rule_id);
    Ok(())
}

#[tauri::command]
fn get_port_forward_status(state: State<'_, AppState>, rule_id: i64) -> Result<bool, String> {
    Ok(state.forward_manager.is_active(rule_id))
}

#[tauri::command]
fn write_file(path: String, content: String) -> Result<(), String> {
    std::fs::write(&path, content).map_err(|e| e.to_string())
}

#[tauri::command]
async fn docker_ps(
    state: State<'_, AppState>,
    host_id: i64,
    all: bool,
) -> Result<Vec<docker::Container>, String> {
    cached_per_host(
        &state,
        &state.docker_cache,
        &state.docker_ps_fetching,
        host_id,
        5,
        15,
        false,
        move |session| docker::docker_ps(session, all),
    )
    .await
}

fn invalidate_docker_cache(state: &State<'_, AppState>, host_id: i64) -> Result<(), String> {
    let mut cache = state.docker_cache.lock().map_err(|e| e.to_string())?;
    cache.remove(&host_id);
    Ok(())
}

#[tauri::command]
async fn docker_start(
    state: State<'_, AppState>,
    host_id: i64,
    container_id: String,
) -> Result<(), String> {
    invalidate_docker_cache(&state, host_id)?;
    with_exec_session(&state, host_id, 60, move |session| {
        docker::docker_start(session, &container_id)
    })
    .await
}

#[tauri::command]
async fn docker_stop(
    state: State<'_, AppState>,
    host_id: i64,
    container_id: String,
) -> Result<(), String> {
    invalidate_docker_cache(&state, host_id)?;
    with_exec_session(&state, host_id, 60, move |session| {
        docker::docker_stop(session, &container_id)
    })
    .await
}

#[tauri::command]
async fn docker_restart(
    state: State<'_, AppState>,
    host_id: i64,
    container_id: String,
) -> Result<(), String> {
    invalidate_docker_cache(&state, host_id)?;
    with_exec_session(&state, host_id, 60, move |session| {
        docker::docker_restart(session, &container_id)
    })
    .await
}

#[tauri::command]
async fn docker_inspect_shell(
    state: State<'_, AppState>,
    host_id: i64,
    container_id: String,
) -> Result<String, String> {
    with_exec_session(&state, host_id, 60, move |session| {
        docker::docker_inspect_shell(session, &container_id)
    })
    .await
}

#[tauri::command]
async fn docker_install(state: State<'_, AppState>, host_id: i64) -> Result<String, String> {
    invalidate_docker_cache(&state, host_id)?;
    with_exec_session(&state, host_id, 60, move |session| {
        docker::install_docker(session)
    })
    .await
}

#[tauri::command]
async fn run_security_audit(
    state: State<'_, AppState>,
    host_id: i64,
    force: bool,
) -> Result<security::SecurityReport, String> {
    cached_per_host(
        &state,
        &state.security_report_cache,
        &state.security_report_fetching,
        host_id,
        30,
        300,
        force,
        security::run_security_audit,
    )
    .await
}

#[tauri::command]
async fn get_system_stats(
    state: State<'_, AppState>,
    host_id: i64,
) -> Result<serde_json::Value, String> {
    with_exec_session(&state, host_id, 60, move |session| {
        system::get_system_stats(session)
    })
    .await
}

#[tauri::command]
async fn get_system_panel(
    state: State<'_, AppState>,
    host_id: i64,
) -> Result<system::SystemPanel, String> {
    with_exec_session(&state, host_id, 60, move |session| {
        system::get_system_panel(session)
    })
    .await
}

#[tauri::command]
fn get_setting(state: State<'_, AppState>, key: String) -> Result<Option<String>, String> {
    with_db(&state, |conn| db::get_setting(conn, &key))
}

#[tauri::command]
fn set_setting(state: State<'_, AppState>, key: String, value: String) -> Result<(), String> {
    with_db(&state, |conn| db::set_setting(conn, &key, &value))
}

#[tauri::command]
async fn mongodb_list_databases(
    state: State<'_, AppState>,
    host_id: i64,
) -> Result<Vec<String>, String> {
    let client = mongo_client(&state, host_id).await?;
    with_async_timeout(mongodb::list_databases_with(&client), 30).await
}

#[tauri::command]
async fn mongodb_list_collections(
    state: State<'_, AppState>,
    host_id: i64,
    db: String,
) -> Result<Vec<String>, String> {
    let client = mongo_client(&state, host_id).await?;
    with_async_timeout(mongodb::list_collections_with(&client, &db), 30).await
}

#[tauri::command]
async fn mongodb_dump(
    window: Window,
    state: State<'_, AppState>,
    op_id: String,
    host_id: i64,
    db: String,
    collections: Vec<String>,
    output_dir: String,
    is_archive: bool,
) -> Result<(), String> {
    let uri = load_mongo_uri(&state, host_id)?;
    with_mongo_op(&state, &op_id, |cancelled, mongo_ops| {
        mongodb::dump_collections(
            window,
            cancelled,
            mongo_ops,
            op_id.clone(),
            &uri,
            &db,
            collections,
            &output_dir,
            is_archive,
        )
    })
    .await
}

#[tauri::command]
async fn mongodb_restore(
    window: Window,
    state: State<'_, AppState>,
    op_id: String,
    host_id: i64,
    db: String,
    collections: Vec<String>,
    input_dir: String,
    is_archive: bool,
    drop_first: bool,
) -> Result<(), String> {
    let uri = load_mongo_uri(&state, host_id)?;
    with_mongo_op(&state, &op_id, |cancelled, mongo_ops| {
        mongodb::restore_collections(
            window,
            cancelled,
            mongo_ops,
            op_id.clone(),
            &uri,
            &db,
            collections,
            &input_dir,
            is_archive,
            drop_first,
        )
    })
    .await
}

#[tauri::command]
async fn mongodb_restore_archive(
    window: Window,
    state: State<'_, AppState>,
    op_id: String,
    host_id: i64,
    includes: Vec<String>,
    input_path: String,
    drop_first: bool,
) -> Result<(), String> {
    let uri = load_mongo_uri(&state, host_id)?;
    with_mongo_op(&state, &op_id, |cancelled, mongo_ops| {
        mongodb::restore_archive(
            window,
            cancelled,
            mongo_ops,
            op_id.clone(),
            &uri,
            includes,
            &input_path,
            drop_first,
        )
    })
    .await
}

/// Read a page of documents from a collection.
///
/// Read-only by construction: there is no aggregate command and no write
/// command, so a pipeline stage like $out or $merge cannot reach the server
/// from here.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
async fn mongodb_find(
    state: State<'_, AppState>,
    host_id: i64,
    db: String,
    collection: String,
    filter: Option<String>,
    sort: Option<String>,
    projection: Option<String>,
    skip: u64,
    limit: i64,
) -> Result<mongodb::FindResult, String> {
    let client = mongo_client(&state, host_id).await?;
    let filter = mongodb::parse_filter(filter.as_deref().unwrap_or(""))?;
    let sort = parse_optional_doc(sort.as_deref(), "sort")?;
    let projection = parse_optional_doc(projection.as_deref(), "projection")?;

    with_async_timeout(
        mongodb::find_documents(
            &client,
            &db,
            &collection,
            filter,
            sort,
            projection,
            skip,
            limit,
        ),
        60,
    )
    .await
}

/// Count documents matching a filter.
#[tauri::command]
async fn mongodb_count(
    state: State<'_, AppState>,
    host_id: i64,
    db: String,
    collection: String,
    filter: Option<String>,
) -> Result<mongodb::CountResult, String> {
    let client = mongo_client(&state, host_id).await?;
    let filter = mongodb::parse_filter(filter.as_deref().unwrap_or(""))?;
    with_async_timeout(
        mongodb::count_documents(&client, &db, &collection, filter),
        60,
    )
    .await
}

/// A collection's indexes.
#[tauri::command]
async fn mongodb_list_indexes(
    state: State<'_, AppState>,
    host_id: i64,
    db: String,
    collection: String,
) -> Result<Vec<String>, String> {
    let client = mongo_client(&state, host_id).await?;
    with_async_timeout(mongodb::list_indexes(&client, &db, &collection), 30).await
}

/// A collection's storage statistics.
#[tauri::command]
async fn mongodb_collection_stats(
    state: State<'_, AppState>,
    host_id: i64,
    db: String,
    collection: String,
) -> Result<String, String> {
    let client = mongo_client(&state, host_id).await?;
    with_async_timeout(mongodb::collection_stats(&client, &db, &collection), 30).await
}

/// Parse an optional user-supplied JSON document, naming the field on failure.
fn parse_optional_doc(
    text: Option<&str>,
    field: &str,
) -> Result<Option<::mongodb::bson::Document>, String> {
    match text.map(str::trim).filter(|t| !t.is_empty()) {
        None => Ok(None),
        Some(t) => serde_json::from_str(t)
            .map(Some)
            .map_err(|e| format!("{} is not valid JSON: {}", field, e)),
    }
}

/// Release the pooled clients for a host, called when its tab closes.
#[tauri::command]
async fn mongodb_disconnect(state: State<'_, AppState>, host_id: i64) -> Result<(), String> {
    forget_mongo_clients(&state, host_id).await;
    Ok(())
}

/// Store the password for one side of a MongoDB host.
///
/// Only ever *sets* a secret. Clearing one is `mongodb_clear_secret`, so an
/// empty password field in the edit dialog can never silently delete it.
#[tauri::command]
fn mongodb_store_secret(host_id: i64, password: String) -> Result<(), String> {
    if password.is_empty() {
        return Err("refusing to store an empty MongoDB password".to_string());
    }
    crypto::store_secret(&mongo_account(host_id), &password)
}

/// Whether a password is stored, so the UI can say so without revealing it.
#[tauri::command]
fn mongodb_has_secret(host_id: i64) -> bool {
    crypto::get_secret(&mongo_account(host_id)).is_ok()
}

/// Forget the stored password for one side.
#[tauri::command]
fn mongodb_clear_secret(host_id: i64) -> Result<(), String> {
    crypto::delete_secret(&mongo_account(host_id))
}

// ---------------------------------------------------------------------------
// Redis
// ---------------------------------------------------------------------------

/// The keyring account holding a host's Redis password.
///
/// Non-numeric by construction, so it cannot collide with an SSH host password
/// (stored under the bare id) or with `mongo-remote-<id>` in the shared
/// fallback file. `crypto::tests::named_accounts_cannot_collide_with_host_ids`
/// pins that property.
fn redis_account(host_id: i64) -> String {
    format!("redis-{}", host_id)
}

/// The full connection URI for a Redis host.
///
/// As with MongoDB, the stored URI carries no password and the secret is
/// spliced in here, so the credential never crosses the IPC boundary.
///
/// Redis's usual form has no username at all (`redis://:password@host`), so a
/// stripped URI is `redis://@host`; `uri::expects_password` recognises that as
/// still wanting a secret. The error wording repeats the SSH path's
/// "keyring retrieve failed" so one frontend detector drives prompt-and-retry
/// for all three.
fn load_redis_uri(state: &State<'_, AppState>, host_id: i64) -> Result<String, String> {
    let host = with_db(state, |conn| db::get_host_by_id(conn, host_id))?
        .ok_or_else(|| format!("host {} not found", host_id))?;

    let uri = host
        .redis_uri
        .filter(|u| !u.trim().is_empty())
        .ok_or_else(|| "no Redis connection configured".to_string())?;

    // Already carries a password (a row written by hand or imported).
    if uri::split_password(&uri).1.is_some() {
        return Ok(uri);
    }

    match crypto::get_secret(&redis_account(host_id)) {
        Ok(password) => Ok(uri::with_password(&uri, &password)),
        Err(e) if uri::expects_password(&uri) => {
            Err(format!("keyring retrieve failed for Redis: {}", e))
        }
        // No credentials in the URI, so none are expected.
        Err(_) => Ok(uri),
    }
}

/// Bring up this host's SSH tunnel if it has one, returning the local port.
///
/// The tunnel is opened once per host and reused by every database connection,
/// so a host with four databases browsed at once still holds one SSH session.
async fn redis_tunnel_port(
    state: &State<'_, AppState>,
    host: &db::Host,
    ssh_password: Option<String>,
) -> Result<u16, String> {
    let via_id = host
        .redis_tunnel_host_id
        .ok_or_else(|| "this Redis connection has no tunnel".to_string())?;

    if let Some(tunnel) = state.redis_tunnels.lock().await.get(&host.id) {
        return Ok(tunnel.local_port);
    }

    let via = with_db(state, |conn| db::get_host_by_id(conn, via_id))?.ok_or_else(|| {
        "the SSH host this Redis connection tunnels through no longer exists".to_string()
    })?;

    // The URI is written from the bastion's point of view, so its host and port
    // are the tunnel's remote end.
    let uri = host
        .redis_uri
        .as_deref()
        .filter(|u| !u.trim().is_empty())
        .ok_or_else(|| "no Redis connection configured".to_string())?;
    let (remote_host, remote_port) = redis::uri_endpoint(uri)?;

    let (auth_password, auth_key_path) = resolve_auth(&via, ssh_password)?;
    let (ssh_host, ssh_port, username) = (via.host.clone(), via.port as u16, via.username.clone());

    // Holding the lock across the handshake keeps two tabs opening at once from
    // building two tunnels, the same reason `mongo_client` holds its lock.
    let mut tunnels = state.redis_tunnels.lock().await;
    if let Some(tunnel) = tunnels.get(&host.id) {
        return Ok(tunnel.local_port);
    }

    let tunnel = tokio::task::spawn_blocking(move || {
        port_forward::start_ephemeral_local(
            ssh_host,
            ssh_port,
            username,
            auth_password,
            auth_key_path,
            remote_host,
            remote_port,
        )
    })
    .await
    .map_err(|e| format!("redis tunnel task panicked: {}", e))??;

    let port = tunnel.local_port;
    tunnels.insert(host.id, tunnel);
    tracing::info!(host_id = host.id, local_port = port, "redis tunnel opened");
    Ok(port)
}

/// The URI to actually dial for a host, tunnelled or not.
///
/// `rediss://` through a tunnel is refused rather than silently downgraded: the
/// driver would see `127.0.0.1` as the hostname and certificate verification
/// could never succeed, so the only way to make it work is to turn verification
/// off. SSH already authenticates and encrypts that hop, so plain `redis://` is
/// the right answer and saying so is better than a checkbox that weakens TLS.
async fn redis_dial_uri(
    state: &State<'_, AppState>,
    host: &db::Host,
    ssh_password: Option<String>,
) -> Result<(String, bool), String> {
    let uri = load_redis_uri(state, host.id)?;
    if host.redis_tunnel_host_id.is_none() {
        return Ok((uri, false));
    }
    if redis::is_tls_uri(&uri) {
        return Err(
            "A rediss:// connection cannot be tunnelled: the certificate names the real \
             host, but through a tunnel the driver only sees 127.0.0.1. The SSH tunnel \
             already encrypts this hop, so use redis:// instead."
                .to_string(),
        );
    }
    let port = redis_tunnel_port(state, host, ssh_password).await?;
    let (rewritten, _original) = redis::retarget_uri(&uri, port)?;
    Ok((rewritten, true))
}

/// A pooled connection to one database of a Redis host, opened on first use.
async fn redis_conn(
    state: &State<'_, AppState>,
    host_id: i64,
    db_index: i64,
) -> Result<::redis::aio::MultiplexedConnection, String> {
    if let Some(conn) = state.redis_conns.lock().await.get(&(host_id, db_index)) {
        return Ok(conn.clone());
    }

    let host = with_db(state, |conn| db::get_host_by_id(conn, host_id))?
        .ok_or_else(|| format!("host {} not found", host_id))?;
    let (uri, _tunnelled) = redis_dial_uri(state, &host, None).await?;

    let mut conns = state.redis_conns.lock().await;
    if let Some(conn) = conns.get(&(host_id, db_index)) {
        return Ok(conn.clone());
    }
    let conn = redis::build_client(&uri, db_index).await?;
    conns.insert((host_id, db_index), conn.clone());
    Ok(conn)
}

/// Drop every pooled connection for a host and close its tunnel.
///
/// Called when the last Redis tab closes and when the host is edited — without
/// the latter a changed URI would stay invisible until the app restarted.
async fn forget_redis(state: &State<'_, AppState>, host_id: i64) {
    state
        .redis_conns
        .lock()
        .await
        .retain(|(id, _), _| *id != host_id);
    // Dropping the handle signals the accept loop to stop.
    state.redis_tunnels.lock().await.remove(&host_id);
}

/// Open a Redis connection and report what the server is.
///
/// `ssh_password` supplies the bastion's password on the prompt-and-retry path,
/// for a tunnelled host whose SSH password is not in the keyring.
#[tauri::command]
async fn redis_connect(
    state: State<'_, AppState>,
    host_id: i64,
    ssh_password: Option<String>,
) -> Result<redis::RedisServerInfo, String> {
    let host = with_db(&state, |conn| db::get_host_by_id(conn, host_id))?
        .ok_or_else(|| format!("host {} not found", host_id))?;

    let (uri, tunnelled) = redis_dial_uri(&state, &host, ssh_password).await?;
    let db_index = redis::uri_db_index(&uri).unwrap_or(0);

    let mut conn = redis::build_client(&uri, db_index).await?;
    let info = with_async_timeout(
        redis::server_info(&mut conn, crate::uri::redact_uri(&uri), tunnelled),
        30,
    )
    .await?;

    state
        .redis_conns
        .lock()
        .await
        .insert((host_id, db_index), conn);
    Ok(info)
}

/// Re-read the server's INFO, so key counts follow a backup or restore.
#[tauri::command]
async fn redis_server_info(
    state: State<'_, AppState>,
    host_id: i64,
) -> Result<redis::RedisServerInfo, String> {
    let host = with_db(&state, |conn| db::get_host_by_id(conn, host_id))?
        .ok_or_else(|| format!("host {} not found", host_id))?;
    let (uri, tunnelled) = redis_dial_uri(&state, &host, None).await?;
    let db_index = redis::uri_db_index(&uri).unwrap_or(0);
    let mut conn = redis_conn(&state, host_id, db_index).await?;
    with_async_timeout(
        redis::server_info(&mut conn, crate::uri::redact_uri(&uri), tunnelled),
        30,
    )
    .await
}

/// Drop this host's pooled connections and close its tunnel.
#[tauri::command]
async fn redis_disconnect(state: State<'_, AppState>, host_id: i64) -> Result<(), String> {
    forget_redis(&state, host_id).await;
    Ok(())
}

/// One page of a database's keyspace.
///
/// `cursor` is a decimal string, not a number: Redis cursors are u64 and would
/// lose precision as a JS number. `"0"` both starts and ends an iteration.
#[allow(clippy::too_many_arguments)]
#[tauri::command]
async fn redis_scan_keys(
    state: State<'_, AppState>,
    host_id: i64,
    db: i64,
    cursor: String,
    pattern: Option<String>,
    type_filter: Option<String>,
    limit: i64,
    with_memory: bool,
    supports_scan_type: bool,
) -> Result<redis::RedisScanPage, String> {
    let mut conn = redis_conn(&state, host_id, db).await?;
    with_async_timeout(
        redis::scan_keys(
            &mut conn,
            &cursor,
            pattern.as_deref().filter(|p| !p.is_empty()),
            type_filter.as_deref().filter(|t| !t.is_empty()),
            limit,
            with_memory,
            supports_scan_type,
        ),
        30,
    )
    .await
}

/// The prefix groups of a database, from a bounded scan.
#[tauri::command]
async fn redis_key_tree(
    state: State<'_, AppState>,
    host_id: i64,
    db: i64,
    pattern: Option<String>,
) -> Result<redis::KeyTree, String> {
    let mut conn = redis_conn(&state, host_id, db).await?;
    with_async_timeout(
        redis::key_tree(
            &mut conn,
            pattern.as_deref().filter(|p| !p.is_empty()),
            redis::TREE_SCAN_CAP,
            Duration::from_secs(3),
        ),
        30,
    )
    .await
}

/// One page of a single key's value.
///
/// `key_b64` carries the key's exact bytes: a key that is not valid UTF-8 has
/// no faithful string form, and addressing it by a lossy one would fetch the
/// wrong key or nothing at all.
#[tauri::command]
async fn redis_key_value(
    state: State<'_, AppState>,
    host_id: i64,
    db: i64,
    key_b64: String,
    cursor: Option<String>,
    offset: i64,
    limit: i64,
) -> Result<redis::RedisValuePage, String> {
    let key = redis::decode_key(&key_b64)?;
    let mut conn = redis_conn(&state, host_id, db).await?;
    with_async_timeout(
        redis::key_value(
            &mut conn,
            &key,
            cursor.as_deref().unwrap_or("0"),
            offset,
            limit,
        ),
        60,
    )
    .await
}

/// Run a cancellable Redis operation, registering its flag for the duration.
async fn with_redis_op<T, Fut>(
    state: &State<'_, AppState>,
    op_id: &str,
    f: impl FnOnce(Arc<AtomicBool>) -> Fut,
) -> Result<T, String>
where
    Fut: std::future::Future<Output = Result<T, String>>,
{
    let cancelled = Arc::new(AtomicBool::new(false));
    state
        .redis_ops
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(op_id.to_string(), cancelled.clone());

    let result = f(cancelled).await;

    state
        .redis_ops
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(op_id);
    result
}

/// Ask an in-flight export or import to stop.
#[tauri::command]
fn redis_cancel(state: State<'_, AppState>, op_id: String) {
    let ops = state.redis_ops.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(flag) = ops.get(&op_id) {
        flag.store(true, Ordering::Relaxed);
    }
}

/// Back up a database to a `.tdredis` file.
///
/// Deliberately not wrapped in `with_async_timeout`: a backup of a large
/// keyspace legitimately runs for a long time, and cancellation is the control
/// the user has over it.
#[allow(clippy::too_many_arguments)]
#[tauri::command]
async fn redis_export(
    window: Window,
    state: State<'_, AppState>,
    op_id: String,
    host_id: i64,
    db: i64,
    pattern: Option<String>,
    output_path: String,
    max_key_bytes: Option<u64>,
) -> Result<redis::RedisOpSummary, String> {
    let host = with_db(&state, |conn| db::get_host_by_id(conn, host_id))?
        .ok_or_else(|| format!("host {} not found", host_id))?;
    let (uri, tunnelled) = redis_dial_uri(&state, &host, None).await?;

    let mut conn = redis_conn(&state, host_id, db).await?;
    let info = redis::server_info(&mut conn, crate::uri::redact_uri(&uri), tunnelled).await?;

    let registry_id = op_id.clone();
    with_redis_op(&state, &registry_id, |cancelled| async move {
        redis::export_keys(
            &window,
            cancelled,
            &op_id,
            &mut conn,
            db,
            pattern.as_deref().filter(|p| !p.is_empty()),
            &output_path,
            max_key_bytes.unwrap_or(redis::MAX_DUMP_KEY_BYTES),
            &uri,
            &info.version,
            &info.mode,
        )
        .await
    })
    .await
}

/// A backup file's header, for the restore dialog.
#[tauri::command]
fn redis_backup_info(path: String) -> Result<redis_backup::BackupHeader, String> {
    redis::read_backup_header(&path)
}

/// Restore a `.tdredis` file into a database.
#[allow(clippy::too_many_arguments)]
#[tauri::command]
async fn redis_import(
    window: Window,
    state: State<'_, AppState>,
    op_id: String,
    host_id: i64,
    db: i64,
    input_path: String,
    replace: bool,
    flush_first: bool,
) -> Result<redis::RedisOpSummary, String> {
    let host = with_db(&state, |conn| db::get_host_by_id(conn, host_id))?
        .ok_or_else(|| format!("host {} not found", host_id))?;
    let (uri, tunnelled) = redis_dial_uri(&state, &host, None).await?;

    let mut conn = redis_conn(&state, host_id, db).await?;
    let info = redis::server_info(&mut conn, crate::uri::redact_uri(&uri), tunnelled).await?;

    let registry_id = op_id.clone();
    with_redis_op(&state, &registry_id, |cancelled| async move {
        redis::import_keys(
            &window,
            cancelled,
            &op_id,
            &mut conn,
            db,
            &input_path,
            replace,
            flush_first,
            &info.version,
        )
        .await
    })
    .await
}

/// Store the password for a Redis host.
///
/// Only ever *sets* a secret, so an empty password field in the edit dialog
/// cannot silently delete one; clearing is `redis_clear_secret`.
#[tauri::command]
fn redis_store_secret(host_id: i64, password: String) -> Result<(), String> {
    if password.is_empty() {
        return Err("refusing to store an empty Redis password".to_string());
    }
    crypto::store_secret(&redis_account(host_id), &password)
}

/// Whether a password is stored, so the UI can say so without revealing it.
#[tauri::command]
fn redis_has_secret(host_id: i64) -> bool {
    crypto::get_secret(&redis_account(host_id)).is_ok()
}

/// Forget the stored password for a Redis host.
#[tauri::command]
fn redis_clear_secret(host_id: i64) -> Result<(), String> {
    crypto::delete_secret(&redis_account(host_id))
}

#[tauri::command]
fn mongodb_cancel(state: State<'_, AppState>, op_id: String) {
    let ops = state.mongo_ops.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(handle) = ops.get(&op_id) {
        handle.cancelled.store(true, Ordering::Relaxed);
    }
}

fn main() {
    // Initialize structured logging to file
    let log_dir = dirs::data_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("termdrop")
        .join("logs");
    std::fs::create_dir_all(&log_dir).ok();

    // A run killed mid-operation cannot drop its --config guard, so clear any
    // credential files an earlier process left in the temp directory.
    mongodb::sweep_stale_config_files();
    let file_appender = tracing_appender::rolling::daily(&log_dir, "termdrop.log");
    let (non_blocking, _guard) = tracing_appender::non_blocking(file_appender);
    let env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        // Not `termdrop::mongodb=debug`: that module logs command arguments, and
        // debug-by-default put connection strings in every user's log file.
        // Opt in with RUST_LOG=termdrop::mongodb=debug when diagnosing.
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));

    let file_layer = tracing_subscriber::fmt::layer()
        .with_writer(non_blocking)
        .with_ansi(false);
    let console_layer = tracing_subscriber::fmt::layer()
        .with_writer(std::io::stderr)
        .with_ansi(false);

    tracing_subscriber::registry()
        .with(env_filter)
        .with(file_layer)
        .with(console_layer)
        .init();
    info!("TermDrop starting up");

    let db_path = dirs::data_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("termdrop.db");
    let manager = SqliteConnectionManager::file(&db_path);
    let pool = Pool::builder()
        .max_size(10)
        .build(manager)
        .expect("Failed to create database pool");

    // Run initialization with a dedicated connection
    {
        let conn = pool.get().expect("Failed to get initial DB connection");
        db::init_db(&conn).expect("Failed to initialize database");
        db::init_port_forwards(&conn).expect("Failed to initialize port forwards");
        db::init_settings(&conn).expect("Failed to initialize settings");
        split_local_mongo_hosts(&conn);
        migrate_mongo_credentials(&conn);
    }

    tauri::Builder::default()
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            let _ = app
                .get_webview_window("main")
                .expect("no main window")
                .set_focus();
        }))
        .plugin(tauri_plugin_window_state::Builder::new().build())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .manage(AppState {
            db: pool,
            sessions: Mutex::new(HashMap::new()),
            exec_sessions: Mutex::new(HashMap::new()),
            sftp_sessions: Mutex::new(HashMap::new()),
            exec_pty_sessions: Mutex::new(HashMap::new()),
            docker_cache: Arc::new(Mutex::new(HashMap::new())),
            docker_ps_fetching: Arc::new(Mutex::new(HashMap::new())),
            security_report_cache: Arc::new(Mutex::new(HashMap::new())),
            security_report_fetching: Arc::new(Mutex::new(HashMap::new())),
            forward_manager: port_forward::ForwardManager::new(),
            mongo_ops: Arc::new(Mutex::new(HashMap::new())),
            mongo_clients: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            redis_conns: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            redis_tunnels: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            redis_ops: Arc::new(Mutex::new(HashMap::new())),
        })
        .invoke_handler(tauri::generate_handler![
            get_hosts,
            add_host,
            update_host,
            delete_host,
            get_host_by_id,
            store_password,
            ssh_connect,
            ssh_write,
            ssh_resize,
            open_data_channel,
            open_exec_pty_data_channel,
            ssh_disconnect,
            ssh_reconnect,
            sftp_connect,
            sftp_list,
            sftp_upload,
            sftp_download,
            sftp_download_dir,
            sftp_delete,
            sftp_rename,
            sftp_mkdir,
            sftp_rmdir,
            sftp_realpath,
            sftp_stat_file,
            sftp_read_file,
            sftp_read_file_base64,
            sftp_write_file,
            sftp_disconnect,
            update_host_group,
            batch_update_host_group,
            batch_clear_host_group,
            update_host_favorite,
            update_host_last_connected,
            export_hosts,
            import_hosts,
            parse_ssh_config,
            write_file,
            get_setting,
            set_setting,
            get_port_forwards,
            add_port_forward,
            update_port_forward,
            delete_port_forward,
            start_port_forward,
            stop_port_forward,
            get_port_forward_status,
            docker_ps,
            docker_start,
            docker_stop,
            docker_restart,
            docker_inspect_shell,
            exec_pty_connect,
            exec_pty_write,
            exec_pty_disconnect,
            docker_install,
            run_security_audit,
            get_system_stats,
            get_system_panel,
            mongodb_list_databases,
            mongodb_list_collections,
            mongodb_dump,
            mongodb_restore,
            mongodb_restore_archive,
            mongodb_cancel,
            mongodb_disconnect,
            mongodb_find,
            mongodb_count,
            mongodb_list_indexes,
            mongodb_collection_stats,
            mongodb_store_secret,
            mongodb_has_secret,
            mongodb_clear_secret,
            redis_connect,
            redis_disconnect,
            redis_server_info,
            redis_scan_keys,
            redis_key_tree,
            redis_key_value,
            redis_export,
            redis_import,
            redis_backup_info,
            redis_cancel,
            redis_store_secret,
            redis_has_secret,
            redis_clear_secret,
            mongodb::scan_restore_folder,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mongo_host(name: &str, remote: Option<&str>, local: Option<&str>) -> db::NewHost {
        db::NewHost {
            name: name.to_string(),
            host: String::new(),
            port: 0,
            username: String::new(),
            auth_type: "password".to_string(),
            key_path: None,
            group: None,
            favorite: None,
            mongo_uri: remote.map(String::from),
            mongo_local_uri: local.map(String::from),
            redis_uri: None,
            redis_tunnel_host_id: None,
        }
    }

    #[test]
    fn migration_moves_passwords_out_and_is_idempotent() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        db::init_db(&conn).unwrap();
        let id = db::add_host(
            &conn,
            &mongo_host("m", Some("mongodb://u:hunter2@remote:27017"), None),
        )
        .unwrap();

        let mut stored: Vec<(String, String)> = Vec::new();
        let count = migrate_mongo_credentials_with(&conn, |a, s| {
            stored.push((a.to_string(), s.to_string()));
            Ok(())
        });

        assert_eq!(count, 1);
        assert_eq!(stored, vec![(mongo_account(id), "hunter2".to_string())]);

        let host = db::get_host_by_id(&conn, id).unwrap().unwrap();
        assert_eq!(host.mongo_uri.unwrap(), "mongodb://u@remote:27017");

        // Running again finds nothing left to move.
        let second = migrate_mongo_credentials_with(&conn, |_, _| {
            panic!("nothing should be stored on a second run")
        });
        assert_eq!(second, 0);
    }

    #[test]
    fn migration_keeps_the_plaintext_when_the_secret_cannot_be_stored() {
        // The ordering matters: if the row were rewritten first and the keyring
        // then failed, the password would be gone for good. Leaving it in place
        // means the migration simply retries on the next launch.
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        db::init_db(&conn).unwrap();
        let id = db::add_host(
            &conn,
            &mongo_host("m", Some("mongodb://u:hunter2@remote:27017"), None),
        )
        .unwrap();

        let count =
            migrate_mongo_credentials_with(&conn, |_, _| Err("keyring unavailable".to_string()));

        assert_eq!(count, 0);
        let host = db::get_host_by_id(&conn, id).unwrap().unwrap();
        assert_eq!(
            host.mongo_uri.unwrap(),
            "mongodb://u:hunter2@remote:27017",
            "the password must survive a failed store"
        );
    }

    #[test]
    fn migration_ignores_uris_that_carry_no_password() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        db::init_db(&conn).unwrap();
        db::add_host(
            &conn,
            &mongo_host("a", Some("mongodb://remote:27017"), None),
        )
        .unwrap();
        db::add_host(
            &conn,
            &mongo_host("b", Some("mongodb://user@remote:27017"), None),
        )
        .unwrap();

        let count =
            migrate_mongo_credentials_with(&conn, |_, _| panic!("there is no password to store"));
        assert_eq!(count, 0);
    }

    #[test]
    fn a_second_connection_becomes_its_own_host() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        db::init_db(&conn).unwrap();
        let id = db::add_host(
            &conn,
            &mongo_host(
                "prod",
                Some("mongodb://u@remote:27017"),
                Some("mongodb://u@local:27017"),
            ),
        )
        .unwrap();

        assert_eq!(split_local_mongo_hosts(&conn), 1);

        // The configured connection is kept rather than discarded.
        let hosts = db::get_hosts(&conn).unwrap();
        let moved = hosts
            .iter()
            .find(|h| h.name == "prod (local)")
            .expect("the local connection should have become its own host");
        assert_eq!(moved.mongo_uri.as_deref(), Some("mongodb://u@local:27017"));
        assert!(moved.mongo_local_uri.is_none());

        // The original keeps its own connection and loses the second one.
        let original = db::get_host_by_id(&conn, id).unwrap().unwrap();
        assert_eq!(
            original.mongo_uri.as_deref(),
            Some("mongodb://u@remote:27017")
        );
        assert!(original.mongo_local_uri.is_none());

        // Idempotent: a second launch creates nothing further.
        assert_eq!(split_local_mongo_hosts(&conn), 0);
        assert_eq!(db::get_hosts(&conn).unwrap().len(), hosts.len());
    }

    #[test]
    fn a_host_without_a_second_connection_is_left_alone() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        db::init_db(&conn).unwrap();
        db::add_host(
            &conn,
            &mongo_host("solo", Some("mongodb://remote:27017"), None),
        )
        .unwrap();

        assert_eq!(split_local_mongo_hosts(&conn), 0);
        assert_eq!(db::get_hosts(&conn).unwrap().len(), 1);
    }

    fn host(auth_type: &str, key_path: Option<&str>) -> db::Host {
        db::Host {
            id: 7,
            name: "n".into(),
            host: "h".into(),
            port: 22,
            username: "u".into(),
            auth_type: auth_type.into(),
            key_path: key_path.map(String::from),
            group: None,
            favorite: 0,
            last_connected_at: None,
            created_at: String::new(),
            mongo_uri: None,
            mongo_local_uri: None,
            redis_uri: None,
            redis_tunnel_host_id: None,
        }
    }

    #[test]
    fn resolve_auth_password_host_uses_override_without_keyring() {
        let r = resolve_auth(&host("password", Some("/ignored")), Some("pw".into())).unwrap();
        assert_eq!(r, (Some("pw".into()), None));
    }

    #[test]
    fn resolve_auth_key_host_ignores_override() {
        let r = resolve_auth(&host("key", Some("/k")), Some("pw".into())).unwrap();
        assert_eq!(r, (None, Some("/k".into())));
        let r = resolve_auth(&host("key", None), None).unwrap();
        assert_eq!(r, (None, None));
    }

    #[test]
    fn resolve_auth_unknown_type_passes_override_and_key_through() {
        let r = resolve_auth(&host("other", Some("/k")), Some("pw".into())).unwrap();
        assert_eq!(r, (Some("pw".into()), Some("/k".into())));
        let r = resolve_auth(&host("other", Some("/k")), None).unwrap();
        assert_eq!(r, (None, Some("/k".into())));
    }
}
