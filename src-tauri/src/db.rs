use rusqlite::{params, Connection, Result as SqlResult};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Host {
    pub id: i64,
    pub name: String,
    pub host: String,
    pub port: i64,
    pub username: String,
    pub auth_type: String,
    pub key_path: Option<String>,
    pub group: Option<String>,
    pub favorite: i64,
    pub last_connected_at: Option<String>,
    pub created_at: String,
    pub mongo_uri: Option<String>,
    pub mongo_local_uri: Option<String>,
    pub redis_uri: Option<String>,
    /// The id of the SSH host to tunnel through, or `None` to connect directly.
    pub redis_tunnel_host_id: Option<i64>,
    /// The id of the SSH host this SSH host is reached through (ProxyJump), or
    /// `None` to connect directly.
    pub jump_host_id: Option<i64>,
    /// The VNC server's address **as seen from the far end of the tunnel**
    /// when tunnelled, or directly reachable otherwise. Its presence is what
    /// makes a row VNC-capable; a row with this and no `host` is VNC-only.
    pub vnc_host: Option<String>,
    pub vnc_port: Option<i64>,
    /// Only Apple's Screen Sharing authentication uses a user name.
    pub vnc_username: Option<String>,
    /// As `redis_tunnel_host_id`, but with one extra rule: an SSH row that
    /// carries VNC settings and leaves this unset tunnels through *itself*,
    /// because a VNC server almost always binds 127.0.0.1.
    pub vnc_tunnel_host_id: Option<i64>,
    /// Persisted per host, so a machine you only ever watch cannot be typed
    /// into by accident after a reconnect.
    pub vnc_view_only: i64,
}

#[derive(Debug, Deserialize, Clone)]
pub struct NewHost {
    pub name: String,
    pub host: String,
    pub port: i64,
    pub username: String,
    pub auth_type: String,
    pub key_path: Option<String>,
    pub group: Option<String>,
    pub favorite: Option<i64>,
    pub mongo_uri: Option<String>,
    pub mongo_local_uri: Option<String>,
    pub redis_uri: Option<String>,
    pub redis_tunnel_host_id: Option<i64>,
    pub jump_host_id: Option<i64>,
    // Defaulted, unlike the fields above: an exported file written before VNC
    // existed has no such keys, and rejecting the whole import over a missing
    // optional would be a poor trade.
    #[serde(default)]
    pub vnc_host: Option<String>,
    #[serde(default)]
    pub vnc_port: Option<i64>,
    #[serde(default)]
    pub vnc_username: Option<String>,
    #[serde(default)]
    pub vnc_tunnel_host_id: Option<i64>,
    #[serde(default)]
    pub vnc_view_only: Option<i64>,
}

pub fn init_db(conn: &Connection) -> SqlResult<()> {
    conn.execute(
        "CREATE TABLE IF NOT EXISTS hosts (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            name TEXT NOT NULL,
            host TEXT,
            port INTEGER DEFAULT 22,
            username TEXT,
            auth_type TEXT CHECK(auth_type IN ('password', 'key')) DEFAULT 'password',
            key_path TEXT,
            mongo_uri TEXT,
            mongo_local_uri TEXT,
            redis_uri TEXT,
            redis_tunnel_host_id INTEGER,
            jump_host_id INTEGER,
            vnc_host TEXT,
            vnc_port INTEGER,
            vnc_username TEXT,
            vnc_tunnel_host_id INTEGER,
            vnc_view_only INTEGER DEFAULT 0,
            created_at DATETIME DEFAULT CURRENT_TIMESTAMP
        )",
        [],
    )?;

    // Migrate old tables missing new columns
    let columns: Vec<String> = conn
        .prepare("PRAGMA table_info(hosts)")?
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<Result<Vec<_>, _>>()?;

    if !columns.contains(&"auth_type".to_string()) {
        conn.execute(
            "ALTER TABLE hosts ADD COLUMN auth_type TEXT CHECK(auth_type IN ('password', 'key')) DEFAULT 'password'",
            [],
        )?;
    }
    if !columns.contains(&"key_path".to_string()) {
        conn.execute("ALTER TABLE hosts ADD COLUMN key_path TEXT", [])?;
    }
    if !columns.contains(&"group".to_string()) {
        conn.execute("ALTER TABLE hosts ADD COLUMN \"group\" TEXT DEFAULT ''", [])?;
    }
    if !columns.contains(&"favorite".to_string()) {
        conn.execute(
            "ALTER TABLE hosts ADD COLUMN favorite INTEGER DEFAULT 0",
            [],
        )?;
    }
    if !columns.contains(&"last_connected_at".to_string()) {
        conn.execute(
            "ALTER TABLE hosts ADD COLUMN last_connected_at DATETIME",
            [],
        )?;
    }
    if !columns.contains(&"mongo_uri".to_string()) {
        conn.execute("ALTER TABLE hosts ADD COLUMN mongo_uri TEXT", [])?;
    }
    if !columns.contains(&"mongo_local_uri".to_string()) {
        conn.execute("ALTER TABLE hosts ADD COLUMN mongo_local_uri TEXT", [])?;
    }
    if !columns.contains(&"redis_uri".to_string()) {
        conn.execute("ALTER TABLE hosts ADD COLUMN redis_uri TEXT", [])?;
    }
    // No FOREIGN KEY: the table was created without one and SQLite cannot add a
    // constraint to an existing table without rebuilding it. A dangling id is
    // reported when the connection is opened, which beats a cascade that would
    // silently delete the Redis host along with its bastion.
    if !columns.contains(&"redis_tunnel_host_id".to_string()) {
        conn.execute(
            "ALTER TABLE hosts ADD COLUMN redis_tunnel_host_id INTEGER",
            [],
        )?;
    }
    // No FOREIGN KEY, for the same reason. Deleting a bastion leaves the hosts
    // behind it pointing at nothing, and connecting one says so. Clearing the
    // id instead would quietly reroute them to a direct connection.
    if !columns.contains(&"jump_host_id".to_string()) {
        conn.execute("ALTER TABLE hosts ADD COLUMN jump_host_id INTEGER", [])?;
    }
    if !columns.contains(&"vnc_host".to_string()) {
        conn.execute("ALTER TABLE hosts ADD COLUMN vnc_host TEXT", [])?;
    }
    if !columns.contains(&"vnc_port".to_string()) {
        conn.execute("ALTER TABLE hosts ADD COLUMN vnc_port INTEGER", [])?;
    }
    if !columns.contains(&"vnc_username".to_string()) {
        conn.execute("ALTER TABLE hosts ADD COLUMN vnc_username TEXT", [])?;
    }
    // No FOREIGN KEY, for the same reason as redis_tunnel_host_id above.
    if !columns.contains(&"vnc_tunnel_host_id".to_string()) {
        conn.execute(
            "ALTER TABLE hosts ADD COLUMN vnc_tunnel_host_id INTEGER",
            [],
        )?;
    }
    // The DEFAULT backfills existing rows with 0, which is why the Host field
    // can be a plain i64 rather than an Option.
    if !columns.contains(&"vnc_view_only".to_string()) {
        conn.execute(
            "ALTER TABLE hosts ADD COLUMN vnc_view_only INTEGER DEFAULT 0",
            [],
        )?;
    }

    Ok(())
}

pub fn get_hosts(conn: &Connection) -> SqlResult<Vec<Host>> {
    let mut stmt = conn.prepare(
        "SELECT id, name, host, port, username, auth_type, key_path, \"group\", favorite, last_connected_at, created_at, mongo_uri, mongo_local_uri, redis_uri, redis_tunnel_host_id, jump_host_id, vnc_host, vnc_port, vnc_username, vnc_tunnel_host_id, vnc_view_only FROM hosts ORDER BY favorite DESC, name ASC"
    )?;
    let hosts = stmt.query_map([], |row| {
        Ok(Host {
            id: row.get(0)?,
            name: row.get(1)?,
            host: row.get(2)?,
            port: row.get(3)?,
            username: row.get(4)?,
            auth_type: row.get(5)?,
            key_path: row.get(6)?,
            group: row.get(7)?,
            favorite: row.get(8)?,
            last_connected_at: row.get(9)?,
            created_at: row.get(10)?,
            mongo_uri: row.get(11)?,
            mongo_local_uri: row.get(12)?,
            redis_uri: row.get(13)?,
            redis_tunnel_host_id: row.get(14)?,
            jump_host_id: row.get(15)?,
            vnc_host: row.get(16)?,
            vnc_port: row.get(17)?,
            vnc_username: row.get(18)?,
            vnc_tunnel_host_id: row.get(19)?,
            vnc_view_only: row.get(20)?,
        })
    })?;
    hosts.collect()
}

pub fn add_host(conn: &Connection, host: &NewHost) -> SqlResult<i64> {
    conn.execute(
        "INSERT INTO hosts (name, host, port, username, auth_type, key_path, \"group\", favorite, mongo_uri, mongo_local_uri, redis_uri, redis_tunnel_host_id, jump_host_id, vnc_host, vnc_port, vnc_username, vnc_tunnel_host_id, vnc_view_only) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18)",
        params![
            &host.name,
            &host.host,
            host.port,
            &host.username,
            &host.auth_type,
            host.key_path.as_deref().unwrap_or(""),
            host.group.as_deref().unwrap_or(""),
            host.favorite.unwrap_or(0),
            host.mongo_uri.as_deref(),
            host.mongo_local_uri.as_deref(),
            host.redis_uri.as_deref(),
            host.redis_tunnel_host_id,
            host.jump_host_id,
            host.vnc_host.as_deref(),
            host.vnc_port,
            host.vnc_username.as_deref(),
            host.vnc_tunnel_host_id,
            host.vnc_view_only.unwrap_or(0),
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

/// Returns the number of rows updated, which is 0 when `id` no longer exists.
pub fn update_host(conn: &Connection, id: i64, host: &NewHost) -> SqlResult<usize> {
    conn.execute(
        "UPDATE hosts SET name = ?1, host = ?2, port = ?3, username = ?4, auth_type = ?5, key_path = ?6, \"group\" = ?7, favorite = ?8, mongo_uri = ?9, mongo_local_uri = ?10, redis_uri = ?11, redis_tunnel_host_id = ?12, jump_host_id = ?13, vnc_host = ?14, vnc_port = ?15, vnc_username = ?16, vnc_tunnel_host_id = ?17, vnc_view_only = ?18 WHERE id = ?19",
        params![
            &host.name,
            &host.host,
            host.port,
            &host.username,
            &host.auth_type,
            host.key_path.as_deref().unwrap_or(""),
            host.group.as_deref().unwrap_or(""),
            host.favorite.unwrap_or(0),
            host.mongo_uri.as_deref(),
            host.mongo_local_uri.as_deref(),
            host.redis_uri.as_deref(),
            host.redis_tunnel_host_id,
            host.jump_host_id,
            host.vnc_host.as_deref(),
            host.vnc_port,
            host.vnc_username.as_deref(),
            host.vnc_tunnel_host_id,
            host.vnc_view_only.unwrap_or(0),
            id
        ],
    )
}

pub fn delete_host(conn: &Connection, id: i64) -> SqlResult<()> {
    conn.execute("DELETE FROM hosts WHERE id = ?1", params![id])?;
    Ok(())
}

pub fn get_host_by_id(conn: &Connection, id: i64) -> SqlResult<Option<Host>> {
    let mut stmt = conn.prepare(
        "SELECT id, name, host, port, username, auth_type, key_path, \"group\", favorite, last_connected_at, created_at, mongo_uri, mongo_local_uri, redis_uri, redis_tunnel_host_id, jump_host_id, vnc_host, vnc_port, vnc_username, vnc_tunnel_host_id, vnc_view_only FROM hosts WHERE id = ?1"
    )?;
    let mut rows = stmt.query(params![id])?;
    if let Some(row) = rows.next()? {
        Ok(Some(Host {
            id: row.get(0)?,
            name: row.get(1)?,
            host: row.get(2)?,
            port: row.get(3)?,
            username: row.get(4)?,
            auth_type: row.get(5)?,
            key_path: row.get(6)?,
            group: row.get(7)?,
            favorite: row.get(8)?,
            last_connected_at: row.get(9)?,
            created_at: row.get(10)?,
            mongo_uri: row.get(11)?,
            mongo_local_uri: row.get(12)?,
            redis_uri: row.get(13)?,
            redis_tunnel_host_id: row.get(14)?,
            jump_host_id: row.get(15)?,
            vnc_host: row.get(16)?,
            vnc_port: row.get(17)?,
            vnc_username: row.get(18)?,
            vnc_tunnel_host_id: row.get(19)?,
            vnc_view_only: row.get(20)?,
        }))
    } else {
        Ok(None)
    }
}

pub fn update_host_group(conn: &Connection, id: i64, group: &str) -> SqlResult<()> {
    conn.execute(
        "UPDATE hosts SET \"group\" = ?1 WHERE id = ?2",
        params![group, id],
    )?;
    Ok(())
}

pub fn update_hosts_group_by_name(
    conn: &Connection,
    old_group: &str,
    new_group: &str,
) -> SqlResult<usize> {
    let count = conn.execute(
        "UPDATE hosts SET \"group\" = ?1 WHERE \"group\" = ?2",
        params![new_group, old_group],
    )?;
    Ok(count)
}

pub fn clear_hosts_group_by_name(conn: &Connection, group: &str) -> SqlResult<usize> {
    let count = conn.execute(
        "UPDATE hosts SET \"group\" = '' WHERE \"group\" = ?1",
        params![group],
    )?;
    Ok(count)
}

pub fn update_host_favorite(conn: &Connection, id: i64, favorite: i64) -> SqlResult<()> {
    conn.execute(
        "UPDATE hosts SET favorite = ?1 WHERE id = ?2",
        params![favorite, id],
    )?;
    Ok(())
}

pub fn update_host_last_connected(conn: &Connection, id: i64) -> SqlResult<()> {
    conn.execute(
        "UPDATE hosts SET last_connected_at = CURRENT_TIMESTAMP WHERE id = ?1",
        params![id],
    )?;
    Ok(())
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct PortForward {
    pub id: i64,
    pub host_id: i64,
    pub name: String,
    pub kind: String,
    pub local_host: String,
    pub local_port: i64,
    pub remote_host: Option<String>,
    pub remote_port: Option<i64>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct NewPortForward {
    pub host_id: i64,
    pub name: String,
    pub kind: String,
    pub local_host: String,
    pub local_port: i64,
    pub remote_host: Option<String>,
    pub remote_port: Option<i64>,
}

pub fn init_port_forwards(conn: &Connection) -> SqlResult<()> {
    conn.execute(
        "CREATE TABLE IF NOT EXISTS port_forwards (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            host_id INTEGER NOT NULL,
            name TEXT NOT NULL,
            kind TEXT CHECK(kind IN ('local', 'dynamic')) NOT NULL,
            local_host TEXT DEFAULT '127.0.0.1',
            local_port INTEGER NOT NULL,
            remote_host TEXT,
            remote_port INTEGER,
            FOREIGN KEY (host_id) REFERENCES hosts(id) ON DELETE CASCADE
        )",
        [],
    )?;
    Ok(())
}

pub fn get_port_forwards(conn: &Connection, host_id: i64) -> SqlResult<Vec<PortForward>> {
    let mut stmt = conn.prepare(
        "SELECT id, host_id, name, kind, local_host, local_port, remote_host, remote_port FROM port_forwards WHERE host_id = ?1 ORDER BY name ASC"
    )?;
    let forwards = stmt.query_map(params![host_id], |row| {
        Ok(PortForward {
            id: row.get(0)?,
            host_id: row.get(1)?,
            name: row.get(2)?,
            kind: row.get(3)?,
            local_host: row.get(4)?,
            local_port: row.get(5)?,
            remote_host: row.get(6)?,
            remote_port: row.get(7)?,
        })
    })?;
    forwards.collect()
}

pub fn add_port_forward(conn: &Connection, fw: &NewPortForward) -> SqlResult<i64> {
    conn.execute(
        "INSERT INTO port_forwards (host_id, name, kind, local_host, local_port, remote_host, remote_port) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            fw.host_id,
            &fw.name,
            &fw.kind,
            &fw.local_host,
            fw.local_port,
            fw.remote_host.as_deref(),
            fw.remote_port,
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn get_port_forward_by_id(conn: &Connection, id: i64) -> SqlResult<Option<PortForward>> {
    let mut stmt = conn.prepare(
        "SELECT id, host_id, name, kind, local_host, local_port, remote_host, remote_port FROM port_forwards WHERE id = ?1"
    )?;
    let mut rows = stmt.query(params![id])?;
    if let Some(row) = rows.next()? {
        Ok(Some(PortForward {
            id: row.get(0)?,
            host_id: row.get(1)?,
            name: row.get(2)?,
            kind: row.get(3)?,
            local_host: row.get(4)?,
            local_port: row.get(5)?,
            remote_host: row.get(6)?,
            remote_port: row.get(7)?,
        }))
    } else {
        Ok(None)
    }
}

pub fn update_port_forward(conn: &Connection, id: i64, fw: &NewPortForward) -> SqlResult<()> {
    conn.execute(
        "UPDATE port_forwards SET name = ?1, kind = ?2, local_host = ?3, local_port = ?4, remote_host = ?5, remote_port = ?6 WHERE id = ?7",
        params![
            &fw.name,
            &fw.kind,
            &fw.local_host,
            fw.local_port,
            fw.remote_host.as_deref(),
            fw.remote_port,
            id,
        ],
    )?;
    Ok(())
}

pub fn delete_port_forward(conn: &Connection, id: i64) -> SqlResult<()> {
    conn.execute("DELETE FROM port_forwards WHERE id = ?1", params![id])?;
    Ok(())
}

pub fn init_settings(conn: &Connection) -> SqlResult<()> {
    conn.execute(
        "CREATE TABLE IF NOT EXISTS settings (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL
        )",
        [],
    )?;
    Ok(())
}

pub fn get_setting(conn: &Connection, key: &str) -> SqlResult<Option<String>> {
    let mut stmt = conn.prepare("SELECT value FROM settings WHERE key = ?1")?;
    let mut rows = stmt.query(params![key])?;
    if let Some(row) = rows.next()? {
        Ok(Some(row.get(0)?))
    } else {
        Ok(None)
    }
}

pub fn set_setting(conn: &Connection, key: &str, value: &str) -> SqlResult<()> {
    conn.execute(
        "INSERT INTO settings (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key, value],
    )?;
    Ok(())
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ExportHost {
    pub name: String,
    pub host: String,
    pub port: i64,
    pub username: String,
    pub auth_type: String,
    pub key_path: Option<String>,
    pub group: Option<String>,
    pub favorite: i64,
    pub mongo_uri: Option<String>,
    pub mongo_local_uri: Option<String>,
    pub redis_uri: Option<String>,
    pub vnc_host: Option<String>,
    pub vnc_port: Option<i64>,
    pub vnc_username: Option<String>,
    pub vnc_view_only: i64,
}

/// One host's stored MongoDB URI: `(id, uri)`.
pub type MongoUriRow = (i64, Option<String>);

/// Every host's stored MongoDB URI, for the credential migration.
pub fn all_mongo_uris(conn: &Connection) -> SqlResult<Vec<MongoUriRow>> {
    let mut stmt = conn.prepare("SELECT id, mongo_uri FROM hosts WHERE mongo_uri IS NOT NULL")?;
    let rows = stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
    rows.collect()
}

/// Replace one host's stored MongoDB URI.
pub fn set_mongo_uri(conn: &Connection, id: i64, uri: &str) -> SqlResult<()> {
    conn.execute(
        "UPDATE hosts SET mongo_uri = ?1 WHERE id = ?2",
        rusqlite::params![uri, id],
    )?;
    Ok(())
}

/// Hosts that still carry a second MongoDB connection: `(id, name, local_uri)`.
///
/// A host used to hold a remote and a local URI so the two could be synced.
/// These rows are moved to their own host on launch; nothing else reads the
/// column any more.
pub fn hosts_with_local_mongo_uri(conn: &Connection) -> SqlResult<Vec<(i64, String, String)>> {
    let mut stmt = conn.prepare(
        "SELECT id, name, mongo_local_uri FROM hosts \
         WHERE mongo_local_uri IS NOT NULL AND TRIM(mongo_local_uri) != ''",
    )?;
    let rows = stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?;
    rows.collect()
}

/// Forget a host's second MongoDB connection once it has its own host row.
pub fn clear_mongo_local_uri(conn: &Connection, id: i64) -> SqlResult<()> {
    conn.execute(
        "UPDATE hosts SET mongo_local_uri = NULL WHERE id = ?1",
        rusqlite::params![id],
    )?;
    Ok(())
}

/// Reclaim free pages so overwritten plaintext does not linger in the file.
pub fn vacuum(conn: &Connection) -> SqlResult<()> {
    conn.execute_batch("VACUUM")
}

pub fn export_hosts(conn: &Connection) -> SqlResult<Vec<ExportHost>> {
    let mut stmt = conn.prepare(
        "SELECT name, host, port, username, auth_type, key_path, \"group\", favorite, mongo_uri, mongo_local_uri, redis_uri, vnc_host, vnc_port, vnc_username, vnc_view_only FROM hosts ORDER BY name ASC"
    )?;
    let hosts = stmt.query_map([], |row| {
        Ok(ExportHost {
            name: row.get(0)?,
            host: row.get(1)?,
            port: row.get(2)?,
            username: row.get(3)?,
            auth_type: row.get(4)?,
            key_path: row.get(5)?,
            group: row.get(6)?,
            favorite: row.get(7)?,
            // Defensive: after the credential migration these columns hold no
            // password, but an export must never carry one even if a row was
            // written before the migration ran.
            mongo_uri: row
                .get::<_, Option<String>>(8)?
                .map(|u| crate::mongodb::split_mongo_password(&u).0),
            mongo_local_uri: row
                .get::<_, Option<String>>(9)?
                .map(|u| crate::mongodb::split_mongo_password(&u).0),
            // Redis URIs are never written with a password, but strip anyway:
            // an export leaves the machine and this is the last gate.
            redis_uri: row
                .get::<_, Option<String>>(10)?
                .map(|u| crate::uri::split_password(&u).0),
            // redis_tunnel_host_id is deliberately not exported: a row id means
            // nothing on another machine and would point at an unrelated host.
            // vnc_tunnel_host_id is omitted for exactly the same reason.
            vnc_host: row.get(11)?,
            vnc_port: row.get(12)?,
            vnc_username: row.get(13)?,
            vnc_view_only: row.get(14)?,
            // No password strip: a VNC credential lives only in the keyring and
            // is never written to a column in the first place.
        })
    })?;
    hosts.collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(name: &str) -> NewHost {
        NewHost {
            name: name.to_string(),
            host: "10.0.0.1".to_string(),
            port: 22,
            username: "admin".to_string(),
            auth_type: "password".to_string(),
            key_path: None,
            group: None,
            favorite: None,
            mongo_uri: None,
            mongo_local_uri: None,
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
    fn validate_new_host_rejects_what_the_form_rejects() {
        assert!(validate_new_host(&sample("ok")).is_ok());

        let mut h = sample("");
        assert!(validate_new_host(&h).unwrap_err().contains("Name"));

        h = sample("x");
        h.port = 0;
        assert!(validate_new_host(&h).unwrap_err().contains("Port"));
        h.port = 65536;
        assert!(validate_new_host(&h).unwrap_err().contains("Port"));

        h = sample("x");
        h.auth_type = "agent".to_string();
        assert!(validate_new_host(&h).unwrap_err().contains("Auth type"));

        h = sample("x");
        h.host = String::new();
        assert!(validate_new_host(&h).unwrap_err().contains("Host"));
    }

    #[test]
    fn validate_new_host_allows_a_mongo_only_row() {
        // These legitimately carry no SSH host, username, or port.
        let mut h = sample("mongo");
        h.host = String::new();
        h.username = String::new();
        h.port = 0;
        h.mongo_uri = Some("mongodb://localhost".to_string());
        assert!(validate_new_host(&h).is_ok());
    }

    #[test]
    fn a_host_payload_without_the_redis_fields_still_deserializes() {
        // The frontend's existing SSH and MongoDB save paths do not send the
        // new columns. Serde treats a missing Option field as None, and this
        // pins that: if it ever stopped being true, every save from those two
        // dialogs would fail at runtime with no compile-time warning.
        let json = r#"{
            "name": "old client",
            "host": "10.0.0.1",
            "port": 22,
            "username": "admin",
            "auth_type": "password",
            "key_path": null,
            "group": null,
            "favorite": null,
            "mongo_uri": null,
            "mongo_local_uri": null
        }"#;
        let host: NewHost = serde_json::from_str(json).expect("payload must still deserialize");
        assert_eq!(host.name, "old client");
        assert_eq!(host.redis_uri, None);
        assert_eq!(host.redis_tunnel_host_id, None);
    }

    #[test]
    fn validate_new_host_allows_a_redis_only_row() {
        // Same shape as a MongoDB-only row: no SSH host, username, or port.
        let mut h = sample("redis");
        h.host = String::new();
        h.username = String::new();
        h.port = 0;
        h.redis_uri = Some("redis://localhost:6379/0".to_string());
        assert!(validate_new_host(&h).is_ok());
    }

    #[test]
    fn validate_new_host_allows_a_vnc_only_row() {
        // Same shape as the datastore-only rows: no SSH host, username or
        // port. Without `vnc_host` in `waives_ssh_fields`, importing one of
        // these would be rejected outright.
        let mut h = sample("kiosk");
        h.host = String::new();
        h.username = String::new();
        h.port = 0;
        h.vnc_host = Some("127.0.0.1".to_string());
        h.vnc_port = Some(5901);
        assert!(validate_new_host(&h).is_ok());
    }

    #[test]
    fn a_blank_vnc_host_does_not_waive_the_ssh_checks() {
        // `Some("")` is what an empty form field deserialises to, and it must
        // not count as "this row is a VNC host" -- otherwise a half-filled SSH
        // form would validate with no address at all.
        let mut h = sample("blank");
        h.host = String::new();
        h.vnc_host = Some("   ".to_string());
        assert!(validate_new_host(&h).unwrap_err().contains("Host"));
    }

    #[test]
    fn a_vnc_host_round_trips_through_add_read_and_update() {
        // The five columns are bound positionally in four separate statements;
        // a wrong index there surfaces as a type error on an unrelated column,
        // which reads like database corruption. This walks all four.
        let conn = Connection::open_in_memory().unwrap();
        init_db(&conn).unwrap();

        let bastion = add_host(&conn, &sample("bastion")).unwrap();
        let mut h = sample("kiosk");
        h.host = String::new();
        h.username = String::new();
        h.port = 0;
        h.vnc_host = Some("10.0.0.5".to_string());
        h.vnc_port = Some(5901);
        h.vnc_username = Some("admin".to_string());
        h.vnc_tunnel_host_id = Some(bastion);
        h.vnc_view_only = Some(1);
        let id = add_host(&conn, &h).unwrap();

        // get_host_by_id
        let stored = get_host_by_id(&conn, id).unwrap().unwrap();
        assert_eq!(stored.vnc_host.as_deref(), Some("10.0.0.5"));
        assert_eq!(stored.vnc_port, Some(5901));
        assert_eq!(stored.vnc_username.as_deref(), Some("admin"));
        assert_eq!(stored.vnc_tunnel_host_id, Some(bastion));
        assert_eq!(stored.vnc_view_only, 1);
        // The columns either side must not have been shifted by the new ones.
        assert_eq!(stored.name, "kiosk");
        assert_eq!(stored.redis_tunnel_host_id, None);

        // get_hosts
        let listed = get_hosts(&conn).unwrap();
        let listed = listed.iter().find(|x| x.id == id).unwrap();
        assert_eq!(listed.vnc_host.as_deref(), Some("10.0.0.5"));
        assert_eq!(listed.vnc_view_only, 1);

        // update_host
        h.vnc_port = Some(5902);
        h.vnc_view_only = Some(0);
        assert_eq!(update_host(&conn, id, &h).unwrap(), 1);
        let stored = get_host_by_id(&conn, id).unwrap().unwrap();
        assert_eq!(stored.vnc_port, Some(5902));
        assert_eq!(stored.vnc_view_only, 0);
        assert_eq!(stored.name, "kiosk", "an off-by-one would corrupt this");
    }

    #[test]
    fn an_ssh_host_may_also_carry_vnc_settings() {
        // The other half of the host model: VNC is a capability, not a kind,
        // so a fully populated SSH row can have it too.
        let conn = Connection::open_in_memory().unwrap();
        init_db(&conn).unwrap();

        let mut h = sample("web-01");
        h.vnc_host = Some("127.0.0.1".to_string());
        h.vnc_port = Some(5900);
        let id = add_host(&conn, &h).unwrap();

        let stored = get_host_by_id(&conn, id).unwrap().unwrap();
        assert_eq!(stored.host, "10.0.0.1", "still an SSH host");
        assert_eq!(stored.vnc_host.as_deref(), Some("127.0.0.1"));
    }

    #[test]
    fn the_vnc_columns_are_added_to_a_table_that_predates_them() {
        // The pre-VNC schema, verbatim, as an existing install has on disk.
        let conn = Connection::open_in_memory().unwrap();
        conn.execute(
            "CREATE TABLE hosts (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                name TEXT NOT NULL,
                host TEXT,
                port INTEGER DEFAULT 22,
                username TEXT,
                auth_type TEXT CHECK(auth_type IN ('password', 'key')) DEFAULT 'password',
                key_path TEXT,
                mongo_uri TEXT,
                mongo_local_uri TEXT,
                redis_uri TEXT,
                redis_tunnel_host_id INTEGER,
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP
            )",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO hosts (name, host, port, username) VALUES ('old', 'h', 22, 'u')",
            [],
        )
        .unwrap();

        init_db(&conn).unwrap();

        // The row survives, and reading it through the new column list works.
        let hosts = get_hosts(&conn).unwrap();
        assert_eq!(hosts.len(), 1);
        assert_eq!(hosts[0].name, "old");
        assert_eq!(hosts[0].vnc_host, None);
        // DEFAULT 0 backfills the existing row, which is what lets the Host
        // field be a plain i64 rather than an Option.
        assert_eq!(hosts[0].vnc_view_only, 0);

        // Idempotent: a second run must not try to add the columns again.
        init_db(&conn).unwrap();
    }

    #[test]
    fn an_export_carries_the_vnc_settings_but_not_the_tunnel_row_id() {
        let conn = Connection::open_in_memory().unwrap();
        init_db(&conn).unwrap();

        let bastion = add_host(&conn, &sample("bastion")).unwrap();
        let mut h = sample("kiosk");
        h.vnc_host = Some("10.0.0.5".to_string());
        h.vnc_port = Some(5901);
        h.vnc_username = Some("admin".to_string());
        h.vnc_tunnel_host_id = Some(bastion);
        add_host(&conn, &h).unwrap();

        let exported = export_hosts(&conn).unwrap();
        let kiosk = exported.iter().find(|x| x.name == "kiosk").unwrap();
        assert_eq!(kiosk.vnc_host.as_deref(), Some("10.0.0.5"));
        assert_eq!(kiosk.vnc_username.as_deref(), Some("admin"));

        // A row id means nothing on another machine and would point at an
        // unrelated host, so it is deliberately absent from ExportHost --
        // asserted by the type not having the field, and by this serialisation
        // check so a future addition has to be deliberate.
        let json = serde_json::to_string(kiosk).unwrap();
        assert!(
            !json.contains("vnc_tunnel_host_id"),
            "tunnel row id leaked into the export: {}",
            json
        );
    }

    #[test]
    fn a_host_payload_written_before_vnc_existed_still_deserialises() {
        // An exported file from an older version has none of these keys, and
        // rejecting the whole import over a missing optional would be a poor
        // trade -- hence #[serde(default)] on the new fields.
        let json = r#"{
            "name": "old", "host": "h", "port": 22, "username": "u",
            "auth_type": "password", "key_path": null, "group": null,
            "favorite": 0, "mongo_uri": null, "mongo_local_uri": null,
            "redis_uri": null, "redis_tunnel_host_id": null
        }"#;
        let parsed: NewHost = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.name, "old");
        assert_eq!(parsed.vnc_host, None);
        assert_eq!(parsed.vnc_view_only, None);
    }

    #[test]
    fn a_row_with_neither_uri_still_needs_its_ssh_fields() {
        // The relaxation must be tied to actually having a datastore URI --
        // otherwise every malformed SSH host would silently validate.
        let mut h = sample("neither");
        h.host = String::new();
        assert!(validate_new_host(&h).unwrap_err().contains("Host"));
        h.host = "10.0.0.1".to_string();
        h.port = 0;
        assert!(validate_new_host(&h).unwrap_err().contains("Port"));
    }

    #[test]
    fn a_redis_host_round_trips_through_add_and_read() {
        let conn = Connection::open_in_memory().unwrap();
        init_db(&conn).unwrap();

        let bastion = add_host(&conn, &sample("bastion")).unwrap();
        let mut h = sample("cache");
        h.host = String::new();
        h.port = 0;
        h.username = String::new();
        h.redis_uri = Some("redis://@10.0.1.5:6379/2".to_string());
        h.redis_tunnel_host_id = Some(bastion);
        let id = add_host(&conn, &h).unwrap();

        let read = get_host_by_id(&conn, id).unwrap().unwrap();
        assert_eq!(read.redis_uri.as_deref(), Some("redis://@10.0.1.5:6379/2"));
        assert_eq!(read.redis_tunnel_host_id, Some(bastion));

        // The same values must survive an update, not just an insert.
        let mut edited = h.clone();
        edited.redis_uri = Some("redis://@10.0.1.9:6380/0".to_string());
        edited.redis_tunnel_host_id = None;
        update_host(&conn, id, &edited).unwrap();
        let read = get_host_by_id(&conn, id).unwrap().unwrap();
        assert_eq!(read.redis_uri.as_deref(), Some("redis://@10.0.1.9:6380/0"));
        assert_eq!(read.redis_tunnel_host_id, None);

        // And through the list query, which has its own column list.
        let listed = get_hosts(&conn).unwrap();
        let found = listed.iter().find(|r| r.id == id).unwrap();
        assert_eq!(found.redis_uri.as_deref(), Some("redis://@10.0.1.9:6380/0"));
    }

    #[test]
    fn a_jump_host_round_trips_and_survives_its_bastion_being_deleted() {
        let conn = Connection::open_in_memory().unwrap();
        init_db(&conn).unwrap();

        let bastion = add_host(&conn, &sample("bastion")).unwrap();
        let mut h = sample("db");
        h.jump_host_id = Some(bastion);
        let id = add_host(&conn, &h).unwrap();
        assert_eq!(
            get_host_by_id(&conn, id).unwrap().unwrap().jump_host_id,
            Some(bastion)
        );

        // Both queries have their own column list.
        let listed = get_hosts(&conn).unwrap();
        assert_eq!(
            listed.iter().find(|r| r.id == id).unwrap().jump_host_id,
            Some(bastion)
        );

        let mut edited = h.clone();
        edited.jump_host_id = None;
        update_host(&conn, id, &edited).unwrap();
        assert_eq!(
            get_host_by_id(&conn, id).unwrap().unwrap().jump_host_id,
            None
        );

        // Deleting the bastion must not reroute the host behind it to a direct
        // connection: the id stays, and connecting reports it as missing.
        update_host(&conn, id, &h).unwrap();
        delete_host(&conn, bastion).unwrap();
        assert_eq!(
            get_host_by_id(&conn, id).unwrap().unwrap().jump_host_id,
            Some(bastion)
        );
    }

    #[test]
    fn export_never_carries_a_jump_host_row_id() {
        let conn = Connection::open_in_memory().unwrap();
        init_db(&conn).unwrap();
        let bastion = add_host(&conn, &sample("bastion")).unwrap();
        let mut h = sample("db");
        h.jump_host_id = Some(bastion);
        add_host(&conn, &h).unwrap();

        let json = serde_json::to_string(&export_hosts(&conn).unwrap()).unwrap();
        assert!(
            !json.contains("jump_host_id"),
            "a row id must not leave the machine: {}",
            json
        );
    }

    #[test]
    fn export_never_carries_a_redis_password_or_a_tunnel_row_id() {
        let conn = Connection::open_in_memory().unwrap();
        init_db(&conn).unwrap();

        let bastion = add_host(&conn, &sample("bastion")).unwrap();
        let mut h = sample("cache");
        // A row written before the split, or by hand: it still must not export.
        h.redis_uri = Some("redis://:hunter2@10.0.1.5:6379/0".to_string());
        h.redis_tunnel_host_id = Some(bastion);
        add_host(&conn, &h).unwrap();

        let exported = export_hosts(&conn).unwrap();
        let row = exported.iter().find(|r| r.name == "cache").unwrap();
        assert_eq!(row.redis_uri.as_deref(), Some("redis://@10.0.1.5:6379/0"));
        let json = serde_json::to_string(&exported).unwrap();
        assert!(!json.contains("hunter2"), "password survived: {}", json);
        assert!(
            !json.contains("redis_tunnel_host_id"),
            "a row id must not leave the machine: {}",
            json
        );
    }

    #[test]
    fn init_db_adds_the_redis_columns_to_an_older_table_without_losing_rows() {
        let conn = Connection::open_in_memory().unwrap();
        // The pre-Redis schema, as an existing install has it.
        conn.execute(
            "CREATE TABLE hosts (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                name TEXT NOT NULL,
                host TEXT,
                port INTEGER DEFAULT 22,
                username TEXT,
                auth_type TEXT DEFAULT 'password',
                key_path TEXT,
                mongo_uri TEXT,
                mongo_local_uri TEXT,
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP
            )",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO hosts (name, host, port, username) VALUES ('old', '10.0.0.1', 22, 'admin')",
            [],
        )
        .unwrap();

        init_db(&conn).unwrap();

        let hosts = get_hosts(&conn).unwrap();
        assert_eq!(hosts.len(), 1, "the existing row was lost");
        assert_eq!(hosts[0].name, "old");
        assert_eq!(hosts[0].redis_uri, None);
        assert_eq!(hosts[0].redis_tunnel_host_id, None);

        // Idempotent: a second run must not fail on duplicate columns.
        init_db(&conn).unwrap();
    }

    #[test]
    fn mongo_uri_migration_reads_and_rewrites_the_uri() {
        let conn = Connection::open_in_memory().unwrap();
        init_db(&conn).unwrap();

        let mut h = sample("mongo");
        h.mongo_uri = Some("mongodb://u:pw@remote:27017".to_string());
        let id = add_host(&conn, &h).unwrap();

        // A host with no MongoDB URI is not offered to the migration.
        add_host(&conn, &sample("ssh-only")).unwrap();

        let rows = all_mongo_uris(&conn).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].0, id);

        set_mongo_uri(&conn, id, "mongodb://u@remote:27017").unwrap();
        let after = get_host_by_id(&conn, id).unwrap().unwrap();
        assert_eq!(after.mongo_uri.unwrap(), "mongodb://u@remote:27017");
    }

    #[test]
    fn a_second_connection_is_found_once_and_then_cleared() {
        let conn = Connection::open_in_memory().unwrap();
        init_db(&conn).unwrap();

        let mut h = sample("pair");
        h.mongo_uri = Some("mongodb://u@remote:27017".to_string());
        h.mongo_local_uri = Some("mongodb://u@local:27017".to_string());
        let id = add_host(&conn, &h).unwrap();

        let rows = hosts_with_local_mongo_uri(&conn).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].0, id);
        assert_eq!(rows[0].2, "mongodb://u@local:27017");

        // Clearing it is what stops the split running twice.
        clear_mongo_local_uri(&conn, id).unwrap();
        assert!(hosts_with_local_mongo_uri(&conn).unwrap().is_empty());
        // The host's own connection is untouched.
        let after = get_host_by_id(&conn, id).unwrap().unwrap();
        assert_eq!(after.mongo_uri.unwrap(), "mongodb://u@remote:27017");
    }

    #[test]
    fn export_never_carries_a_mongo_password() {
        let conn = Connection::open_in_memory().unwrap();
        init_db(&conn).unwrap();

        // A row written before the migration ran.
        let mut h = sample("legacy");
        h.mongo_uri = Some("mongodb://admin:hunter2@localhost:27017".to_string());
        add_host(&conn, &h).unwrap();

        let exported = export_hosts(&conn).unwrap();
        let uri = exported[0].mongo_uri.clone().unwrap();
        assert!(
            !uri.contains("hunter2"),
            "export leaked a password: {}",
            uri
        );
        assert_eq!(uri, "mongodb://admin@localhost:27017");
    }

    #[test]
    fn update_host_reports_rows_affected() {
        let conn = Connection::open_in_memory().unwrap();
        init_db(&conn).unwrap();
        let id = add_host(&conn, &sample("a")).unwrap();
        assert_eq!(update_host(&conn, id, &sample("b")).unwrap(), 1);
        // was: a silent no-op, so a stale id looked like a successful edit.
        assert_eq!(update_host(&conn, 9999, &sample("b")).unwrap(), 0);
    }

    #[test]
    fn import_adds_replaces_and_reports_failures_without_losing_good_rows() {
        let conn = Connection::open_in_memory().unwrap();
        init_db(&conn).unwrap();
        let existing = add_host(&conn, &sample("existing")).unwrap();

        let mut bad = sample("bad");
        bad.port = -1;
        let mut replacement = sample("existing");
        replacement.username = "replaced".to_string();

        let summary = import_entries(
            &conn,
            vec![
                ImportEntry {
                    host: sample("fresh"),
                    replace_id: None,
                },
                ImportEntry {
                    host: replacement,
                    replace_id: Some(existing),
                },
                ImportEntry {
                    host: bad,
                    replace_id: None,
                },
                // A replace whose target is gone becomes an insert.
                ImportEntry {
                    host: sample("orphan"),
                    replace_id: Some(4242),
                },
            ],
        );

        assert_eq!(summary.added, 2, "fresh and orphan");
        assert_eq!(summary.replaced, 1);
        assert_eq!(summary.failed.len(), 1);
        assert_eq!(summary.failed[0].name, "bad");
        assert!(summary.failed[0].reason.contains("Port"));

        // The good rows are committed even though one row failed.
        let hosts = get_hosts(&conn).unwrap();
        assert_eq!(hosts.len(), 3);
        let replaced = hosts.iter().find(|h| h.id == existing).unwrap();
        assert_eq!(replaced.username, "replaced", "replace keeps the row id");
    }

    #[test]
    fn test_host_crud() {
        let conn = Connection::open_in_memory().unwrap();
        init_db(&conn).unwrap();

        let new = NewHost {
            name: "Test Server".to_string(),
            host: "192.168.1.1".to_string(),
            port: 22,
            username: "admin".to_string(),
            auth_type: "password".to_string(),
            key_path: None,
            group: None,
            favorite: None,
            mongo_uri: None,
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

        let id = add_host(&conn, &new).unwrap();
        assert_eq!(id, 1);

        let hosts = get_hosts(&conn).unwrap();
        assert_eq!(hosts.len(), 1);
        assert_eq!(hosts[0].name, "Test Server");

        let mut updated = new.clone();
        updated.name = "Updated Server".to_string();
        update_host(&conn, id, &updated).unwrap();

        let host = get_host_by_id(&conn, id).unwrap().unwrap();
        assert_eq!(host.name, "Updated Server");

        delete_host(&conn, id).unwrap();
        let hosts = get_hosts(&conn).unwrap();
        assert_eq!(hosts.len(), 0);
    }
}

// ---------------------------------------------------------------------------
// Host import
// ---------------------------------------------------------------------------

/// One host to import. `replace_id` names an existing row to update in place
/// rather than inserting; updating preserves the row id, and therefore the
/// keyring entry and any port forwards that reference it.
#[derive(Debug, Deserialize)]
pub struct ImportEntry {
    pub host: NewHost,
    pub replace_id: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct ImportFailure {
    pub name: String,
    pub reason: String,
}

#[derive(Debug, Serialize, Default)]
pub struct ImportSummary {
    pub added: usize,
    pub replaced: usize,
    pub failed: Vec<ImportFailure>,
}

/// Whether this row's service configuration waives the SSH field checks.
///
/// A MongoDB-, Redis- or VNC-only host legitimately has no SSH host, port or
/// username. Renamed from `is_datastore_only` when VNC arrived: VNC is not a
/// datastore, and a predicate whose name is a lie is worse than a rename.
fn waives_ssh_fields(host: &NewHost) -> bool {
    host.mongo_uri.is_some()
        || host.redis_uri.is_some()
        || host
            .vnc_host
            .as_deref()
            .is_some_and(|h| !h.trim().is_empty())
}

/// Reject rows the host form would not accept either. Username is not
/// required and a missing key file is not an error here, because a
/// datastore-only host legitimately has no SSH fields at all.
pub fn validate_new_host(host: &NewHost) -> Result<(), String> {
    if host.name.trim().is_empty() {
        return Err("Name is required".to_string());
    }
    if !(1..=65535).contains(&host.port) && !waives_ssh_fields(host) {
        return Err(format!(
            "Port must be between 1 and 65535, got {}",
            host.port
        ));
    }
    if host.auth_type != "password" && host.auth_type != "key" {
        return Err(format!(
            "Auth type must be \"password\" or \"key\", got \"{}\"",
            host.auth_type
        ));
    }
    if host.host.trim().is_empty() && !waives_ssh_fields(host) {
        return Err("Host is required".to_string());
    }
    Ok(())
}

/// Insert or replace each entry, collecting per-row failures instead of
/// aborting the whole import. The caller has already reviewed the list, so
/// one bad row should not discard the rest.
pub fn import_entries(conn: &Connection, entries: Vec<ImportEntry>) -> ImportSummary {
    let mut summary = ImportSummary::default();

    for entry in entries {
        let name = entry.host.name.clone();
        if let Err(reason) = validate_new_host(&entry.host) {
            summary.failed.push(ImportFailure { name, reason });
            continue;
        }

        // A replace whose target has since been deleted falls back to an insert.
        let replaced = match entry.replace_id {
            Some(id) => match update_host(conn, id, &entry.host) {
                Ok(rows) => rows > 0,
                Err(e) => {
                    summary.failed.push(ImportFailure {
                        name,
                        reason: e.to_string(),
                    });
                    continue;
                }
            },
            None => false,
        };

        if replaced {
            summary.replaced += 1;
        } else if let Err(e) = add_host(conn, &entry.host) {
            summary.failed.push(ImportFailure {
                name,
                reason: e.to_string(),
            });
        } else {
            summary.added += 1;
        }
    }

    summary
}
