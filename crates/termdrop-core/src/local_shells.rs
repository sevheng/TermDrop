//! Which shells this machine has, for the local terminal's shell menu.
//!
//! Everything that decides is pure over its inputs (file text, env values,
//! "does this exist" closures), so both platforms' lists are tested from
//! either. Only `detect` touches the system.
//!
//! The frontend sends back an `id`, never a program: `local_connect` looks the
//! id up in this list, so the only things it can start are shells found here.

use serde::Serialize;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LocalShell {
    pub id: String,
    pub label: String,
    pub program: String,
    pub args: Vec<String>,
    /// The one Ctrl+Shift+T opens. Exactly one entry has it.
    pub default: bool,
}

/// Listed in `/etc/shells` on some distros, but not something to open a tab in.
const NOT_INTERACTIVE: &[&str] = &["nologin", "false", "git-shell", "rbash", "tmux", "screen"];

/// WSL distros that Docker Desktop installs for its own use. They have no
/// user shell worth opening.
const HIDDEN_DISTROS: &[&str] = &["docker-desktop", "docker-desktop-data"];

/// `wsl.exe -l -q` can hang when the WSL service is wedged; the menu must not.
const WSL_LIST_TIMEOUT: Duration = Duration::from_secs(3);

fn basename(path: &str) -> &str {
    path.rsplit(['/', '\\']).next().unwrap_or(path)
}

/// Paths listed in `/etc/shells`, in order, without comments or blanks.
fn parse_etc_shells(text: &str) -> Vec<&str> {
    text.lines()
        .map(|l| l.split('#').next().unwrap_or("").trim())
        .filter(|l| !l.is_empty())
        .collect()
}

/// Unix shells: `/etc/shells` filtered to ones that exist, deduplicated by
/// where they really live (`/bin` is a symlink to `/usr/bin` on most distros,
/// so every shell is listed twice), plus `$SHELL` if it is not listed.
///
/// `canonical` returns `None` for a path that does not exist. The default is
/// `$SHELL` and goes first; each is started as a login shell, which is what
/// restores the user's PATH for a GUI app on macOS.
fn unix_shells(
    etc_shells: &str,
    shell_env: Option<&str>,
    canonical: impl Fn(&str) -> Option<PathBuf>,
) -> Vec<LocalShell> {
    let shell_env = shell_env.filter(|s| !s.trim().is_empty());
    let default_real = shell_env.and_then(&canonical);

    let mut seen: Vec<PathBuf> = Vec::new();
    let mut found: Vec<(String, bool)> = Vec::new();
    let candidates = shell_env.into_iter().chain(parse_etc_shells(etc_shells));
    for path in candidates {
        if NOT_INTERACTIVE.contains(&basename(path)) {
            continue;
        }
        let Some(real) = canonical(path) else {
            continue;
        };
        if seen.contains(&real) {
            continue;
        }
        let is_default = default_real.as_ref() == Some(&real);
        seen.push(real);
        found.push((path.to_string(), is_default));
    }

    if found.is_empty() {
        found.push(("/bin/sh".to_string(), true));
    } else if !found.iter().any(|(_, d)| *d) {
        found[0].1 = true;
    }

    // Two different binaries with one name (a Homebrew bash beside the
    // system's) are told apart by path.
    let label_for = |path: &str| {
        let name = basename(path);
        let clashes = found.iter().filter(|(p, _)| basename(p) == name).count() > 1;
        if clashes {
            path.to_string()
        } else {
            name.to_string()
        }
    };

    found
        .iter()
        .map(|(path, is_default)| LocalShell {
            id: format!("unix:{}", path),
            label: label_for(path),
            program: path.clone(),
            args: vec!["-l".to_string()],
            default: *is_default,
        })
        .collect()
}

/// The distro names from `wsl.exe -l -q`.
///
/// wsl.exe writes **UTF-16LE** to a pipe (unless `WSL_UTF8=1` is set, when it
/// writes UTF-8), often with a BOM and `\r\n`. Reading it as UTF-8 gives a
/// NUL between every letter, so the encoding is decided by whether NULs are
/// present.
fn parse_wsl_list(bytes: &[u8]) -> Vec<String> {
    let text = if bytes.contains(&0) {
        let units: Vec<u16> = bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|&pair| u16::from_le_bytes(pair))
            .collect();
        String::from_utf16_lossy(&units)
    } else {
        String::from_utf8_lossy(bytes).into_owned()
    };
    text.trim_start_matches('\u{feff}')
        .lines()
        .map(|l| l.trim_matches(|c: char| c.is_whitespace() || c == '\0'))
        .filter(|l| !l.is_empty() && !HIDDEN_DISTROS.contains(&l.to_ascii_lowercase().as_str()))
        .map(str::to_string)
        .collect()
}

/// Windows shells, in the order the menu shows them: PowerShell 7, Windows
/// PowerShell, Command Prompt, Git Bash, then one entry per WSL distro. The
/// default is the first of the PowerShells found, else cmd.
fn windows_shells(
    on_path: impl Fn(&str) -> bool,
    comspec: Option<&str>,
    git_bash: Option<String>,
    distros: &[String],
) -> Vec<LocalShell> {
    let mut shells = Vec::new();
    let mut push = |id: &str, label: &str, program: &str, args: &[&str]| {
        shells.push(LocalShell {
            id: id.to_string(),
            label: label.to_string(),
            program: program.to_string(),
            args: args.iter().map(|a| a.to_string()).collect(),
            default: false,
        });
    };

    if on_path("pwsh.exe") {
        push("pwsh", "PowerShell", "pwsh.exe", &["-NoLogo"]);
    }
    if on_path("powershell.exe") {
        push(
            "powershell",
            "Windows PowerShell",
            "powershell.exe",
            &["-NoLogo"],
        );
    }
    let cmd = comspec
        .filter(|c| !c.trim().is_empty())
        .unwrap_or("cmd.exe");
    push("cmd", "Command Prompt", cmd, &[]);
    if let Some(bash) = git_bash {
        push("git-bash", "Git Bash", &bash, &["--login", "-i"]);
    }
    for distro in distros {
        push(
            &format!("wsl:{}", distro),
            &format!("{} (WSL)", distro),
            "wsl.exe",
            &["-d", distro, "--cd", "~"],
        );
    }

    if let Some(first) = shells.first_mut() {
        first.default = true;
    }
    shells
}

/// Where Git for Windows puts bash. `bash.exe` on PATH is deliberately not
/// consulted: on most machines that is `System32\bash.exe`, the WSL launcher.
fn find_git_bash(
    env: impl Fn(&str) -> Option<String>,
    exists: impl Fn(&Path) -> bool,
) -> Option<String> {
    let roots = [
        env("ProgramFiles").map(PathBuf::from),
        env("ProgramW6432").map(PathBuf::from),
        env("LOCALAPPDATA").map(|p| PathBuf::from(p).join("Programs")),
    ];
    roots
        .into_iter()
        .flatten()
        .map(|root| root.join("Git").join("bin").join("bash.exe"))
        .find(|p| exists(p))
        .map(|p| p.to_string_lossy().into_owned())
}

fn on_path(program: &str) -> bool {
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&path).any(|dir| dir.join(program).is_file())
}

/// Run `wsl.exe -l -q` without flashing a console window, giving up after
/// `WSL_LIST_TIMEOUT`. No WSL, or a broken one, is an empty list.
fn wsl_distros() -> Vec<String> {
    let mut cmd = Command::new("wsl.exe");
    cmd.args(["-l", "-q"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let Ok(mut child) = cmd.spawn() else {
        return Vec::new();
    };
    let deadline = Instant::now() + WSL_LIST_TIMEOUT;
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => break,
            Ok(Some(_)) | Err(_) => return Vec::new(),
            Ok(None) if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                tracing::warn!("wsl.exe -l timed out; listing no WSL distros");
                return Vec::new();
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
        }
    }
    let mut out = Vec::new();
    if let Some(mut stdout) = child.stdout.take() {
        let _ = std::io::Read::read_to_end(&mut stdout, &mut out);
    }
    parse_wsl_list(&out)
}

/// Every shell on this machine. Blocking (it may run `wsl.exe`), so call it
/// off the async runtime.
pub fn detect() -> Vec<LocalShell> {
    if cfg!(windows) {
        let env = |k: &str| std::env::var(k).ok();
        windows_shells(
            on_path,
            env("COMSPEC").as_deref(),
            find_git_bash(env, Path::is_file),
            &wsl_distros(),
        )
    } else {
        let etc_shells = std::fs::read_to_string("/etc/shells").unwrap_or_default();
        unix_shells(&etc_shells, std::env::var("SHELL").ok().as_deref(), |p| {
            std::fs::canonicalize(p).ok()
        })
    }
}

/// The shell Ctrl+Shift+T opens, without the slow parts of `detect`: the
/// default is never Git Bash or a WSL distro, so `wsl.exe` need not run.
pub fn default_shell() -> LocalShell {
    let shells = if cfg!(windows) {
        windows_shells(on_path, std::env::var("COMSPEC").ok().as_deref(), None, &[])
    } else {
        detect()
    };
    default_of(&shells)
        .cloned()
        .expect("both platforms always list at least one shell")
}

/// The default entry of a list.
pub fn default_of(shells: &[LocalShell]) -> Option<&LocalShell> {
    shells.iter().find(|s| s.default).or(shells.first())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A filesystem where `/bin` links to `/usr/bin`, as on most distros.
    fn usrmerge(existing: &'static [&'static str]) -> impl Fn(&str) -> Option<PathBuf> {
        move |p: &str| {
            let real = p
                .replacen("/bin/", "/usr/bin/", 1)
                .replace("/usr/usr/", "/usr/");
            existing
                .contains(&real.as_str())
                .then(|| PathBuf::from(real))
        }
    }

    #[test]
    fn etc_shells_skips_comments_and_blanks() {
        let text = "# /etc/shells: valid login shells\n/bin/sh\n\n  /bin/zsh  # mine\n";
        assert_eq!(parse_etc_shells(text), vec!["/bin/sh", "/bin/zsh"]);
    }

    #[test]
    fn unix_dedupes_symlinked_paths_and_puts_shell_env_first() {
        let etc = "/bin/sh\n/usr/bin/sh\n/bin/bash\n/usr/bin/bash\n/bin/zsh\n/usr/bin/zsh\n";
        let shells = unix_shells(
            etc,
            Some("/usr/bin/zsh"),
            usrmerge(&["/usr/bin/sh", "/usr/bin/bash", "/usr/bin/zsh"]),
        );
        let labels: Vec<_> = shells.iter().map(|s| s.label.as_str()).collect();
        assert_eq!(labels, vec!["zsh", "sh", "bash"]);
        assert!(shells[0].default);
        assert_eq!(shells.iter().filter(|s| s.default).count(), 1);
        assert!(shells.iter().all(|s| s.args == vec!["-l"]));
    }

    #[test]
    fn unix_drops_missing_and_non_interactive_entries() {
        let etc = "/bin/sh\n/usr/bin/tmux\n/usr/sbin/nologin\n/usr/bin/fish\n/bin/rbash\n";
        let shells = unix_shells(
            etc,
            None,
            usrmerge(&["/usr/bin/sh", "/usr/bin/tmux", "/usr/bin/rbash"]),
        );
        let ids: Vec<_> = shells.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(
            ids,
            vec!["unix:/bin/sh"],
            "fish is not installed; tmux and rbash are not shells to open"
        );
        assert!(
            shells[0].default,
            "with no $SHELL the first becomes the default"
        );
    }

    #[test]
    fn unix_labels_same_named_shells_by_path() {
        let etc = "/bin/bash\n/opt/homebrew/bin/bash\n";
        let exists = |p: &str| Some(PathBuf::from(p));
        let shells = unix_shells(etc, Some("/opt/homebrew/bin/bash"), exists);
        let labels: Vec<_> = shells.iter().map(|s| s.label.as_str()).collect();
        assert_eq!(labels, vec!["/opt/homebrew/bin/bash", "/bin/bash"]);
    }

    #[test]
    fn unix_with_nothing_usable_still_offers_bin_sh() {
        let shells = unix_shells("", None, |_| None);
        assert_eq!(shells.len(), 1);
        assert_eq!(shells[0].program, "/bin/sh");
        assert!(shells[0].default);
    }

    fn utf16le(s: &str) -> Vec<u8> {
        s.encode_utf16().flat_map(|u| u.to_le_bytes()).collect()
    }

    #[test]
    fn wsl_list_decodes_utf16_with_bom_and_crlf() {
        let bytes = utf16le("\u{feff}Ubuntu-22.04\r\ndocker-desktop\r\nDebian\r\n\r\n");
        assert_eq!(parse_wsl_list(&bytes), vec!["Ubuntu-22.04", "Debian"]);
    }

    #[test]
    fn wsl_list_reads_utf8_when_wsl_utf8_is_set() {
        assert_eq!(
            parse_wsl_list(b"Ubuntu\nkali-linux\n"),
            vec!["Ubuntu", "kali-linux"]
        );
        assert!(parse_wsl_list(b"").is_empty());
    }

    #[test]
    fn windows_lists_powershells_cmd_git_bash_then_distros() {
        let distros = vec!["Ubuntu".to_string()];
        let shells = windows_shells(
            |_| true,
            Some("C:\\Windows\\system32\\cmd.exe"),
            Some("C:\\Program Files\\Git\\bin\\bash.exe".to_string()),
            &distros,
        );
        let ids: Vec<_> = shells.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(
            ids,
            vec!["pwsh", "powershell", "cmd", "git-bash", "wsl:Ubuntu"]
        );
        assert!(shells[0].default);
        assert_eq!(shells.iter().filter(|s| s.default).count(), 1);
        let ubuntu = shells.last().unwrap();
        assert_eq!(ubuntu.label, "Ubuntu (WSL)");
        assert_eq!(ubuntu.program, "wsl.exe");
        assert_eq!(ubuntu.args, vec!["-d", "Ubuntu", "--cd", "~"]);
        assert_eq!(shells[2].program, "C:\\Windows\\system32\\cmd.exe");
    }

    #[test]
    fn windows_without_powershell_defaults_to_cmd() {
        let shells = windows_shells(|_| false, None, None, &[]);
        assert_eq!(shells.len(), 1);
        assert_eq!(shells[0].program, "cmd.exe");
        assert!(shells[0].default);
    }

    #[test]
    fn git_bash_is_found_under_program_files_not_on_path() {
        let env = |k: &str| match k {
            "ProgramFiles" => Some("C:\\Program Files".to_string()),
            "LOCALAPPDATA" => Some("C:\\Users\\me\\AppData\\Local".to_string()),
            _ => None,
        };
        let only_user_install = |p: &Path| p.to_string_lossy().contains("AppData");
        let found = find_git_bash(env, only_user_install).expect("found");
        assert!(
            found.contains("AppData") && found.ends_with("bash.exe"),
            "{}",
            found
        );
        assert_eq!(find_git_bash(env, |_| false), None);
    }

    #[test]
    fn default_of_picks_the_flagged_entry() {
        let shells = windows_shells(|p| p == "powershell.exe", None, None, &[]);
        assert_eq!(default_of(&shells).unwrap().id, "powershell");
        assert!(default_of(&[]).is_none());
    }
}
