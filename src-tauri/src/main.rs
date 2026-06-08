#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};
use tauri::{Emitter, Manager, State, Window};
use rusqlite::Connection;

mod db;
mod crypto;
mod ssh;
mod sftp;
mod port_forward;

pub struct AppState {
    db: Mutex<Connection>,
    sessions: Mutex<HashMap<String, ssh::SshSessionHandle>>,
    sftp_sessions: Mutex<HashMap<String, Arc<sftp::SftpSessionHandle>>>,
    forward_manager: port_forward::ForwardManager,
}

#[tauri::command]
fn get_hosts(state: State<'_, AppState>) -> Result<Vec<db::Host>, String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    db::get_hosts(&conn).map_err(|e| e.to_string())
}

#[tauri::command]
fn add_host(state: State<'_, AppState>, host: db::NewHost) -> Result<i64, String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    db::add_host(&conn, &host).map_err(|e| e.to_string())
}

#[tauri::command]
fn update_host(state: State<'_, AppState>, id: i64, host: db::NewHost) -> Result<(), String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    db::update_host(&conn, id, &host).map_err(|e| e.to_string())
}

#[tauri::command]
fn delete_host(state: State<'_, AppState>, id: i64) -> Result<(), String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    db::delete_host(&conn, id).map_err(|e| e.to_string())?;
    crypto::delete_password(id).ok();
    Ok(())
}

#[tauri::command]
fn get_host_by_id(state: State<'_, AppState>, id: i64) -> Result<Option<db::Host>, String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;
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
        let conn = state.db.lock().map_err(|e| e.to_string())?;
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
        host.host,
        host.port as u16,
        host.username,
        password,
        key_path,
    )?;

    let mut sessions = state.sessions.lock().map_err(|e| e.to_string())?;
    sessions.insert(session_id.clone(), handle);

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
    let mut sessions = state.sessions.lock().map_err(|e| e.to_string())?;
    if let Some(session) = sessions.remove(&session_id) {
        let _ = session.disconnect_tx.send(());
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
        let conn = state.db.lock().map_err(|e| e.to_string())?;
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

    // Open new connection with same session_id
    let handle = ssh::connect(
        window.clone(),
        session_id.clone(),
        host_id,
        host.host,
        host.port as u16,
        host.username,
        password,
        key_path,
    )?;

    {
        let mut sessions = state.sessions.lock().map_err(|e| e.to_string())?;
        sessions.insert(session_id.clone(), handle);
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
        let conn = state.db.lock().map_err(|e| e.to_string())?;
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
    let host = {
        let conn = state.db.lock().map_err(|e| e.to_string())?;
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

    ssh::exec(host.host, host.port as u16, host.username, password, key_path, command)
}

#[tauri::command]
fn update_host_group(state: State<'_, AppState>, id: i64, group: String) -> Result<(), String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    db::update_host_group(&conn, id, &group).map_err(|e| e.to_string())
}

#[tauri::command]
fn update_host_favorite(state: State<'_, AppState>, id: i64, favorite: i64) -> Result<(), String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    db::update_host_favorite(&conn, id, favorite).map_err(|e| e.to_string())
}

#[tauri::command]
fn update_host_last_connected(state: State<'_, AppState>, id: i64) -> Result<(), String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    db::update_host_last_connected(&conn, id).map_err(|e| e.to_string())
}

#[tauri::command]
fn export_hosts(state: State<'_, AppState>) -> Result<String, String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    let hosts = db::export_hosts(&conn).map_err(|e| e.to_string())?;
    serde_json::to_string(&hosts).map_err(|e| e.to_string())
}

#[tauri::command]
fn import_hosts(state: State<'_, AppState>, json: String) -> Result<i64, String> {
    let hosts: Vec<db::NewHost> = serde_json::from_str(&json).map_err(|e| e.to_string())?;
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    let mut count = 0;
    for host in hosts {
        db::add_host(&conn, &host).map_err(|e| e.to_string())?;
        count += 1;
    }
    Ok(count)
}

#[tauri::command]
fn get_port_forwards(state: State<'_, AppState>, host_id: i64) -> Result<Vec<db::PortForward>, String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    db::get_port_forwards(&conn, host_id).map_err(|e| e.to_string())
}

#[tauri::command]
fn add_port_forward(state: State<'_, AppState>, forward: db::NewPortForward) -> Result<i64, String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    db::add_port_forward(&conn, &forward).map_err(|e| e.to_string())
}

#[tauri::command]
fn delete_port_forward(state: State<'_, AppState>, id: i64) -> Result<(), String> {
    state.forward_manager.stop(id);
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    db::delete_port_forward(&conn, id).map_err(|e| e.to_string())
}

#[tauri::command]
fn start_port_forward(
    state: State<'_, AppState>,
    rule_id: i64,
) -> Result<(), String> {
    let (host, forward) = {
        let conn = state.db.lock().map_err(|e| e.to_string())?;
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
fn get_setting(state: State<'_, AppState>, key: String) -> Result<Option<String>, String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    db::get_setting(&conn, &key).map_err(|e| e.to_string())
}

#[tauri::command]
fn set_setting(state: State<'_, AppState>, key: String, value: String) -> Result<(), String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    db::set_setting(&conn, &key, &value).map_err(|e| e.to_string())
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState {
            db: Mutex::new(
                Connection::open(
                    dirs::data_dir()
                        .unwrap_or_else(|| std::env::temp_dir())
                        .join("termdrop.db")
                ).expect("Failed to open database"),
            ),
            sessions: Mutex::new(HashMap::new()),
            sftp_sessions: Mutex::new(HashMap::new()),
            forward_manager: port_forward::ForwardManager::new(),
        })
        .setup(|app| {
            let state = app.state::<AppState>();
            let conn = state.db.lock().unwrap();
            db::init_db(&conn).expect("Failed to initialize database");
            db::init_port_forwards(&conn).expect("Failed to initialize port forwards");
            db::init_settings(&conn).expect("Failed to initialize settings");
            Ok(())
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
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
