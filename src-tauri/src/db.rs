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
    Ok(())
}

pub fn get_hosts(conn: &Connection) -> SqlResult<Vec<Host>> {
    let mut stmt = conn.prepare(
        "SELECT id, name, host, port, username, auth_type, key_path, created_at FROM hosts ORDER BY created_at DESC"
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
            created_at: row.get(7)?,
        })
    })?;
    hosts.collect()
}

pub fn add_host(conn: &Connection, host: &NewHost) -> SqlResult<i64> {
    conn.execute(
        "INSERT INTO hosts (name, host, port, username, auth_type, key_path) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            &host.name,
            &host.host,
            host.port,
            &host.username,
            &host.auth_type,
            host.key_path.as_deref().unwrap_or("")
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn update_host(conn: &Connection, id: i64, host: &NewHost) -> SqlResult<()> {
    conn.execute(
        "UPDATE hosts SET name = ?1, host = ?2, port = ?3, username = ?4, auth_type = ?5, key_path = ?6 WHERE id = ?7",
        params![
            &host.name,
            &host.host,
            host.port,
            &host.username,
            &host.auth_type,
            host.key_path.as_deref().unwrap_or(""),
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
        "SELECT id, name, host, port, username, auth_type, key_path, created_at FROM hosts WHERE id = ?1"
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
            created_at: row.get(7)?,
        }))
    } else {
        Ok(None)
    }
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
