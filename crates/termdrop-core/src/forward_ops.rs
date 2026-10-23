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
