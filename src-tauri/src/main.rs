#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
// Tauri command signatures mirror the frontend payloads one argument per
// field; grouping them into structs would change the IPC contract.
#![allow(clippy::too_many_arguments)]

use r2d2::Pool;
use r2d2_sqlite::SqliteConnectionManager;
use tauri::ipc::{Channel, InvokeResponseBody};
use tauri::window::Color;
use tauri::{Manager, State, Window};
use tracing::{info, instrument};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

// The backend lives in termdrop-core; these commands are adapters over it.
// `AppState` is the core itself, so every `State<'_, AppState>` derefs to the
// `&Core` its helpers take.
use termdrop_core::state::*;
use termdrop_core::Core as AppState;
use termdrop_core::{db, docker, hosts, local_shells, mongodb, redis};
use termdrop_core::{redis_backup, security, sftp, sftp_ops, ssh_config_parser, system, vnc};

mod bridge;
#[cfg(debug_assertions)]
use termdrop_core::dev_perf;

/// The window's background colour, applied at creation so the very first
/// frame is already the app's own canvas rather than the webview's default
/// white. Which one is used depends on the persisted theme.
///
/// These must stay equal to `--td-canvas` in `src/themes/tokens.css` (light in
/// the bare `:root`, dark under `html.dark`) and to the `#td-splash`
/// backgrounds in `index.html`. `canvas_colours_match_the_design_tokens`
/// below fails if they drift.
const CANVAS_DARK: Color = Color(15, 17, 22, 255); // #0F1116
const CANVAS_LIGHT: Color = Color(250, 251, 253, 255); // #FAFBFD

#[tauri::command]
fn get_hosts(state: State<'_, AppState>) -> Result<Vec<db::Host>, String> {
    with_db(&state, db::get_hosts)
}

#[tauri::command]
fn add_host(state: State<'_, AppState>, host: db::NewHost) -> Result<i64, String> {
    state.add_host(&host)
}

#[tauri::command]
async fn update_host(state: State<'_, AppState>, id: i64, host: db::NewHost) -> Result<(), String> {
    state.update_host(id, &host).await
}

#[tauri::command]
async fn delete_host(state: State<'_, AppState>, id: i64) -> Result<(), String> {
    state.delete_host(id).await
}

#[tauri::command]
fn get_host_by_id(state: State<'_, AppState>, id: i64) -> Result<Option<db::Host>, String> {
    with_db(&state, |conn| db::get_host_by_id(conn, id))
}

#[tauri::command]
fn store_password(host_id: i64, password: String) -> Result<(), String> {
    hosts::store_password(host_id, &password)
}

#[tauri::command]
// `skip_all` rather than `skip(window, state)`: a skipped-by-default list
// records every other argument with Debug, which put `password` into the span
// and so into every log line written inside it.
#[instrument(skip_all, fields(host_id = host_id))]
async fn ssh_connect(
    window: Window,
    state: State<'_, AppState>,
    host_id: i64,
    password: Option<String>,
    // The host key fingerprint the user was shown and agreed to, when they
    // were asked. Compared against the key the server actually presents, so an
    // acceptance cannot be replayed to let a different server through.
    accept_host_key: Option<String>,
    // The same, for the jump host in front of this one.
    accept_jump_host_key: Option<String>,
    cols: u32,
    rows: u32,
) -> Result<String, String> {
    // The typed error turns back into the exact text it was built from, which
    // is what `utils/trustError.js` and `utils/secretPrompt.js` match on.
    state
        .ssh_connect(
            bridge::events(&window),
            host_id,
            password,
            accept_host_key,
            accept_jump_host_key,
            cols,
            rows,
        )
        .await
        .map_err(String::from)
}

/// The shells this machine has, for the local terminal's menu.
#[tauri::command]
async fn list_local_shells(
    state: State<'_, AppState>,
) -> Result<Vec<local_shells::LocalShell>, String> {
    state.list_local_shells().await
}

/// Open a shell on this machine in a terminal tab. `shell_id` names an entry
/// from `list_local_shells`, or is omitted for the default.
#[tauri::command]
async fn local_connect(
    window: Window,
    state: State<'_, AppState>,
    shell_id: Option<String>,
    cols: u32,
    rows: u32,
) -> Result<String, String> {
    state
        .local_connect(bridge::events(&window), shell_id, cols, rows)
        .await
}

/// Debug builds only: the terminal benchmark (`src/dev/termPerf.js`) hands its
/// results here, since a webview has no other way to write a file. Not
/// registered in release builds at all.
#[cfg(debug_assertions)]
#[tauri::command]
fn dev_perf_report(name: String, report: String) -> Result<String, String> {
    if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        return Err("report name must be [A-Za-z0-9-]".into());
    }
    let dir = dirs::data_dir()
        .ok_or("no data dir")?
        .join("termdrop")
        .join("perf");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = dir.join(format!("{}.json", name));
    std::fs::write(&path, report).map_err(|e| e.to_string())?;
    info!(path = %path.display(), "terminal perf report written");
    Ok(path.display().to_string())
}

/// Dev builds only: start or stop recording backend stage timestamps and
/// return those gathered since the last call (see `dev_perf`).
#[cfg(debug_assertions)]
#[tauri::command]
fn dev_perf_marks(record: bool) -> Vec<(&'static str, f64)> {
    dev_perf::take(record)
}

#[tauri::command]
fn ssh_write(state: State<'_, AppState>, session_id: String, data: String) -> Result<(), String> {
    state.ssh_write(&session_id, data)
}

#[tauri::command]
fn ssh_resize(
    state: State<'_, AppState>,
    session_id: String,
    cols: u32,
    rows: u32,
) -> Result<(), String> {
    state.ssh_resize(&session_id, cols, rows)
}

#[tauri::command]
fn open_data_channel(
    state: State<'_, AppState>,
    session_id: String,
    channel: Channel<InvokeResponseBody>,
) -> Result<(), String> {
    state.attach_terminal_sink(&session_id, bridge::byte_sink(channel))
}

#[tauri::command]
fn open_exec_pty_data_channel(
    state: State<'_, AppState>,
    pty_session_id: String,
    channel: Channel<InvokeResponseBody>,
) -> Result<(), String> {
    state.attach_exec_pty_sink(&pty_session_id, bridge::byte_sink(channel))
}

#[tauri::command]
fn parse_ssh_config() -> Result<Vec<ssh_config_parser::SshConfigHost>, String> {
    ssh_config_parser::parse_ssh_config()
}

#[tauri::command]
fn ssh_disconnect(state: State<'_, AppState>, session_id: String) -> Result<(), String> {
    state.ssh_disconnect(&session_id)
}

#[tauri::command]
// `skip_all` for the same reason as `ssh_connect`: `password` must stay out of the span.
#[instrument(skip_all, fields(session_id = %session_id))]
async fn ssh_reconnect(
    window: Window,
    state: State<'_, AppState>,
    session_id: String,
    sftp_session_id: Option<String>,
    password: Option<String>,
) -> Result<(), String> {
    state
        .ssh_reconnect(
            bridge::events(&window),
            &session_id,
            sftp_session_id,
            password,
        )
        .await
        .map_err(String::from)
}

#[tauri::command]
// `skip_all` for the same reason as `ssh_connect`: `password` must stay out of the span.
#[instrument(skip_all, fields(host_id = host_id))]
async fn sftp_connect(
    state: State<'_, AppState>,
    host_id: i64,
    password: Option<String>,
) -> Result<String, String> {
    state
        .sftp_connect(host_id, password)
        .await
        .map_err(String::from)
}

#[tauri::command]
async fn sftp_list(
    state: State<'_, AppState>,
    sftp_session_id: String,
    path: String,
) -> Result<Vec<sftp::SftpFile>, String> {
    state.sftp_list(&sftp_session_id, path).await
}

#[tauri::command]
async fn sftp_upload(
    window: Window,
    state: State<'_, AppState>,
    sftp_session_id: String,
    local_path: String,
    remote_path: String,
) -> Result<(), String> {
    state
        .sftp_upload(
            bridge::events(&window),
            &sftp_session_id,
            local_path,
            remote_path,
        )
        .await
}

#[tauri::command]
async fn sftp_download(
    window: Window,
    state: State<'_, AppState>,
    sftp_session_id: String,
    remote_path: String,
) -> Result<String, String> {
    state
        .sftp_download(bridge::events(&window), &sftp_session_id, remote_path)
        .await
}

#[tauri::command]
async fn sftp_download_dir(
    _window: Window,
    state: State<'_, AppState>,
    sftp_session_id: String,
    remote_path: String,
) -> Result<String, String> {
    state.sftp_download_dir(&sftp_session_id, remote_path).await
}

#[tauri::command]
async fn sftp_write_file(
    state: State<'_, AppState>,
    sftp_session_id: String,
    remote_path: String,
    content: String,
) -> Result<(), String> {
    state
        .sftp_write_file(&sftp_session_id, remote_path, content)
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
    state.sftp_stat_file(&sftp_session_id, remote_path).await
}

#[tauri::command]
async fn sftp_realpath(
    state: State<'_, AppState>,
    sftp_session_id: String,
    remote_path: String,
) -> Result<String, String> {
    state.sftp_realpath(&sftp_session_id, remote_path).await
}

#[tauri::command]
async fn sftp_delete(
    state: State<'_, AppState>,
    sftp_session_id: String,
    remote_path: String,
) -> Result<(), String> {
    state.sftp_delete(&sftp_session_id, remote_path).await
}

#[tauri::command]
async fn sftp_rename(
    state: State<'_, AppState>,
    sftp_session_id: String,
    old_path: String,
    new_path: String,
) -> Result<(), String> {
    state
        .sftp_rename(&sftp_session_id, old_path, new_path)
        .await
}

#[tauri::command]
async fn sftp_mkdir(
    state: State<'_, AppState>,
    sftp_session_id: String,
    remote_path: String,
) -> Result<(), String> {
    state.sftp_mkdir(&sftp_session_id, remote_path).await
}

#[tauri::command]
async fn sftp_rmdir(
    state: State<'_, AppState>,
    sftp_session_id: String,
    remote_path: String,
) -> Result<(), String> {
    state.sftp_rmdir(&sftp_session_id, remote_path).await
}

#[tauri::command]
async fn sftp_read_file(
    state: State<'_, AppState>,
    sftp_session_id: String,
    remote_path: String,
) -> Result<String, String> {
    state.sftp_read_file(&sftp_session_id, remote_path).await
}

#[tauri::command]
async fn sftp_read_file_base64(
    state: State<'_, AppState>,
    sftp_session_id: String,
    remote_path: String,
) -> Result<String, String> {
    state
        .sftp_read_file_base64(&sftp_session_id, remote_path)
        .await
}

#[tauri::command]
fn sftp_disconnect(state: State<'_, AppState>, sftp_session_id: String) -> Result<(), String> {
    state.sftp_disconnect(&sftp_session_id)
}

#[tauri::command]
async fn exec_pty_connect(
    window: Window,
    state: State<'_, AppState>,
    host_id: i64,
    pty_session_id: String,
    command: String,
) -> Result<String, String> {
    state
        .exec_pty_connect(bridge::events(&window), host_id, pty_session_id, command)
        .await
}

#[tauri::command]
fn exec_pty_write(
    state: State<'_, AppState>,
    pty_session_id: String,
    data: String,
) -> Result<(), String> {
    state.exec_pty_write(&pty_session_id, data)
}

#[tauri::command]
fn exec_pty_disconnect(state: State<'_, AppState>, pty_session_id: String) -> Result<(), String> {
    state.exec_pty_disconnect(&pty_session_id)
}

#[tauri::command]
fn update_host_group(state: State<'_, AppState>, id: i64, group: String) -> Result<(), String> {
    state.update_host_group(id, &group)
}

#[tauri::command]
fn batch_update_host_group(
    state: State<'_, AppState>,
    old_group: String,
    new_group: String,
) -> Result<usize, String> {
    state.batch_update_host_group(&old_group, &new_group)
}

#[tauri::command]
fn batch_clear_host_group(state: State<'_, AppState>, group: String) -> Result<usize, String> {
    state.batch_clear_host_group(&group)
}

#[tauri::command]
fn update_host_favorite(state: State<'_, AppState>, id: i64, favorite: i64) -> Result<(), String> {
    state.update_host_favorite(id, favorite)
}

#[tauri::command]
fn update_host_last_connected(state: State<'_, AppState>, id: i64) -> Result<(), String> {
    state.update_host_last_connected(id)
}

#[tauri::command]
fn export_hosts(state: State<'_, AppState>) -> Result<String, String> {
    state.export_hosts()
}

#[tauri::command]
/// Import hosts from either source. Rows that fail validation or insertion
/// are reported individually rather than aborting the import, so the caller
/// can tell the user exactly what landed and what did not.
fn import_hosts(
    state: State<'_, AppState>,
    entries: Vec<db::ImportEntry>,
) -> Result<db::ImportSummary, String> {
    state.import_hosts(entries)
}

#[tauri::command]
fn get_port_forwards(
    state: State<'_, AppState>,
    host_id: i64,
) -> Result<Vec<db::PortForward>, String> {
    state.get_port_forwards(host_id)
}

#[tauri::command]
fn add_port_forward(
    state: State<'_, AppState>,
    forward: db::NewPortForward,
) -> Result<i64, String> {
    state.add_port_forward(&forward)
}

#[tauri::command]
fn update_port_forward(
    state: State<'_, AppState>,
    id: i64,
    forward: db::NewPortForward,
) -> Result<(), String> {
    state.update_port_forward(id, &forward)
}

#[tauri::command]
fn delete_port_forward(state: State<'_, AppState>, id: i64) -> Result<(), String> {
    state.delete_port_forward(id)
}

#[tauri::command]
fn start_port_forward(state: State<'_, AppState>, rule_id: i64) -> Result<(), String> {
    state.start_port_forward(rule_id)
}

#[tauri::command]
fn stop_port_forward(state: State<'_, AppState>, rule_id: i64) -> Result<(), String> {
    state.stop_port_forward(rule_id);
    Ok(())
}

#[tauri::command]
fn get_port_forward_status(state: State<'_, AppState>, rule_id: i64) -> Result<bool, String> {
    Ok(state.port_forward_status(rule_id))
}

#[tauri::command]
fn write_file(path: String, content: String) -> Result<(), String> {
    sftp_ops::write_file(&path, &content)
}

#[tauri::command]
async fn docker_ps(
    state: State<'_, AppState>,
    host_id: i64,
    all: bool,
) -> Result<Vec<docker::Container>, String> {
    state.docker_ps(host_id, all).await
}

#[tauri::command]
async fn docker_start(
    state: State<'_, AppState>,
    host_id: i64,
    container_id: String,
) -> Result<(), String> {
    state.docker_start(host_id, container_id).await
}

#[tauri::command]
async fn docker_stop(
    state: State<'_, AppState>,
    host_id: i64,
    container_id: String,
) -> Result<(), String> {
    state.docker_stop(host_id, container_id).await
}

#[tauri::command]
async fn docker_restart(
    state: State<'_, AppState>,
    host_id: i64,
    container_id: String,
) -> Result<(), String> {
    state.docker_restart(host_id, container_id).await
}

#[tauri::command]
async fn docker_inspect_shell(
    state: State<'_, AppState>,
    host_id: i64,
    container_id: String,
) -> Result<String, String> {
    state.docker_inspect_shell(host_id, container_id).await
}

#[tauri::command]
async fn docker_install(state: State<'_, AppState>, host_id: i64) -> Result<String, String> {
    state.docker_install(host_id).await
}

#[tauri::command]
async fn run_security_audit(
    state: State<'_, AppState>,
    host_id: i64,
    force: bool,
) -> Result<security::SecurityReport, String> {
    state.run_security_audit(host_id, force).await
}

#[tauri::command]
async fn get_system_stats(
    state: State<'_, AppState>,
    host_id: i64,
) -> Result<serde_json::Value, String> {
    state.get_system_stats(host_id).await
}

#[tauri::command]
async fn get_system_panel(
    state: State<'_, AppState>,
    host_id: i64,
) -> Result<system::SystemPanel, String> {
    state.get_system_panel(host_id).await
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
    state.mongodb_list_databases(host_id).await
}

#[tauri::command]
async fn mongodb_list_collections(
    state: State<'_, AppState>,
    host_id: i64,
    db: String,
) -> Result<Vec<String>, String> {
    state.mongodb_list_collections(host_id, db).await
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
    state
        .mongodb_dump(
            bridge::events(&window),
            op_id,
            host_id,
            db,
            collections,
            output_dir,
            is_archive,
        )
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
    state
        .mongodb_restore(
            bridge::events(&window),
            op_id,
            host_id,
            db,
            collections,
            input_dir,
            is_archive,
            drop_first,
        )
        .await
}

#[tauri::command]
fn scan_restore_folder(path: String) -> Result<Vec<mongodb::RestoreSourceDb>, String> {
    mongodb::scan_restore_folder(path)
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
    state
        .mongodb_restore_archive(
            bridge::events(&window),
            op_id,
            host_id,
            includes,
            input_path,
            drop_first,
        )
        .await
}

/// Read a page of documents from a collection (read-only by construction;
/// see `Core::mongodb_find`).
#[tauri::command]
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
    state
        .mongodb_find(
            host_id, db, collection, filter, sort, projection, skip, limit,
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
    state.mongodb_count(host_id, db, collection, filter).await
}

/// A collection's indexes.
#[tauri::command]
async fn mongodb_list_indexes(
    state: State<'_, AppState>,
    host_id: i64,
    db: String,
    collection: String,
) -> Result<Vec<String>, String> {
    state.mongodb_list_indexes(host_id, db, collection).await
}

/// A collection's storage statistics.
#[tauri::command]
async fn mongodb_collection_stats(
    state: State<'_, AppState>,
    host_id: i64,
    db: String,
    collection: String,
) -> Result<String, String> {
    state
        .mongodb_collection_stats(host_id, db, collection)
        .await
}

/// Release the pooled clients for a host, called when its tab closes.
#[tauri::command]
async fn mongodb_disconnect(state: State<'_, AppState>, host_id: i64) -> Result<(), String> {
    state.mongodb_disconnect(host_id).await
}

/// Store the password for one side of a MongoDB host.
///
/// Only ever *sets* a secret. Clearing one is `mongodb_clear_secret`, so an
/// empty password field in the edit dialog can never silently delete it.
#[tauri::command]
fn mongodb_store_secret(host_id: i64, password: String) -> Result<(), String> {
    hosts::store_mongo_secret(host_id, &password)
}

/// Whether a password is stored, so the UI can say so without revealing it.
#[tauri::command]
fn mongodb_has_secret(host_id: i64) -> bool {
    hosts::has_mongo_secret(host_id)
}

/// Forget the stored password for one side.
#[tauri::command]
fn mongodb_clear_secret(host_id: i64) -> Result<(), String> {
    hosts::clear_mongo_secret(host_id)
}

/// Open a Redis connection and report what the server is. `ssh_password`
/// is the bastion's password on the prompt-and-retry path.
#[tauri::command]
async fn redis_connect(
    state: State<'_, AppState>,
    host_id: i64,
    ssh_password: Option<String>,
) -> Result<redis::RedisServerInfo, String> {
    state.redis_connect(host_id, ssh_password).await
}

/// Re-read the server's INFO, so key counts follow a backup or restore.
#[tauri::command]
async fn redis_server_info(
    state: State<'_, AppState>,
    host_id: i64,
) -> Result<redis::RedisServerInfo, String> {
    state.redis_server_info(host_id).await
}

/// Drop this host's pooled connections and close its tunnel.
#[tauri::command]
async fn redis_disconnect(state: State<'_, AppState>, host_id: i64) -> Result<(), String> {
    state.redis_disconnect(host_id).await
}

/// One page of a database's keyspace. `cursor` is a decimal string, not a
/// number: Redis cursors are u64 and would lose precision as a JS number.
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
    state
        .redis_scan_keys(
            host_id,
            db,
            cursor,
            pattern,
            type_filter,
            limit,
            with_memory,
            supports_scan_type,
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
    state.redis_key_tree(host_id, db, pattern).await
}

/// One page of a single key's value, the key addressed by its exact bytes.
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
    state
        .redis_key_value(host_id, db, key_b64, cursor, offset, limit)
        .await
}

/// Ask an in-flight export or import to stop.
#[tauri::command]
fn redis_cancel(state: State<'_, AppState>, op_id: String) {
    state.redis_cancel(&op_id)
}

/// Back up a database to a `.tdredis` file. Not under a timeout: a backup of
/// a large keyspace legitimately runs for a long time.
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
    state
        .redis_export(
            bridge::events(&window),
            op_id,
            host_id,
            db,
            pattern,
            output_path,
            max_key_bytes,
            env!("CARGO_PKG_VERSION"),
        )
        .await
}

/// A backup file's header, for the restore dialog.
#[tauri::command]
fn redis_backup_info(path: String) -> Result<redis_backup::BackupHeader, String> {
    redis::read_backup_header(&path)
}

/// Restore a `.tdredis` file into a database.
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
    state
        .redis_import(
            bridge::events(&window),
            op_id,
            host_id,
            db,
            input_path,
            replace,
            flush_first,
        )
        .await
}

/// Store the password for a Redis host.
///
/// Only ever *sets* a secret, so an empty password field in the edit dialog
/// cannot silently delete one; clearing is `redis_clear_secret`.
#[tauri::command]
fn redis_store_secret(host_id: i64, password: String) -> Result<(), String> {
    hosts::store_redis_secret(host_id, &password)
}

/// Whether a password is stored, so the UI can say so without revealing it.
#[tauri::command]
fn redis_has_secret(host_id: i64) -> bool {
    hosts::has_redis_secret(host_id)
}

/// Forget the stored password for a Redis host.
#[tauri::command]
fn redis_clear_secret(host_id: i64) -> Result<(), String> {
    hosts::clear_redis_secret(host_id)
}

#[tauri::command]
async fn vnc_connect(
    window: Window,
    state: State<'_, AppState>,
    host_id: i64,
    password: Option<String>,
    ssh_password: Option<String>,
) -> Result<vnc::VncSessionInfo, String> {
    state
        .vnc_connect(bridge::events(&window), host_id, password, ssh_password)
        .await
}

/// Hand the session the channel its frames go down.
///
/// Until this arrives the thread requests no frames at all, which is what
/// removes the dropped-first-frame race rather than papering over it.
#[tauri::command]
fn vnc_open_frame_channel(
    state: State<'_, AppState>,
    session_id: String,
    channel: Channel<InvokeResponseBody>,
) -> Result<(), String> {
    state.vnc_open_frame_channel(&session_id, bridge::byte_sink(channel))
}

#[tauri::command]
async fn vnc_disconnect(state: State<'_, AppState>, session_id: String) -> Result<(), String> {
    state.vnc_disconnect(&session_id).await
}

/// Close every session for a host. Used when the last tab for it goes away.
#[tauri::command]
async fn vnc_disconnect_host(state: State<'_, AppState>, host_id: i64) -> Result<(), String> {
    state.vnc_disconnect_host(host_id).await
}

#[tauri::command]
fn vnc_pointer(
    state: State<'_, AppState>,
    session_id: String,
    x: u16,
    y: u16,
    button_mask: u8,
) -> Result<(), String> {
    state.vnc_pointer(&session_id, x, y, button_mask)
}

#[tauri::command]
fn vnc_key(
    state: State<'_, AppState>,
    session_id: String,
    keysym: u32,
    down: bool,
) -> Result<(), String> {
    state.vnc_key(&session_id, keysym, down)
}

/// Press every keysym in order, then release them in reverse (Ctrl+Alt+Delete).
#[tauri::command]
fn vnc_key_sequence(
    state: State<'_, AppState>,
    session_id: String,
    keysyms: Vec<u32>,
) -> Result<(), String> {
    state.vnc_key_sequence(&session_id, &keysyms)
}

#[tauri::command]
fn vnc_frame_ack(state: State<'_, AppState>, session_id: String, seq: u32) -> Result<(), String> {
    state.vnc_frame_ack(&session_id, seq)
}

#[tauri::command]
fn vnc_refresh(state: State<'_, AppState>, session_id: String) -> Result<(), String> {
    state.vnc_refresh(&session_id)
}

/// Stop or resume requesting frames while the panel is out of sight.
#[tauri::command]
fn vnc_set_paused(
    state: State<'_, AppState>,
    session_id: String,
    paused: bool,
) -> Result<(), String> {
    state.vnc_set_paused(&session_id, paused)
}

#[tauri::command]
fn vnc_set_view_only(
    state: State<'_, AppState>,
    session_id: String,
    view_only: bool,
) -> Result<(), String> {
    state.vnc_set_view_only(&session_id, view_only)
}

/// Push the local clipboard to the remote; returns whether anything was lost
/// (the base protocol's cut text is ISO-8859-1).
#[tauri::command]
fn vnc_send_clipboard(
    state: State<'_, AppState>,
    session_id: String,
    text: String,
) -> Result<bool, String> {
    state.vnc_send_clipboard(&session_id, &text)
}

#[tauri::command]
fn vnc_store_secret(host_id: i64, password: String) -> Result<(), String> {
    hosts::store_vnc_secret(host_id, &password)
}

#[tauri::command]
fn vnc_has_secret(host_id: i64) -> Result<bool, String> {
    Ok(hosts::has_vnc_secret(host_id))
}

#[tauri::command]
fn vnc_clear_secret(host_id: i64) -> Result<(), String> {
    hosts::clear_vnc_secret(host_id)
}

#[tauri::command]
fn mongodb_cancel(state: State<'_, AppState>, op_id: String) {
    state.mongodb_cancel(&op_id)
}

fn main() {
    let start = std::time::Instant::now();

    // Initialize structured logging to file
    let log_dir = dirs::data_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("termdrop")
        .join("logs");
    std::fs::create_dir_all(&log_dir).ok();

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

    // Run initialization with a dedicated connection. The window's background
    // colour is decided here too: this scope already owns the pool, and the
    // colour has to be known before the window is created.
    let canvas = {
        let conn = pool.get().expect("Failed to get initial DB connection");
        db::init_db(&conn).expect("Failed to initialize database");
        db::init_port_forwards(&conn).expect("Failed to initialize port forwards");
        db::init_settings(&conn).expect("Failed to initialize settings");
        split_local_mongo_hosts(&conn);
        migrate_mongo_credentials(&conn);

        match db::get_setting(&conn, "theme") {
            Ok(Some(t)) if t == "light" => CANVAS_LIGHT,
            // Unset (first launch) or unreadable: dark, matching the frontend
            // default in index.html's head script and stores/connection.js.
            // A failed read must never be able to leave the app windowless.
            _ => CANVAS_DARK,
        }
    };
    let db_ready_ms = start.elapsed().as_millis();

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
        .plugin(tauri_plugin_notification::init())
        .manage(AppState::new(pool))
        .setup(move |app| {
            // The window is built here, rather than declared in
            // tauri.conf.json, so that it is created already carrying the
            // right background colour. Until the document's first frame the
            // webview paints the platform default -- white -- and that flash
            // is what this exists to prevent.
            //
            // It has to be the colour at *creation*, not a later
            // `set_background_color`: that setter is compiled out entirely on
            // macOS (wry 0.55.1 gates the body on `target_os = "ios"`), so a
            // runtime override would silently do nothing there. The
            // creation-time path reaches both the native window and the
            // webview layer on all three platforms.
            //
            // The colour cannot live in the JSON config because it depends on
            // the persisted theme, so `create: false` hands the build to us.
            let config = app
                .config()
                .app
                .windows
                .iter()
                .find(|w| w.label == "main")
                .cloned()
                .expect("no `main` window in tauri.conf.json");

            tauri::WebviewWindowBuilder::from_config(app.handle(), &config)?
                .background_color(canvas)
                .build()?;

            info!(
                db_ready_ms,
                window_ms = start.elapsed().as_millis(),
                "startup: database ready, main window created"
            );

            // A run killed mid-operation cannot drop its --config guard, so
            // clear any credential files an earlier process left in the temp
            // directory. Nothing reads the result, so it runs off the path to
            // the first frame rather than delaying it. A plain thread, not the
            // async runtime: this is blocking std::fs work.
            std::thread::spawn(mongodb::sweep_stale_config_files);

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_hosts,
            local_connect,
            list_local_shells,
            add_host,
            update_host,
            delete_host,
            get_host_by_id,
            store_password,
            ssh_connect,
            ssh_write,
            #[cfg(debug_assertions)]
            dev_perf_report,
            #[cfg(debug_assertions)]
            dev_perf_marks,
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
            vnc_connect,
            vnc_open_frame_channel,
            vnc_disconnect,
            vnc_disconnect_host,
            vnc_pointer,
            vnc_key,
            vnc_key_sequence,
            vnc_frame_ack,
            vnc_refresh,
            vnc_set_paused,
            vnc_set_view_only,
            vnc_send_clipboard,
            vnc_store_secret,
            vnc_has_secret,
            vnc_clear_secret,
            scan_restore_folder,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `setup` builds the main window itself, which means tauri.conf.json must
    /// keep holding up its end: a window labelled `main` that Tauri does not
    /// create on our behalf. If either half drifts the app launches with no
    /// window at all -- a far harder failure than the flash this replaced, and
    /// one that only shows up at runtime.
    #[test]
    fn the_main_window_is_declared_for_setup_to_build() {
        let config: serde_json::Value =
            serde_json::from_str(include_str!("../tauri.conf.json")).expect("tauri.conf.json");
        let windows = config["app"]["windows"]
            .as_array()
            .expect("app.windows should be an array");

        let main = windows
            .iter()
            .find(|w| w["label"] == "main")
            .expect("no window labelled `main`; setup() looks it up by that label");
        assert_eq!(
            main["create"], false,
            "`main` must be create:false, or Tauri builds it before setup can colour it"
        );
        assert_eq!(
            windows.len(),
            1,
            "setup only builds `main`; another declared window would never be created"
        );
    }

    /// The window background is set in Rust but the same two colours are
    /// declared as design tokens and duplicated again in index.html's inline
    /// splash, because none of those three can read the others at the moment
    /// they are needed. This is the only thing stopping them drifting apart on
    /// the next palette change -- which would reintroduce exactly the
    /// launch-flash bug the colours exist to prevent.
    #[test]
    fn canvas_colours_match_the_design_tokens() {
        let tokens = include_str!("../../src/themes/tokens.css");

        // `--td-canvas: R G B;` appears once per theme block: the light value
        // in the bare `:root`, then the dark one under `html.dark`.
        let found: Vec<Color> = tokens
            .lines()
            .filter_map(|l| l.trim().strip_prefix("--td-canvas:"))
            .map(|v| {
                let parts: Vec<u8> = v
                    .trim_end_matches(';')
                    .split_whitespace()
                    .map(|n| n.parse().expect("--td-canvas channel is not a u8"))
                    .collect();
                assert_eq!(parts.len(), 3, "--td-canvas should be three channels");
                Color(parts[0], parts[1], parts[2], 255)
            })
            .collect();

        // Guards the guard: a regex that matched nothing would otherwise let
        // the assertions below pass by checking nothing at all.
        assert_eq!(
            found.len(),
            2,
            "expected one --td-canvas per theme in tokens.css, found {found:?}"
        );
        assert_eq!(found[0], CANVAS_LIGHT, "light --td-canvas drifted");
        assert_eq!(found[1], CANVAS_DARK, "dark --td-canvas drifted");
    }
}
