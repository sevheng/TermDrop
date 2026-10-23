//! The backend's live state and the helpers every command is built from.
//!
//! [`Core`] is what the Tauri app manages as its `AppState` and what the
//! native app owns directly. Its registries are public so that adapters can
//! keep reaching them the way the Tauri commands always have; the helpers below
//! (`with_db`, `resolve_target`, `with_exec_session`, `cached_per_host`, ...)
//! are the supported way to use them, and the `impl Core` block at the bottom
//! holds the whole operations a UI calls (open a terminal, write to it, ...).

use crate::error::CoreResult;
use crate::events::{ByteSink, CoreEvent, Events};
use crate::{crypto, db, docker, local_pty, local_shells, mongodb, port_forward, redis, security};
use crate::{sftp, ssh, uri, vnc};
use r2d2::Pool;
use r2d2_sqlite::SqliteConnectionManager;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Child;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tracing::info;

/// Per-host coalescing locks so concurrent requests share one fetch.
pub type FetchLocks = Arc<Mutex<HashMap<i64, Arc<tokio::sync::Mutex<()>>>>>;

/// A per-host cached value with its fetch time.
pub struct Cached<T> {
    pub value: T,
    pub cached_at: std::time::Instant,
}

pub struct MongoOpHandle {
    pub cancelled: Arc<AtomicBool>,
    pub child: Option<Child>,
}

/// Everything live in the backend: the database pool and one registry per
/// kind of connection, each keyed as CLAUDE.md describes. Build it with
/// [`Core::open`] (or [`Core::new`] over a prepared pool) and keep exactly one
/// for the life of the app.
pub struct Core {
    pub db: Pool<SqliteConnectionManager>,
    pub sessions: Mutex<HashMap<String, ssh::SshSessionHandle>>,
    /// The last `list_local_shells` result, which `local_connect` resolves
    /// shell ids against.
    pub local_shells: Mutex<Vec<local_shells::LocalShell>>,
    pub exec_sessions: Mutex<HashMap<i64, Arc<Mutex<ssh2::Session>>>>,
    pub sftp_sessions: Mutex<HashMap<String, Arc<sftp::SftpSessionHandle>>>,
    pub exec_pty_sessions: Mutex<HashMap<String, ssh::ExecPtyHandle>>,
    pub docker_cache: Arc<Mutex<HashMap<i64, Cached<Vec<docker::Container>>>>>,
    pub docker_ps_fetching: FetchLocks,
    pub security_report_cache: Arc<Mutex<HashMap<i64, Cached<security::SecurityReport>>>>,
    pub security_report_fetching: FetchLocks,
    pub forward_manager: port_forward::ForwardManager,
    pub mongo_ops: Arc<Mutex<HashMap<String, MongoOpHandle>>>,
    /// One driver client per host, so browsing a database does not open a fresh
    /// connection pool and topology monitor per call.
    ///
    /// Keyed by host id, never by URI: a URI key would put a credential into a
    /// long-lived map and into any Debug output.
    pub mongo_clients: Arc<tokio::sync::Mutex<HashMap<i64, ::mongodb::Client>>>,
    /// One pooled connection per `(host_id, database)`.
    ///
    /// Keyed by database as well as host because `SELECT` is *connection*
    /// state and `MultiplexedConnection` pipelines commands from every task
    /// over one socket: a single per-host connection would let the key list's
    /// `SELECT 1` land between another pane's `TYPE` and its `HSCAN`. Baking
    /// the index into the connection removes the race rather than locking
    /// around it.
    pub redis_conns:
        Arc<tokio::sync::Mutex<HashMap<(i64, i64), ::redis::aio::MultiplexedConnection>>>,
    /// One SSH tunnel per Redis host, alive while any of its tabs is open.
    pub redis_tunnels: Arc<tokio::sync::Mutex<HashMap<i64, port_forward::Tunnel>>>,
    /// Cancel flags for in-flight Redis exports and imports, keyed by op id.
    ///
    /// Deliberately not the MongoDB registry: that one also carries a child
    /// process to kill, and a Redis operation has none -- it is a loop in this
    /// process, so a flag is the whole mechanism.
    pub redis_ops: Arc<Mutex<HashMap<String, Arc<AtomicBool>>>>,
    /// One live RFB session per VNC tab, keyed by session id.
    ///
    /// Deliberately not keyed by host id the way `redis_conns` is: two VNC tabs
    /// on one host are two independent connections with their own framebuffer,
    /// cursor and decoder state, and have nothing they could share.
    pub vnc_sessions: Mutex<HashMap<String, vnc::VncSessionHandle>>,
    /// One SSH tunnel per VNC host, alive while any session for it is.
    pub vnc_tunnels: Arc<tokio::sync::Mutex<HashMap<i64, port_forward::Tunnel>>>,
}

pub fn db_err(e: r2d2::Error) -> String {
    e.to_string()
}

pub async fn with_timeout<F, R>(f: F, secs: u64) -> Result<R, String>
where
    F: FnOnce() -> Result<R, String> + Send + 'static,
    R: Send + 'static,
{
    tokio::time::timeout(Duration::from_secs(secs), tokio::task::spawn_blocking(f))
        .await
        .map_err(|_| format!("Operation timed out after {} seconds", secs))?
        .map_err(|e| e.to_string())?
}

pub fn register_mongo_op(state: &Core, op_id: String) -> Arc<AtomicBool> {
    let cancelled = Arc::new(AtomicBool::new(false));
    let handle = MongoOpHandle {
        cancelled: cancelled.clone(),
        child: None,
    };
    mongodb::lock_or_recover(&state.mongo_ops).insert(op_id, handle);
    cancelled
}

pub fn unregister_mongo_op(state: &Core, op_id: &str) {
    mongodb::lock_or_recover(&state.mongo_ops).remove(op_id);
}

/// The keyring account holding a host's MongoDB password.
///
/// Non-numeric by construction, so it cannot collide with an SSH host password
/// in the shared fallback file. The `-remote-` segment is historical, from when
/// a host had two connections: renaming it would mean migrating every stored
/// secret, and a keyring failure mid-migration would lose passwords for no
/// user-visible gain.
pub fn mongo_account(host_id: i64) -> String {
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
pub fn load_mongo_uri(state: &Core, host_id: i64) -> Result<String, String> {
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
pub async fn mongo_client(state: &Core, host_id: i64) -> Result<::mongodb::Client, String> {
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
pub async fn forget_mongo_clients(state: &Core, host_id: i64) {
    state.mongo_clients.lock().await.remove(&host_id);
}

/// Await a future with a timeout, for the async driver calls that
/// `with_timeout` (which wraps a blocking closure) cannot cover.
///
/// Deliberately not applied to dump or restore: those legitimately run for hours.
pub async fn with_async_timeout<T>(
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
pub fn migrate_mongo_credentials(conn: &rusqlite::Connection) {
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
pub fn split_local_mongo_hosts(conn: &rusqlite::Connection) -> usize {
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
            jump_host_id: None,
            vnc_host: None,
            vnc_port: None,
            vnc_username: None,
            vnc_tunnel_host_id: None,
            vnc_view_only: None,
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
pub fn migrate_mongo_credentials_with(
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
pub async fn with_mongo_op<Fut>(
    state: &Core,
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
pub fn load_host(state: &Core, host_id: i64) -> Result<db::Host, String> {
    with_db(state, |conn| db::get_host_by_id(conn, host_id))?
        .ok_or_else(|| "Host not found".to_string())
}

/// Resolve `(password, key_path)` for a host. Password hosts use
/// `password_override` when given and the keyring otherwise; key hosts use
/// the stored key path. Unknown auth types pass the override through.
/// The keyring account for a private key's passphrase.
///
/// Non-numeric by construction, so it cannot collide with the bare host id
/// that SSH passwords use, nor with `redis-<id>` or `vnc-<id>`.
/// `crypto::tests::named_accounts_cannot_collide_with_host_ids` pins that.
#[allow(dead_code)] // used once the passphrase prompt lands
pub fn key_passphrase_account(host_id: i64) -> String {
    format!("sshkey-{}", host_id)
}

/// Everything needed to authenticate to a host.
///
/// A struct rather than a growing tuple: `passphrase` is meaningful only
/// alongside `key_path`, and positional arguments stopped carrying that at
/// three fields.
pub struct Auth {
    pub password: Option<String>,
    pub key_path: Option<String>,
    /// For an encrypted private key. `None` means the key is unencrypted, or
    /// no passphrase has been stored yet.
    pub passphrase: Option<String>,
}

pub fn resolve_auth(host: &db::Host, password_override: Option<String>) -> Result<Auth, String> {
    // A miss is normal: most keys the app has not been told about yet are
    // unencrypted, and an encrypted one surfaces as an auth failure that the
    // frontend turns into a passphrase prompt.
    let passphrase = crypto::get_secret(&key_passphrase_account(host.id)).ok();

    match host.auth_type.as_str() {
        "password" => {
            let pw = match password_override {
                Some(p) => Some(p),
                None => Some(crypto::get_password(host.id)?),
            };
            Ok(Auth {
                password: pw,
                key_path: None,
                passphrase: None,
            })
        }
        "key" => Ok(Auth {
            password: None,
            key_path: host.key_path.clone(),
            passphrase,
        }),
        _ => Ok(Auth {
            password: password_override,
            key_path: host.key_path.clone(),
            passphrase,
        }),
    }
}

/// An `SshTarget` for one host, with no jump.
pub fn target_from(host: &db::Host, auth: Auth) -> ssh::SshTarget {
    ssh::SshTarget {
        host: host.host.clone(),
        port: host.port as u16,
        username: host.username.clone(),
        password: auth.password,
        key_path: auth.key_path,
        passphrase: auth.passphrase,
        jump: None,
    }
}

/// Why a host's jump host cannot be used, if it cannot.
///
/// One hop only. A chain would work mechanically, but each hop is another trust
/// prompt and another place for a cycle, and nobody has asked for it yet.
pub fn check_jump(host: &db::Host, jump: Option<&db::Host>) -> Result<(), String> {
    let Some(jump) = jump else {
        return Err(format!(
            "{} connects through a jump host that no longer exists. Edit the host to choose another.",
            host.name
        ));
    };
    if jump.id == host.id {
        return Err(format!("{} cannot be its own jump host.", host.name));
    }
    if jump.jump_host_id.is_some() {
        return Err(format!(
            "{} connects through {}, which has a jump host of its own. Only one hop is supported.",
            host.name, jump.name
        ));
    }
    if jump.host.is_empty() {
        return Err(format!(
            "{} is not an SSH host, so it cannot be a jump host.",
            jump.name
        ));
    }
    Ok(())
}

/// Everything needed to open an SSH session to a host, including the jump
/// host in front of it. Use this rather than `resolve_auth` for anything that
/// connects.
pub fn resolve_target(
    state: &Core,
    host: &db::Host,
    password_override: Option<String>,
) -> Result<ssh::SshTarget, String> {
    let mut target = target_from(host, resolve_auth(host, password_override)?);

    if let Some(jump_id) = host.jump_host_id {
        let jump = with_db(state, |conn| db::get_host_by_id(conn, jump_id))?;
        check_jump(host, jump.as_ref())?;
        let jump = jump.expect("check_jump rejects a missing jump host");

        // Never prompted for: the password prompt the frontend shows is for
        // the host being opened, and it must not be mistaken for the bastion's.
        // The wording deliberately avoids the keyring error text it matches on.
        let auth = resolve_auth(&jump, None).map_err(|_| {
            format!(
                "The jump host {} has no saved password. Connect to it once and save its password.",
                jump.name
            )
        })?;
        target.jump = Some(Box::new(target_from(&jump, auth)));
    }

    Ok(target)
}

/// The shared non-PTY session for a host, used for exec-style commands.
pub fn exec_session(
    state: &Core,
    host_id: i64,
    missing: &'static str,
) -> Result<Arc<Mutex<ssh2::Session>>, String> {
    let exec_sessions = state.exec_sessions.lock().map_err(|e| e.to_string())?;
    exec_sessions
        .get(&host_id)
        .cloned()
        .ok_or_else(|| missing.to_string())
}

pub fn sftp_handle(
    state: &Core,
    sftp_session_id: &str,
) -> Result<Arc<sftp::SftpSessionHandle>, String> {
    let sftp_sessions = state.sftp_sessions.lock().map_err(|e| e.to_string())?;
    sftp_sessions
        .get(sftp_session_id)
        .cloned()
        .ok_or_else(|| "SFTP session not found".to_string())
}

/// Run a query on a pooled connection, stringifying either error.
pub fn with_db<T>(
    state: &Core,
    f: impl FnOnce(&rusqlite::Connection) -> rusqlite::Result<T>,
) -> Result<T, String> {
    let conn = state.db.get().map_err(db_err)?;
    f(&conn).map_err(|e| e.to_string())
}

/// Run a blocking SFTP operation on the named session off the async thread.
pub async fn sftp_blocking<T, F>(state: &Core, sftp_session_id: &str, f: F) -> Result<T, String>
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
pub async fn with_exec_session<T, F>(
    state: &Core,
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
pub async fn cached_per_host<T, F>(
    state: &Core,
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

/// Label an exec-session failure, except a host key refusal.
///
/// The frontend recognises a refusal only when `TRUST_PREFIX` is at index 0 --
/// deliberately, so no hostname or banner can forge one -- and a label in front
/// of it turned "do you trust this host?" into a plain error toast.
pub fn exec_session_error(e: String) -> String {
    if e.starts_with(ssh::trust::TRUST_PREFIX) {
        e
    } else {
        format!("exec session: {}", e)
    }
}

/// Drop everything cached or running for a host once its last tab closes:
/// docker and security caches, their coalescing locks, and any docker exec
/// PTY sessions still attached to it.
pub fn forget_host_state(state: &Core, host_id: i64) -> Result<(), String> {
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

pub fn invalidate_docker_cache(state: &Core, host_id: i64) -> Result<(), String> {
    let mut cache = state.docker_cache.lock().map_err(|e| e.to_string())?;
    cache.remove(&host_id);
    Ok(())
}

/// Parse an optional user-supplied JSON document, naming the field on failure.
pub fn parse_optional_doc(
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

// ---------------------------------------------------------------------------
// Redis
// ---------------------------------------------------------------------------

/// The keyring account holding a host's Redis password.
///
/// Non-numeric by construction, so it cannot collide with an SSH host password
/// (stored under the bare id) or with `mongo-remote-<id>` in the shared
/// fallback file. `crypto::tests::named_accounts_cannot_collide_with_host_ids`
/// pins that property.
pub fn redis_account(host_id: i64) -> String {
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
pub fn load_redis_uri(state: &Core, host_id: i64) -> Result<String, String> {
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
pub async fn redis_tunnel_port(
    state: &Core,
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

    let target = resolve_target(state, &via, ssh_password)?;

    // Holding the lock across the handshake keeps two tabs opening at once from
    // building two tunnels, the same reason `mongo_client` holds its lock.
    let mut tunnels = state.redis_tunnels.lock().await;
    if let Some(tunnel) = tunnels.get(&host.id) {
        return Ok(tunnel.local_port);
    }

    let tunnel = tokio::task::spawn_blocking(move || {
        port_forward::start_ephemeral_local(target, remote_host, remote_port)
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
pub async fn redis_dial_uri(
    state: &Core,
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
pub async fn redis_conn(
    state: &Core,
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
pub async fn forget_redis(state: &Core, host_id: i64) {
    state
        .redis_conns
        .lock()
        .await
        .retain(|(id, _), _| *id != host_id);
    // Dropping the handle signals the accept loop to stop.
    state.redis_tunnels.lock().await.remove(&host_id);
}

/// Run a cancellable Redis operation, registering its flag for the duration.
pub async fn with_redis_op<T, Fut>(
    state: &Core,
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

// ---------------------------------------------------------------------------
// VNC
// ---------------------------------------------------------------------------

/// The keyring account for a host's VNC password.
///
/// Non-numeric by construction, so it cannot collide with an SSH host password
/// stored under the bare id, nor with `redis-<id>` or `mongo-remote-<id>` in the
/// shared fallback file.
pub fn vnc_account(host_id: i64) -> String {
    format!("vnc-{}", host_id)
}

/// The VNC password for a host, or `None` when it has none stored.
///
/// The two "keyring retrieve failed" messages in this module are worded
/// differently on purpose. A VNC tab on an SSH host can need *two* secrets --
/// one for the bastion, one for the VNC server -- and the frontend's
/// prompt-and-retry has to know which one to ask for. Both still contain the
/// substring the shared detector looks for.
pub fn load_vnc_password(host_id: i64) -> Result<Option<String>, String> {
    match crypto::get_secret(&vnc_account(host_id)) {
        Ok(password) => Ok(Some(password)),
        // No entry is normal: a server with no password, or `None` security.
        Err(_) => Ok(None),
    }
}

/// Where to actually dial for a host's VNC server, and whether it is tunnelled.
///
/// An SSH host carrying VNC settings tunnels **through itself** unless told
/// otherwise, because a VNC server almost always binds 127.0.0.1 and would
/// not be reachable any other way.
pub async fn vnc_dial_target(
    state: &Core,
    host: &db::Host,
    ssh_password: Option<String>,
) -> Result<(String, u16, bool), String> {
    let address = host
        .vnc_host
        .as_deref()
        .map(str::trim)
        .filter(|h| !h.is_empty())
        .ok_or_else(|| "no VNC connection configured".to_string())?;
    let port = u16::try_from(host.vnc_port.unwrap_or(5900))
        .map_err(|_| format!("invalid VNC port {:?}", host.vnc_port))?;

    let bastion_id = match host.vnc_tunnel_host_id {
        Some(id) => Some(id),
        None if !host.host.trim().is_empty() => Some(host.id),
        None => None,
    };
    let Some(bastion_id) = bastion_id else {
        return Ok((address.to_string(), port, false));
    };

    let local_port = vnc_tunnel_port(state, host, bastion_id, address, port, ssh_password).await?;
    Ok(("127.0.0.1".to_string(), local_port, true))
}

/// Bring up this host's SSH tunnel if it has one, returning the local port.
///
/// Mirrors `redis_tunnel_port`: double-checked so two tabs opening at once
/// cannot build two tunnels, and `start_ephemeral_local` is blocking so it goes
/// to a blocking task.
///
/// The tunnel opens its **own** SSH session per connection
/// (`port_forward::handle_local_connection`), entirely separate from this
/// host's entry in `exec_sessions`. That is what makes a VNC tab safe beside a
/// shell tab on the same host: the rule against running two things on one
/// `ssh2::Session` is not in play here, and must not be "optimised" into being.
pub async fn vnc_tunnel_port(
    state: &Core,
    host: &db::Host,
    bastion_id: i64,
    remote_host: &str,
    remote_port: u16,
    ssh_password: Option<String>,
) -> Result<u16, String> {
    if let Some(tunnel) = state.vnc_tunnels.lock().await.get(&host.id) {
        return Ok(tunnel.local_port);
    }

    let via = if bastion_id == host.id {
        host.clone()
    } else {
        with_db(state, |conn| db::get_host_by_id(conn, bastion_id))?.ok_or_else(|| {
            "the SSH host this VNC connection tunnels through no longer exists".to_string()
        })?
    };

    // `resolve_target`, so a bastion that is itself behind a jump host works,
    // and its host key is checked like every other SSH connection.
    let target = resolve_target(state, &via, ssh_password)?;
    let (remote_host, remote_port) = (remote_host.to_string(), remote_port);

    let mut tunnels = state.vnc_tunnels.lock().await;
    if let Some(tunnel) = tunnels.get(&host.id) {
        return Ok(tunnel.local_port);
    }

    let tunnel = tokio::task::spawn_blocking(move || {
        port_forward::start_ephemeral_local(target, remote_host, remote_port)
    })
    .await
    .map_err(|e| format!("VNC tunnel task panicked: {}", e))??;

    let port = tunnel.local_port;
    tunnels.insert(host.id, tunnel);
    tracing::info!(host_id = host.id, local_port = port, "VNC tunnel opened");
    Ok(port)
}

/// Drop a host's tunnel once no session is using it any more.
///
/// The refcount lives here rather than in the frontend because a VNC tab can
/// sit on an SSH host that also has a shell tab open, and the store's
/// "last tab for this host" test cannot see the difference.
pub async fn release_vnc_host(state: &Core, host_id: i64) {
    let still_in_use = state
        .vnc_sessions
        .lock()
        .map(|s| s.values().any(|h| h.host_id == host_id))
        .unwrap_or(false);
    if !still_in_use {
        state.vnc_tunnels.lock().await.remove(&host_id);
    }
}

/// Run `f` against a live session, or report that it is gone.
pub fn with_vnc<T>(
    state: &Core,
    session_id: &str,
    f: impl FnOnce(&vnc::VncSessionHandle) -> Result<T, String>,
) -> Result<T, String> {
    let sessions = state
        .vnc_sessions
        .lock()
        .map_err(|_| "session registry poisoned".to_string())?;
    let handle = sessions
        .get(session_id)
        .ok_or_else(|| "this VNC session is no longer open".to_string())?;
    f(handle)
}

/// Where the app keeps its database, shared by the Tauri and native apps so
/// either one finds the hosts the other saved.
pub fn default_db_path() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("termdrop.db")
}

/// Create every table and run the launch-time migrations, in the order the
/// Tauri app has always run them. Idempotent, so it runs on every launch.
pub fn prepare_database(conn: &rusqlite::Connection) -> Result<(), String> {
    db::init_db(conn).map_err(|e| format!("Failed to initialize database: {}", e))?;
    db::init_port_forwards(conn)
        .map_err(|e| format!("Failed to initialize port forwards: {}", e))?;
    db::init_settings(conn).map_err(|e| format!("Failed to initialize settings: {}", e))?;
    split_local_mongo_hosts(conn);
    migrate_mongo_credentials(conn);
    Ok(())
}

impl Core {
    /// A core over an already prepared database pool, with nothing connected.
    pub fn new(db: Pool<SqliteConnectionManager>) -> Self {
        Core {
            db,
            sessions: Mutex::new(HashMap::new()),
            local_shells: Mutex::new(Vec::new()),
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
            vnc_sessions: Mutex::new(HashMap::new()),
            vnc_tunnels: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
        }
    }

    /// Open the database at `path` (see [`default_db_path`]), prepare it, and
    /// build a core over it.
    pub fn open(path: &Path) -> Result<Self, String> {
        let pool = Pool::builder()
            .max_size(10)
            .build(SqliteConnectionManager::file(path))
            .map_err(|e| format!("Failed to create database pool: {}", e))?;
        {
            let conn = pool.get().map_err(db_err)?;
            prepare_database(&conn)?;
        }
        Ok(Core::new(pool))
    }

    pub fn get_hosts(&self) -> Result<Vec<db::Host>, String> {
        with_db(self, db::get_hosts)
    }

    pub fn get_host(&self, id: i64) -> Result<Option<db::Host>, String> {
        with_db(self, |conn| db::get_host_by_id(conn, id))
    }

    pub fn get_setting(&self, key: &str) -> Result<Option<String>, String> {
        with_db(self, |conn| db::get_setting(conn, key))
    }

    pub fn set_setting(&self, key: &str, value: &str) -> Result<(), String> {
        with_db(self, |conn| db::set_setting(conn, key, value))
    }

    /// Connect a terminal to a saved SSH host, returning its session id.
    ///
    /// `accept_host_key` / `accept_jump_host_key` are the fingerprints the user
    /// was shown and agreed to, after an earlier call failed with
    /// [`crate::CoreError::Trust`]. They are compared against the key the
    /// server actually presents, so an acceptance cannot be replayed to let a
    /// different server through.
    ///
    /// Output goes to the sink attached with [`Core::attach_terminal_sink`],
    /// and until then to `CoreEvent::PtyData` on `events`, which also receives
    /// the session's connect, disconnect and error events.
    #[allow(clippy::too_many_arguments)]
    pub async fn ssh_connect(
        &self,
        events: Events,
        host_id: i64,
        password: Option<String>,
        accept_host_key: Option<String>,
        accept_jump_host_key: Option<String>,
        cols: u32,
        rows: u32,
    ) -> CoreResult<String> {
        let host = load_host(self, host_id)?;
        let target = resolve_target(self, &host, password)?;

        // The exec session is opened **first**, and it is the one that carries
        // the user's acceptance. Two reasons, both of which bit:
        //
        // - Its error is what this command returns, so a trust refusal has to
        //   come from here to reach the frontend's dialog. The PTY thread's
        //   refusal only goes out as an `ssh-error` event into a terminal that
        //   does not exist yet.
        // - Opened alongside the terminal, it checked the key while the PTY
        //   thread was still recording it -- a race the exec session could lose.
        //
        // Once this succeeds the keys are on file, so the terminal never prompts.
        let accept = ssh::Accept {
            target: accept_host_key,
            jump: accept_jump_host_key,
        };
        let exec_target = target.clone();
        let exec_session =
            tokio::task::spawn_blocking(move || ssh::create_exec_session(&exec_target, &accept))
                .await
                .map_err(|e| format!("exec session task failed: {}", e))?
                .map_err(exec_session_error)?;

        let session_id = uuid::Uuid::new_v4().to_string();
        let handle = ssh::connect(events, session_id.clone(), host_id, target, cols, rows)?;

        {
            let mut sessions = self.sessions.lock().map_err(|e| e.to_string())?;
            sessions.insert(session_id.clone(), handle);
        }
        {
            let mut exec_sessions = self.exec_sessions.lock().map_err(|e| e.to_string())?;
            exec_sessions.insert(host_id, Arc::new(Mutex::new(exec_session)));
        }

        info!(session_id = %session_id, host_id = host_id, "SSH connected");
        Ok(session_id)
    }

    /// The shells this machine has, for the local terminal's menu. Re-detected
    /// on every call (the menu asks each time it opens) and cached for
    /// `local_connect`, which resolves the id the menu sends back.
    pub async fn list_local_shells(&self) -> Result<Vec<local_shells::LocalShell>, String> {
        let shells = tokio::task::spawn_blocking(local_shells::detect)
            .await
            .map_err(|e| format!("shell detection failed: {}", e))?;
        *self.local_shells.lock().map_err(|e| e.to_string())? = shells.clone();
        Ok(shells)
    }

    /// Open a shell on this machine in a terminal tab. It shares every other
    /// terminal call with SSH tabs; see `local_pty`.
    ///
    /// `shell_id` names an entry from `list_local_shells`, or is `None` for the
    /// default. Only detected shells can be started: the UI never names a
    /// program.
    pub async fn local_connect(
        &self,
        events: Events,
        shell_id: Option<String>,
        cols: u32,
        rows: u32,
    ) -> Result<String, String> {
        let shell = match shell_id {
            None => local_shells::default_shell(),
            Some(id) => {
                let cached = self
                    .local_shells
                    .lock()
                    .map_err(|e| e.to_string())?
                    .iter()
                    .find(|s| s.id == id)
                    .cloned();
                match cached {
                    Some(shell) => shell,
                    None => self
                        .list_local_shells()
                        .await?
                        .into_iter()
                        .find(|s| s.id == id)
                        .ok_or("That shell is no longer available")?,
                }
            }
        };

        let session_id = uuid::Uuid::new_v4().to_string();
        let label = shell.label.clone();
        let handle = local_pty::connect(
            events,
            session_id.clone(),
            shell,
            cols,
            rows,
            Arc::new(Mutex::new(None)),
        )?;
        self.sessions
            .lock()
            .map_err(|e| e.to_string())?
            .insert(session_id.clone(), handle);
        info!(session_id = %session_id, shell = %label, "Local shell started");
        Ok(session_id)
    }

    /// Send keystrokes (or a paste) to a terminal, SSH or local.
    pub fn ssh_write(&self, session_id: &str, data: String) -> Result<(), String> {
        self.ssh_write_bytes(session_id, data.into_bytes())
    }

    /// `ssh_write` for input that is not text. A legacy (X10) mouse report
    /// carries each coordinate as one raw byte, 0x80 and up past column 94,
    /// which a `String` could only hold by replacing it with U+FFFD.
    pub fn ssh_write_bytes(&self, session_id: &str, data: Vec<u8>) -> Result<(), String> {
        crate::dev_perf::mark("cmd");
        let sessions = self.sessions.lock().map_err(|e| e.to_string())?;
        let session = sessions.get(session_id).ok_or("Session not found")?;
        session.write_tx.send(data).map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn ssh_resize(&self, session_id: &str, cols: u32, rows: u32) -> Result<(), String> {
        let sessions = self.sessions.lock().map_err(|e| e.to_string())?;
        let session = sessions.get(session_id).ok_or("Session not found")?;
        session
            .resize_tx
            .send((cols, rows))
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Route a terminal's output to `sink` from now on, instead of the
    /// `PtyData` event fallback. A local shell waits briefly for this before it
    /// starts, so its prompt is not lost.
    pub fn attach_terminal_sink(
        &self,
        session_id: &str,
        sink: Box<dyn ByteSink>,
    ) -> Result<(), String> {
        let mut sessions = self.sessions.lock().map_err(|e| e.to_string())?;
        let session = sessions.get_mut(session_id).ok_or("Session not found")?;
        *session.data_channel.lock().map_err(|e| e.to_string())? = Some(sink);
        Ok(())
    }

    /// Close a terminal. When it was the last one for its host, the host's
    /// exec session, caches and docker exec PTYs go with it.
    pub fn ssh_disconnect(&self, session_id: &str) -> Result<(), String> {
        let host_id = {
            let mut sessions = self.sessions.lock().map_err(|e| e.to_string())?;
            match sessions.remove(session_id) {
                Some(session) => {
                    let _ = session.disconnect_tx.send(());
                    // A local shell has no host state to tear down.
                    match session.host_id {
                        Some(host_id) => host_id,
                        None => return Ok(()),
                    }
                }
                None => return Ok(()),
            }
        };

        // Only remove exec session if no other tabs use this host
        let sessions = self.sessions.lock().map_err(|e| e.to_string())?;
        let still_connected = sessions.values().any(|s| s.host_id == Some(host_id));
        drop(sessions);

        if !still_connected {
            let mut exec_sessions = self.exec_sessions.lock().map_err(|e| e.to_string())?;
            exec_sessions.remove(&host_id);
            drop(exec_sessions);
            forget_host_state(self, host_id)?;
        }

        Ok(())
    }

    /// Reconnect a terminal under the same session id, so the tab stays put.
    /// Emits `SshReconnected` on success. `sftp_session_id` names the tab's
    /// SFTP handle, which died with the connection and is replaced in place.
    pub async fn ssh_reconnect(
        &self,
        events: Events,
        session_id: &str,
        sftp_session_id: Option<String>,
        password: Option<String>,
    ) -> CoreResult<()> {
        let session_id = session_id.to_string();
        let (host_id, local_shell, data_channel) = {
            let sessions = self.sessions.lock().map_err(|e| e.to_string())?;
            let session = sessions.get(&session_id).ok_or("Session not found")?;
            (
                session.host_id,
                session.local_shell.clone(),
                session.data_channel.clone(),
            )
        };

        let Some(host_id) = host_id else {
            // A local shell: start a fresh one under the same id. The tab's data
            // channel is still live on the frontend, so it is carried over.
            let handle = local_pty::connect(
                events.clone(),
                session_id.clone(),
                local_shell.unwrap_or_else(local_shells::default_shell),
                80,
                24,
                data_channel,
            )?;
            let mut sessions = self.sessions.lock().map_err(|e| e.to_string())?;
            if let Some(old) = sessions.insert(session_id.clone(), handle) {
                let _ = old.disconnect_tx.send(());
            }
            drop(sessions);
            events.emit(CoreEvent::SshReconnected { session_id });
            return Ok(());
        };

        let host = load_host(self, host_id)?;

        let target = resolve_target(self, &host, password)?;

        // Remove old session
        {
            let mut sessions = self.sessions.lock().map_err(|e| e.to_string())?;
            if let Some(old) = sessions.remove(&session_id) {
                let _ = old.disconnect_tx.send(());
            }
        }

        // Remove old exec session and create new one off the async thread
        {
            let mut exec_sessions = self.exec_sessions.lock().map_err(|e| e.to_string())?;
            exec_sessions.remove(&host_id);
        }
        let exec_target = target.clone();
        let exec_session = tokio::task::spawn_blocking(move || {
            ssh::create_exec_session(&exec_target, &ssh::Accept::none())
        })
        .await
        .map_err(|e| format!("exec session task failed: {}", e))?
        .map_err(exec_session_error)?;

        // Open new connection with same session_id. A reconnect targets a host
        // whose key was accepted when the tab was first opened; if the key is no
        // longer what it was, the reconnect fails rather than asking again.
        let handle = ssh::connect(
            events.clone(),
            session_id.clone(),
            host_id,
            target.clone(),
            80,
            24,
        )?;

        {
            let mut sessions = self.sessions.lock().map_err(|e| e.to_string())?;
            sessions.insert(session_id.clone(), handle);
        }
        {
            let mut exec_sessions = self.exec_sessions.lock().map_err(|e| e.to_string())?;
            exec_sessions.insert(host_id, Arc::new(Mutex::new(exec_session)));
        }

        // The tab's SFTP handle died with the connection. Replace it under the
        // same id so the panel keeps working instead of holding a dead session
        // until the tab is closed. A failure here is not fatal to the shell.
        if let Some(sftp_id) = sftp_session_id {
            let sftp_target = target.clone();
            match tokio::task::spawn_blocking(move || sftp::sftp_connect(&sftp_target, host_id))
                .await
            {
                Ok(Ok(handle)) => {
                    let mut sftp_sessions = self.sftp_sessions.lock().map_err(|e| e.to_string())?;
                    sftp_sessions.insert(sftp_id, Arc::new(handle));
                }
                Ok(Err(e)) => tracing::warn!("SFTP reconnect failed: {}", e),
                Err(e) => tracing::warn!("SFTP reconnect task failed: {}", e),
            }
        }

        events.emit(CoreEvent::SshReconnected { session_id });
        Ok(())
    }

    /// Open the SFTP connection a terminal tab's file panel uses, returning
    /// its id. A separate SSH connection from the terminal's.
    pub async fn sftp_connect(&self, host_id: i64, password: Option<String>) -> CoreResult<String> {
        let host = load_host(self, host_id)?;

        let target = resolve_target(self, &host, password)?;

        let sftp_id = uuid::Uuid::new_v4().to_string();

        // Run blocking SFTP connect off the async thread
        let handle = tokio::task::spawn_blocking(move || sftp::sftp_connect(&target, host_id))
            .await
            .map_err(|e| format!("sftp connect task failed: {}", e))?
            .map_err(|e| format!("sftp connect: {}", e))?;

        let mut sftp_sessions = self.sftp_sessions.lock().map_err(|e| e.to_string())?;
        sftp_sessions.insert(sftp_id.clone(), Arc::new(handle));

        info!(sftp_id = %sftp_id, host_id = host_id, "SFTP connected");
        Ok(sftp_id)
    }

    /// Run `command` in a PTY on the host (a docker exec or logs pane). Events
    /// arrive as `PtyKind::ExecPty`; the caller picks the id.
    pub async fn exec_pty_connect(
        &self,
        events: Events,
        host_id: i64,
        pty_session_id: String,
        command: String,
    ) -> Result<String, String> {
        let host = load_host(self, host_id)?;

        let target = resolve_target(self, &host, None)?;

        let handle =
            ssh::exec_pty_connect(events, pty_session_id.clone(), host_id, target, command)?;

        {
            let mut sessions = self.exec_pty_sessions.lock().map_err(|e| e.to_string())?;
            sessions.insert(pty_session_id.clone(), handle);
        }

        Ok(pty_session_id)
    }

    pub fn attach_exec_pty_sink(
        &self,
        pty_session_id: &str,
        sink: Box<dyn ByteSink>,
    ) -> Result<(), String> {
        let mut sessions = self.exec_pty_sessions.lock().map_err(|e| e.to_string())?;
        let session = sessions
            .get_mut(pty_session_id)
            .ok_or("PTY session not found")?;
        *session.data_channel.lock().map_err(|e| e.to_string())? = Some(sink);
        Ok(())
    }

    pub fn exec_pty_write(&self, pty_session_id: &str, data: String) -> Result<(), String> {
        let sessions = self.exec_pty_sessions.lock().map_err(|e| e.to_string())?;
        let session = sessions
            .get(pty_session_id)
            .ok_or("PTY session not found")?;
        session
            .write_tx
            .send(data.into_bytes())
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn exec_pty_disconnect(&self, pty_session_id: &str) -> Result<(), String> {
        let mut sessions = self.exec_pty_sessions.lock().map_err(|e| e.to_string())?;
        if let Some(session) = sessions.remove(pty_session_id) {
            let _ = session.disconnect_tx.send(());
        }
        Ok(())
    }
}

/// Shared by the tests of the modules that add `impl Core` blocks.
#[cfg(test)]
pub(crate) mod tests_support {
    use super::*;

    /// A core over one in-memory database. One connection, because each
    /// in-memory connection is its own database.
    pub fn memory_core() -> Core {
        let pool = Pool::builder()
            .max_size(1)
            .build(SqliteConnectionManager::memory())
            .unwrap();
        prepare_database(&pool.get().unwrap()).unwrap();
        Core::new(pool)
    }
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
            jump_host_id: None,
            vnc_host: None,
            vnc_port: None,
            vnc_username: None,
            vnc_tunnel_host_id: None,
            vnc_view_only: None,
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
            jump_host_id: None,
            vnc_host: None,
            vnc_port: None,
            vnc_username: None,
            vnc_tunnel_host_id: None,
            vnc_view_only: 0,
        }
    }

    #[test]
    fn resolve_auth_password_host_uses_override_without_keyring() {
        let r = resolve_auth(&host("password", Some("/ignored")), Some("pw".into())).unwrap();
        assert_eq!(r.password.as_deref(), Some("pw"));
        assert_eq!(r.key_path, None);
    }

    #[test]
    fn a_jump_host_is_accepted_when_it_is_a_direct_ssh_host() {
        let mut target = host("password", None);
        let mut jump = host("password", None);
        jump.id = 8;
        jump.name = "bastion".into();
        target.jump_host_id = Some(8);
        assert!(check_jump(&target, Some(&jump)).is_ok());
    }

    #[test]
    fn a_jump_host_that_cannot_work_is_refused_by_name() {
        let mut target = host("password", None);
        target.name = "db".into();
        target.jump_host_id = Some(8);

        // Deleted.
        assert!(check_jump(&target, None)
            .unwrap_err()
            .contains("no longer exists"));

        // Itself.
        let same = target.clone();
        assert!(check_jump(&target, Some(&same))
            .unwrap_err()
            .contains("its own jump host"));

        // A chain.
        let mut chained = host("password", None);
        chained.id = 8;
        chained.name = "bastion".into();
        chained.jump_host_id = Some(9);
        let e = check_jump(&target, Some(&chained)).unwrap_err();
        assert!(e.contains("bastion") && e.contains("one hop"), "{}", e);

        // A datastore row.
        let mut redis = host("password", None);
        redis.id = 8;
        redis.host = String::new();
        assert!(check_jump(&target, Some(&redis))
            .unwrap_err()
            .contains("not an SSH host"));
    }

    #[test]
    fn a_trust_refusal_from_the_exec_session_keeps_its_prefix_at_index_0() {
        // The frontend only opens the trust dialog when the prefix is first.
        let refusal = format!("{}{{\"kind\":\"unknown\"}}", ssh::trust::TRUST_PREFIX);
        assert_eq!(exec_session_error(refusal.clone()), refusal);

        assert_eq!(
            exec_session_error("auth: denied".into()),
            "exec session: auth: denied"
        );
    }

    #[test]
    fn a_password_host_carries_no_key_passphrase() {
        // A passphrase belongs to a private key. Resolving one for a host that
        // authenticates by password would put a secret where nothing reads it.
        let r = resolve_auth(&host("password", Some("/ignored")), Some("pw".into())).unwrap();
        assert_eq!(r.passphrase, None);
    }

    #[test]
    fn resolve_auth_key_host_ignores_override() {
        let r = resolve_auth(&host("key", Some("/k")), Some("pw".into())).unwrap();
        assert_eq!(r.password, None);
        assert_eq!(r.key_path.as_deref(), Some("/k"));

        let r = resolve_auth(&host("key", None), None).unwrap();
        assert_eq!(r.password, None);
        assert_eq!(r.key_path, None);
    }

    #[test]
    fn resolve_auth_unknown_type_passes_override_and_key_through() {
        let r = resolve_auth(&host("other", Some("/k")), Some("pw".into())).unwrap();
        assert_eq!(r.password.as_deref(), Some("pw"));
        assert_eq!(r.key_path.as_deref(), Some("/k"));

        let r = resolve_auth(&host("other", Some("/k")), None).unwrap();
        assert_eq!(r.password, None);
        assert_eq!(r.key_path.as_deref(), Some("/k"));
    }

    /// What the native app does at startup: open a database, then read hosts
    /// and settings through the core.
    #[test]
    fn a_core_opened_on_a_fresh_file_is_ready_to_use() {
        let path = std::env::temp_dir().join(format!(
            "termdrop-core-open-{}-{}.db",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        let core = Core::open(&path).expect("open");
        assert!(core.get_hosts().unwrap().is_empty());
        assert_eq!(core.get_setting("theme").unwrap(), None);
        core.set_setting("theme", "light").unwrap();
        assert_eq!(core.get_setting("theme").unwrap().as_deref(), Some("light"));
        drop(core);

        // A second launch runs the same preparation over existing tables.
        let core = Core::open(&path).expect("reopen");
        assert_eq!(core.get_setting("theme").unwrap().as_deref(), Some("light"));
        drop(core);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn terminal_input_reaches_the_session_as_raw_bytes() {
        // A legacy mouse report at column 100: its coordinate byte is 0xa5,
        // which is not UTF-8 and must not become U+FFFD on the way.
        let core = Core::new(
            Pool::builder()
                .max_size(1)
                .build(SqliteConnectionManager::memory())
                .unwrap(),
        );
        let (write_tx, mut write_rx) = tokio::sync::mpsc::unbounded_channel();
        let (disconnect_tx, _d) = tokio::sync::mpsc::unbounded_channel();
        let (resize_tx, _r) = tokio::sync::mpsc::unbounded_channel();
        core.sessions.lock().unwrap().insert(
            "s".into(),
            ssh::SshSessionHandle {
                host_id: None,
                local_shell: None,
                write_tx,
                disconnect_tx,
                resize_tx,
                data_channel: Arc::new(Mutex::new(None)),
            },
        );
        let report = vec![0x1b, b'[', b'M', 0x20, 0xa5, 0x21];
        core.ssh_write_bytes("s", report.clone()).unwrap();
        assert_eq!(write_rx.try_recv().unwrap(), report);
        core.ssh_write("s", "é".into()).unwrap();
        assert_eq!(write_rx.try_recv().unwrap(), "é".as_bytes());
    }

    #[test]
    fn terminal_calls_on_an_unknown_session_fail_quietly_or_say_so() {
        let core = Core::new(
            Pool::builder()
                .max_size(1)
                .build(SqliteConnectionManager::memory())
                .unwrap(),
        );
        assert_eq!(
            core.ssh_write("nope", "x".into()).unwrap_err(),
            "Session not found"
        );
        assert_eq!(
            core.ssh_resize("nope", 80, 24).unwrap_err(),
            "Session not found"
        );
        // Closing a tab that is already gone is not an error: the frontend
        // closes tabs whose session died on its own.
        assert!(core.ssh_disconnect("nope").is_ok());
    }
}
