#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::sync::Mutex;
use tauri::{Manager, State};
use rusqlite::Connection;

mod db;
mod crypto;

pub struct AppState {
    db: Mutex<Connection>,
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

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(AppState {
            db: Mutex::new(
                Connection::open(
                    dirs::data_dir()
                        .unwrap_or_else(|| std::env::temp_dir())
                        .join("ssh-client.db")
                ).expect("Failed to open database"),
            ),
        })
        .setup(|app| {
            let state = app.state::<AppState>();
            let conn = state.db.lock().unwrap();
            db::init_db(&conn).expect("Failed to initialize database");
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_hosts,
            add_host,
            update_host,
            delete_host,
            get_host_by_id,
            store_password,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
