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

/// What to do about a finding.
///
/// `command` is present only when a single-line, non-destructive command
/// exists, and is safe to insert into a live shell for the user to review.
/// Config edits and service restarts belong in `summary`, never here: the
/// panel can put `command` straight into a root-capable terminal.
///
/// Every command is a `&'static str` or is chosen from a closed set of
/// distributions. **Remote output must never reach a command string**, or a
/// value read off the audited host would end up as text in that terminal.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Remediation {
    pub summary: String,
    pub command: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SecurityCheck {
    pub name: String,
    pub status: String,
    pub message: String,
    pub detail: Option<String>,
    pub remediation: Option<Remediation>,
}

impl SecurityCheck {
    fn with_fix(mut self, summary: &str, command: Option<&'static str>) -> Self {
        self.remediation = Some(Remediation {
            summary: summary.to_string(),
            command: command.map(String::from),
        });
        self
    }
}

/// The distributions whose command names differ. Everything that varies by
/// distribution is resolved through this, so remediation text is picked from a
/// closed set rather than built from whatever the host reported.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Family {
    Debian,
    Rhel,
    Suse,
    Arch,
    Alpine,
    Unknown,
}

fn family_of(os_id: &str) -> Family {
    match os_id {
        "ubuntu" | "debian" | "linuxmint" | "pop" | "elementary" | "zorin" => Family::Debian,
        "centos" | "rhel" | "fedora" | "rocky" | "almalinux" | "amazon" | "amzn" => Family::Rhel,
        "opensuse" | "opensuse-leap" | "opensuse-tumbleweed" | "sles" => Family::Suse,
        "arch" | "manjaro" | "endeavouros" => Family::Arch,
        "alpine" => Family::Alpine,
        _ => Family::Unknown,
    }
}

/// The systemd unit that runs sshd. Debian and Ubuntu call it `ssh`.
fn ssh_unit(family: Family) -> &'static str {
    match family {
        Family::Debian => "ssh",
        _ => "sshd",
    }
}

/// A read-only command that shows the recent failed authentication lines.
/// The whole command is a literal per family rather than a path spliced into
/// a format string, so no value read off the host can reach it.
fn failed_login_inspect(family: Family) -> &'static str {
    match family {
        Family::Rhel | Family::Suse => {
            "sudo grep -i 'authentication failure\\|failed password' /var/log/secure | tail -n 50"
        }
        _ => {
            "sudo grep -i 'authentication failure\\|failed password' /var/log/auth.log | tail -n 50"
        }
    }
}

/// Where the sshd directives actually come from. Both Debian and RHEL ship an
/// `Include /etc/ssh/sshd_config.d/*.conf` at the top of the main file, and
/// sshd keeps the *first* value it reads, so a low-numbered drop-in beats the
/// main file. Editing blind is how a change silently does nothing.
const LOCATE_PASSWORD_AUTH: &str =
    "sudo grep -rniE '^[[:space:]]*passwordauthentication' /etc/ssh/sshd_config /etc/ssh/sshd_config.d/";
const LOCATE_ROOT_LOGIN: &str =
    "sudo grep -rniE '^[[:space:]]*permitrootlogin' /etc/ssh/sshd_config /etc/ssh/sshd_config.d/";
const SHOW_PASSWORD_AUTH: &str = "sudo sshd -T | grep -i passwordauthentication";
const SHOW_ROOT_LOGIN: &str = "sudo sshd -T | grep -i permitrootlogin";
const SHOW_LISTENERS: &str = "sudo ss -tlnp | grep -i sshd";
const SHOW_SUDOERS: &str = "getent group sudo wheel";

/// Advice for a check that came back undetermined for lack of rights.
const NEEDS_ELEVATION: &str =
    "Re-run the audit from an account with sudo, or run this yourself on a shell that has it.";

/// A read-only command that shows the current firewall rules.
fn firewall_inspect(family: Family) -> &'static str {
    match family {
        Family::Debian => "sudo ufw status verbose",
        Family::Rhel | Family::Suse => "sudo firewall-cmd --list-all",
        _ => "sudo nft list ruleset",
    }
}

/// Package manager name, the command that counts pending security updates,
/// and the command that applies them. `None` when the family is unrecognized.
fn package_manager(family: Family) -> Option<(&'static str, &'static str, &'static str)> {
    match family {
        Family::Debian => Some((
            "apt",
            "apt list --upgradable 2>/dev/null | grep -c security || echo 0",
            "sudo apt-get update && sudo apt-get upgrade",
        )),
        Family::Rhel => Some((
            "dnf",
            // `check-update --security` lists packages, not the word "security",
            // so the old grep always counted zero. updateinfo lists advisories.
            "dnf -q updateinfo list --security 2>/dev/null | grep -c . || echo 0",
            "sudo dnf upgrade --security",
        )),
        Family::Alpine => Some((
            "apk",
            "apk upgrade --simulate 2>/dev/null | grep -c Upgrading || echo 0",
            "sudo apk upgrade",
        )),
        Family::Arch => Some((
            "pacman",
            "pacman -Qu 2>/dev/null | grep -c . || echo 0",
            "sudo pacman -Syu",
        )),
        Family::Suse => Some((
            "zypper",
            "zypper --quiet list-patches --category security 2>/dev/null | grep -c ' | ' || echo 0",
            "sudo zypper patch --category security",
        )),
        Family::Unknown => None,
    }
}

/// What the audit knew about the host while classifying it.
#[derive(Debug, Clone, Copy)]
struct Ctx {
    family: Family,
    has_privilege: bool,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SecurityReport {
    /// Unix seconds at which the audit ran. The report is cached on both
    /// sides, so "updated N ago" has to come from the run rather than from
    /// when the panel happened to read it.
    pub generated_at: u64,
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
        remediation: None,
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

fn classify_password_auth(directive: &Directive, ctx: Ctx) -> SecurityCheck {
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
        )
        .with_fix(&password_auth_fix(ctx.family), Some(LOCATE_PASSWORD_AUTH)),
        // OpenSSH defaults this to "yes", so an absent directive means
        // password login is accepted.
        Directive::NotSet => check(
            name,
            FAIL,
            "Password authentication is enabled".to_string(),
            Some("Not set; the OpenSSH default is yes".to_string()),
        )
        .with_fix(&password_auth_fix(ctx.family), Some(LOCATE_PASSWORD_AUTH)),
        Directive::Unknown => check(
            name,
            UNKNOWN,
            "Could not determine".to_string(),
            Some(unreadable_detail(SSHD_UNREADABLE, ctx.has_privilege)),
        )
        .with_fix(NEEDS_ELEVATION, Some(SHOW_PASSWORD_AUTH)),
    }
}

fn password_auth_fix(family: Family) -> String {
    format!(
        "Confirm every account has a working key, then set PasswordAuthentication no \
         and reload the service with `systemctl reload {}`. Find the file that sets it \
         first: sshd keeps the first value it reads, so a drop-in in sshd_config.d can \
         override an edit to the main file and leave the setting unchanged.",
        ssh_unit(family)
    )
}

fn classify_root_login(directive: &Directive, ctx: Ctx) -> SecurityCheck {
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
        )
        .with_fix(&root_login_fix(ctx.family), Some(LOCATE_ROOT_LOGIN)),
        // The OpenSSH default is prohibit-password, which is acceptable, but
        // some distributions have shipped a different default.
        Directive::NotSet => check(
            name,
            WARN,
            "Using the default (prohibit-password)".to_string(),
            Some("Not set; set PermitRootLogin explicitly to be sure".to_string()),
        )
        .with_fix(&root_login_fix(ctx.family), Some(LOCATE_ROOT_LOGIN)),
        Directive::Unknown => check(
            name,
            UNKNOWN,
            "Could not determine".to_string(),
            Some(unreadable_detail(SSHD_UNREADABLE, ctx.has_privilege)),
        )
        .with_fix(NEEDS_ELEVATION, Some(SHOW_ROOT_LOGIN)),
    }
}

fn root_login_fix(family: Family) -> String {
    format!(
        "Set PermitRootLogin to no, or to prohibit-password if root needs key access, \
         then reload the service with `systemctl reload {}`. Check that you can still \
         reach a sudo-capable account before you disconnect. As with the other sshd \
         directives, the first file that sets it wins.",
        ssh_unit(family)
    )
}

/// Ports sshd listens on, parsed from one or more `Port` values.
fn parse_ports(value: &str) -> Vec<u16> {
    value
        .split_whitespace()
        .filter_map(|p| p.parse::<u16>().ok())
        .collect()
}

fn classify_ssh_port(directive: &Directive, ctx: Ctx) -> SecurityCheck {
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
                Some(unreadable_detail(SSHD_UNREADABLE, ctx.has_privilege)),
            )
            .with_fix(NEEDS_ELEVATION, Some(SHOW_LISTENERS))
        }
    };

    let ports = parse_ports(&value);
    if ports.is_empty() {
        return check(
            name,
            UNKNOWN,
            "Could not determine".to_string(),
            Some(format!("Unrecognized Port value: {}", value)),
        )
        .with_fix(
            "The Port directive did not parse as a list of port numbers. Check what sshd \
             is actually listening on.",
            Some(SHOW_LISTENERS),
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
        .with_fix(
            "A non-default port only quiets untargeted scanners; it is not a substitute \
             for key-only authentication. If you move it, open the new port in the \
             firewall first and confirm a second login works before closing this session.",
            Some(SHOW_LISTENERS),
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
    ctx: Ctx,
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
        .with_fix(
            "Nothing is filtering inbound traffic. Enabling a default-deny firewall from \
             this SSH session will lock you out unless you allow the SSH port first, so \
             add that rule, then enable it, and keep this session open while you test a \
             second login. Look at what is configured now before changing anything.",
            Some(firewall_inspect(ctx.family)),
        )
    } else {
        check(
            name,
            UNKNOWN,
            "Could not determine".to_string(),
            Some(unreadable_detail(
                "Firewall status could not be read",
                ctx.has_privilege,
            )),
        )
        .with_fix(NEEDS_ELEVATION, Some(firewall_inspect(ctx.family)))
    }
}

// ---------------------------------------------------------------------------
// Remaining checks
// ---------------------------------------------------------------------------

fn classify_failed_logins(
    auth_log: Option<&str>,
    journal: Option<&str>,
    ctx: Ctx,
) -> SecurityCheck {
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
        )
        .with_fix(NEEDS_ELEVATION, Some(failed_login_inspect(ctx.family)));
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
        .with_fix(
            "A handful of failures is usually a mistyped password rather than an attack. \
             Read the lines and see which account and source address they came from.",
            Some(failed_login_inspect(ctx.family)),
        )
    } else {
        check(
            name,
            FAIL,
            format!("{} recent failed login attempts", total),
            Some("Potential brute-force attack — consider fail2ban".to_string()),
        )
        .with_fix(
            "This volume usually means an automated attack. Read the lines to see which \
             accounts are being tried, then disable password authentication so the \
             attempts cannot succeed, and install fail2ban to cut the noise.",
            Some(failed_login_inspect(ctx.family)),
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
    .with_fix(
        "Informational, so it is not scored. Check that every account listed still needs \
         elevation, and that none of them is a shared or service account.",
        Some(SHOW_SUDOERS),
    )
}

fn os_family_from(output: &str) -> String {
    output.trim().to_lowercase()
}

/// Runs one package-manager command chosen by the OS family, so it stays
/// outside the batched script.
fn check_security_updates(session: &Session, family: Family) -> SecurityCheck {
    let Some((manager, count_cmd, upgrade_cmd)) = package_manager(family) else {
        return check(
            "Security Updates",
            UNKNOWN,
            "Could not determine".to_string(),
            Some("No supported package manager was detected".to_string()),
        )
        .with_fix(
            "The audit did not recognize this distribution, so it could not count pending \
             updates. Check for them the way this system normally does.",
            None,
        );
    };

    match run_command(session, count_cmd) {
        Ok(text) => classify_security_updates(parse_count(&text), manager, upgrade_cmd),
        Err(_) => check(
            "Security Updates",
            UNKNOWN,
            "Could not determine".to_string(),
            Some(format!("The {} query did not run", manager)),
        )
        .with_fix(
            "The update query did not run, so this is not a verdict. The package \
             manager's own upgrade command lists what is pending and prompts before \
             it changes anything.",
            Some(upgrade_cmd),
        ),
    }
}

fn classify_security_updates(
    total: i32,
    manager: &str,
    upgrade_cmd: &'static str,
) -> SecurityCheck {
    let name = "Security Updates";
    if total == 0 {
        check(
            name,
            PASS,
            format!("No pending security updates ({})", manager),
            None,
        )
    } else {
        check(
            name,
            FAIL,
            format!("{} security updates pending", total),
            Some(format!("Run {} upgrade to patch", manager)),
        )
        .with_fix(
            "Applying updates can restart services and, for a kernel update, needs a \
             reboot to take effect. The command is written to the terminal without a \
             newline, so review it and confirm the package list before you run it.",
            Some(upgrade_cmd),
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
        // Stamped by the caller that actually ran the probes, so scoring stays
        // a pure function of the checks.
        generated_at: 0,
        score,
        passed,
        scored,
        checks,
    }
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
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
/// The distribution and the elevation the script reported, which together
/// decide both the wording of a verdict and which remediation command applies.
/// The script reports elevation once, so a check that came back empty can say
/// whether it was blocked or the setting is genuinely absent.
fn ctx_from_sections(sections: &HashMap<String, Option<String>>) -> Ctx {
    Ctx {
        family: family_of(&os_family_from(section(sections, "OS_ID").unwrap_or(""))),
        has_privilege: section(sections, "PRIVILEGE") == Some("yes"),
    }
}

fn audit_from_sections(
    sections: &HashMap<String, Option<String>>,
    updates: SecurityCheck,
) -> SecurityReport {
    let effective = section(sections, "SSHD_EFFECTIVE");
    let raw = section(sections, "SSHD_RAW");
    let ctx = ctx_from_sections(sections);

    let checks = vec![
        classify_password_auth(
            &resolve_directive(effective, raw, "passwordauthentication"),
            ctx,
        ),
        classify_root_login(&resolve_directive(effective, raw, "permitrootlogin"), ctx),
        classify_ssh_port(&resolve_directive(effective, raw, "port"), ctx),
        classify_firewall(
            section(sections, "UFW"),
            section(sections, "FIREWALLD"),
            section(sections, "RULE_COUNT"),
            ctx,
        ),
        classify_failed_logins(
            section(sections, "FAILED_AUTH_LOG"),
            section(sections, "FAILED_JOURNAL"),
            ctx,
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
    let updates = check_security_updates(session, ctx_from_sections(&sections).family);
    let mut report = audit_from_sections(&sections, updates);
    report.generated_at = unix_now();
    Ok(report)
}

#[cfg(test)]
mod tests {

    // -- remediation --------------------------------------------------------

    const FAMILIES: [Family; 6] = [
        Family::Debian,
        Family::Rhel,
        Family::Suse,
        Family::Arch,
        Family::Alpine,
        Family::Unknown,
    ];

    /// Every command string the module is allowed to emit. The list is written
    /// out by hand so that adding a command means deliberately adding it here.
    fn allowed_commands() -> Vec<&'static str> {
        let mut all = vec![
            LOCATE_PASSWORD_AUTH,
            LOCATE_ROOT_LOGIN,
            SHOW_PASSWORD_AUTH,
            SHOW_ROOT_LOGIN,
            SHOW_LISTENERS,
            SHOW_SUDOERS,
        ];
        for f in FAMILIES {
            all.push(firewall_inspect(f));
            all.push(failed_login_inspect(f));
            if let Some((_, _, upgrade)) = package_manager(f) {
                all.push(upgrade);
            }
        }
        all
    }

    /// Produces a check for every branch that can carry a remediation, across
    /// every family and both elevation states.
    fn every_check() -> Vec<SecurityCheck> {
        // Values a hostile or broken host could report. If any of these ever
        // reached a command string, this is where it would show up.
        let hostile = "yes\n$(id)`id`; rm -rf / |tee /etc/passwd";
        let values = [
            Directive::Value("no".into()),
            Directive::Value("yes".into()),
            Directive::Value("prohibit-password".into()),
            Directive::Value(hostile.into()),
            Directive::Value("22 2222".into()),
            Directive::NotSet,
            Directive::Unknown,
        ];
        let mut out = Vec::new();
        for family in FAMILIES {
            for has_privilege in [true, false] {
                let ctx = Ctx {
                    family,
                    has_privilege,
                };
                for v in &values {
                    out.push(classify_password_auth(v, ctx));
                    out.push(classify_root_login(v, ctx));
                    out.push(classify_ssh_port(v, ctx));
                }
                for probe in [None, Some("Status: inactive"), Some(hostile)] {
                    out.push(classify_firewall(probe, probe, probe, ctx));
                }
                for count in [None, Some("0"), Some("2"), Some("40"), Some(hostile)] {
                    out.push(classify_failed_logins(count, count, ctx));
                    out.push(classify_sudo_users(count, count));
                }
                if let Some((manager, _, upgrade)) = package_manager(family) {
                    out.push(classify_security_updates(0, manager, upgrade));
                    out.push(classify_security_updates(9, manager, upgrade));
                }
            }
        }
        out
    }

    /// The load-bearing invariant of the whole remediation feature: a command
    /// can be typed into a live, often root-capable shell, so no value read off
    /// the audited host may reach one. Enforced by requiring every emitted
    /// command to be one of the module's own literals.
    #[test]
    fn remediation_commands_never_carry_host_output() {
        let allowed = allowed_commands();
        for c in every_check() {
            let Some(cmd) = c.remediation.as_ref().and_then(|r| r.command.as_deref()) else {
                continue;
            };
            assert!(
                allowed.contains(&cmd),
                "{} produced a command that is not a module literal: {:?}",
                c.name,
                cmd
            );
        }
    }

    /// The literals themselves have to be safe to insert: one line, nothing
    /// that mutates state beyond the package upgrades, and short enough that
    /// the user can read the whole thing before pressing Enter.
    #[test]
    fn remediation_commands_are_safe_to_insert() {
        // Enabling a default-deny firewall over SSH is the classic lockout, and
        // restarting sshd from the session it serves is the other one. Both are
        // described in prose instead.
        const FORBIDDEN: [&str; 12] = [
            "rm ",
            "sed -i",
            "tee ",
            ">",
            "systemctl restart",
            "systemctl stop",
            "reboot",
            "ufw enable",
            "ufw disable",
            "--assume-yes",
            "--noconfirm",
            " -y",
        ];
        for cmd in allowed_commands() {
            assert!(!cmd.is_empty());
            assert!(cmd.len() <= 512, "too long to review: {:?}", cmd);
            assert!(
                !cmd.chars().any(|c| c.is_control()),
                "control character in {:?}",
                cmd
            );
            for bad in FORBIDDEN {
                assert!(!cmd.contains(bad), "{:?} contains {:?}", cmd, bad);
            }
        }
    }

    /// Without elevation the sshd probes cannot answer, and the panel has to
    /// say why rather than implying the host is misconfigured.
    #[test]
    fn undetermined_checks_name_the_missing_elevation() {
        for c in [
            classify_password_auth(&Directive::Unknown, NOPRIV),
            classify_root_login(&Directive::Unknown, NOPRIV),
            classify_ssh_port(&Directive::Unknown, NOPRIV),
            classify_firewall(None, None, None, NOPRIV),
        ] {
            assert_eq!(c.status, UNKNOWN);
            assert!(
                c.detail.as_deref().unwrap_or("").contains("no elevation"),
                "{} does not explain the missing elevation",
                c.name
            );
        }
        // With elevation the same probes failing means something else, so the
        // detail must not blame permissions.
        let elevated = classify_firewall(None, None, None, PRIV);
        assert!(!elevated.detail.unwrap().contains("no elevation"));
    }

    /// A failing check is the one a user most needs to act on, so it must not
    /// be the one left without advice.
    #[test]
    fn failing_and_undetermined_checks_carry_advice() {
        for c in every_check() {
            if c.status == FAIL || c.status == UNKNOWN {
                assert!(
                    c.remediation
                        .as_ref()
                        .is_some_and(|r| !r.summary.is_empty()),
                    "{} is {} with no remediation",
                    c.name,
                    c.status
                );
            }
        }
    }

    use super::*;

    /// The two contexts the classifiers branch on. The family only selects
    /// wording and remediation commands, so it is fixed here.
    const PRIV: Ctx = Ctx {
        family: Family::Debian,
        has_privilege: true,
    };
    const NOPRIV: Ctx = Ctx {
        family: Family::Debian,
        has_privilege: false,
    };

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
                PRIV
            )),
            PASS
        );
        assert_eq!(
            status(&classify_password_auth(
                &Directive::Value("yes".into()),
                PRIV
            )),
            FAIL
        );
        // was: false pass. "notset" contains "no", so an unset directive used to
        // report as disabled. The OpenSSH default is yes, so it is enabled.
        assert_eq!(
            status(&classify_password_auth(&Directive::NotSet, PRIV)),
            FAIL
        );
        assert_eq!(
            status(&classify_password_auth(&Directive::Unknown, PRIV)),
            UNKNOWN
        );
    }

    #[test]
    fn root_login_classification() {
        assert_eq!(
            status(&classify_root_login(&Directive::Value("no".into()), PRIV)),
            PASS
        );
        let keyonly = classify_root_login(&Directive::Value("prohibit-password".into()), PRIV);
        assert_eq!(status(&keyonly), PASS);
        assert_eq!(keyonly.message, "Root login requires key authentication");
        assert_eq!(
            status(&classify_root_login(
                &Directive::Value("without-password".into()),
                PRIV
            )),
            PASS
        );
        assert_eq!(
            status(&classify_root_login(&Directive::Value("yes".into()), PRIV)),
            FAIL
        );
        // was: false pass, for the same "notset" reason.
        assert_eq!(status(&classify_root_login(&Directive::NotSet, PRIV)), WARN);
        assert_eq!(
            status(&classify_root_login(&Directive::Unknown, PRIV)),
            UNKNOWN
        );
    }

    #[test]
    fn ssh_port_classification_parses_numbers_instead_of_substrings() {
        assert_eq!(
            status(&classify_ssh_port(&Directive::Value("22".into()), PRIV)),
            WARN
        );
        assert_eq!(
            status(&classify_ssh_port(&Directive::Value("2222".into()), PRIV)),
            PASS
        );
        assert_eq!(
            status(&classify_ssh_port(&Directive::Value("220".into()), PRIV)),
            PASS
        );
        // was: false warn. 8022 contains "22" but is not the default port.
        assert_eq!(
            status(&classify_ssh_port(&Directive::Value("8022".into()), PRIV)),
            PASS
        );
        // was: false pass. 22 is listening even though 2222 contains "222".
        assert_eq!(
            status(&classify_ssh_port(
                &Directive::Value("22 2222".into()),
                PRIV
            )),
            WARN
        );
        assert_eq!(status(&classify_ssh_port(&Directive::NotSet, PRIV)), WARN);
        assert_eq!(
            status(&classify_ssh_port(&Directive::Unknown, PRIV)),
            UNKNOWN
        );
        assert_eq!(
            status(&classify_ssh_port(&Directive::Value("http".into()), PRIV)),
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
        assert_eq!(status(&classify_firewall(None, None, None, PRIV)), UNKNOWN);

        assert_eq!(
            classify_firewall(Some("Status: active"), None, None, PRIV).message,
            "UFW firewall is active"
        );
        assert_eq!(
            classify_firewall(Some("Status: inactive"), Some("running"), None, PRIV).message,
            "firewalld is active"
        );
        // was: false pass. The old probe counted blank lines left by its filter.
        assert_eq!(
            status(&classify_firewall(
                Some("Status: inactive"),
                Some("not running"),
                Some("0"),
                PRIV
            )),
            FAIL
        );
        assert_eq!(
            status(&classify_firewall(None, None, Some("7"), PRIV)),
            PASS
        );
    }

    #[test]
    fn failed_logins_take_max_of_available_sources() {
        assert_eq!(
            status(&classify_failed_logins(Some("0"), Some("0"), PRIV)),
            PASS
        );
        let warn = classify_failed_logins(Some("2"), Some("4"), PRIV);
        assert_eq!(status(&warn), WARN);
        assert_eq!(warn.message, "4 recent failed login attempts");
        assert_eq!(status(&classify_failed_logins(Some("5"), None, PRIV)), FAIL);
        // was: false pass. An unreadable auth.log counted as zero failures.
        assert_eq!(status(&classify_failed_logins(None, None, PRIV)), UNKNOWN);
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
        let ok = classify_security_updates(0, "apt", "sudo apt-get upgrade");
        assert_eq!(status(&ok), PASS);
        assert_eq!(ok.message, "No pending security updates (apt)");
        let bad = classify_security_updates(7, "dnf", "sudo dnf upgrade --security");
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
        let report = audit_from_sections(
            &sections,
            classify_security_updates(0, "apt", "sudo apt-get upgrade"),
        );

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
