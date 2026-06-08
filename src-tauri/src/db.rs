use rusqlite::{Connection, Result as SqlResult, params};
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
}

pub fn init_db(conn: &Connection) -> SqlResult<()> {
    conn.execute(
        "CREATE TABLE IF NOT EXISTS hosts (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            name TEXT NOT NULL,
            host TEXT NOT NULL,
            port INTEGER DEFAULT 22,
            username TEXT NOT NULL,
            auth_type TEXT CHECK(auth_type IN ('password', 'key')) DEFAULT 'password',
            key_path TEXT,
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
        conn.execute("ALTER TABLE hosts ADD COLUMN favorite INTEGER DEFAULT 0", [])?;
    }
    if !columns.contains(&"last_connected_at".to_string()) {
        conn.execute("ALTER TABLE hosts ADD COLUMN last_connected_at DATETIME", [])?;
    }

    Ok(())
}

pub fn get_hosts(conn: &Connection) -> SqlResult<Vec<Host>> {
    let mut stmt = conn.prepare(
        "SELECT id, name, host, port, username, auth_type, key_path, \"group\", favorite, last_connected_at, created_at FROM hosts ORDER BY favorite DESC, name ASC"
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
        })
    })?;
    hosts.collect()
}

pub fn add_host(conn: &Connection, host: &NewHost) -> SqlResult<i64> {
    conn.execute(
        "INSERT INTO hosts (name, host, port, username, auth_type, key_path, \"group\", favorite) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            &host.name,
            &host.host,
            host.port,
            &host.username,
            &host.auth_type,
            host.key_path.as_deref().unwrap_or(""),
            host.group.as_deref().unwrap_or(""),
            host.favorite.unwrap_or(0)
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn update_host(conn: &Connection, id: i64, host: &NewHost) -> SqlResult<()> {
    conn.execute(
        "UPDATE hosts SET name = ?1, host = ?2, port = ?3, username = ?4, auth_type = ?5, key_path = ?6, \"group\" = ?7, favorite = ?8 WHERE id = ?9",
        params![
            &host.name,
            &host.host,
            host.port,
            &host.username,
            &host.auth_type,
            host.key_path.as_deref().unwrap_or(""),
            host.group.as_deref().unwrap_or(""),
            host.favorite.unwrap_or(0),
            id
        ],
    )?;
    Ok(())
}

pub fn delete_host(conn: &Connection, id: i64) -> SqlResult<()> {
    conn.execute("DELETE FROM hosts WHERE id = ?1", params![id])?;
    Ok(())
}

pub fn get_host_by_id(conn: &Connection, id: i64) -> SqlResult<Option<Host>> {
    let mut stmt = conn.prepare(
        "SELECT id, name, host, port, username, auth_type, key_path, \"group\", favorite, last_connected_at, created_at FROM hosts WHERE id = ?1"
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
}

pub fn export_hosts(conn: &Connection) -> SqlResult<Vec<ExportHost>> {
    let mut stmt = conn.prepare(
        "SELECT name, host, port, username, auth_type, key_path, \"group\", favorite FROM hosts ORDER BY name ASC"
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
        })
    })?;
    hosts.collect()
}

#[cfg(test)]
mod tests {
    use super::*;

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
