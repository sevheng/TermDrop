use crate::ssh::exec::run_command;
use serde::{Deserialize, Serialize};
use ssh2::Session;

pub const DOCKER_NOT_INSTALLED: &str = "DOCKER_NOT_INSTALLED";
pub const DOCKER_PERMISSION_DENIED: &str = "DOCKER_PERMISSION_DENIED";
pub const DOCKER_DAEMON_NOT_RUNNING: &str = "DOCKER_DAEMON_NOT_RUNNING";

/// Map a failed `docker` invocation to one of the sentinels the UI knows how
/// to explain, or `None` to pass the original error through.
///
/// Both the permission-denied and daemon-down messages mention `dial unix`,
/// so matching on that alone told users to join the docker group when the
/// daemon was simply stopped. The distinguishing text is what follows
/// `connect:`, or the daemon's own "Is the docker daemon running?" hint.
fn classify_docker_error(err: &str) -> Option<&'static str> {
    let lower = err.to_lowercase();

    if lower.contains("command not found")
        || lower.contains("docker: not found")
        || lower.contains("executable file not found")
    {
        return Some(DOCKER_NOT_INSTALLED);
    }
    if lower.contains("permission denied") {
        return Some(DOCKER_PERMISSION_DENIED);
    }
    if lower.contains("is the docker daemon running")
        || lower.contains("cannot connect to the docker daemon")
        || lower.contains("connection refused")
    {
        return Some(DOCKER_DAEMON_NOT_RUNNING);
    }
    None
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Container {
    pub id: String,
    pub name: String,
    pub image: String,
    pub status: String,
    pub ports: String,
    pub created: String,
    pub state: String,
    pub running: bool,
}

fn run_docker_command(session: &Session, args: &str) -> Result<String, String> {
    let command = format!("docker {}", args);
    match run_command(session, &command) {
        Ok(output) => Ok(output),
        Err(e) => {
            if let Some(sentinel) = classify_docker_error(&e) {
                Err(sentinel.to_string())
            } else {
                Err(e)
            }
        }
    }
}

pub fn install_docker(session: &Session) -> Result<String, String> {
    let output = run_command(session, "curl -fsSL https://get.docker.com | sh")?;
    Ok(output.trim().to_string())
}

pub fn docker_ps(session: &Session, all: bool) -> Result<Vec<Container>, String> {
    let flag = if all { "-a" } else { "" };
    let format = "{{.ID}}|{{.Names}}|{{.Image}}|{{.Status}}|{{.Ports}}|{{.CreatedAt}}";
    // run_docker_command already maps a missing binary, a stopped daemon, and
    // a permissions problem to their sentinels.
    let output = run_docker_command(session, &format!("ps {} --format '{}'", flag, format))?;

    Ok(parse_docker_ps(&output))
}

/// Parse `docker ps --format '{{.ID}}|{{.Names}}|...'` output, one container per line.
fn parse_docker_ps(output: &str) -> Vec<Container> {
    let mut containers = Vec::new();
    for line in output.lines() {
        let parts: Vec<&str> = line.split('|').collect();
        if parts.len() >= 4 {
            let status = parts[3].to_string();
            let running = status.to_lowercase().starts_with("up");
            containers.push(Container {
                id: parts.first().unwrap_or(&"").to_string(),
                name: parts.get(1).unwrap_or(&"").to_string(),
                image: parts.get(2).unwrap_or(&"").to_string(),
                status: status.clone(),
                ports: parts.get(4).unwrap_or(&"").to_string(),
                created: parts.get(5).unwrap_or(&"").to_string(),
                state: status.clone(),
                running,
            });
        }
    }

    containers
}

pub fn docker_start(session: &Session, container_id: &str) -> Result<(), String> {
    run_docker_command(session, &format!("start {}", container_id))?;
    Ok(())
}

pub fn docker_stop(session: &Session, container_id: &str) -> Result<(), String> {
    run_docker_command(session, &format!("stop {}", container_id))?;
    Ok(())
}

pub fn docker_restart(session: &Session, container_id: &str) -> Result<(), String> {
    run_docker_command(session, &format!("restart {}", container_id))?;
    Ok(())
}

pub fn docker_inspect_shell(session: &Session, container_id: &str) -> Result<String, String> {
    // Use docker inspect to check the container's configured shell/cmd
    // Much faster than docker exec which spawns a process inside the container
    let out = run_docker_command(
        session,
        &format!("inspect --format='{{{{.Config.Cmd}}}}' {}", container_id),
    )?;
    let out_lower = out.to_lowercase();
    if out_lower.contains("bash") || out_lower.contains("/bin/bash") {
        Ok("bash".to_string())
    } else {
        Ok("sh".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PS: &str = include_str!("fixtures/docker_ps.txt");

    #[test]
    fn parses_ps_lines_and_running_flag() {
        let c = parse_docker_ps(PS);
        assert_eq!(c.len(), 3);

        assert_eq!(c[0].id, "abc123");
        assert_eq!(c[0].name, "web");
        assert_eq!(c[0].image, "nginx:latest");
        assert_eq!(c[0].status, "Up 3 hours");
        assert_eq!(c[0].state, "Up 3 hours");
        assert_eq!(c[0].ports, "0.0.0.0:80->80/tcp");
        assert_eq!(c[0].created, "2026-06-01 10:00:00 +0000 UTC");
        assert!(c[0].running);

        assert_eq!(c[1].name, "db");
        assert_eq!(c[1].ports, "");
        assert!(!c[1].running);

        // Four fields is enough; missing ports/created become empty, "up" is case-insensitive.
        assert_eq!(c[2].name, "worker");
        assert_eq!(c[2].ports, "");
        assert_eq!(c[2].created, "");
        assert!(c[2].running);
    }

    #[test]
    fn classifies_the_three_docker_failure_modes() {
        // was: "dial unix" matched both, so a stopped daemon told the user to
        // join the docker group.
        assert_eq!(
            classify_docker_error(
                "Cannot connect to the Docker daemon at unix:///var/run/docker.sock. \
                 Is the docker daemon running?"
            ),
            Some(DOCKER_DAEMON_NOT_RUNNING)
        );
        assert_eq!(
            classify_docker_error("dial unix /var/run/docker.sock: connect: connection refused"),
            Some(DOCKER_DAEMON_NOT_RUNNING)
        );
        assert_eq!(
            classify_docker_error(
                "permission denied while trying to connect to the Docker daemon socket at \
                 unix:///var/run/docker.sock: dial unix /var/run/docker.sock: connect: permission denied"
            ),
            Some(DOCKER_PERMISSION_DENIED)
        );
        assert_eq!(
            classify_docker_error("bash: docker: command not found"),
            Some(DOCKER_NOT_INSTALLED)
        );
        assert_eq!(
            classify_docker_error("sh: 1: docker: not found"),
            Some(DOCKER_NOT_INSTALLED)
        );
    }

    #[test]
    fn unrelated_errors_pass_through_unchanged() {
        // was: any stderr containing "not found" became DOCKER_NOT_INSTALLED,
        // so a missing container was reported as a missing Docker install.
        assert_eq!(
            classify_docker_error("Error response from daemon: No such container: web"),
            None
        );
        assert_eq!(classify_docker_error("network mynet not found"), None);
        assert_eq!(classify_docker_error(""), None);
    }

    #[test]
    fn skips_short_lines_and_empty_output() {
        assert!(parse_docker_ps("").is_empty());
        assert!(parse_docker_ps("a|b|c\n").is_empty());
    }
}
