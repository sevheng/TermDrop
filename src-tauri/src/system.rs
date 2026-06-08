use std::io::Read;
use ssh2::Session;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Process {
    pub pid: String,
    pub command: String,
    pub cpu: String,
    pub mem: String,
    pub uptime: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct NetInterface {
    pub name: String,
    pub rx_bytes: u64,
    pub tx_bytes: u64,
    pub rx_rate: u64,
    pub tx_rate: u64,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct NetPort {
    pub proto: String,
    pub local: String,
    pub state: String,
    pub process: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct NetworkInfo {
    pub ports: Vec<NetPort>,
    pub interfaces: Vec<NetInterface>,
    pub established_count: i32,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DiskMount {
    pub filesystem: String,
    pub size: String,
    pub used: String,
    pub available: String,
    pub percent: String,
    pub mount: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DiskDir {
    pub path: String,
    pub size: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DiskInfo {
    pub mounts: Vec<DiskMount>,
    pub dirs: Vec<DiskDir>,
}

pub fn run_command(session: &Session, command: &str) -> Result<String, String> {
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

pub fn get_processes(session: &Session) -> Result<Vec<Process>, String> {
    let output = run_command(
        session,
        "ps -eo pid,pcpu,pmem,etime,comm --sort=-pcpu | head -21",
    )?;

    let mut processes = Vec::new();
    for line in output.lines().skip(1) {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() >= 5 {
            // comm is last and may contain spaces — join everything after the fixed columns
            let command = parts[4..].join(" ");
            processes.push(Process {
                pid: parts[0].to_string(),
                cpu: parts[1].to_string(),
                mem: parts[2].to_string(),
                uptime: parts[3].to_string(),
                command,
            });
        }
    }
    Ok(processes)
}

pub fn get_network(session: &Session) -> Result<NetworkInfo, String> {
    // Listening ports
    let ports_out = run_command(session, "ss -tlnp 2>/dev/null | tail -n +2 | head -30");
    let mut ports = Vec::new();
    if let Ok(out) = ports_out {
        for line in out.lines() {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 5 {
                let proto = parts.get(0).unwrap_or(&"tcp").to_string();
                let state = parts.get(1).unwrap_or(&"").to_string();
                let local = parts.get(3).unwrap_or(&"").to_string();
                let process = parts.get(parts.len() - 1).unwrap_or(&"").to_string();
                ports.push(NetPort { proto, state, local, process });
            }
        }
    }

    // Established connections count
    let established = run_command(session, "ss -tn state established 2>/dev/null | wc -l");
    let established_count = established.unwrap_or_default().trim().parse::<i32>().unwrap_or(0);

    // Interface stats from /proc/net/dev
    let dev_out = run_command(session, "cat /proc/net/dev 2>/dev/null | tail -n +3");
    let mut interfaces = Vec::new();
    if let Ok(out) = dev_out {
        for line in out.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() { continue; }
            let Some((name, rest)) = trimmed.split_once(':') else { continue; };
            let name = name.trim().to_string();
            let nums: Vec<&str> = rest.split_whitespace().collect();
            if nums.len() >= 9 {
                if let (Ok(rx), Ok(tx)) = (
                    nums[0].parse::<u64>(),
                    nums[8].parse::<u64>(),
                ) {
                    interfaces.push(NetInterface {
                        name,
                        rx_bytes: rx,
                        tx_bytes: tx,
                        rx_rate: 0,
                        tx_rate: 0,
                    });
                }
            }
        }
    }

    Ok(NetworkInfo { ports, interfaces, established_count })
}

pub fn get_disk_usage(session: &Session) -> Result<DiskInfo, String> {
    // Filesystems
    let df_out = run_command(session, "df -hP 2>/dev/null | tail -n +2");
    let mut mounts = Vec::new();
    if let Ok(out) = df_out {
        for line in out.lines() {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 6 {
                mounts.push(DiskMount {
                    filesystem: parts[0].to_string(),
                    size: parts[1].to_string(),
                    used: parts[2].to_string(),
                    available: parts[3].to_string(),
                    percent: parts[4].trim_end_matches('%').to_string(),
                    mount: parts[5].to_string(),
                });
            }
        }
    }

    // Top-level directory usage
    let du_out = run_command(session, "du -hd1 / 2>/dev/null | sort -rh | head -15");
    let mut dirs = Vec::new();
    if let Ok(out) = du_out {
        for line in out.lines() {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 2 {
                dirs.push(DiskDir {
                    size: parts[0].to_string(),
                    path: parts[1].to_string(),
                });
            }
        }
    }

    Ok(DiskInfo { mounts, dirs })
}
