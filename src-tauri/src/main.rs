#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};
use tauri::{Emitter, State, Window};
use r2d2::Pool;
use r2d2_sqlite::SqliteConnectionManager;

mod db;
mod crypto;
mod ssh;
mod sftp;
mod port_forward;
mod docker;
mod security;

pub struct AppState {
    db: Pool<SqliteConnectionManager>,
    sessions: Mutex<HashMap<String, ssh::SshSessionHandle>>,
    exec_sessions: Mutex<HashMap<i64, Arc<Mutex<ssh2::Session>>>>,
    sftp_sessions: Mutex<HashMap<String, Arc<sftp::SftpSessionHandle>>>,
    forward_manager: port_forward::ForwardManager,
}

fn db_err(e: r2d2::Error) -> String {
    e.to_string()
}

#[tauri::command]
fn get_hosts(state: State<'_, AppState>) -> Result<Vec<db::Host>, String> {
    let conn = state.db.get().map_err(db_err)?;
    db::get_hosts(&conn).map_err(|e| e.to_string())
}

#[tauri::command]
fn add_host(state: State<'_, AppState>, host: db::NewHost) -> Result<i64, String> {
    let conn = state.db.get().map_err(db_err)?;
    db::add_host(&conn, &host).map_err(|e| e.to_string())
}

#[tauri::command]
fn update_host(state: State<'_, AppState>, id: i64, host: db::NewHost) -> Result<(), String> {
    let conn = state.db.get().map_err(db_err)?;
    db::update_host(&conn, id, &host).map_err(|e| e.to_string())
}

#[tauri::command]
fn delete_host(state: State<'_, AppState>, id: i64) -> Result<(), String> {
    let conn = state.db.get().map_err(db_err)?;
    db::delete_host(&conn, id).map_err(|e| e.to_string())?;
    crypto::delete_password(id).ok();
    Ok(())
}

#[tauri::command]
fn get_host_by_id(state: State<'_, AppState>, id: i64) -> Result<Option<db::Host>, String> {
    let conn = state.db.get().map_err(db_err)?;
    db::get_host_by_id(&conn, id).map_err(|e| e.to_string())
}

#[tauri::command]
fn store_password(host_id: i64, password: String) -> Result<(), String> {
    crypto::store_password(host_id, &password)
}

#[tauri::command]
async fn ssh_connect(
    window: Window,
    state: State<'_, AppState>,
    host_id: i64,
    password: Option<String>,
) -> Result<String, String> {
    let host = {
        let conn = state.db.get().map_err(db_err)?;
        db::get_host_by_id(&conn, host_id)
            .map_err(|e| e.to_string())?
            .ok_or("Host not found")?
    };

    let (password, key_path) = match host.auth_type.as_str() {
        "password" => {
            let pw = match password {
                Some(p) => Some(p),
                None => Some(crypto::get_password(host_id)?),
            };
            (pw, None)
        }
        "key" => (None, host.key_path.clone()),
        _ => (password, host.key_path.clone()),
    };

    let session_id = uuid::Uuid::new_v4().to_string();
    let handle = ssh::connect(
        window,
        session_id.clone(),
        host_id,
        host.host.clone(),
        host.port as u16,
        host.username.clone(),
        password.clone(),
        key_path.clone(),
    )?;

    // Create a persistent exec session for this host
    let exec_session = ssh::create_exec_session(
        &host.host,
        host.port as u16,
        &host.username,
        password.as_deref(),
        key_path.as_deref(),
    )?;

    {
        let mut sessions = state.sessions.lock().map_err(|e| e.to_string())?;
        sessions.insert(session_id.clone(), handle);
    }
    {
        let mut exec_sessions = state.exec_sessions.lock().map_err(|e| e.to_string())?;
        exec_sessions.insert(host_id, Arc::new(Mutex::new(exec_session)));
    }

    Ok(session_id)
}

#[tauri::command]
fn ssh_write(
    state: State<'_, AppState>,
    session_id: String,
    data: String,
) -> Result<(), String> {
    let sessions = state.sessions.lock().map_err(|e| e.to_string())?;
    let session = sessions.get(&session_id).ok_or("Session not found")?;
    session.write_tx.send(data).map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
fn ssh_disconnect(
    state: State<'_, AppState>,
    session_id: String,
) -> Result<(), String> {
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
    }

    Ok(())
}

#[tauri::command]
async fn ssh_reconnect(
    window: Window,
    state: State<'_, AppState>,
    session_id: String,
    password: Option<String>,
) -> Result<(), String> {
    let host_id = {
        let sessions = state.sessions.lock().map_err(|e| e.to_string())?;
        let session = sessions.get(&session_id).ok_or("Session not found")?;
        session.host_id
    };

    let host = {
        let conn = state.db.get().map_err(db_err)?;
        db::get_host_by_id(&conn, host_id)
            .map_err(|e| e.to_string())?
            .ok_or("Host not found")?
    };

    let (password, key_path) = match host.auth_type.as_str() {
        "password" => {
            let pw = match password {
                Some(p) => Some(p),
                None => Some(crypto::get_password(host_id)?),
            };
            (pw, None)
        }
        "key" => (None, host.key_path.clone()),
        _ => (password, host.key_path.clone()),
    };

    // Remove old session
    {
        let mut sessions = state.sessions.lock().map_err(|e| e.to_string())?;
        if let Some(old) = sessions.remove(&session_id) {
            let _ = old.disconnect_tx.send(());
        }
    }

    // Remove old exec session and create new one
    {
        let mut exec_sessions = state.exec_sessions.lock().map_err(|e| e.to_string())?;
        exec_sessions.remove(&host_id);
    }
    let exec_session = ssh::create_exec_session(
        &host.host,
        host.port as u16,
        &host.username,
        password.as_deref(),
        key_path.as_deref(),
    )?;

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
    )?;

    {
        let mut sessions = state.sessions.lock().map_err(|e| e.to_string())?;
        sessions.insert(session_id.clone(), handle);
    }
    {
        let mut exec_sessions = state.exec_sessions.lock().map_err(|e| e.to_string())?;
        exec_sessions.insert(host_id, Arc::new(Mutex::new(exec_session)));
    }

    let _ = window.emit("ssh-reconnected", session_id);
    Ok(())
}

#[tauri::command]
async fn sftp_connect(
    state: State<'_, AppState>,
    host_id: i64,
    password: Option<String>,
) -> Result<String, String> {
    let host = {
        let conn = state.db.get().map_err(db_err)?;
        db::get_host_by_id(&conn, host_id)
            .map_err(|e| e.to_string())?
            .ok_or("Host not found")?
    };

    let (password, key_path) = match host.auth_type.as_str() {
        "password" => {
            let pw = match password {
                Some(p) => Some(p),
                None => Some(crypto::get_password(host_id)?),
            };
            (pw, None)
        }
        "key" => (None, host.key_path.clone()),
        _ => (password, host.key_path.clone()),
    };

    let sftp_id = uuid::Uuid::new_v4().to_string();
    let handle = sftp::sftp_connect(
        host.host,
        host.port as u16,
        host.username,
        password,
        key_path,
    )?;

    let mut sftp_sessions = state.sftp_sessions.lock().map_err(|e| e.to_string())?;
    sftp_sessions.insert(sftp_id.clone(), Arc::new(handle));

    Ok(sftp_id)
}

#[tauri::command]
fn sftp_list(
    state: State<'_, AppState>,
    sftp_session_id: String,
    path: String,
) -> Result<Vec<sftp::SftpFile>, String> {
    let sftp_sessions = state.sftp_sessions.lock().map_err(|e| e.to_string())?;
    let handle = sftp_sessions.get(&sftp_session_id).ok_or("SFTP session not found")?;
    sftp::sftp_list(handle, &path)
}

#[tauri::command]
async fn sftp_upload(
    window: Window,
    state: State<'_, AppState>,
    sftp_session_id: String,
    local_path: String,
    remote_path: String,
) -> Result<(), String> {
    let handle = {
        let sftp_sessions = state.sftp_sessions.lock().map_err(|e| e.to_string())?;
        sftp_sessions.get(&sftp_session_id).cloned().ok_or("SFTP session not found")?
    };
    sftp::sftp_upload(window, &handle, &local_path, &remote_path)
}

#[tauri::command]
async fn sftp_download(
    window: Window,
    state: State<'_, AppState>,
    sftp_session_id: String,
    remote_path: String,
) -> Result<String, String> {
    let handle = {
        let sftp_sessions = state.sftp_sessions.lock().map_err(|e| e.to_string())?;
        sftp_sessions.get(&sftp_session_id).cloned().ok_or("SFTP session not found")?
    };
    let file_name = Path::new(&remote_path).file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "download".to_string());
    let download_dir = dirs::download_dir()
        .or_else(|| dirs::home_dir().map(|h| h.join("Downloads")))
        .unwrap_or_else(|| std::env::temp_dir());
    let local_path = download_dir.join(&file_name);
    let local_path_str = local_path.to_string_lossy().to_string();
    sftp::sftp_download(window, &handle, &remote_path, &local_path_str)?;
    Ok(local_path_str)
}

#[tauri::command]
fn sftp_realpath(
    state: State<'_, AppState>,
    sftp_session_id: String,
    remote_path: String,
) -> Result<String, String> {
    let sftp_sessions = state.sftp_sessions.lock().map_err(|e| e.to_string())?;
    let handle = sftp_sessions.get(&sftp_session_id).ok_or("SFTP session not found")?;
    sftp::sftp_realpath(handle, &remote_path)
}

#[tauri::command]
fn sftp_delete(
    state: State<'_, AppState>,
    sftp_session_id: String,
    remote_path: String,
) -> Result<(), String> {
    let sftp_sessions = state.sftp_sessions.lock().map_err(|e| e.to_string())?;
    let handle = sftp_sessions.get(&sftp_session_id).ok_or("SFTP session not found")?;
    sftp::sftp_delete(handle, &remote_path)
}

#[tauri::command]
fn sftp_rename(
    state: State<'_, AppState>,
    sftp_session_id: String,
    old_path: String,
    new_path: String,
) -> Result<(), String> {
    let sftp_sessions = state.sftp_sessions.lock().map_err(|e| e.to_string())?;
    let handle = sftp_sessions.get(&sftp_session_id).ok_or("SFTP session not found")?;
    sftp::sftp_rename(handle, &old_path, &new_path)
}

#[tauri::command]
fn sftp_mkdir(
    state: State<'_, AppState>,
    sftp_session_id: String,
    remote_path: String,
) -> Result<(), String> {
    let sftp_sessions = state.sftp_sessions.lock().map_err(|e| e.to_string())?;
    let handle = sftp_sessions.get(&sftp_session_id).ok_or("SFTP session not found")?;
    sftp::sftp_mkdir(handle, &remote_path)
}

#[tauri::command]
fn sftp_rmdir(
    state: State<'_, AppState>,
    sftp_session_id: String,
    remote_path: String,
) -> Result<(), String> {
    let sftp_sessions = state.sftp_sessions.lock().map_err(|e| e.to_string())?;
    let handle = sftp_sessions.get(&sftp_session_id).ok_or("SFTP session not found")?;
    sftp::sftp_rmdir(handle, &remote_path)
}

#[tauri::command]
fn sftp_disconnect(
    state: State<'_, AppState>,
    sftp_session_id: String,
) -> Result<(), String> {
    let mut sftp_sessions = state.sftp_sessions.lock().map_err(|e| e.to_string())?;
    sftp_sessions.remove(&sftp_session_id);
    Ok(())
}

#[tauri::command]
async fn ssh_exec(
    state: State<'_, AppState>,
    host_id: i64,
    command: String,
) -> Result<String, String> {
    // Try to reuse existing exec session
    {
        let exec_sessions = state.exec_sessions.lock().map_err(|e| e.to_string())?;
        if let Some(session_arc) = exec_sessions.get(&host_id) {
            let session = session_arc.lock().map_err(|e| e.to_string())?;
            return ssh::exec_with_session(&session, &command);
        }
    }

    // Fall back: create a new session for this one-off command
    let host = {
        let conn = state.db.get().map_err(db_err)?;
        db::get_host_by_id(&conn, host_id)
            .map_err(|e| e.to_string())?
            .ok_or("Host not found")?
    };

    let (password, key_path) = match host.auth_type.as_str() {
        "password" => {
            let pw = Some(crypto::get_password(host_id)?);
            (pw, None)
        }
        "key" => (None, host.key_path.clone()),
        _ => (None, host.key_path.clone()),
    };

    let session = ssh::create_exec_session(
        &host.host,
        host.port as u16,
        &host.username,
        password.as_deref(),
        key_path.as_deref(),
    )?;
    ssh::exec_with_session(&session, &command)
}

#[tauri::command]
fn update_host_group(state: State<'_, AppState>, id: i64, group: String) -> Result<(), String> {
    let conn = state.db.get().map_err(db_err)?;
    db::update_host_group(&conn, id, &group).map_err(|e| e.to_string())
}

#[tauri::command]
fn batch_update_host_group(state: State<'_, AppState>, old_group: String, new_group: String) -> Result<usize, String> {
    let conn = state.db.get().map_err(db_err)?;
    db::update_hosts_group_by_name(&conn, &old_group, &new_group).map_err(|e| e.to_string())
}

#[tauri::command]
fn batch_clear_host_group(state: State<'_, AppState>, group: String) -> Result<usize, String> {
    let conn = state.db.get().map_err(db_err)?;
    db::clear_hosts_group_by_name(&conn, &group).map_err(|e| e.to_string())
}

#[tauri::command]
fn update_host_favorite(state: State<'_, AppState>, id: i64, favorite: i64) -> Result<(), String> {
    let conn = state.db.get().map_err(db_err)?;
    db::update_host_favorite(&conn, id, favorite).map_err(|e| e.to_string())
}

#[tauri::command]
fn update_host_last_connected(state: State<'_, AppState>, id: i64) -> Result<(), String> {
    let conn = state.db.get().map_err(db_err)?;
    db::update_host_last_connected(&conn, id).map_err(|e| e.to_string())
}

#[tauri::command]
fn export_hosts(state: State<'_, AppState>) -> Result<String, String> {
    let conn = state.db.get().map_err(db_err)?;
    let hosts = db::export_hosts(&conn).map_err(|e| e.to_string())?;
    serde_json::to_string(&hosts).map_err(|e| e.to_string())
}

#[tauri::command]
fn import_hosts(state: State<'_, AppState>, json: String) -> Result<i64, String> {
    let hosts: Vec<db::NewHost> = serde_json::from_str(&json).map_err(|e| e.to_string())?;
    let conn = state.db.get().map_err(db_err)?;
    let mut count = 0;
    for host in hosts {
        db::add_host(&conn, &host).map_err(|e| e.to_string())?;
        count += 1;
    }
    Ok(count)
}

#[tauri::command]
fn get_port_forwards(state: State<'_, AppState>, host_id: i64) -> Result<Vec<db::PortForward>, String> {
    let conn = state.db.get().map_err(db_err)?;
    db::get_port_forwards(&conn, host_id).map_err(|e| e.to_string())
}

#[tauri::command]
fn add_port_forward(state: State<'_, AppState>, forward: db::NewPortForward) -> Result<i64, String> {
    let conn = state.db.get().map_err(db_err)?;
    db::add_port_forward(&conn, &forward).map_err(|e| e.to_string())
}

#[tauri::command]
fn delete_port_forward(state: State<'_, AppState>, id: i64) -> Result<(), String> {
    state.forward_manager.stop(id);
    let conn = state.db.get().map_err(db_err)?;
    db::delete_port_forward(&conn, id).map_err(|e| e.to_string())
}

#[tauri::command]
fn start_port_forward(
    state: State<'_, AppState>,
    rule_id: i64,
) -> Result<(), String> {
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

    let (password, key_path) = match host.auth_type.as_str() {
        "password" => {
            let pw = crypto::get_password(host.id)?;
            (Some(pw), None)
        }
        "key" => (None, host.key_path.clone()),
        _ => (None, host.key_path.clone()),
    };

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
fn stop_port_forward(
    state: State<'_, AppState>,
    rule_id: i64,
) -> Result<(), String> {
    state.forward_manager.stop(rule_id);
    Ok(())
}

#[tauri::command]
fn get_port_forward_status(
    state: State<'_, AppState>,
    rule_id: i64,
) -> Result<bool, String> {
    Ok(state.forward_manager.is_active(rule_id))
}

#[tauri::command]
fn write_file(path: String, content: String) -> Result<(), String> {
    std::fs::write(&path, content).map_err(|e| e.to_string())
}

#[tauri::command]
fn docker_ps(
    state: State<'_, AppState>,
    host_id: i64,
    all: bool,
) -> Result<Vec<docker::Container>, String> {
    let exec_sessions = state.exec_sessions.lock().map_err(|e| e.to_string())?;
    let session_arc = exec_sessions.get(&host_id).ok_or("No active session for this host")?;
    let session = session_arc.lock().map_err(|e| e.to_string())?;
    docker::docker_ps(&session, all).map_err(|e| e.to_string())
}

#[tauri::command]
fn docker_start(
    state: State<'_, AppState>,
    host_id: i64,
    container_id: String,
) -> Result<(), String> {
    let exec_sessions = state.exec_sessions.lock().map_err(|e| e.to_string())?;
    let session_arc = exec_sessions.get(&host_id).ok_or("No active session for this host")?;
    let session = session_arc.lock().map_err(|e| e.to_string())?;
    docker::docker_start(&session, &container_id).map_err(|e| e.to_string())
}

#[tauri::command]
fn docker_stop(
    state: State<'_, AppState>,
    host_id: i64,
    container_id: String,
) -> Result<(), String> {
    let exec_sessions = state.exec_sessions.lock().map_err(|e| e.to_string())?;
    let session_arc = exec_sessions.get(&host_id).ok_or("No active session for this host")?;
    let session = session_arc.lock().map_err(|e| e.to_string())?;
    docker::docker_stop(&session, &container_id).map_err(|e| e.to_string())
}

#[tauri::command]
fn docker_restart(
    state: State<'_, AppState>,
    host_id: i64,
    container_id: String,
) -> Result<(), String> {
    let exec_sessions = state.exec_sessions.lock().map_err(|e| e.to_string())?;
    let session_arc = exec_sessions.get(&host_id).ok_or("No active session for this host")?;
    let session = session_arc.lock().map_err(|e| e.to_string())?;
    docker::docker_restart(&session, &container_id).map_err(|e| e.to_string())
}

#[tauri::command]
fn docker_logs(
    state: State<'_, AppState>,
    host_id: i64,
    container_id: String,
    tail: usize,
) -> Result<String, String> {
    let exec_sessions = state.exec_sessions.lock().map_err(|e| e.to_string())?;
    let session_arc = exec_sessions.get(&host_id).ok_or("No active session for this host")?;
    let session = session_arc.lock().map_err(|e| e.to_string())?;
    docker::docker_logs(&session, &container_id, tail).map_err(|e| e.to_string())
}

#[tauri::command]
fn docker_inspect_shell(
    state: State<'_, AppState>,
    host_id: i64,
    container_id: String,
) -> Result<String, String> {
    let exec_sessions = state.exec_sessions.lock().map_err(|e| e.to_string())?;
    let session_arc = exec_sessions.get(&host_id).ok_or("No active session for this host")?;
    let session = session_arc.lock().map_err(|e| e.to_string())?;
    docker::docker_inspect_shell(&session, &container_id).map_err(|e| e.to_string())
}

#[tauri::command]
fn docker_install(
    state: State<'_, AppState>,
    host_id: i64,
) -> Result<String, String> {
    let exec_sessions = state.exec_sessions.lock().map_err(|e| e.to_string())?;
    let session_arc = exec_sessions.get(&host_id).ok_or("No active session for this host")?;
    let session = session_arc.lock().map_err(|e| e.to_string())?;
    docker::install_docker(&session).map_err(|e| e.to_string())
}

#[tauri::command]
async fn run_security_audit(
    state: State<'_, AppState>,
    host_id: i64,
) -> Result<security::SecurityReport, String> {
    let session_arc = {
        let exec_sessions = state.exec_sessions.lock().map_err(|e| e.to_string())?;
        exec_sessions.get(&host_id).cloned().ok_or("No active session for this host")?
    };
    let session = session_arc.lock().map_err(|e| e.to_string())?;
    security::run_security_audit(&session).map_err(|e| e.to_string())
}

#[tauri::command]
fn get_setting(state: State<'_, AppState>, key: String) -> Result<Option<String>, String> {
    let conn = state.db.get().map_err(db_err)?;
    db::get_setting(&conn, &key).map_err(|e| e.to_string())
}

#[tauri::command]
fn set_setting(state: State<'_, AppState>, key: String, value: String) -> Result<(), String> {
    let conn = state.db.get().map_err(db_err)?;
    db::set_setting(&conn, &key, &value).map_err(|e| e.to_string())
}

fn main() {
    let db_path = dirs::data_dir()
        .unwrap_or_else(|| std::env::temp_dir())
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
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState {
            db: pool,
            sessions: Mutex::new(HashMap::new()),
            exec_sessions: Mutex::new(HashMap::new()),
            sftp_sessions: Mutex::new(HashMap::new()),
            forward_manager: port_forward::ForwardManager::new(),
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
            ssh_disconnect,
            ssh_reconnect,
            ssh_exec,
            sftp_connect,
            sftp_list,
            sftp_upload,
            sftp_download,
            sftp_delete,
            sftp_rename,
            sftp_mkdir,
            sftp_rmdir,
            sftp_realpath,
            sftp_disconnect,
            update_host_group,
            batch_update_host_group,
            batch_clear_host_group,
            update_host_favorite,
            update_host_last_connected,
            export_hosts,
            import_hosts,
            write_file,
            get_setting,
            set_setting,
            get_port_forwards,
            add_port_forward,
            delete_port_forward,
            start_port_forward,
            stop_port_forward,
            get_port_forward_status,
            docker_ps,
            docker_start,
            docker_stop,
            docker_restart,
            docker_logs,
            docker_inspect_shell,
            docker_install,
            run_security_audit,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
