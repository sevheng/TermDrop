//! What the Docker, Security and system panels ask of a host: commands run on
//! the host's shared exec session (`Core::exec_sessions`), which exists while
//! at least one terminal tab is open on the host.
//!
//! These were the bodies of the Tauri commands of the same names; error text
//! is unchanged. Every call goes through [`with_exec_session`] or
//! [`cached_per_host`], which hold the session's mutex for the whole command:
//! never run two commands concurrently on one `ssh2::Session` (parallel audit
//! checks corrupted it).
//!
//! `docker_ps` and `run_security_audit` are cached per host. A stale value is
//! returned at once while a background refresh runs, which uses
//! `tokio::spawn`: call these from inside a tokio runtime.

use crate::state::{cached_per_host, invalidate_docker_cache, with_exec_session};
use crate::{docker, security, system, Core};

impl Core {
    /// `docker ps`, cached per host: fresh for 5s, served stale (and
    /// refreshed behind) up to 15s.
    pub async fn docker_ps(
        &self,
        host_id: i64,
        all: bool,
    ) -> Result<Vec<docker::Container>, String> {
        cached_per_host(
            self,
            &self.docker_cache,
            &self.docker_ps_fetching,
            host_id,
            5,
            15,
            false,
            move |session| docker::docker_ps(session, all),
        )
        .await
    }

    pub async fn docker_start(&self, host_id: i64, container_id: String) -> Result<(), String> {
        invalidate_docker_cache(self, host_id)?;
        with_exec_session(self, host_id, 60, move |session| {
            docker::docker_start(session, &container_id)
        })
        .await
    }

    pub async fn docker_stop(&self, host_id: i64, container_id: String) -> Result<(), String> {
        invalidate_docker_cache(self, host_id)?;
        with_exec_session(self, host_id, 60, move |session| {
            docker::docker_stop(session, &container_id)
        })
        .await
    }

    pub async fn docker_restart(&self, host_id: i64, container_id: String) -> Result<(), String> {
        invalidate_docker_cache(self, host_id)?;
        with_exec_session(self, host_id, 60, move |session| {
            docker::docker_restart(session, &container_id)
        })
        .await
    }

    /// The shell to `docker exec` into a container with: `bash` when the
    /// container's configured command mentions bash, else `sh`.
    /// `container_id` is put into the remote command as is, so pass only ids
    /// that came from [`Self::docker_ps`] (as for start/stop/restart).
    pub async fn docker_inspect_shell(
        &self,
        host_id: i64,
        container_id: String,
    ) -> Result<String, String> {
        with_exec_session(self, host_id, 60, move |session| {
            docker::docker_inspect_shell(session, &container_id)
        })
        .await
    }

    pub async fn docker_install(&self, host_id: i64) -> Result<String, String> {
        invalidate_docker_cache(self, host_id)?;
        with_exec_session(self, host_id, 60, move |session| {
            docker::install_docker(session)
        })
        .await
    }

    /// The security audit, cached per host: fresh for 30s, served stale up to
    /// 300s. `force` skips the cache. Run lazily: only when the user opens
    /// the panel.
    pub async fn run_security_audit(
        &self,
        host_id: i64,
        force: bool,
    ) -> Result<security::SecurityReport, String> {
        cached_per_host(
            self,
            &self.security_report_cache,
            &self.security_report_fetching,
            host_id,
            30,
            300,
            force,
            security::run_security_audit,
        )
        .await
    }

    /// CPU, memory, disk and uptime for the status bar, as JSON.
    pub async fn get_system_stats(&self, host_id: i64) -> Result<serde_json::Value, String> {
        with_exec_session(self, host_id, 60, move |session| {
            system::get_system_stats(session)
        })
        .await
    }

    /// Processes, network and disks for the system panel.
    pub async fn get_system_panel(&self, host_id: i64) -> Result<system::SystemPanel, String> {
        with_exec_session(self, host_id, 60, move |session| {
            system::get_system_panel(session)
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use crate::state::tests_support::memory_core;

    const NO_SESSION: &str = "No active session for this host";

    #[test]
    fn every_call_needs_a_terminal_on_the_host() {
        let core = memory_core();
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            assert_eq!(core.docker_ps(7, true).await.unwrap_err(), NO_SESSION);
            assert_eq!(
                core.docker_start(7, "c".into()).await.unwrap_err(),
                NO_SESSION
            );
            assert_eq!(
                core.docker_stop(7, "c".into()).await.unwrap_err(),
                NO_SESSION
            );
            assert_eq!(
                core.docker_restart(7, "c".into()).await.unwrap_err(),
                NO_SESSION
            );
            assert_eq!(
                core.docker_inspect_shell(7, "c".into()).await.unwrap_err(),
                NO_SESSION
            );
            assert_eq!(core.docker_install(7).await.unwrap_err(), NO_SESSION);
            assert_eq!(
                core.run_security_audit(7, false).await.unwrap_err(),
                NO_SESSION
            );
            assert_eq!(core.get_system_stats(7).await.unwrap_err(), NO_SESSION);
            assert_eq!(core.get_system_panel(7).await.unwrap_err(), NO_SESSION);
        });
    }

    #[test]
    fn docker_actions_drop_the_hosts_cached_listing() {
        let core = memory_core();
        core.docker_cache.lock().unwrap().insert(
            7,
            crate::Cached {
                value: Vec::new(),
                cached_at: std::time::Instant::now(),
            },
        );
        let rt = tokio::runtime::Runtime::new().unwrap();
        let _ = rt.block_on(core.docker_stop(7, "c".into()));
        assert!(core.docker_cache.lock().unwrap().get(&7).is_none());
    }

    #[test]
    fn a_fresh_cached_listing_is_served_without_a_session() {
        let core = memory_core();
        core.docker_cache.lock().unwrap().insert(
            7,
            crate::Cached {
                value: Vec::new(),
                cached_at: std::time::Instant::now(),
            },
        );
        let rt = tokio::runtime::Runtime::new().unwrap();
        assert!(rt.block_on(core.docker_ps(7, false)).unwrap().is_empty());
    }
}
