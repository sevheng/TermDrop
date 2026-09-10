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

    Ok(())
}

pub fn get_hosts(conn: &Connection) -> SqlResult<Vec<Host>> {
    let mut stmt = conn.prepare(
        "SELECT id, name, host, port, username, auth_type, key_path, \"group\", favorite, last_connected_at, created_at, mongo_uri, mongo_local_uri FROM hosts ORDER BY favorite DESC, name ASC"
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
        })
    })?;
    hosts.collect()
}

pub fn add_host(conn: &Connection, host: &NewHost) -> SqlResult<i64> {
    conn.execute(
        "INSERT INTO hosts (name, host, port, username, auth_type, key_path, \"group\", favorite, mongo_uri, mongo_local_uri) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
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
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

/// Returns the number of rows updated, which is 0 when `id` no longer exists.
pub fn update_host(conn: &Connection, id: i64, host: &NewHost) -> SqlResult<usize> {
    conn.execute(
        "UPDATE hosts SET name = ?1, host = ?2, port = ?3, username = ?4, auth_type = ?5, key_path = ?6, \"group\" = ?7, favorite = ?8, mongo_uri = ?9, mongo_local_uri = ?10 WHERE id = ?11",
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
        "SELECT id, name, host, port, username, auth_type, key_path, \"group\", favorite, last_connected_at, created_at, mongo_uri, mongo_local_uri FROM hosts WHERE id = ?1"
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
        "SELECT name, host, port, username, auth_type, key_path, \"group\", favorite, mongo_uri, mongo_local_uri FROM hosts ORDER BY name ASC"
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

/// Reject rows the host form would not accept either. Username is not
/// required and a missing key file is not an error here, because a
/// MongoDB-only host legitimately has no SSH fields at all.
pub fn validate_new_host(host: &NewHost) -> Result<(), String> {
    if host.name.trim().is_empty() {
        return Err("Name is required".to_string());
    }
    if !(1..=65535).contains(&host.port) && host.mongo_uri.is_none() {
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
    if host.host.trim().is_empty() && host.mongo_uri.is_none() {
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
