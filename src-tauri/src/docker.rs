use serde::{Deserialize, Serialize};
use ssh2::Session;
use std::io::Read;
use std::time::Instant;

pub const DOCKER_NOT_INSTALLED: &str = "DOCKER_NOT_INSTALLED";
pub const DOCKER_PERMISSION_DENIED: &str = "DOCKER_PERMISSION_DENIED";

#[derive(Debug, Clone)]
pub struct CachedDockerInfo {
    pub containers: Vec<Container>,
    pub cached_at: Instant,
}

fn is_permission_error(err: &str) -> bool {
    err.to_lowercase().contains("permission denied")
        || err.to_lowercase().contains("connect: permission denied")
        || err.to_lowercase().contains("dial unix")
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

fn run_command(session: &Session, command: &str) -> Result<String, String> {
    let mut channel = session
        .channel_session()
        .map_err(|e| format!("channel: {}", e))?;
    channel.exec(command).map_err(|e| format!("exec: {}", e))?;

    let mut stdout = String::new();
    channel
        .read_to_string(&mut stdout)
        .map_err(|e| format!("read: {}", e))?;

    let mut stderr = String::new();
    channel
        .stderr()
        .read_to_string(&mut stderr)
        .map_err(|e| format!("read stderr: {}", e))?;

    channel.wait_close().ok();

    let exit_status = channel.exit_status().unwrap_or(0);
    if exit_status != 0 {
        let err = if stderr.trim().is_empty() {
            stdout.trim().to_string()
        } else {
            stderr.trim().to_string()
        };
        if err.is_empty() {
            return Err(format!("command failed with exit code {}", exit_status));
        }
        return Err(err);
    }

    Ok(stdout)
}

fn run_docker_command(session: &Session, args: &str) -> Result<String, String> {
    let command = format!("docker {}", args);
    match run_command(session, &command) {
        Ok(output) => Ok(output),
        Err(e) => {
            if is_permission_error(&e) {
                Err(DOCKER_PERMISSION_DENIED.to_string())
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
    let output = match run_docker_command(session, &format!("ps {} --format '{}'", flag, format)) {
        Ok(out) => out,
        Err(e) => {
            if e.contains("not found")
                || e.contains("No such file")
                || e.contains("command not found")
            {
                return Err(DOCKER_NOT_INSTALLED.to_string());
            }
            return Err(e);
        }
    };

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
                id: parts.get(0).unwrap_or(&"").to_string(),
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
    fn skips_short_lines_and_empty_output() {
        assert!(parse_docker_ps("").is_empty());
        assert!(parse_docker_ps("a|b|c\n").is_empty());
    }
}
