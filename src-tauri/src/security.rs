use std::io::Read;
use ssh2::Session;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SecurityCheck {
    pub name: String,
    pub status: String,
    pub message: String,
    pub detail: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SecurityReport {
    pub score: u8,
    pub checks: Vec<SecurityCheck>,
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

fn check_ssh_password_auth(session: &Session) -> SecurityCheck {
    let output = run_command(session, "grep -E '^PasswordAuthentication' /etc/ssh/sshd_config 2>/dev/null || echo 'NOTSET'");
    let val = output.unwrap_or_default().trim().to_lowercase();

    if val.contains("no") {
        SecurityCheck {
            name: "SSH Password Authentication".to_string(),
            status: "pass".to_string(),
            message: "Password authentication is disabled".to_string(),
            detail: Some(val),
        }
    } else if val == "notset" || val.is_empty() {
        SecurityCheck {
            name: "SSH Password Authentication".to_string(),
            status: "warn".to_string(),
            message: "Using default (check /etc/ssh/sshd_config)".to_string(),
            detail: Some("PasswordAuthentication not explicitly set".to_string()),
        }
    } else {
        SecurityCheck {
            name: "SSH Password Authentication".to_string(),
            status: "fail".to_string(),
            message: "Password authentication is enabled".to_string(),
            detail: Some(val),
        }
    }
}

fn check_ssh_root_login(session: &Session) -> SecurityCheck {
    let output = run_command(session, "grep -E '^PermitRootLogin' /etc/ssh/sshd_config 2>/dev/null || echo 'NOTSET'");
    let val = output.unwrap_or_default().trim().to_lowercase();

    if val.contains("no") {
        SecurityCheck {
            name: "SSH Root Login".to_string(),
            status: "pass".to_string(),
            message: "Root login is disabled".to_string(),
            detail: Some(val),
        }
    } else if val.contains("prohibit-password") || val.contains("without-password") {
        SecurityCheck {
            name: "SSH Root Login".to_string(),
            status: "pass".to_string(),
            message: "Root login requires key authentication".to_string(),
            detail: Some(val),
        }
    } else if val == "notset" || val.is_empty() {
        SecurityCheck {
            name: "SSH Root Login".to_string(),
            status: "warn".to_string(),
            message: "Using default (check /etc/ssh/sshd_config)".to_string(),
            detail: Some("PermitRootLogin not explicitly set".to_string()),
        }
    } else {
        SecurityCheck {
            name: "SSH Root Login".to_string(),
            status: "fail".to_string(),
            message: "Root login is allowed".to_string(),
            detail: Some(val),
        }
    }
}

fn check_ssh_port(session: &Session) -> SecurityCheck {
    let output = run_command(session, "grep -E '^Port' /etc/ssh/sshd_config 2>/dev/null || echo 'Port 22'");
    let val = output.unwrap_or_default().trim().to_string();

    if val.contains("22") && !val.contains("222") && !val.contains("220") {
        SecurityCheck {
            name: "SSH Port".to_string(),
            status: "warn".to_string(),
            message: "Using default port 22".to_string(),
            detail: Some(val),
        }
    } else {
        SecurityCheck {
            name: "SSH Port".to_string(),
            status: "pass".to_string(),
            message: "Using non-default SSH port".to_string(),
            detail: Some(val),
        }
    }
}

fn check_firewall(session: &Session) -> SecurityCheck {
    // Try ufw first, then firewalld, then iptables
    let ufw = run_command(session, "sudo ufw status numbered 2>/dev/null | head -1");
    if let Ok(out) = ufw {
        let s = out.trim();
        if s.to_lowercase().contains("active") || s.to_lowercase().contains("status") {
            return SecurityCheck {
                name: "Firewall".to_string(),
                status: "pass".to_string(),
                message: "UFW firewall is active".to_string(),
                detail: Some(s.to_string()),
            };
        }
    }

    let fw = run_command(session, "sudo firewall-cmd --state 2>/dev/null");
    if let Ok(out) = fw {
        let s = out.trim();
        if s.to_lowercase().contains("running") {
            return SecurityCheck {
                name: "Firewall".to_string(),
                status: "pass".to_string(),
                message: "firewalld is active".to_string(),
                detail: Some(s.to_string()),
            };
        }
    }

    let ipt = run_command(session, "sudo iptables -L -n 2>/dev/null | grep -v '^Chain' | grep -v '^target' | head -5 | wc -l");
    if let Ok(out) = ipt {
        if out.trim().parse::<i32>().unwrap_or(0) > 0 {
            return SecurityCheck {
                name: "Firewall".to_string(),
                status: "pass".to_string(),
                message: "iptables rules are configured".to_string(),
                detail: Some("Custom iptables rules detected".to_string()),
            };
        }
    }

    SecurityCheck {
        name: "Firewall".to_string(),
        status: "fail".to_string(),
        message: "No active firewall detected".to_string(),
        detail: Some("UFW, firewalld, and iptables all appear inactive".to_string()),
    }
}

fn check_failed_logins(session: &Session) -> SecurityCheck {
    // Try auth.log first, then journalctl
    let count = run_command(session, "grep 'Failed password' /var/log/auth.log 2>/dev/null | tail -n 20 | wc -l");
    let count_val = count.unwrap_or_default().trim().parse::<i32>().unwrap_or(0);

    let count2 = run_command(session, "journalctl _SYSTEMD_UNIT=sshd.service 2>/dev/null | grep 'Failed password' | tail -n 20 | wc -l");
    let count_val2 = count2.unwrap_or_default().trim().parse::<i32>().unwrap_or(0);

    let total = count_val.max(count_val2);

    if total == 0 {
        SecurityCheck {
            name: "Failed Login Attempts".to_string(),
            status: "pass".to_string(),
            message: "No recent failed login attempts".to_string(),
            detail: Some("Checked last 20 log entries".to_string()),
        }
    } else if total < 5 {
        SecurityCheck {
            name: "Failed Login Attempts".to_string(),
            status: "warn".to_string(),
            message: format!("{} recent failed login attempts", total),
            detail: Some("Consider reviewing logs".to_string()),
        }
    } else {
        SecurityCheck {
            name: "Failed Login Attempts".to_string(),
            status: "fail".to_string(),
            message: format!("{} recent failed login attempts", total),
            detail: Some("Potential brute-force attack — consider fail2ban".to_string()),
        }
    }
}

fn check_sudo_users(session: &Session) -> SecurityCheck {
    let sudo_group = run_command(session, "getent group sudo 2>/dev/null | cut -d: -f4");
    let wheel_group = run_command(session, "getent group wheel 2>/dev/null | cut -d: -f4");

    let sudo_users = sudo_group.unwrap_or_default().trim().to_string();
    let wheel_users = wheel_group.unwrap_or_default().trim().to_string();

    let users = if !sudo_users.is_empty() && !wheel_users.is_empty() {
        format!("sudo: {} | wheel: {}", sudo_users, wheel_users)
    } else if !sudo_users.is_empty() {
        format!("sudo: {}", sudo_users)
    } else if !wheel_users.is_empty() {
        format!("wheel: {}", wheel_users)
    } else {
        "No sudo/wheel group found".to_string()
    };

    SecurityCheck {
        name: "Users with Elevated Privileges".to_string(),
        status: "pass".to_string(),
        message: "Sudo users enumerated".to_string(),
        detail: Some(users),
    }
}

fn check_security_updates(session: &Session) -> SecurityCheck {
    // Try apt, then yum/dnf
    let apt_count = run_command(session, "apt list --upgradable 2>/dev/null | grep -c security || echo 0");
    let yum_count = run_command(session, "yum --security check-update 2>/dev/null | grep -c security || echo 0");

    let apt_val = apt_count.unwrap_or_default().trim().parse::<i32>().unwrap_or(0);
    let yum_val = yum_count.unwrap_or_default().trim().parse::<i32>().unwrap_or(0);
    let total = apt_val.max(yum_val);

    if total == 0 {
        SecurityCheck {
            name: "Security Updates".to_string(),
            status: "pass".to_string(),
            message: "No pending security updates".to_string(),
            detail: None,
        }
    } else {
        SecurityCheck {
            name: "Security Updates".to_string(),
            status: "fail".to_string(),
            message: format!("{} security updates pending", total),
            detail: Some("Run system update to patch".to_string()),
        }
    }
}

fn check_disk_space(session: &Session) -> SecurityCheck {
    let output = run_command(session, "df -h / | awk 'NR==2{print $5}' | tr -d '%'");
    let pct = output.unwrap_or_default().trim().parse::<u8>().unwrap_or(0);

    if pct < 70 {
        SecurityCheck {
            name: "Root Disk Space".to_string(),
            status: "pass".to_string(),
            message: format!("{}% used", pct),
            detail: None,
        }
    } else if pct < 85 {
        SecurityCheck {
            name: "Root Disk Space".to_string(),
            status: "warn".to_string(),
            message: format!("{}% used", pct),
            detail: Some("Consider cleaning up".to_string()),
        }
    } else {
        SecurityCheck {
            name: "Root Disk Space".to_string(),
            status: "fail".to_string(),
            message: format!("{}% used — critical", pct),
            detail: Some("Disk is nearly full".to_string()),
        }
    }
}

pub fn run_security_audit(session: &Session) -> Result<SecurityReport, String> {
    let checks = vec![
        check_ssh_password_auth(session),
        check_ssh_root_login(session),
        check_ssh_port(session),
        check_firewall(session),
        check_failed_logins(session),
        check_sudo_users(session),
        check_security_updates(session),
        check_disk_space(session),
    ];

    let pass_count = checks.iter().filter(|c| c.status == "pass").count() as u8;
    let total = checks.len() as u8;
    let score = if total > 0 { (pass_count * 100) / total } else { 0 };

    Ok(SecurityReport { score, checks })
}
