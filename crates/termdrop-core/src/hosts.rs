//! Saving hosts: what the host editors, the sidebar's group and favourite
//! actions, and import/export do to the `hosts` table and the keyring.
//!
//! These were the bodies of the Tauri commands of the same names; they live
//! here so the native app runs exactly the same steps (which caches to drop
//! when an address changes, which keyring entries a deleted row leaves
//! behind) rather than a copy that drifts. Error text is unchanged, because
//! the Vue frontend shows it as is.
//!
//! The secret helpers are free functions: they touch the keyring only, never
//! the core's state. Each one only ever *sets* a secret when given a
//! non-empty one, so an empty password field in an edit dialog can never
//! silently delete what is stored; clearing is always its own call.

use crate::state::{
    db_err, forget_mongo_clients, forget_redis, mongo_account, redis_account, release_vnc_host,
    vnc_account, with_db,
};
use crate::{crypto, db, Core};

/// Every keyring entry stored under a host's id, removed once its row is.
pub fn delete_host_secrets(id: i64) {
    crypto::delete_password(id).ok();
    crypto::delete_secret(&mongo_account(id)).ok();
    // Both of these were orphaned: the row went, the keyring entry stayed.
    crypto::delete_secret(&redis_account(id)).ok();
    crypto::delete_secret(&vnc_account(id)).ok();
}

impl Core {
    pub fn add_host(&self, host: &db::NewHost) -> Result<i64, String> {
        with_db(self, |conn| db::add_host(conn, host))
    }

    /// Rewrite every column of a host, then drop whatever was connected to
    /// its old address.
    ///
    /// A row that is gone updates nothing and still returns `Ok`: that is what
    /// the Tauri command has always done. Use [`Self::update_existing_host`]
    /// where a vanished row must be an error.
    pub async fn update_host(&self, id: i64, host: &db::NewHost) -> Result<(), String> {
        self.update_host_rows(id, host).await.map(|_| ())
    }

    /// As [`Self::update_host`], but a row deleted in the meantime (by the
    /// other app on the same database) is reported as `host N not found`
    /// rather than a silent success, so a caller does not go on to store
    /// secrets under an id no row owns.
    pub async fn update_existing_host(&self, id: i64, host: &db::NewHost) -> Result<(), String> {
        match self.update_host_rows(id, host).await? {
            0 => Err(format!("host {id} not found")),
            _ => Ok(()),
        }
    }

    async fn update_host_rows(&self, id: i64, host: &db::NewHost) -> Result<usize, String> {
        let rows = self.write_host_row(id, host)?;
        self.forget_host_connections(id).await?;
        Ok(rows)
    }

    /// The row half of an update: rewrite the columns and return how many
    /// rows changed (0 when the host is gone). No connection is touched, so
    /// this never waits; follow it with [`Self::forget_host_connections`].
    pub fn write_host_row(&self, id: i64, host: &db::NewHost) -> Result<usize, String> {
        with_db(self, |conn| db::update_host(conn, id, host))
    }

    /// Drop everything connected to a host's old address. This takes the
    /// pool locks, which a connect in flight holds across its network work
    /// (a tunnel handshake, a Redis connect), so it can wait for seconds:
    /// a UI thread should run it in the background rather than block on it.
    pub async fn forget_host_connections(&self, id: i64) -> Result<(), String> {
        // The address may have changed, and every one of these caches the old one.
        // Redis was missing here: an edited URI stayed invisible until the app was
        // restarted, and the stale tunnel stayed open behind it.
        forget_mongo_clients(self, id).await;
        forget_redis(self, id).await;
        self.vnc_disconnect_host(id).await
    }

    /// Delete a host and every secret stored under its id.
    pub async fn delete_host(&self, id: i64) -> Result<(), String> {
        self.delete_host_row(id)?;
        self.forget_host_connections(id).await?;
        delete_host_secrets(id);
        Ok(())
    }

    /// The row half of a delete; see [`Self::write_host_row`].
    pub fn delete_host_row(&self, id: i64) -> Result<(), String> {
        with_db(self, |conn| db::delete_host(conn, id))
    }

    /// End every VNC session on a host and release its tunnel.
    pub async fn vnc_disconnect_host(&self, host_id: i64) -> Result<(), String> {
        {
            let mut sessions = self
                .vnc_sessions
                .lock()
                .map_err(|_| "session registry poisoned".to_string())?;
            sessions.retain(|_, handle| {
                if handle.host_id == host_id {
                    handle.disconnect();
                    false
                } else {
                    true
                }
            });
        }
        release_vnc_host(self, host_id).await;
        Ok(())
    }

    pub fn update_host_group(&self, id: i64, group: &str) -> Result<(), String> {
        with_db(self, |conn| db::update_host_group(conn, id, group))
    }

    /// Rename a group: every host in `old_group` moves to `new_group`.
    /// Returns how many moved.
    pub fn batch_update_host_group(
        &self,
        old_group: &str,
        new_group: &str,
    ) -> Result<usize, String> {
        with_db(self, |conn| {
            db::update_hosts_group_by_name(conn, old_group, new_group)
        })
    }

    /// Delete a group: its hosts become ungrouped. Returns how many moved.
    pub fn batch_clear_host_group(&self, group: &str) -> Result<usize, String> {
        with_db(self, |conn| db::clear_hosts_group_by_name(conn, group))
    }

    pub fn update_host_favorite(&self, id: i64, favorite: i64) -> Result<(), String> {
        with_db(self, |conn| db::update_host_favorite(conn, id, favorite))
    }

    pub fn update_host_last_connected(&self, id: i64) -> Result<(), String> {
        with_db(self, |conn| db::update_host_last_connected(conn, id))
    }

    /// Every host as the JSON array an export file holds. Never a secret:
    /// passwords live in the keyring, and stored URIs carry none.
    pub fn export_hosts(&self) -> Result<String, String> {
        let hosts = with_db(self, db::export_hosts)?;
        serde_json::to_string(&hosts).map_err(|e| e.to_string())
    }

    /// Import hosts from either source. Rows that fail validation or insertion
    /// are reported individually rather than aborting the import, so the caller
    /// can tell the user exactly what landed and what did not.
    pub fn import_hosts(&self, entries: Vec<db::ImportEntry>) -> Result<db::ImportSummary, String> {
        let mut conn = self.db.get().map_err(db_err)?;
        // One transaction instead of one fsync per row.
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        let summary = db::import_entries(&tx, entries);
        tx.commit().map_err(|e| e.to_string())?;
        Ok(summary)
    }
}

/// Store an SSH host's password.
pub fn store_password(host_id: i64, password: &str) -> Result<(), String> {
    crypto::store_password(host_id, password)
}

/// Store a MongoDB host's password. Refuses an empty one.
pub fn store_mongo_secret(host_id: i64, password: &str) -> Result<(), String> {
    if password.is_empty() {
        return Err("refusing to store an empty MongoDB password".to_string());
    }
    crypto::store_secret(&mongo_account(host_id), password)
}

/// Whether a password is stored, so the UI can say so without revealing it.
pub fn has_mongo_secret(host_id: i64) -> bool {
    crypto::get_secret(&mongo_account(host_id)).is_ok()
}

pub fn clear_mongo_secret(host_id: i64) -> Result<(), String> {
    crypto::delete_secret(&mongo_account(host_id))
}

/// Store a Redis host's password. Refuses an empty one.
pub fn store_redis_secret(host_id: i64, password: &str) -> Result<(), String> {
    if password.is_empty() {
        return Err("refusing to store an empty Redis password".to_string());
    }
    crypto::store_secret(&redis_account(host_id), password)
}

pub fn has_redis_secret(host_id: i64) -> bool {
    crypto::get_secret(&redis_account(host_id)).is_ok()
}

pub fn clear_redis_secret(host_id: i64) -> Result<(), String> {
    crypto::delete_secret(&redis_account(host_id))
}

/// Store a VNC password. Refuses an empty one.
pub fn store_vnc_secret(host_id: i64, password: &str) -> Result<(), String> {
    if password.is_empty() {
        return Err("password is empty".to_string());
    }
    crypto::store_secret(&vnc_account(host_id), password)
}

pub fn has_vnc_secret(host_id: i64) -> bool {
    crypto::get_secret(&vnc_account(host_id)).is_ok()
}

pub fn clear_vnc_secret(host_id: i64) -> Result<(), String> {
    crypto::delete_secret(&vnc_account(host_id))
}

#[cfg(test)]
mod tests {
    use super::*;
    use r2d2::Pool;
    use r2d2_sqlite::SqliteConnectionManager;

    /// A core over one in-memory database. One connection, because each
    /// in-memory connection is its own database.
    fn core() -> Core {
        let pool = Pool::builder()
            .max_size(1)
            .build(SqliteConnectionManager::memory())
            .unwrap();
        crate::state::prepare_database(&pool.get().unwrap()).unwrap();
        Core::new(pool)
    }

    fn ssh_host(name: &str, group: Option<&str>) -> db::NewHost {
        db::NewHost {
            name: name.into(),
            host: "10.0.0.1".into(),
            port: 22,
            username: "me".into(),
            auth_type: "password".into(),
            key_path: None,
            group: group.map(String::from),
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

    fn rt() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
    }

    #[test]
    fn add_update_and_group_operations_round_trip() {
        let core = core();
        let a = core.add_host(&ssh_host("a", Some("Lab"))).unwrap();
        let b = core.add_host(&ssh_host("b", Some("Lab"))).unwrap();
        rt().block_on(core.update_host(a, &ssh_host("a2", Some("Lab"))))
            .unwrap();
        assert_eq!(core.get_host(a).unwrap().unwrap().name, "a2");

        assert_eq!(core.batch_update_host_group("Lab", "Prod").unwrap(), 2);
        core.update_host_group(b, "Other").unwrap();
        assert_eq!(core.batch_clear_host_group("Prod").unwrap(), 1);
        assert_eq!(
            core.get_host(a).unwrap().unwrap().group.as_deref(),
            Some("")
        );
        assert_eq!(
            core.get_host(b).unwrap().unwrap().group.as_deref(),
            Some("Other")
        );

        core.update_host_favorite(b, 1).unwrap();
        assert_eq!(core.get_host(b).unwrap().unwrap().favorite, 1);
    }

    #[test]
    fn updating_a_vanished_host_is_an_error_only_where_asked() {
        let core = core();
        let a = core.add_host(&ssh_host("a", None)).unwrap();
        // Straight to the table: `Core::delete_host` would also clear the
        // keyring entries for this id, which on a test database belong to
        // whichever real host has the same id.
        with_db(&core, |conn| db::delete_host(conn, a)).unwrap();
        // The Tauri command's contract: silently nothing.
        rt().block_on(core.update_host(a, &ssh_host("a2", None)))
            .unwrap();
        assert_eq!(
            rt().block_on(core.update_existing_host(a, &ssh_host("a2", None))),
            Err(format!("host {a} not found"))
        );
        let b = core.add_host(&ssh_host("b", None)).unwrap();
        rt().block_on(core.update_existing_host(b, &ssh_host("b2", None)))
            .unwrap();
        assert_eq!(core.get_host(b).unwrap().unwrap().name, "b2");
    }

    #[test]
    fn export_then_import_recreates_the_hosts() {
        let core = core();
        core.add_host(&ssh_host("a", Some("Lab"))).unwrap();
        let json = core.export_hosts().unwrap();
        assert!(json.starts_with('['), "an export is a bare array: {json}");

        let entries = vec![
            db::ImportEntry {
                host: ssh_host("b", None),
                replace_id: None,
            },
            db::ImportEntry {
                host: db::NewHost {
                    host: String::new(),
                    ..ssh_host("broken", None)
                },
                replace_id: None,
            },
        ];
        let summary = core.import_hosts(entries).unwrap();
        assert_eq!(summary.added, 1);
        assert_eq!(summary.failed.len(), 1);
        assert_eq!(summary.failed[0].reason, "Host is required");
        assert_eq!(core.get_hosts().unwrap().len(), 2);
    }

    #[test]
    fn empty_secrets_are_refused_before_the_keyring_is_touched() {
        // These return before reaching the keyring, so they are safe to run
        // anywhere; the wording is what the Vue dialogs show.
        assert_eq!(
            store_mongo_secret(1, "").unwrap_err(),
            "refusing to store an empty MongoDB password"
        );
        assert_eq!(
            store_redis_secret(1, "").unwrap_err(),
            "refusing to store an empty Redis password"
        );
        assert_eq!(store_vnc_secret(1, "").unwrap_err(), "password is empty");
    }

    #[test]
    fn disconnecting_a_host_without_sessions_is_a_no_op() {
        let core = core();
        rt().block_on(core.vnc_disconnect_host(42)).unwrap();
    }
}
