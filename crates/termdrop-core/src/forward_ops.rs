//! Saved port forwards (the Tunnels panel): the bodies of the Tauri commands
//! `get_port_forwards`, `add_port_forward`, `update_port_forward`,
//! `delete_port_forward`, `start_port_forward`, `stop_port_forward` and
//! `get_port_forward_status`, moved here unchanged so the native app shares
//! them. Error text is byte for byte what the commands returned.
//!
//! Everything is synchronous. `start_port_forward` reads the keyring and
//! binds a listener (each accepted connection then opens its own SSH session,
//! which checks the host key and fails closed), so run it off a UI thread.
//! Forwards keep running after the host's tabs close, as in the Tauri app:
//! `forget_host_state` does not touch them.

use crate::state::{db_err, resolve_target, with_db};
use crate::{db, Core};

impl Core {
    pub fn get_port_forwards(&self, host_id: i64) -> Result<Vec<db::PortForward>, String> {
        with_db(self, |conn| db::get_port_forwards(conn, host_id))
    }

    pub fn add_port_forward(&self, forward: &db::NewPortForward) -> Result<i64, String> {
        with_db(self, |conn| db::add_port_forward(conn, forward))
    }

    pub fn update_port_forward(&self, id: i64, forward: &db::NewPortForward) -> Result<(), String> {
        with_db(self, |conn| db::update_port_forward(conn, id, forward))
    }

    /// `update_port_forward` for the native app: a running forward is
    /// restarted on the new row, so the listener and target are the ones the
    /// panel shows. The old listener is joined before the new bind, so an
    /// edit that keeps the port does not race it. `Ok(Some(e))` is a row
    /// that saved but could not restart (its port is taken, say); the
    /// forward is then stopped. The Tauri command keeps the plain update.
    pub fn update_port_forward_restarting(
        &self,
        id: i64,
        forward: &db::NewPortForward,
    ) -> Result<Option<String>, String> {
        self.update_port_forward(id, forward)?;
        if !self.forward_manager.is_active(id) {
            return Ok(None);
        }
        self.forward_manager.stop_and_wait(id);
        Ok(self.start_port_forward(id).err())
    }

    /// `delete_host_row` for the native app: the host's port forwards go
    /// with its row (`ON DELETE CASCADE`), so the running ones are stopped
    /// too, or they would keep their ports and the deleted host's
    /// credentials with no panel left to stop them.
    pub fn delete_host_row_and_forwards(&self, host_id: i64) -> Result<(), String> {
        let forwards = self.get_port_forwards(host_id).unwrap_or_default();
        self.delete_host_row(host_id)?;
        for forward in forwards {
            self.forward_manager.stop(forward.id);
        }
        Ok(())
    }

    /// Stops the forward first, so deleting a running rule frees its port.
    pub fn delete_port_forward(&self, id: i64) -> Result<(), String> {
        self.forward_manager.stop(id);
        with_db(self, |conn| db::delete_port_forward(conn, id))
    }

    pub fn start_port_forward(&self, rule_id: i64) -> Result<(), String> {
        let (host, forward) = {
            let conn = self.db.get().map_err(db_err)?;
            let forward = db::get_port_forward_by_id(&conn, rule_id)
                .map_err(|e| e.to_string())?
                .ok_or("Port forward rule not found")?;
            let host = db::get_host_by_id(&conn, forward.host_id)
                .map_err(|e| e.to_string())?
                .ok_or("Host not found")?;
            (host, forward)
        };

        let target = resolve_target(self, &host, None)?;

        match forward.kind.as_str() {
            "local" => {
                let remote_host = forward.remote_host.ok_or("Remote host not set")?;
                let remote_port = forward.remote_port.ok_or("Remote port not set")? as u16;
                self.forward_manager.start_local(
                    rule_id,
                    target,
                    forward.local_host,
                    forward.local_port as u16,
                    remote_host,
                    remote_port,
                )?;
            }
            "dynamic" => {
                self.forward_manager.start_dynamic(
                    rule_id,
                    target,
                    forward.local_host,
                    forward.local_port as u16,
                )?;
            }
            _ => return Err(format!("Unsupported forward kind: {}", forward.kind)),
        }

        Ok(())
    }

    pub fn stop_port_forward(&self, rule_id: i64) {
        self.forward_manager.stop(rule_id);
    }

    pub fn port_forward_status(&self, rule_id: i64) -> bool {
        self.forward_manager.is_active(rule_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn core(name: &str) -> (Core, std::path::PathBuf) {
        let path = std::env::temp_dir().join(format!(
            "termdrop-forward-ops-{}-{}.db",
            name,
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        (Core::open(&path).unwrap(), path)
    }

    fn rule(host_id: i64, kind: &str) -> db::NewPortForward {
        db::NewPortForward {
            host_id,
            name: "Web".into(),
            kind: kind.into(),
            local_host: "127.0.0.1".into(),
            local_port: 18080,
            remote_host: (kind == "local").then(|| "localhost".into()),
            remote_port: (kind == "local").then_some(80),
        }
    }

    fn host(core: &Core) -> i64 {
        with_db(core, |conn| {
            conn.execute(
                "INSERT INTO hosts (name, host, username) VALUES ('t', 'h', 'u')",
                [],
            )?;
            Ok(conn.last_insert_rowid())
        })
        .unwrap()
    }

    #[test]
    fn saved_forwards_round_trip_per_host() {
        let (core, path) = core("crud");
        let (a, b) = (host(&core), host(&core));
        let id = core.add_port_forward(&rule(a, "local")).unwrap();
        core.add_port_forward(&rule(b, "dynamic")).unwrap();

        let list = core.get_port_forwards(a).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].id, id);
        assert_eq!(list[0].remote_host.as_deref(), Some("localhost"));
        assert!(!core.port_forward_status(id));

        core.update_port_forward(id, &rule(a, "dynamic")).unwrap();
        let list = core.get_port_forwards(a).unwrap();
        assert_eq!(list[0].kind, "dynamic");
        assert_eq!(list[0].remote_host, None);

        core.delete_port_forward(id).unwrap();
        assert!(core.get_port_forwards(a).unwrap().is_empty());
        assert_eq!(core.get_port_forwards(b).unwrap().len(), 1);
        let _ = std::fs::remove_file(path);
    }

    fn listen(core: &Core, id: i64) {
        core.forward_manager
            .start_dynamic(id, Default::default(), "127.0.0.1".into(), 0)
            .unwrap();
        assert!(core.port_forward_status(id));
    }

    #[test]
    fn deleting_a_host_stops_its_running_forwards() {
        let (core, path) = core("delete-host");
        let (a, b) = (host(&core), host(&core));
        let mine = core.add_port_forward(&rule(a, "dynamic")).unwrap();
        let other = core.add_port_forward(&rule(b, "dynamic")).unwrap();
        listen(&core, mine);
        listen(&core, other);
        core.delete_host_row_and_forwards(a).unwrap();
        assert!(!core.port_forward_status(mine));
        assert!(
            core.port_forward_status(other),
            "another host's forward runs on"
        );
        core.stop_port_forward(other);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn editing_a_running_forward_restarts_it() {
        let (core, path) = core("edit-running");
        let a = host(&core);
        let idle = core.add_port_forward(&rule(a, "local")).unwrap();
        // Not running: a plain update, nothing started.
        assert_eq!(
            core.update_port_forward_restarting(idle, &rule(a, "dynamic")),
            Ok(None)
        );
        assert!(!core.port_forward_status(idle));

        // Running: the old listener stops and the new row is started. This
        // rule's host row is gone, so the restart fails before any keyring
        // or network work, which shows the attempt was made on the new row.
        let orphan = with_db(&core, |conn| {
            conn.execute_batch("PRAGMA foreign_keys = OFF")?;
            let id = db::add_port_forward(conn, &rule(999_999, "local"))?;
            conn.execute_batch("PRAGMA foreign_keys = ON")?;
            Ok(id)
        })
        .unwrap();
        listen(&core, orphan);
        let mut edited = rule(999_999, "local");
        edited.local_port = 18081;
        assert_eq!(
            core.update_port_forward_restarting(orphan, &edited),
            Ok(Some("Host not found".to_string()))
        );
        assert!(!core.port_forward_status(orphan));
        assert_eq!(
            with_db(&core, |conn| db::get_port_forward_by_id(conn, orphan))
                .unwrap()
                .unwrap()
                .local_port,
            18081
        );
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn start_reports_the_tauri_errors() {
        let (core, path) = core("errors");
        assert_eq!(
            core.start_port_forward(42).unwrap_err(),
            "Port forward rule not found"
        );
        // A rule whose host row is gone (written with the foreign-key check
        // off, as an older database could hold).
        let id = with_db(&core, |conn| {
            conn.execute_batch("PRAGMA foreign_keys = OFF")?;
            let id = db::add_port_forward(conn, &rule(999_999, "local"))?;
            conn.execute_batch("PRAGMA foreign_keys = ON")?;
            Ok(id)
        })
        .unwrap();
        assert_eq!(core.start_port_forward(id).unwrap_err(), "Host not found");
        let _ = std::fs::remove_file(path);
    }
}
