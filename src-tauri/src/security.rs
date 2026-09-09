use serde::{Deserialize, Serialize};
use ssh2::Session;
use std::io::Read;

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

/// Integer count from command output; anything unparsable counts as 0.
fn parse_count(output: &str) -> i32 {
    output.trim().parse::<i32>().unwrap_or(0)
}

// ---------------------------------------------------------------------------
// Each check is split into a fetch step (runs remote commands, maps a failed
// command to empty output) and a pure classify step that is unit tested.
// ---------------------------------------------------------------------------

fn check_ssh_password_auth(session: &Session) -> SecurityCheck {
    let output = run_command(
        session,
        "grep -E '^PasswordAuthentication' /etc/ssh/sshd_config 2>/dev/null || echo 'NOTSET'",
    );
    classify_password_auth(&output.unwrap_or_default())
}

fn classify_password_auth(output: &str) -> SecurityCheck {
    let val = output.trim().to_lowercase();

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
    let output = run_command(
        session,
        "grep -E '^PermitRootLogin' /etc/ssh/sshd_config 2>/dev/null || echo 'NOTSET'",
    );
    classify_root_login(&output.unwrap_or_default())
}

fn classify_root_login(output: &str) -> SecurityCheck {
    let val = output.trim().to_lowercase();

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
    let output = run_command(
        session,
        "grep -E '^Port' /etc/ssh/sshd_config 2>/dev/null || echo 'Port 22'",
    );
    classify_ssh_port(&output.unwrap_or_default())
}

fn classify_ssh_port(output: &str) -> SecurityCheck {
    let val = output.trim().to_string();

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
    if let Ok(out) = run_command(session, "sudo ufw status numbered 2>/dev/null | head -1") {
        if let Some(check) = firewall_from_ufw(&out) {
            return check;
        }
    }

    if let Ok(out) = run_command(session, "sudo firewall-cmd --state 2>/dev/null") {
        if let Some(check) = firewall_from_firewalld(&out) {
            return check;
        }
    }

    if let Ok(out) = run_command(
        session,
        "sudo iptables -L -n 2>/dev/null | grep -v '^Chain' | grep -v '^target' | head -5 | wc -l",
    ) {
        if let Some(check) = firewall_from_iptables(&out) {
            return check;
        }
    }

    firewall_not_detected()
}

fn firewall_from_ufw(output: &str) -> Option<SecurityCheck> {
    let s = output.trim();
    if s.to_lowercase().contains("active") || s.to_lowercase().contains("status") {
        Some(SecurityCheck {
            name: "Firewall".to_string(),
            status: "pass".to_string(),
            message: "UFW firewall is active".to_string(),
            detail: Some(s.to_string()),
        })
    } else {
        None
    }
}

fn firewall_from_firewalld(output: &str) -> Option<SecurityCheck> {
    let s = output.trim();
    if s.to_lowercase().contains("running") {
        Some(SecurityCheck {
            name: "Firewall".to_string(),
            status: "pass".to_string(),
            message: "firewalld is active".to_string(),
            detail: Some(s.to_string()),
        })
    } else {
        None
    }
}

fn firewall_from_iptables(output: &str) -> Option<SecurityCheck> {
    if parse_count(output) > 0 {
        Some(SecurityCheck {
            name: "Firewall".to_string(),
            status: "pass".to_string(),
            message: "iptables rules are configured".to_string(),
            detail: Some("Custom iptables rules detected".to_string()),
        })
    } else {
        None
    }
}

fn firewall_not_detected() -> SecurityCheck {
    SecurityCheck {
        name: "Firewall".to_string(),
        status: "fail".to_string(),
        message: "No active firewall detected".to_string(),
        detail: Some("UFW, firewalld, and iptables all appear inactive".to_string()),
    }
}

fn check_failed_logins(session: &Session) -> SecurityCheck {
    // Try auth.log first, then journalctl
    let count = run_command(
        session,
        "grep 'Failed password' /var/log/auth.log 2>/dev/null | tail -n 20 | wc -l",
    );
    let count2 = run_command(session, "journalctl _SYSTEMD_UNIT=sshd.service 2>/dev/null | grep 'Failed password' | tail -n 20 | wc -l");
    classify_failed_logins(&count.unwrap_or_default(), &count2.unwrap_or_default())
}

fn classify_failed_logins(auth_log: &str, journal: &str) -> SecurityCheck {
    let total = parse_count(auth_log).max(parse_count(journal));

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
    classify_sudo_users(
        &sudo_group.unwrap_or_default(),
        &wheel_group.unwrap_or_default(),
    )
}

fn classify_sudo_users(sudo_group: &str, wheel_group: &str) -> SecurityCheck {
    let sudo_users = sudo_group.trim().to_string();
    let wheel_users = wheel_group.trim().to_string();

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

fn detect_os_family(session: &Session) -> String {
    let id = run_command(
        session,
        "grep '^ID=' /etc/os-release 2>/dev/null | sed 's/ID=//; s/\"//g'",
    );
    os_family_from(&id.unwrap_or_default())
}

fn os_family_from(output: &str) -> String {
    output.trim().to_lowercase()
}

fn check_security_updates(session: &Session) -> SecurityCheck {
    let os_family = detect_os_family(session);

    let (total, pkg_manager) = match os_family.as_str() {
        "ubuntu" | "debian" | "linuxmint" | "pop" | "elementary" | "zorin" => {
            let out = run_command(
                session,
                "apt list --upgradable 2>/dev/null | grep -c 'security' || echo 0",
            );
            (parse_count(&out.unwrap_or_default()), "apt")
        }
        "centos" | "rhel" | "fedora" | "rocky" | "almalinux" | "amazon" | "amzn" => {
            // Try dnf first, then yum
            let out = run_command(session, "dnf check-update --security 2>/dev/null | grep -c 'security' || yum --security check-update 2>/dev/null | grep -c 'security' || echo 0");
            (parse_count(&out.unwrap_or_default()), "dnf/yum")
        }
        "alpine" => {
            let out = run_command(
                session,
                "apk upgrade --simulate 2>/dev/null | grep -c 'Upgrading' || echo 0",
            );
            (parse_count(&out.unwrap_or_default()), "apk")
        }
        "arch" | "manjaro" | "endeavouros" => {
            let out = run_command(session, "pacman -Qu 2>/dev/null | wc -l || echo 0");
            (parse_count(&out.unwrap_or_default()), "pacman")
        }
        "opensuse" | "opensuse-leap" | "opensuse-tumbleweed" | "sles" => {
            let out = run_command(
                session,
                "zypper list-updates 2>/dev/null | grep -c 'v' || echo 0",
            );
            (parse_count(&out.unwrap_or_default()), "zypper")
        }
        _ => {
            // Fallback: try apt, then dnf/yum
            let apt = run_command(
                session,
                "apt list --upgradable 2>/dev/null | grep -c 'security' || echo 0",
            );
            let apt_count = parse_count(&apt.unwrap_or_default());
            if apt_count > 0 {
                (apt_count, "apt")
            } else {
                let yum = run_command(session, "dnf check-update --security 2>/dev/null | grep -c 'security' || yum --security check-update 2>/dev/null | grep -c 'security' || echo 0");
                (parse_count(&yum.unwrap_or_default()), "dnf/yum")
            }
        }
    };

    classify_security_updates(total, pkg_manager)
}

fn classify_security_updates(total: i32, pkg_manager: &str) -> SecurityCheck {
    if total == 0 {
        SecurityCheck {
            name: "Security Updates".to_string(),
            status: "pass".to_string(),
            message: format!("No pending updates ({})", pkg_manager),
            detail: None,
        }
    } else {
        SecurityCheck {
            name: "Security Updates".to_string(),
            status: "fail".to_string(),
            message: format!("{} updates pending", total),
            detail: Some(format!("Run {} upgrade to patch", pkg_manager)),
        }
    }
}

fn check_disk_space(session: &Session) -> SecurityCheck {
    let output = run_command(session, "df -h / | awk 'NR==2{print $5}' | tr -d '%'");
    classify_disk_space(&output.unwrap_or_default())
}

fn classify_disk_space(output: &str) -> SecurityCheck {
    let pct = output.trim().parse::<u8>().unwrap_or(0);

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

/// Score is the integer percentage of checks that passed.
fn score_report(checks: Vec<SecurityCheck>) -> SecurityReport {
    let pass_count = checks.iter().filter(|c| c.status == "pass").count() as u16;
    let total = checks.len() as u16;
    let score = if total > 0 {
        ((pass_count * 100) / total) as u8
    } else {
        0
    };

    SecurityReport { score, checks }
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

    Ok(score_report(checks))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status(check: &SecurityCheck) -> &str {
        check.status.as_str()
    }

    #[test]
    fn password_auth_classification() {
        let c = classify_password_auth("PasswordAuthentication no\n");
        assert_eq!(status(&c), "pass");
        assert_eq!(c.detail.as_deref(), Some("passwordauthentication no"));

        // Existing quirk: "notset" contains "no", so an unset directive is
        // reported as a pass and the "warn" branch is only reachable for
        // empty output. Preserved as-is; a fix is a behavior change.
        assert_eq!(status(&classify_password_auth("NOTSET\n")), "pass");
        assert_eq!(status(&classify_password_auth("")), "warn");
        assert_eq!(
            classify_password_auth("").detail.as_deref(),
            Some("PasswordAuthentication not explicitly set")
        );

        assert_eq!(
            status(&classify_password_auth("PasswordAuthentication yes")),
            "fail"
        );
    }

    #[test]
    fn root_login_classification() {
        assert_eq!(status(&classify_root_login("PermitRootLogin no")), "pass");
        let c = classify_root_login("PermitRootLogin prohibit-password");
        assert_eq!(status(&c), "pass");
        assert_eq!(c.message, "Root login requires key authentication");
        assert_eq!(
            status(&classify_root_login("PermitRootLogin without-password")),
            "pass"
        );
        // Same "notset" contains "no" quirk as the password-auth check.
        assert_eq!(status(&classify_root_login("NOTSET")), "pass");
        assert_eq!(status(&classify_root_login("")), "warn");
        assert_eq!(status(&classify_root_login("PermitRootLogin yes")), "fail");
    }

    #[test]
    fn ssh_port_classification_including_substring_edge_cases() {
        assert_eq!(status(&classify_ssh_port("Port 22\n")), "warn");
        assert_eq!(status(&classify_ssh_port("Port 2222")), "pass");
        assert_eq!(status(&classify_ssh_port("Port 220")), "pass");
        assert_eq!(status(&classify_ssh_port("Port 8022")), "warn"); // contains "22"
        assert_eq!(status(&classify_ssh_port("Port 443")), "pass");
        assert_eq!(
            classify_ssh_port("Port 443").detail.as_deref(),
            Some("Port 443")
        );
    }

    #[test]
    fn firewall_probes_are_independent_options() {
        let ufw = firewall_from_ufw("Status: active\n").unwrap();
        assert_eq!(ufw.message, "UFW firewall is active");
        assert_eq!(ufw.detail.as_deref(), Some("Status: active"));
        assert!(firewall_from_ufw("").is_none());
        assert!(
            firewall_from_ufw("inactive").is_some(),
            "\"inactive\" contains \"active\""
        );

        assert!(firewall_from_firewalld("running").is_some());
        assert!(
            firewall_from_firewalld("not running").is_some(),
            "raw stdout containing \"running\" matches; callers must drop output of failed commands"
        );
        assert!(firewall_from_firewalld("").is_none());

        assert!(firewall_from_iptables("3\n").is_some());
        assert!(firewall_from_iptables("0").is_none());
        assert!(firewall_from_iptables("garbage").is_none());

        let none = firewall_not_detected();
        assert_eq!(status(&none), "fail");
        assert_eq!(
            none.detail.as_deref(),
            Some("UFW, firewalld, and iptables all appear inactive")
        );
    }

    #[test]
    fn failed_logins_take_max_of_both_sources_with_thresholds() {
        assert_eq!(status(&classify_failed_logins("0\n", "0\n")), "pass");
        assert_eq!(status(&classify_failed_logins("", "")), "pass");

        let warn = classify_failed_logins("2", "4");
        assert_eq!(status(&warn), "warn");
        assert_eq!(warn.message, "4 recent failed login attempts");

        let fail = classify_failed_logins("5", "1");
        assert_eq!(status(&fail), "fail");
        assert_eq!(fail.message, "5 recent failed login attempts");
        assert_eq!(
            fail.detail.as_deref(),
            Some("Potential brute-force attack — consider fail2ban")
        );
    }

    #[test]
    fn sudo_users_detail_formats() {
        assert_eq!(
            classify_sudo_users("alice,bob\n", "root\n")
                .detail
                .as_deref(),
            Some("sudo: alice,bob | wheel: root")
        );
        assert_eq!(
            classify_sudo_users("alice", "").detail.as_deref(),
            Some("sudo: alice")
        );
        assert_eq!(
            classify_sudo_users("", "root").detail.as_deref(),
            Some("wheel: root")
        );
        assert_eq!(
            classify_sudo_users("", "").detail.as_deref(),
            Some("No sudo/wheel group found")
        );
        assert_eq!(status(&classify_sudo_users("", "")), "pass");
    }

    #[test]
    fn security_updates_classification() {
        let ok = classify_security_updates(0, "apt");
        assert_eq!(status(&ok), "pass");
        assert_eq!(ok.message, "No pending updates (apt)");
        assert_eq!(ok.detail, None);

        let bad = classify_security_updates(7, "dnf/yum");
        assert_eq!(status(&bad), "fail");
        assert_eq!(bad.message, "7 updates pending");
        assert_eq!(bad.detail.as_deref(), Some("Run dnf/yum upgrade to patch"));
    }

    #[test]
    fn os_family_is_trimmed_and_lowercased() {
        assert_eq!(os_family_from("Ubuntu\n"), "ubuntu");
        assert_eq!(os_family_from(""), "");
    }

    #[test]
    fn disk_space_thresholds() {
        assert_eq!(status(&classify_disk_space("69\n")), "pass");
        assert_eq!(status(&classify_disk_space("70")), "warn");
        assert_eq!(status(&classify_disk_space("84")), "warn");
        let full = classify_disk_space("85");
        assert_eq!(status(&full), "fail");
        assert_eq!(full.message, "85% used — critical");
        assert_eq!(
            status(&classify_disk_space("")),
            "pass",
            "unparsable counts as 0%"
        );
        assert_eq!(
            status(&classify_disk_space("300")),
            "pass",
            "u8 overflow counts as 0%"
        );
    }

    #[test]
    fn parse_count_defaults_to_zero() {
        assert_eq!(parse_count(" 12 \n"), 12);
        assert_eq!(parse_count(""), 0);
        assert_eq!(parse_count("x"), 0);
    }

    #[test]
    fn score_is_integer_percent_of_passes() {
        let mk = |s: &str| SecurityCheck {
            name: String::new(),
            status: s.to_string(),
            message: String::new(),
            detail: None,
        };
        assert_eq!(score_report(vec![]).score, 0);
        assert_eq!(
            score_report(vec![mk("pass"), mk("pass"), mk("fail")]).score,
            66
        );
        assert_eq!(
            score_report(vec![mk("pass"), mk("warn"), mk("fail"), mk("pass")]).score,
            50
        );
    }
}
