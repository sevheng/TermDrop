use crate::ssh::exec::run_command;
use serde::{Deserialize, Serialize};
use ssh2::Session;
use std::collections::HashMap;

/// Check verdicts. `UNKNOWN` means the probe could not determine the answer,
/// usually for lack of privilege; it is reported rather than being collapsed
/// into a pass, and it does not count toward the score.
const PASS: &str = "pass";
const WARN: &str = "warn";
const FAIL: &str = "fail";
const UNKNOWN: &str = "unknown";

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SecurityCheck {
    pub name: String,
    pub status: String,
    pub message: String,
    pub detail: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SecurityReport {
    /// Percentage of *determinable* checks that passed.
    pub score: u8,
    /// Checks that passed, and how many could be determined at all.
    pub passed: u16,
    pub scored: u16,
    pub checks: Vec<SecurityCheck>,
}

fn check(name: &str, status: &str, message: String, detail: Option<String>) -> SecurityCheck {
    SecurityCheck {
        name: name.to_string(),
        status: status.to_string(),
        message,
        detail,
    }
}

/// Integer count from command output; anything unparsable counts as 0.
fn parse_count(output: &str) -> i32 {
    output.trim().parse::<i32>().unwrap_or(0)
}

// ---------------------------------------------------------------------------
// sshd configuration
//
// `sshd -T` prints the fully resolved configuration, including files pulled in
// by `Include` and every compiled-in default, in lowercase `key value` form.
// That is the only correct source: Ubuntu 22.04+ and RHEL 9 both ship an
// `Include /etc/ssh/sshd_config.d/*.conf` as the first line of the main file,
// and sshd honors the first value it obtains. When `sshd -T` is unavailable we
// fall back to reading the drop-in directory followed by the main file, which
// approximates that precedence, and report `unknown` when neither is readable.
// ---------------------------------------------------------------------------

/// A directive's effective value, or the reason we do not have one.
#[derive(Debug, PartialEq)]
enum Directive {
    Value(String),
    /// The configuration was readable and the directive was absent.
    NotSet,
    /// The configuration could not be read at all.
    Unknown,
}

/// First value for `key` in `sshd -T` output (already lowercase `key value`).
fn sshd_effective_value(output: &str, key: &str) -> Option<String> {
    output.lines().find_map(|line| {
        let mut parts = line.trim().splitn(2, char::is_whitespace);
        let k = parts.next()?;
        if k.eq_ignore_ascii_case(key) {
            Some(parts.next().unwrap_or("").trim().to_string())
        } else {
            None
        }
    })
}

/// First value for `key` in raw sshd_config text. Directive names are
/// case-insensitive, may be indented, and sshd uses the first occurrence.
/// Lines inside `Match` blocks are skipped, since they are conditional.
fn sshd_raw_value(output: &str, key: &str) -> Option<String> {
    let mut in_match = false;
    output.lines().find_map(|line| {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            return None;
        }
        let mut parts = line.splitn(2, |c: char| c.is_whitespace() || c == '=');
        let k = parts.next()?;
        if k.eq_ignore_ascii_case("match") {
            in_match = true;
            return None;
        }
        if in_match {
            return None;
        }
        if k.eq_ignore_ascii_case(key) {
            let v = parts.next().unwrap_or("").trim().trim_matches('"');
            Some(v.to_string())
        } else {
            None
        }
    })
}

/// Resolve one directive, preferring `sshd -T` over the raw files.
fn resolve_directive(effective: Option<&str>, raw: Option<&str>, key: &str) -> Directive {
    if let Some(text) = effective {
        if let Some(v) = sshd_effective_value(text, key) {
            return Directive::Value(v);
        }
    }
    if let Some(text) = raw {
        return match sshd_raw_value(text, key) {
            Some(v) => Directive::Value(v),
            None => Directive::NotSet,
        };
    }
    Directive::Unknown
}

/// Why a check could not answer, phrased for whichever situation applies.
fn unreadable_detail(base: &str, has_privilege: bool) -> String {
    if has_privilege {
        base.to_string()
    } else {
        format!("{}; this needs root and the session has no elevation", base)
    }
}

const SSHD_UNREADABLE: &str = "Could not read the SSH server configuration";

fn classify_password_auth(directive: &Directive, has_privilege: bool) -> SecurityCheck {
    let name = "SSH Password Authentication";
    match directive {
        Directive::Value(v) if v.eq_ignore_ascii_case("no") => check(
            name,
            PASS,
            "Password authentication is disabled".to_string(),
            Some(format!("PasswordAuthentication {}", v)),
        ),
        Directive::Value(v) => check(
            name,
            FAIL,
            "Password authentication is enabled".to_string(),
            Some(format!("PasswordAuthentication {}", v)),
        ),
        // OpenSSH defaults this to "yes", so an absent directive means
        // password login is accepted.
        Directive::NotSet => check(
            name,
            FAIL,
            "Password authentication is enabled".to_string(),
            Some("Not set; the OpenSSH default is yes".to_string()),
        ),
        Directive::Unknown => check(
            name,
            UNKNOWN,
            "Could not determine".to_string(),
            Some(unreadable_detail(SSHD_UNREADABLE, has_privilege)),
        ),
    }
}

fn classify_root_login(directive: &Directive, has_privilege: bool) -> SecurityCheck {
    let name = "SSH Root Login";
    match directive {
        Directive::Value(v) if v.eq_ignore_ascii_case("no") => check(
            name,
            PASS,
            "Root login is disabled".to_string(),
            Some(format!("PermitRootLogin {}", v)),
        ),
        Directive::Value(v)
            if v.eq_ignore_ascii_case("prohibit-password")
                || v.eq_ignore_ascii_case("without-password") =>
        {
            check(
                name,
                PASS,
                "Root login requires key authentication".to_string(),
                Some(format!("PermitRootLogin {}", v)),
            )
        }
        Directive::Value(v) => check(
            name,
            FAIL,
            "Root login is allowed".to_string(),
            Some(format!("PermitRootLogin {}", v)),
        ),
        // The OpenSSH default is prohibit-password, which is acceptable, but
        // some distributions have shipped a different default.
        Directive::NotSet => check(
            name,
            WARN,
            "Using the default (prohibit-password)".to_string(),
            Some("Not set; set PermitRootLogin explicitly to be sure".to_string()),
        ),
        Directive::Unknown => check(
            name,
            UNKNOWN,
            "Could not determine".to_string(),
            Some(unreadable_detail(SSHD_UNREADABLE, has_privilege)),
        ),
    }
}

/// Ports sshd listens on, parsed from one or more `Port` values.
fn parse_ports(value: &str) -> Vec<u16> {
    value
        .split_whitespace()
        .filter_map(|p| p.parse::<u16>().ok())
        .collect()
}

fn classify_ssh_port(directive: &Directive, has_privilege: bool) -> SecurityCheck {
    let name = "SSH Port";
    let value = match directive {
        Directive::Value(v) => v.clone(),
        // sshd listens on 22 when no Port is configured.
        Directive::NotSet => "22".to_string(),
        Directive::Unknown => {
            return check(
                name,
                UNKNOWN,
                "Could not determine".to_string(),
                Some(unreadable_detail(SSHD_UNREADABLE, has_privilege)),
            )
        }
    };

    let ports = parse_ports(&value);
    if ports.is_empty() {
        return check(
            name,
            UNKNOWN,
            "Could not determine".to_string(),
            Some(format!("Unrecognized Port value: {}", value)),
        );
    }

    let detail = Some(
        ports
            .iter()
            .map(|p| p.to_string())
            .collect::<Vec<_>>()
            .join(", "),
    );
    if ports.contains(&22) {
        check(
            name,
            WARN,
            "Listening on the default port 22".to_string(),
            detail,
        )
    } else {
        check(
            name,
            PASS,
            "Using a non-default SSH port".to_string(),
            detail,
        )
    }
}

// ---------------------------------------------------------------------------
// Firewall
// ---------------------------------------------------------------------------

/// `ufw status` prints "Status: active" or "Status: inactive". Testing for the
/// substring "active" matches both, so the status token is compared exactly.
fn ufw_is_active(output: &str) -> Option<bool> {
    let lower = output.to_lowercase();
    let rest = lower.split("status:").nth(1)?;
    let token = rest.split_whitespace().next()?;
    match token {
        "active" => Some(true),
        "inactive" => Some(false),
        _ => None,
    }
}

/// `firewall-cmd --state` prints "running" or "not running".
fn firewalld_is_running(output: &str) -> Option<bool> {
    match output.trim().to_lowercase().as_str() {
        "running" => Some(true),
        "not running" => Some(false),
        _ => None,
    }
}

/// Verdict from whichever firewall probes were able to run. `None` for a probe
/// means it could not run, which is different from it reporting "inactive".
fn classify_firewall(
    ufw: Option<&str>,
    firewalld: Option<&str>,
    rule_count: Option<&str>,
    has_privilege: bool,
) -> SecurityCheck {
    let name = "Firewall";
    let mut probed = false;

    if let Some(out) = ufw {
        match ufw_is_active(out) {
            Some(true) => {
                return check(
                    name,
                    PASS,
                    "UFW firewall is active".to_string(),
                    Some(out.trim().to_string()),
                )
            }
            Some(false) => probed = true,
            None => {}
        }
    }
    if let Some(out) = firewalld {
        match firewalld_is_running(out) {
            Some(true) => {
                return check(
                    name,
                    PASS,
                    "firewalld is active".to_string(),
                    Some(out.trim().to_string()),
                )
            }
            Some(false) => probed = true,
            None => {}
        }
    }
    if let Some(out) = rule_count {
        let rules = parse_count(out);
        probed = true;
        if rules > 0 {
            return check(
                name,
                PASS,
                "Packet filter rules are configured".to_string(),
                Some(format!("{} rules found", rules)),
            );
        }
    }

    if probed {
        check(
            name,
            FAIL,
            "No active firewall detected".to_string(),
            Some("ufw, firewalld, nftables, and iptables all report no rules".to_string()),
        )
    } else {
        check(
            name,
            UNKNOWN,
            "Could not determine".to_string(),
            Some(unreadable_detail(
                "Firewall status could not be read",
                has_privilege,
            )),
        )
    }
}

// ---------------------------------------------------------------------------
// Remaining checks
// ---------------------------------------------------------------------------

fn classify_failed_logins(auth_log: Option<&str>, journal: Option<&str>) -> SecurityCheck {
    let name = "Failed Login Attempts";
    let counts: Vec<i32> = [auth_log, journal]
        .iter()
        .filter_map(|o| o.map(parse_count))
        .collect();

    if counts.is_empty() {
        return check(
            name,
            UNKNOWN,
            "Could not determine".to_string(),
            Some("Authentication logs are not readable by this user".to_string()),
        );
    }

    let total = counts.into_iter().max().unwrap_or(0);
    if total == 0 {
        check(
            name,
            PASS,
            "No recent failed login attempts".to_string(),
            Some("Checked the last 20 log entries".to_string()),
        )
    } else if total < 5 {
        check(
            name,
            WARN,
            format!("{} recent failed login attempts", total),
            Some("Consider reviewing logs".to_string()),
        )
    } else {
        check(
            name,
            FAIL,
            format!("{} recent failed login attempts", total),
            Some("Potential brute-force attack — consider fail2ban".to_string()),
        )
    }
}

/// Enumerates accounts with sudo access. Informational: it reports what it
/// found without asserting a verdict, so it does not affect the score.
fn classify_sudo_users(sudo_group: Option<&str>, wheel_group: Option<&str>) -> SecurityCheck {
    let name = "Users with Elevated Privileges";
    let sudo_users = sudo_group.unwrap_or("").trim().to_string();
    let wheel_users = wheel_group.unwrap_or("").trim().to_string();

    let users = if !sudo_users.is_empty() && !wheel_users.is_empty() {
        format!("sudo: {} | wheel: {}", sudo_users, wheel_users)
    } else if !sudo_users.is_empty() {
        format!("sudo: {}", sudo_users)
    } else if !wheel_users.is_empty() {
        format!("wheel: {}", wheel_users)
    } else {
        "No sudo/wheel group members found".to_string()
    };

    check(
        name,
        UNKNOWN,
        "Sudo users enumerated".to_string(),
        Some(users),
    )
}

fn os_family_from(output: &str) -> String {
    output.trim().to_lowercase()
}

/// Runs 1-2 package-manager commands chosen by the OS family, so it stays
/// outside the batched script.
fn check_security_updates(session: &Session, os_family: String) -> SecurityCheck {
    let (out, pkg_manager) = match os_family.as_str() {
        "ubuntu" | "debian" | "linuxmint" | "pop" | "elementary" | "zorin" => (
            run_command(
                session,
                "apt list --upgradable 2>/dev/null | grep -c security || echo 0",
            ),
            "apt",
        ),
        "centos" | "rhel" | "fedora" | "rocky" | "almalinux" | "amazon" | "amzn" => (
            // `check-update --security` lists packages, not the word "security",
            // so the old grep always counted zero. updateinfo lists advisories.
            run_command(
                session,
                "dnf -q updateinfo list --security 2>/dev/null | grep -c . || echo 0",
            ),
            "dnf",
        ),
        "alpine" => (
            run_command(
                session,
                "apk upgrade --simulate 2>/dev/null | grep -c Upgrading || echo 0",
            ),
            "apk",
        ),
        "arch" | "manjaro" | "endeavouros" => (
            run_command(session, "pacman -Qu 2>/dev/null | grep -c . || echo 0"),
            "pacman",
        ),
        "opensuse" | "opensuse-leap" | "opensuse-tumbleweed" | "sles" => (
            run_command(
                session,
                "zypper --quiet list-patches --category security 2>/dev/null | grep -c ' | ' || echo 0",
            ),
            "zypper",
        ),
        _ => (Ok(String::new()), "unknown"),
    };

    match out {
        Ok(text) if pkg_manager != "unknown" => {
            classify_security_updates(parse_count(&text), pkg_manager)
        }
        _ => check(
            "Security Updates",
            UNKNOWN,
            "Could not determine".to_string(),
            Some("No supported package manager was detected".to_string()),
        ),
    }
}

fn classify_security_updates(total: i32, pkg_manager: &str) -> SecurityCheck {
    let name = "Security Updates";
    if total == 0 {
        check(
            name,
            PASS,
            format!("No pending security updates ({})", pkg_manager),
            None,
        )
    } else {
        check(
            name,
            FAIL,
            format!("{} security updates pending", total),
            Some(format!("Run {} upgrade to patch", pkg_manager)),
        )
    }
}

/// Percentage of checks that produced a verdict and passed. Checks that could
/// not be determined are excluded from both halves rather than counting as
/// failures, so an unprivileged session does not read as an insecure host.
fn score_report(checks: Vec<SecurityCheck>) -> SecurityReport {
    let scored = checks.iter().filter(|c| c.status != UNKNOWN).count() as u16;
    let passed = checks.iter().filter(|c| c.status == PASS).count() as u16;
    let score = if scored > 0 {
        ((passed * 100) / scored) as u8
    } else {
        0
    };

    SecurityReport {
        score,
        passed,
        scored,
        checks,
    }
}

// ---------------------------------------------------------------------------
// The batched probe script
// ---------------------------------------------------------------------------

/// Wrap `s` as a single POSIX shell word.
fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// All single-round-trip probes in one POSIX sh script. Each `section` line
/// carries a status flag: 0 means the probe ran and its output follows, 1
/// means it could not run and the check reports `unknown` rather than
/// guessing.
///
/// Elevation is decided once, up front, and the privileged probes are skipped
/// entirely when it is unavailable. Previously each of them attempted
/// elevation on its own, so an account without rights produced six failed
/// attempts per audit, every one of which the server logs and may mail to
/// root. Now it produces one.
const AUDIT_SCRIPT: &str = r#"section() { printf '\n---TERMDROP-%s rc=%s---\n' "$1" "$2"; }
if command -v timeout >/dev/null 2>&1; then TO="timeout 5"; else TO=""; fi

# Decide once how to run privileged probes. The check uses `-l`, which asks
# what this account may run, rather than attempting a trivial command: a
# sudoers entry scoped to specific commands would let a real probe succeed
# while a trivial one fails, which would silently downgrade a true verdict to
# "could not determine". Running as root skips elevation altogether, which
# also works on images that ship no sudo binary at all.
if [ "$(id -u)" = 0 ]; then PRIV=''; PRIVOK=1
elif $TO sudo -n -l >/dev/null 2>&1; then PRIV='sudo -n'; PRIVOK=1
else PRIV=''; PRIVOK=0
fi
section PRIVILEGE 0
if [ $PRIVOK -eq 1 ]; then printf 'yes'; else printf 'no'; fi

# Effective sshd config: resolves Include, Match defaults and compiled-in
# values. The unprivileged attempt is kept as a last resort because it costs
# nothing and leaves no audit trail.
out=''
if [ $PRIVOK -eq 1 ]; then
  out=$($TO $PRIV sshd -T 2>/dev/null) || out=$($TO $PRIV /usr/sbin/sshd -T 2>/dev/null) || out=''
fi
[ -n "$out" ] || out=$($TO sshd -T 2>/dev/null) || out=''
if [ -n "$out" ]; then
  section SSHD_EFFECTIVE 0; printf '%s' "$out"
else
  section SSHD_EFFECTIVE 1
fi

# Fallback: drop-ins are Included at the top of the main file, so they win.
if out=$( { cat /etc/ssh/sshd_config.d/*.conf 2>/dev/null; cat /etc/ssh/sshd_config; } 2>/dev/null ); then
  section SSHD_RAW 0; printf '%s' "$out"
else
  section SSHD_RAW 1
fi

fw=0
if [ $PRIVOK -eq 1 ] && out=$($TO $PRIV ufw status 2>/dev/null); then
  section UFW 0; printf '%s' "$out"
  case $(printf '%s' "$out" | tr 'A-Z' 'a-z') in *"status: active"*) fw=1;; esac
else
  section UFW 1
fi
if [ $fw -eq 0 ]; then
  if [ $PRIVOK -eq 1 ] && out=$($TO $PRIV firewall-cmd --state 2>/dev/null); then
    section FIREWALLD 0; printf '%s' "$out"
    case $(printf '%s' "$out" | tr 'A-Z' 'a-z') in running) fw=1;; esac
  else
    section FIREWALLD 1
  fi
fi
if [ $fw -eq 0 ]; then
  if [ $PRIVOK -eq 1 ] && raw=$($TO $PRIV nft list ruleset 2>/dev/null) && [ -n "$raw" ]; then
    section RULE_COUNT 0
    printf '%s\n' "$raw" | grep -cE '^[[:space:]]+(tcp|udp|ip|ip6|ct|iif|oif|meta|accept|drop|reject)' || printf '0'
  elif [ $PRIVOK -eq 1 ] && raw=$($TO $PRIV iptables -S 2>/dev/null); then
    section RULE_COUNT 0
    printf '%s\n' "$raw" | grep -vc '^-P' || printf '0'
  else
    section RULE_COUNT 1
  fi
fi

authlog=''
for f in /var/log/auth.log /var/log/secure; do
  if [ -r "$f" ]; then authlog=$f; break; fi
done
if [ -n "$authlog" ]; then
  section FAILED_AUTH_LOG 0
  grep -c 'Failed password' "$authlog" 2>/dev/null || printf '0'
else
  section FAILED_AUTH_LOG 1
fi

# The unit is sshd.service on RHEL and ss3h.service on Debian/Ubuntu; passing
# both ORs them, and an unreadable journal is reported rather than counted as 0.
if $TO journalctl -n 1 >/dev/null 2>&1; then
  section FAILED_JOURNAL 0
  $TO journalctl _SYSTEMD_UNIT=sshd.service _SYSTEMD_UNIT=ssh.service --since '7 days ago' 2>/dev/null | grep -c 'Failed password' || printf '0'
else
  section FAILED_JOURNAL 1
fi

if out=$(getent group sudo 2>/dev/null | cut -d: -f4); then
  section SUDO_GROUP 0; printf '%s' "$out"
else
  section SUDO_GROUP 1
fi
if out=$(getent group wheel 2>/dev/null | cut -d: -f4); then
  section WHEEL_GROUP 0; printf '%s' "$out"
else
  section WHEEL_GROUP 1
fi
if out=$(grep '^ID=' /etc/os-release 2>/dev/null | sed 's/ID=//; s/"//g'); then
  section OS_ID 0; printf '%s' "$out"
else
  section OS_ID 1
fi
printf '\n'
true
"#;

fn audit_command() -> String {
    format!("sh -c {}", shell_quote(AUDIT_SCRIPT))
}

/// Section outputs keyed by name. A section the script marked as failed, or
/// that never ran, is `None` and becomes an `unknown` verdict.
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

/// The section's output, or `None` when the probe could not run.
fn section<'a>(sections: &'a HashMap<String, Option<String>>, key: &str) -> Option<&'a str> {
    sections.get(key).and_then(|v| v.as_deref())
}

/// Build the report from parsed sections plus the separately fetched
/// updates check.
fn audit_from_sections(
    sections: &HashMap<String, Option<String>>,
    updates: SecurityCheck,
) -> SecurityReport {
    let effective = section(sections, "SSHD_EFFECTIVE");
    let raw = section(sections, "SSHD_RAW");
    // The script reports once whether it could elevate at all, so a check that
    // came back empty can say whether it was blocked or genuinely absent.
    let has_privilege = section(sections, "PRIVILEGE") == Some("yes");

    let checks = vec![
        classify_password_auth(
            &resolve_directive(effective, raw, "passwordauthentication"),
            has_privilege,
        ),
        classify_root_login(
            &resolve_directive(effective, raw, "permitrootlogin"),
            has_privilege,
        ),
        classify_ssh_port(&resolve_directive(effective, raw, "port"), has_privilege),
        classify_firewall(
            section(sections, "UFW"),
            section(sections, "FIREWALLD"),
            section(sections, "RULE_COUNT"),
            has_privilege,
        ),
        classify_failed_logins(
            section(sections, "FAILED_AUTH_LOG"),
            section(sections, "FAILED_JOURNAL"),
        ),
        classify_sudo_users(
            section(sections, "SUDO_GROUP"),
            section(sections, "WHEEL_GROUP"),
        ),
        updates,
    ];
    score_report(checks)
}

pub fn run_security_audit(session: &Session) -> Result<SecurityReport, String> {
    // One round trip for every single-command probe. If the batch itself
    // fails, every section is absent and every check reports `unknown`.
    let sections = run_command(session, &audit_command())
        .map(|out| parse_audit_sections(&out))
        .unwrap_or_default();
    let os_family = os_family_from(section(&sections, "OS_ID").unwrap_or(""));
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
    fn parses_sections_and_marks_failed_probes_unavailable() {
        let raw = sections_from(&[
            ("SSHD_RAW", 0, "PasswordAuthentication no"),
            ("UFW", 1, ""),
            ("OS_ID", 0, "ubuntu"),
        ]);
        let sections = parse_audit_sections(&raw);
        assert_eq!(
            section(&sections, "SSHD_RAW"),
            Some("PasswordAuthentication no")
        );
        assert_eq!(section(&sections, "UFW"), None, "rc=1 means unavailable");
        assert_eq!(section(&sections, "OS_ID"), Some("ubuntu"));
        assert_eq!(section(&sections, "MISSING"), None);
    }

    #[test]
    fn sshd_effective_is_preferred_over_raw_files() {
        // sshd -T reports the resolved value even when the main file disagrees,
        // which is the sshd_config.d case on Ubuntu and RHEL.
        let effective = "passwordauthentication no\nport 2222\n";
        let raw = "PasswordAuthentication yes\nPort 22\n";
        assert_eq!(
            resolve_directive(Some(effective), Some(raw), "passwordauthentication"),
            Directive::Value("no".into())
        );
        assert_eq!(
            resolve_directive(None, Some(raw), "passwordauthentication"),
            Directive::Value("yes".into())
        );
        assert_eq!(
            resolve_directive(None, Some("# nothing here\n"), "passwordauthentication"),
            Directive::NotSet
        );
        assert_eq!(
            resolve_directive(None, None, "passwordauthentication"),
            Directive::Unknown
        );
    }

    #[test]
    fn raw_lookup_is_case_insensitive_first_match_and_skips_match_blocks() {
        // sshd honors the first value it obtains, not the last.
        let raw = "  passwordauthentication no\nPasswordAuthentication yes\n";
        assert_eq!(
            sshd_raw_value(raw, "passwordauthentication").as_deref(),
            Some("no")
        );
        assert_eq!(
            sshd_raw_value("Port=2222\n", "port").as_deref(),
            Some("2222")
        );
        assert_eq!(
            sshd_raw_value(
                "Match Address 10.0.0.0/8\n  PermitRootLogin yes\n",
                "permitrootlogin"
            ),
            None,
            "conditional blocks are not the global setting"
        );
        assert_eq!(
            sshd_raw_value("#PasswordAuthentication no\n", "passwordauthentication"),
            None
        );
    }

    #[test]
    fn password_auth_classification() {
        assert_eq!(
            status(&classify_password_auth(
                &Directive::Value("no".into()),
                true
            )),
            PASS
        );
        assert_eq!(
            status(&classify_password_auth(
                &Directive::Value("yes".into()),
                true
            )),
            FAIL
        );
        // was: false pass. "notset" contains "no", so an unset directive used to
        // report as disabled. The OpenSSH default is yes, so it is enabled.
        assert_eq!(
            status(&classify_password_auth(&Directive::NotSet, true)),
            FAIL
        );
        assert_eq!(
            status(&classify_password_auth(&Directive::Unknown, true)),
            UNKNOWN
        );
    }

    #[test]
    fn root_login_classification() {
        assert_eq!(
            status(&classify_root_login(&Directive::Value("no".into()), true)),
            PASS
        );
        let keyonly = classify_root_login(&Directive::Value("prohibit-password".into()), true);
        assert_eq!(status(&keyonly), PASS);
        assert_eq!(keyonly.message, "Root login requires key authentication");
        assert_eq!(
            status(&classify_root_login(
                &Directive::Value("without-password".into()),
                true
            )),
            PASS
        );
        assert_eq!(
            status(&classify_root_login(&Directive::Value("yes".into()), true)),
            FAIL
        );
        // was: false pass, for the same "notset" reason.
        assert_eq!(status(&classify_root_login(&Directive::NotSet, true)), WARN);
        assert_eq!(
            status(&classify_root_login(&Directive::Unknown, true)),
            UNKNOWN
        );
    }

    #[test]
    fn ssh_port_classification_parses_numbers_instead_of_substrings() {
        assert_eq!(
            status(&classify_ssh_port(&Directive::Value("22".into()), true)),
            WARN
        );
        assert_eq!(
            status(&classify_ssh_port(&Directive::Value("2222".into()), true)),
            PASS
        );
        assert_eq!(
            status(&classify_ssh_port(&Directive::Value("220".into()), true)),
            PASS
        );
        // was: false warn. 8022 contains "22" but is not the default port.
        assert_eq!(
            status(&classify_ssh_port(&Directive::Value("8022".into()), true)),
            PASS
        );
        // was: false pass. 22 is listening even though 2222 contains "222".
        assert_eq!(
            status(&classify_ssh_port(
                &Directive::Value("22 2222".into()),
                true
            )),
            WARN
        );
        assert_eq!(status(&classify_ssh_port(&Directive::NotSet, true)), WARN);
        assert_eq!(
            status(&classify_ssh_port(&Directive::Unknown, true)),
            UNKNOWN
        );
        assert_eq!(
            status(&classify_ssh_port(&Directive::Value("http".into()), true)),
            UNKNOWN
        );
    }

    #[test]
    fn ufw_status_token_is_compared_exactly() {
        // was: false pass. "inactive" contains "active".
        assert_eq!(ufw_is_active("Status: inactive"), Some(false));
        assert_eq!(ufw_is_active("Status: active"), Some(true));
        assert_eq!(ufw_is_active("STATUS: ACTIVE"), Some(true));
        assert_eq!(ufw_is_active("something else"), None);
        assert_eq!(firewalld_is_running("running"), Some(true));
        assert_eq!(firewalld_is_running("not running"), Some(false));
    }

    #[test]
    fn firewall_reports_unknown_when_no_probe_could_run() {
        // was: a hard "No active firewall detected" whenever sudo was unavailable.
        assert_eq!(status(&classify_firewall(None, None, None, true)), UNKNOWN);

        assert_eq!(
            classify_firewall(Some("Status: active"), None, None, true).message,
            "UFW firewall is active"
        );
        assert_eq!(
            classify_firewall(Some("Status: inactive"), Some("running"), None, true).message,
            "firewalld is active"
        );
        // was: false pass. The old probe counted blank lines left by its filter.
        assert_eq!(
            status(&classify_firewall(
                Some("Status: inactive"),
                Some("not running"),
                Some("0"),
                true
            )),
            FAIL
        );
        assert_eq!(
            status(&classify_firewall(None, None, Some("7"), true)),
            PASS
        );
    }

    #[test]
    fn failed_logins_take_max_of_available_sources() {
        assert_eq!(status(&classify_failed_logins(Some("0"), Some("0"))), PASS);
        let warn = classify_failed_logins(Some("2"), Some("4"));
        assert_eq!(status(&warn), WARN);
        assert_eq!(warn.message, "4 recent failed login attempts");
        assert_eq!(status(&classify_failed_logins(Some("5"), None)), FAIL);
        // was: false pass. An unreadable auth.log counted as zero failures.
        assert_eq!(status(&classify_failed_logins(None, None)), UNKNOWN);
    }

    #[test]
    fn sudo_users_is_informational_and_unscored() {
        let c = classify_sudo_users(Some("alice,bob"), Some("root"));
        assert_eq!(c.detail.as_deref(), Some("sudo: alice,bob | wheel: root"));
        // was: an unconditional pass, worth a free 12.5 points on every host.
        assert_eq!(status(&c), UNKNOWN);
        assert_eq!(
            classify_sudo_users(None, None).detail.as_deref(),
            Some("No sudo/wheel group members found")
        );
    }

    #[test]
    fn security_updates_classification() {
        let ok = classify_security_updates(0, "apt");
        assert_eq!(status(&ok), PASS);
        assert_eq!(ok.message, "No pending security updates (apt)");
        let bad = classify_security_updates(7, "dnf");
        assert_eq!(status(&bad), FAIL);
        assert_eq!(bad.message, "7 security updates pending");
    }

    #[test]
    fn score_excludes_undeterminable_checks() {
        let mk = |s: &str| check("n", s, String::new(), None);
        let r = score_report(vec![mk(PASS), mk(FAIL), mk(UNKNOWN)]);
        assert_eq!((r.passed, r.scored, r.score), (1, 2, 50));

        let all_unknown = score_report(vec![mk(UNKNOWN), mk(UNKNOWN)]);
        assert_eq!(
            (all_unknown.passed, all_unknown.scored, all_unknown.score),
            (0, 0, 0)
        );

        let perfect = score_report(vec![mk(PASS), mk(PASS), mk(UNKNOWN)]);
        assert_eq!(perfect.score, 100);
    }

    #[test]
    fn parse_count_defaults_to_zero() {
        assert_eq!(parse_count(" 12 \n"), 12);
        assert_eq!(parse_count(""), 0);
        assert_eq!(parse_count("x"), 0);
    }

    #[test]
    fn os_family_is_trimmed_and_lowercased() {
        assert_eq!(os_family_from("Ubuntu\n"), "ubuntu");
        assert_eq!(os_family_from(""), "");
    }

    #[test]
    fn audit_command_is_a_single_quoted_sh_invocation() {
        let cmd = audit_command();
        assert!(cmd.starts_with("sh -c '"));
        assert!(cmd.ends_with("'"));
        assert_eq!(shell_quote("it's"), "'it'\\''s'");
        for needle in [
            "sshd -T",
            "sshd_config.d",
            "$PRIV ufw status",
            "firewall-cmd --state",
            "nft list ruleset",
            "iptables -S",
            "/var/log/secure",
            "_SYSTEMD_UNIT=ssh.service",
            "getent group wheel",
        ] {
            assert!(cmd.contains(needle), "{}", needle);
        }
        // Elevation is attempted exactly once per audit. These assertions are
        // the guarantee: a probe that elevates on its own instead of going
        // through $PRIV brings back the six-failed-attempts-per-audit
        // behaviour, and the server logs every one of those attempts.
        assert!(
            cmd.contains("sudo -n -l"),
            "one capability check, non-interactive"
        );
        for probe in [
            "sudo -n sshd",
            "sudo -n /usr/sbin/sshd",
            "sudo -n ufw",
            "sudo -n firewall-cmd",
            "sudo -n nft",
            "sudo -n iptables",
        ] {
            assert!(!cmd.contains(probe), "{} must go through $PRIV", probe);
        }
        // Disk usage is the status bar's job, not a security finding.
        assert!(!cmd.contains("df -P"));
    }

    #[test]
    fn full_report_from_a_realistic_unprivileged_host() {
        // No sudo, so every privileged probe is unavailable; the report must
        // say so rather than claiming the host is fine.
        let raw = sections_from(&[
            ("PRIVILEGE", 0, "no"),
            ("SSHD_EFFECTIVE", 1, ""),
            ("SSHD_RAW", 0, "PasswordAuthentication yes\nPort 22\n"),
            ("UFW", 1, ""),
            ("FIREWALLD", 1, ""),
            ("RULE_COUNT", 1, ""),
            ("FAILED_AUTH_LOG", 1, ""),
            ("FAILED_JOURNAL", 1, ""),
            ("SUDO_GROUP", 0, "alice"),
            ("WHEEL_GROUP", 1, ""),
            ("OS_ID", 0, "ubuntu"),
        ]);
        let sections = parse_audit_sections(&raw);
        let report = audit_from_sections(&sections, classify_security_updates(0, "apt"));

        let by_name = |n: &str| -> SecurityCheck {
            report.checks.iter().find(|c| c.name == n).unwrap().clone()
        };
        assert_eq!(status(&by_name("SSH Password Authentication")), FAIL);
        assert_eq!(status(&by_name("SSH Port")), WARN);
        assert_eq!(status(&by_name("Firewall")), UNKNOWN);
        assert_eq!(status(&by_name("Failed Login Attempts")), UNKNOWN);
        assert_eq!(status(&by_name("Security Updates")), PASS);
        // The firewall's detail must say elevation was the blocker, not imply
        // the host has no firewall.
        assert!(by_name("Firewall").detail.unwrap().contains("no elevation"));
        // Determinable: password auth, root login, port, updates.
        assert_eq!(report.scored, 4);
        assert_eq!(report.passed, 1);
        assert_eq!(report.score, 25);
    }
}
