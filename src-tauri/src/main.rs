#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};
use tauri::{Manager, State, Window};
use rusqlite::Connection;

mod db;
mod crypto;
mod ssh;
mod sftp;

pub struct AppState {
    db: Mutex<Connection>,
    sessions: Mutex<HashMap<String, ssh::SshSessionHandle>>,
    sftp_sessions: Mutex<HashMap<String, Arc<sftp::SftpSessionHandle>>>,
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
) -> Result<String, String> {
    let host = {
        let conn = state.db.lock().map_err(|e| e.to_string())?;
        db::get_host_by_id(&conn, host_id)
            .map_err(|e| e.to_string())?
            .ok_or("Host not found")?
    };

    let password = crypto::get_password(host_id)?;

    let session_id = uuid::Uuid::new_v4().to_string();
    let handle = ssh::connect(
        window,
        session_id.clone(),
        host.host,
        host.port as u16,
        host.username,
        password,
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
async fn sftp_connect(
    state: State<'_, AppState>,
    host_id: i64,
) -> Result<String, String> {
    let host = {
        let conn = state.db.lock().map_err(|e| e.to_string())?;
        db::get_host_by_id(&conn, host_id)
            .map_err(|e| e.to_string())?
            .ok_or("Host not found")?
    };

    let password = crypto::get_password(host_id)?;

    let sftp_id = uuid::Uuid::new_v4().to_string();
    let handle = sftp::sftp_connect(
        host.host,
        host.port as u16,
        host.username,
        password,
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
fn sftp_disconnect(
    state: State<'_, AppState>,
    sftp_session_id: String,
) -> Result<(), String> {
    let mut sftp_sessions = state.sftp_sessions.lock().map_err(|e| e.to_string())?;
    sftp_sessions.remove(&sftp_session_id);
    Ok(())
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
                        .join("ssh-client.db")
                ).expect("Failed to open database"),
            ),
            sessions: Mutex::new(HashMap::new()),
            sftp_sessions: Mutex::new(HashMap::new()),
        })
        .setup(|app| {
            let state = app.state::<AppState>();
            let conn = state.db.lock().unwrap();
            db::init_db(&conn).expect("Failed to initialize database");
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
            sftp_connect,
            sftp_list,
            sftp_upload,
            sftp_download,
            sftp_delete,
            sftp_rename,
            sftp_disconnect,
            get_setting,
            set_setting,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
