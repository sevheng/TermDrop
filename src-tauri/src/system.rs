use serde::{Deserialize, Serialize};
use ssh2::Session;
use std::io::Read;

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
                ports.push(NetPort {
                    proto,
                    state,
                    local,
                    process,
                });
            }
        }
    }

    // Established connections count
    let established = run_command(session, "ss -tn state established 2>/dev/null | wc -l");
    let established_count = established
        .unwrap_or_default()
        .trim()
        .parse::<i32>()
        .unwrap_or(0);

    // Interface stats from /proc/net/dev
    let dev_out = run_command(session, "cat /proc/net/dev 2>/dev/null | tail -n +3");
    let mut interfaces = Vec::new();
    if let Ok(out) = dev_out {
        for line in out.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            let Some((name, rest)) = trimmed.split_once(':') else {
                continue;
            };
            let name = name.trim().to_string();
            let nums: Vec<&str> = rest.split_whitespace().collect();
            if nums.len() >= 9 {
                if let (Ok(rx), Ok(tx)) = (nums[0].parse::<u64>(), nums[8].parse::<u64>()) {
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

    Ok(NetworkInfo {
        ports,
        interfaces,
        established_count,
    })
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SystemPanel {
    pub processes: Vec<Process>,
    pub network: NetworkInfo,
    pub disk: DiskInfo,
}

/// Batched system panel: processes + network + disk in a single SSH exec.
pub fn get_system_panel(session: &Session) -> Result<SystemPanel, String> {
    let output = run_command(
        session,
        r#"bash -c 'echo "---TERMDROP-PROCESSES---"; ps -eo pid,pcpu,pmem,etime,comm --sort=-pcpu | head -21; echo "---TERMDROP-NETWORK---"; ss -tlnp 2>/dev/null | tail -n +2 | head -30; echo "---TERMDROP-ESTABLISHED---"; ss -tn state established 2>/dev/null | wc -l; echo "---TERMDROP-INTERFACES---"; cat /proc/net/dev 2>/dev/null | tail -n +3; echo "---TERMDROP-DISK-MOUNTS---"; df -hP 2>/dev/null | tail -n +2; echo "---TERMDROP-DISK-DIRS---"; du -hd1 / 2>/dev/null | sort -rh | head -15'"#,
    )?;

    let mut processes = Vec::new();
    let mut network_ports = Vec::new();
    let mut network_interfaces = Vec::new();
    let mut established_count = 0;
    let mut disk_mounts = Vec::new();
    let mut disk_dirs = Vec::new();

    let mut section = "";
    for line in output.lines() {
        if line.starts_with("---TERMDROP-") && line.ends_with("---") {
            section = &line[12..line.len() - 3];
            continue;
        }
        match section {
            "PROCESSES" => {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 5 {
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
            "NETWORK" => {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 5 {
                    network_ports.push(NetPort {
                        proto: parts.get(0).unwrap_or(&"tcp").to_string(),
                        state: parts.get(1).unwrap_or(&"").to_string(),
                        local: parts.get(3).unwrap_or(&"").to_string(),
                        process: parts.get(parts.len() - 1).unwrap_or(&"").to_string(),
                    });
                }
            }
            "ESTABLISHED" => {
                established_count = line.trim().parse::<i32>().unwrap_or(0);
            }
            "INTERFACES" => {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }
                let Some((name, rest)) = trimmed.split_once(':') else {
                    continue;
                };
                let name = name.trim().to_string();
                let nums: Vec<&str> = rest.split_whitespace().collect();
                if nums.len() >= 9 {
                    if let (Ok(rx), Ok(tx)) = (nums[0].parse::<u64>(), nums[8].parse::<u64>()) {
                        network_interfaces.push(NetInterface {
                            name,
                            rx_bytes: rx,
                            tx_bytes: tx,
                            rx_rate: 0,
                            tx_rate: 0,
                        });
                    }
                }
            }
            "DISK-MOUNTS" => {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 6 {
                    disk_mounts.push(DiskMount {
                        filesystem: parts[0].to_string(),
                        size: parts[1].to_string(),
                        used: parts[2].to_string(),
                        available: parts[3].to_string(),
                        percent: parts[4].trim_end_matches('%').to_string(),
                        mount: parts[5].to_string(),
                    });
                }
            }
            "DISK-DIRS" => {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 2 {
                    disk_dirs.push(DiskDir {
                        size: parts[0].to_string(),
                        path: parts[1].to_string(),
                    });
                }
            }
            _ => {}
        }
    }

    Ok(SystemPanel {
        processes,
        network: NetworkInfo {
            ports: network_ports,
            interfaces: network_interfaces,
            established_count,
        },
        disk: DiskInfo {
            mounts: disk_mounts,
            dirs: disk_dirs,
        },
    })
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

pub fn get_system_stats(session: &Session) -> Result<serde_json::Value, String> {
    // Single batched command: all 9 stats in one SSH exec, tab-separated.
    // Tab is used as delimiter because none of these values contain tabs in practice.
    let output = run_command(
        session,
        r#"bash -c 'load=$(awk "{print \$1}" /proc/loadavg); ram=$(free -m | awk "NR==2{used=\$3;total=\$2;pct=used*100/total; if(total>=1024){printf \"%.1f/%.1fGB (%.0f%%)\", used/1024,total/1024,pct} else {printf \"%.0f/%.0fMB (%.0f%%)\", used,total,pct}}"); disk=$(df -h / | awk "NR==2{print \$3\"/\"\$2\" (\"\$5\")\"}"); uptime=$(awk "{d=int(\$1/86400);h=int((\$1%86400)/3600);m=int((\$1%3600)/60); printf \"%dd %dh %dm\", d,h,m}" /proc/uptime); os=$(grep "^PRETTY_NAME=" /etc/os-release 2>/dev/null | sed "s/PRETTY_NAME=//; s/\"//g"); kernel=$(uname -r); arch=$(uname -m); cores=$(nproc); netdev=$(cat /proc/net/dev | tail -n +3 | awk "{print \$1\" \"\$2\" \"\$10}"); printf "%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n" "$load" "$ram" "$disk" "$uptime" "$os" "$kernel" "$arch" "$cores" "$netdev"'"#,
    )?;

    let parts: Vec<&str> = output.trim_end().split('\t').collect();
    let get = |i: usize| -> String { parts.get(i).unwrap_or(&"").to_string() };

    Ok(serde_json::json!({
        "load": get(0),
        "ram": get(1),
        "disk": get(2),
        "uptime": get(3),
        "os": get(4),
        "kernel": get(5),
        "arch": get(6),
        "cores": get(7),
        "netdev": get(8),
    }))
}
