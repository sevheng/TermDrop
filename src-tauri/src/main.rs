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
mod security;
mod sftp;
mod ssh;
mod ssh_config_parser;
mod system;

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
    state.mongo_ops.lock().unwrap().insert(op_id, handle);
    cancelled
}

fn unregister_mongo_op(state: &State<'_, AppState>, op_id: &str) {
    state.mongo_ops.lock().unwrap().remove(op_id);
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
fn update_host(state: State<'_, AppState>, id: i64, host: db::NewHost) -> Result<(), String> {
    with_db(&state, |conn| db::update_host(conn, id, &host))
}

#[tauri::command]
fn delete_host(state: State<'_, AppState>, id: i64) -> Result<(), String> {
    with_db(&state, |conn| db::delete_host(conn, id))?;
    crypto::delete_password(id).ok();
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
fn import_ssh_config_hosts(
    state: State<'_, AppState>,
    hosts: Vec<ssh_config_parser::SshConfigHost>,
) -> Result<usize, String> {
    let mut conn = state.db.get().map_err(db_err)?;
    // One transaction instead of one fsync per row. A row that fails to
    // insert is skipped, as before; SQLite rolls back only that statement.
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    let mut count = 0;
    for h in hosts {
        let new_host = db::NewHost {
            name: h.name,
            host: h.host,
            port: h.port,
            username: h.username,
            auth_type: h.auth_type,
            key_path: h.key_path,
            group: None,
            favorite: None,
            mongo_uri: None,
            mongo_local_uri: None,
        };
        if db::add_host(&tx, &new_host).is_ok() {
            count += 1;
        }
    }
    tx.commit().map_err(|e| e.to_string())?;
    Ok(count)
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
fn import_hosts(state: State<'_, AppState>, json: String) -> Result<i64, String> {
    let hosts: Vec<db::NewHost> = serde_json::from_str(&json).map_err(|e| e.to_string())?;
    let mut conn = state.db.get().map_err(db_err)?;
    // One transaction instead of one fsync per row. On a bad row the rows
    // before it are kept and the error is returned, matching the previous
    // autocommit behavior.
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    let mut count = 0;
    for host in hosts {
        if let Err(e) = db::add_host(&tx, &host) {
            tx.commit().map_err(|e| e.to_string())?;
            return Err(e.to_string());
        }
        count += 1;
    }
    tx.commit().map_err(|e| e.to_string())?;
    Ok(count)
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
async fn mongodb_list_databases(uri: String) -> Result<Vec<String>, String> {
    mongodb::list_databases(&uri).await
}

#[tauri::command]
async fn mongodb_list_collections(uri: String, db: String) -> Result<Vec<String>, String> {
    mongodb::list_collections(&uri, &db).await
}

#[tauri::command]
async fn mongodb_sync(
    window: Window,
    state: State<'_, AppState>,
    op_id: String,
    remote_uri: String,
    local_uri: String,
    db: String,
    collections: Vec<String>,
    drop_first: bool,
) -> Result<(), String> {
    with_mongo_op(&state, &op_id, |cancelled, mongo_ops| {
        mongodb::sync_collections(
            window,
            cancelled,
            mongo_ops,
            op_id.clone(),
            &remote_uri,
            &local_uri,
            &db,
            collections,
            drop_first,
        )
    })
    .await
}

#[tauri::command]
async fn mongodb_dump(
    window: Window,
    state: State<'_, AppState>,
    op_id: String,
    remote_uri: String,
    db: String,
    collections: Vec<String>,
    output_dir: String,
    is_archive: bool,
) -> Result<(), String> {
    with_mongo_op(&state, &op_id, |cancelled, mongo_ops| {
        mongodb::dump_collections(
            window,
            cancelled,
            mongo_ops,
            op_id.clone(),
            &remote_uri,
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
    remote_uri: String,
    db: String,
    collections: Vec<String>,
    input_dir: String,
    is_archive: bool,
) -> Result<(), String> {
    with_mongo_op(&state, &op_id, |cancelled, mongo_ops| {
        mongodb::restore_collections(
            window,
            cancelled,
            mongo_ops,
            op_id.clone(),
            &remote_uri,
            &db,
            collections,
            &input_dir,
            is_archive,
        )
    })
    .await
}

#[tauri::command]
async fn mongodb_restore_archive(
    window: Window,
    state: State<'_, AppState>,
    op_id: String,
    remote_uri: String,
    includes: Vec<String>,
    input_path: String,
) -> Result<(), String> {
    with_mongo_op(&state, &op_id, |cancelled, mongo_ops| {
        mongodb::restore_archive(
            window,
            cancelled,
            mongo_ops,
            op_id.clone(),
            &remote_uri,
            includes,
            &input_path,
        )
    })
    .await
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
    let file_appender = tracing_appender::rolling::daily(&log_dir, "termdrop.log");
    let (non_blocking, _guard) = tracing_appender::non_blocking(file_appender);
    let env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info,termdrop::mongodb=debug"));

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
            import_ssh_config_hosts,
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
            mongodb_sync,
            mongodb_dump,
            mongodb_restore,
            mongodb_restore_archive,
            mongodb_cancel,
            mongodb::scan_restore_folder,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::*;

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
