use std::io::Read;
use ssh2::Session;
use serde::{Deserialize, Serialize};

pub const DOCKER_NOT_INSTALLED: &str = "DOCKER_NOT_INSTALLED";

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
    let mut channel = session.channel_session()
        .map_err(|e| format!("channel: {}", e))?;
    channel.exec(command)
        .map_err(|e| format!("exec: {}", e))?;

    let mut stdout = String::new();
    channel.read_to_string(&mut stdout)
        .map_err(|e| format!("read: {}", e))?;

    let mut stderr = String::new();
    channel.stderr().read_to_string(&mut stderr)
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
    let output = run_command(session, &command)?;
    Ok(output)
}

pub fn is_docker_installed(session: &Session) -> Result<bool, String> {
    match run_command(session, "command -v docker") {
        Ok(out) => Ok(!out.trim().is_empty()),
        Err(e) => {
            if e.contains("not found") || e.contains("No such file") {
                Ok(false)
            } else {
                Err(e)
            }
        }
    }
}

pub fn install_docker(session: &Session) -> Result<String, String> {
    let output = run_command(
        session,
        "curl -fsSL https://get.docker.com | sh",
    )?;
    Ok(output.trim().to_string())
}

pub fn docker_ps(session: &Session, all: bool) -> Result<Vec<Container>, String> {
    if !is_docker_installed(session)? {
        return Err(DOCKER_NOT_INSTALLED.to_string());
    }
    let flag = if all { "-a" } else { "" };
    let format = "{{.ID}}|{{.Names}}|{{.Image}}|{{.Status}}|{{.Ports}}|{{.CreatedAt}}";
    let output = run_docker_command(
        session,
        &format!("ps {} --format '{}'", flag, format),
    )?;

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

    Ok(containers)
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

pub fn docker_logs(session: &Session, container_id: &str, tail: usize) -> Result<String, String> {
    run_docker_command(session, &format!("logs --tail {} {}", tail, container_id))
}

pub fn docker_exec(session: &Session, container_id: &str, command: &str) -> Result<String, String> {
    run_docker_command(session, &format!("exec {} {}", container_id, command))
}

pub fn docker_inspect_shell(session: &Session, container_id: &str) -> Result<String, String> {
    // Try bash first, fallback to sh
    let check = run_docker_command(session, &format!("exec {} bash -c 'echo bash'", container_id));
    if check.is_ok() && check.unwrap().trim() == "bash" {
        Ok("bash".to_string())
    } else {
        Ok("sh".to_string())
    }
}
