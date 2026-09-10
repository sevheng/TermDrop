use crate::ssh::exec::run_command;
use serde::{Deserialize, Serialize};
use ssh2::Session;
use std::collections::HashMap;

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

/// Integer count from command output; anything unparsable counts as 0.
fn parse_count(output: &str) -> i32 {
    output.trim().parse::<i32>().unwrap_or(0)
}

// ---------------------------------------------------------------------------
// Each check is split into a fetch step (runs remote commands, maps a failed
// command to empty output) and a pure classify step that is unit tested.
// ---------------------------------------------------------------------------

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

fn os_family_from(output: &str) -> String {
    output.trim().to_lowercase()
}

/// Runs 1-2 package-manager commands chosen by the OS family, so it stays
/// outside the batched script.
fn check_security_updates(session: &Session, os_family: String) -> SecurityCheck {
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

/// Wrap `s` as a single POSIX shell word.
fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// All single-round-trip probes in one POSIX sh script. Each `section`
/// line carries the command's exit status so the parser can treat a failed
/// command as empty output, exactly as the per-command `run_command` path
/// did. The firewall probes keep their short-circuit order: firewalld is
/// only queried when ufw is not active, iptables only when neither is.
const AUDIT_SCRIPT: &str = r#"section() { printf '\n---TERMDROP-%s rc=%s---\n' "$1" "$2"; }
out=$(grep -E '^PasswordAuthentication' /etc/ssh/sshd_config 2>/dev/null || echo 'NOTSET'); rc=$?; section PASSWORD_AUTH $rc; printf '%s' "$out"
out=$(grep -E '^PermitRootLogin' /etc/ssh/sshd_config 2>/dev/null || echo 'NOTSET'); rc=$?; section ROOT_LOGIN $rc; printf '%s' "$out"
out=$(grep -E '^Port' /etc/ssh/sshd_config 2>/dev/null || echo 'Port 22'); rc=$?; section SSH_PORT $rc; printf '%s' "$out"
fw=0
out=$(sudo ufw status numbered 2>/dev/null | head -1); rc=$?; section UFW $rc; printf '%s' "$out"
if [ $rc -eq 0 ]; then case $(printf '%s' "$out" | tr 'A-Z' 'a-z') in *active*|*status*) fw=1;; esac; fi
if [ $fw -eq 0 ]; then
  out=$(sudo firewall-cmd --state 2>/dev/null); rc=$?; section FIREWALLD $rc; printf '%s' "$out"
  if [ $rc -eq 0 ]; then case $(printf '%s' "$out" | tr 'A-Z' 'a-z') in *running*) fw=1;; esac; fi
fi
if [ $fw -eq 0 ]; then
  out=$(sudo iptables -L -n 2>/dev/null | grep -v '^Chain' | grep -v '^target' | head -5 | wc -l); rc=$?; section IPTABLES $rc; printf '%s' "$out"
fi
out=$(grep 'Failed password' /var/log/auth.log 2>/dev/null | tail -n 20 | wc -l); rc=$?; section FAILED_AUTH_LOG $rc; printf '%s' "$out"
out=$(journalctl _SYSTEMD_UNIT=sshd.service 2>/dev/null | grep 'Failed password' | tail -n 20 | wc -l); rc=$?; section FAILED_JOURNAL $rc; printf '%s' "$out"
out=$(getent group sudo 2>/dev/null | cut -d: -f4); rc=$?; section SUDO_GROUP $rc; printf '%s' "$out"
out=$(getent group wheel 2>/dev/null | cut -d: -f4); rc=$?; section WHEEL_GROUP $rc; printf '%s' "$out"
out=$(grep '^ID=' /etc/os-release 2>/dev/null | sed 's/ID=//; s/"//g'); rc=$?; section OS_ID $rc; printf '%s' "$out"
out=$(df -h / | awk 'NR==2{print $5}' | tr -d '%'); rc=$?; section DISK $rc; printf '%s' "$out"
printf '\n'
true
"#;

fn audit_command() -> String {
    format!("sh -c {}", shell_quote(AUDIT_SCRIPT))
}

/// Section outputs keyed by name. A section whose command exited non-zero
/// (or that was skipped by the script) is `None`.
fn parse_audit_sections(output: &str) -> HashMap<String, Option<String>> {
    let mut sections = HashMap::new();
    let mut current: Option<(String, bool)> = None;
    let mut body: Vec<&str> = Vec::new();

    let flush = |current: &Option<(String, bool)>,
                 body: &Vec<&str>,
                 sections: &mut HashMap<String, Option<String>>| {
        if let Some((key, ok)) = current {
            let text = if *ok { Some(body.join("\n")) } else { None };
            sections.insert(key.clone(), text);
        }
    };

    for line in output.lines() {
        if let Some(rest) = line.strip_prefix("---TERMDROP-") {
            if let Some(rest) = rest.strip_suffix("---") {
                if let Some((key, rc)) = rest.split_once(" rc=") {
                    flush(&current, &body, &mut sections);
                    current = Some((key.to_string(), rc == "0"));
                    body.clear();
                    continue;
                }
            }
        }
        if current.is_some() {
            body.push(line);
        }
    }
    flush(&current, &body, &mut sections);
    sections
}

fn section_text<'a>(sections: &'a HashMap<String, Option<String>>, key: &str) -> &'a str {
    sections.get(key).and_then(|v| v.as_deref()).unwrap_or("")
}

/// Same order and predicates as the sequential probes: ufw, then firewalld,
/// then iptables, each only consulted if present and successful.
fn firewall_from_sections(sections: &HashMap<String, Option<String>>) -> SecurityCheck {
    if let Some(Some(out)) = sections.get("UFW") {
        if let Some(check) = firewall_from_ufw(out) {
            return check;
        }
    }
    if let Some(Some(out)) = sections.get("FIREWALLD") {
        if let Some(check) = firewall_from_firewalld(out) {
            return check;
        }
    }
    if let Some(Some(out)) = sections.get("IPTABLES") {
        if let Some(check) = firewall_from_iptables(out) {
            return check;
        }
    }
    firewall_not_detected()
}

/// Build the report from parsed sections plus the separately fetched
/// updates check.
fn audit_from_sections(
    sections: &HashMap<String, Option<String>>,
    updates: SecurityCheck,
) -> SecurityReport {
    let checks = vec![
        classify_password_auth(section_text(sections, "PASSWORD_AUTH")),
        classify_root_login(section_text(sections, "ROOT_LOGIN")),
        classify_ssh_port(section_text(sections, "SSH_PORT")),
        firewall_from_sections(sections),
        classify_failed_logins(
            section_text(sections, "FAILED_AUTH_LOG"),
            section_text(sections, "FAILED_JOURNAL"),
        ),
        classify_sudo_users(
            section_text(sections, "SUDO_GROUP"),
            section_text(sections, "WHEEL_GROUP"),
        ),
        updates,
        classify_disk_space(section_text(sections, "DISK")),
    ];
    score_report(checks)
}

pub fn run_security_audit(session: &Session) -> Result<SecurityReport, String> {
    // One round trip for every single-command probe. If the batch itself
    // fails, every section reads as empty, as a failed probe always did.
    let sections = run_command(session, &audit_command())
        .map(|out| parse_audit_sections(&out))
        .unwrap_or_default();
    let os_family = os_family_from(section_text(&sections, "OS_ID"));
    let updates = check_security_updates(session, os_family);
    Ok(audit_from_sections(&sections, updates))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status(check: &SecurityCheck) -> &str {
        check.status.as_str()
    }

    fn sections_from(pairs: &[(&str, i32, &str)]) -> String {
        let mut out = String::new();
        for (key, rc, body) in pairs {
            out.push_str(&format!("\n---TERMDROP-{} rc={}---\n{}", key, rc, body));
        }
        out.push('\n');
        out
    }

    #[test]
    fn parses_sections_and_treats_failed_commands_as_none() {
        let raw = sections_from(&[
            ("PASSWORD_AUTH", 0, "PasswordAuthentication no"),
            ("SSH_PORT", 0, "Port 22\nPort 2222"),
            ("FIREWALLD", 252, "not running"),
            ("DISK", 0, ""),
        ]);
        let sections = parse_audit_sections(&raw);
        assert_eq!(
            sections["PASSWORD_AUTH"].as_deref(),
            Some("PasswordAuthentication no")
        );
        assert_eq!(sections["SSH_PORT"].as_deref(), Some("Port 22\nPort 2222"));
        assert_eq!(sections["FIREWALLD"], None, "non-zero exit discards stdout");
        assert_eq!(sections["DISK"].as_deref(), Some(""));
        assert!(!sections.contains_key("UFW"), "absent sections stay absent");
        assert_eq!(section_text(&sections, "FIREWALLD"), "");
        assert_eq!(section_text(&sections, "MISSING"), "");
    }

    #[test]
    fn firewall_sections_follow_probe_order_and_skip_failed_probes() {
        let mut m: HashMap<String, Option<String>> = HashMap::new();
        m.insert("UFW".into(), Some("Status: active".into()));
        m.insert("FIREWALLD".into(), Some("running".into()));
        assert_eq!(firewall_from_sections(&m).message, "UFW firewall is active");

        let mut m: HashMap<String, Option<String>> = HashMap::new();
        m.insert("UFW".into(), None);
        m.insert("FIREWALLD".into(), None); // exited non-zero: "not running" discarded
        m.insert("IPTABLES".into(), Some("4".into()));
        assert_eq!(
            firewall_from_sections(&m).message,
            "iptables rules are configured"
        );

        let mut m: HashMap<String, Option<String>> = HashMap::new();
        m.insert("UFW".into(), Some("".into()));
        m.insert("FIREWALLD".into(), Some("running".into()));
        assert_eq!(firewall_from_sections(&m).message, "firewalld is active");

        assert_eq!(firewall_from_sections(&HashMap::new()).status, "fail");
    }

    #[test]
    fn batched_report_matches_per_command_classification() {
        let raw = sections_from(&[
            ("PASSWORD_AUTH", 0, "PasswordAuthentication no"),
            ("ROOT_LOGIN", 0, "PermitRootLogin prohibit-password"),
            ("SSH_PORT", 0, "Port 2222"),
            ("UFW", 0, "Status: active"),
            ("FAILED_AUTH_LOG", 0, "3"),
            ("FAILED_JOURNAL", 1, ""),
            ("SUDO_GROUP", 0, "alice,bob"),
            ("WHEEL_GROUP", 2, ""),
            ("OS_ID", 0, "ubuntu"),
            ("DISK", 0, "72"),
        ]);
        let sections = parse_audit_sections(&raw);
        assert_eq!(os_family_from(section_text(&sections, "OS_ID")), "ubuntu");
        let updates = classify_security_updates(0, "apt");
        let report = audit_from_sections(&sections, updates.clone());

        let expected = score_report(vec![
            classify_password_auth("PasswordAuthentication no"),
            classify_root_login("PermitRootLogin prohibit-password"),
            classify_ssh_port("Port 2222"),
            firewall_from_ufw("Status: active").unwrap(),
            classify_failed_logins("3", ""),
            classify_sudo_users("alice,bob", ""),
            updates,
            classify_disk_space("72"),
        ]);
        assert_eq!(report.score, expected.score);
        for (a, b) in report.checks.iter().zip(expected.checks.iter()) {
            assert_eq!(
                (&a.name, &a.status, &a.message, &a.detail),
                (&b.name, &b.status, &b.message, &b.detail)
            );
        }
        assert_eq!(report.checks[3].message, "UFW firewall is active");
        assert_eq!(report.checks[4].status, "warn");
        assert_eq!(report.checks[7].status, "warn");
    }

    #[test]
    fn audit_command_is_a_single_quoted_sh_invocation() {
        let cmd = audit_command();
        assert!(cmd.starts_with("sh -c '"));
        assert!(cmd.ends_with("'"));
        assert_eq!(shell_quote("it's"), "'it'\\''s'");
        // Every probe from the sequential version is still present.
        for needle in [
            "PasswordAuthentication",
            "PermitRootLogin",
            "^Port",
            "ufw status numbered",
            "firewall-cmd --state",
            "iptables -L -n",
            "/var/log/auth.log",
            "journalctl _SYSTEMD_UNIT=sshd.service",
            "getent group sudo",
            "getent group wheel",
            "/etc/os-release",
            "df -h /",
        ] {
            assert!(cmd.contains(needle), "{}", needle);
        }
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
