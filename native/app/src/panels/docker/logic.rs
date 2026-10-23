//! The Docker panel's pure parts: which state the list shows, the wording of
//! errors, the optimistic row updates, the pane's commands and the keystroke
//! batch. Everything here is a port of `DockerPanel.vue` or of the exec pane
//! in `TerminalTab.vue`, and is tested below.

use termdrop_core::docker::{
    Container, DOCKER_DAEMON_NOT_RUNNING, DOCKER_NOT_INSTALLED, DOCKER_PERMISSION_DENIED,
};
use termdrop_core::sftp_ops::shell_escape;

/// How often the list refreshes itself while the panel is on screen.
pub const POLL_SECS: u64 = 5;
/// The exec pane's keystroke batch: a flat timer, as the Vue docker pane's.
pub const KEY_FLUSH_MS: u64 = 16;
/// The pane's height: the Vue default and its drag limits.
pub const PANE_HEIGHT: f32 = 200.0;
pub const PANE_MIN: f32 = 80.0;
pub const PANE_MAX: f32 = 400.0;

/// The install command, shown under the "not installed" state and named in
/// the install confirmation. The core runs its own literal; this is text.
pub const INSTALL_COMMAND: &str = "curl -fsSL https://get.docker.com | sh";
pub const START_DAEMON_COMMAND: &str = "sudo systemctl start docker";
pub const ADD_TO_GROUP_COMMAND: &str = "sudo usermod -aG docker $USER";

/// A `docker_ps` failure the panel explains in place of the list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Problem {
    NotInstalled,
    DaemonNotRunning,
    PermissionDenied,
}

/// The sentinel in a core error string, matched with `contains` as Vue does.
/// When more than one is present, the one Vue's template shows first wins.
pub fn problem_of(err: &str) -> Option<Problem> {
    if err.contains(DOCKER_NOT_INSTALLED) {
        Some(Problem::NotInstalled)
    } else if err.contains(DOCKER_DAEMON_NOT_RUNNING) {
        Some(Problem::DaemonNotRunning)
    } else if err.contains(DOCKER_PERMISSION_DENIED) {
        Some(Problem::PermissionDenied)
    } else {
        None
    }
}

/// The toast for a failed start/stop/restart/exec (`handleDockerError`),
/// which checks the sentinels in its own order.
pub fn action_error_message(err: &str, action: &str) -> String {
    if err.contains(DOCKER_PERMISSION_DENIED) {
        "Docker permission denied. Add user to docker group: sudo usermod -aG docker $USER".into()
    } else if err.contains(DOCKER_DAEMON_NOT_RUNNING) {
        "Docker daemon is not running. Start it with: sudo systemctl start docker".into()
    } else if err.contains(DOCKER_NOT_INSTALLED) {
        "Docker is not installed on this host".into()
    } else {
        format!("{action} failed: {err}")
    }
}

/// What the area under the header shows, in the order of the Vue template's
/// `v-if` chain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListView {
    Loading,
    Problem(Problem),
    /// No containers; `show_all` tells "none at all" from "none running".
    Empty {
        show_all: bool,
    },
    List,
}

pub fn list_view(
    loading: bool,
    problem: Option<Problem>,
    count: usize,
    show_all: bool,
) -> ListView {
    if loading {
        ListView::Loading
    } else if let Some(p) = problem {
        ListView::Problem(p)
    } else if count == 0 {
        ListView::Empty { show_all }
    } else {
        ListView::List
    }
}

/// The header's count (`"3 containers"`), empty when there are none.
pub fn count_label(count: usize) -> String {
    if count == 0 {
        String::new()
    } else {
        format!("{count} containers")
    }
}

/// The second line of a row: status, then the ports when there are any.
pub fn status_line(c: &Container) -> String {
    if c.ports.is_empty() {
        c.status.clone()
    } else {
        format!("{} \u{b7} {}", c.status, c.ports)
    }
}

/// A row action, for the optimistic update shown until the list reloads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Start,
    Stop,
    Restart,
}

impl Action {
    /// The word in the error toast (`handleDockerError(err, 'Start')`).
    pub fn verb(self) -> &'static str {
        match self {
            Self::Start => "Start",
            Self::Stop => "Stop",
            Self::Restart => "Restart",
        }
    }

    /// The confirmation, for the two that ask. Start does not.
    pub fn confirmation(self, name: &str) -> Option<(&'static str, String)> {
        match self {
            Self::Start => None,
            Self::Stop => Some(("Stop Container", format!("Stop \"{name}\"?"))),
            Self::Restart => Some(("Restart Container", format!("Restart \"{name}\"?"))),
        }
    }

    /// What the row shows while the command runs.
    pub fn apply(self, c: &mut Container) {
        match self {
            Self::Start => {
                c.running = true;
                c.status = "Starting...".into();
            }
            Self::Stop => {
                c.running = false;
                c.status = "Stopping...".into();
            }
            Self::Restart => c.status = "Restarting...".into(),
        }
    }
}

/// The two kinds of pane under the terminal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaneKind {
    Logs,
    Exec,
}

pub fn pane_title(kind: PaneKind, name: &str) -> String {
    match kind {
        PaneKind::Logs => format!("Logs: {name}"),
        PaneKind::Exec => format!("Exec: {name}"),
    }
}

/// `docker logs`, following only a running container (`viewLogs`).
pub fn logs_command(name: &str, follow: bool) -> String {
    let name = shell_escape(name);
    if follow {
        format!("docker logs -f --tail 200 {name}")
    } else {
        format!("docker logs --tail 200 {name}")
    }
}

/// The shell `docker_inspect_shell` chose, narrowed to the two it can
/// answer, because it goes into the command unquoted.
pub fn exec_shell(answer: &str) -> &'static str {
    if answer.trim() == "bash" {
        "bash"
    } else {
        "sh"
    }
}

pub fn exec_command(name: &str, shell: &str) -> String {
    format!(
        "docker exec -it {} {}",
        shell_escape(name),
        exec_shell(shell)
    )
}

/// `dockerTerm.writeln('\r\n\x1b[31mError: ...\x1b[0m')`.
pub fn pty_error_text(error: &str) -> String {
    format!("\r\n\x1b[31mError: {error}\x1b[0m\r\n")
}

/// `dockerTerm.writeln('\r\n\x1b[31mFailed to start: ...\x1b[0m')`.
pub fn start_failed_text(error: &str) -> String {
    format!("\r\n\x1b[31mFailed to start: {error}\x1b[0m\r\n")
}

/// The drag result, kept within the Vue limits.
pub fn clamp_pane_height(h: f32) -> f32 {
    if h.is_nan() {
        PANE_HEIGHT
    } else {
        h.clamp(PANE_MIN, PANE_MAX)
    }
}

/// Keystrokes waiting for the flush timer. `push` says when the timer must
/// be armed (the first bytes since the last flush); `take` empties the batch
/// and disarms it.
#[derive(Debug, Default)]
pub struct KeyBatch {
    buf: Vec<u8>,
    armed: bool,
}

impl KeyBatch {
    #[must_use]
    pub fn push(&mut self, bytes: &[u8]) -> bool {
        self.buf.extend_from_slice(bytes);
        !std::mem::replace(&mut self.armed, true)
    }

    pub fn take(&mut self) -> Vec<u8> {
        self.armed = false;
        std::mem::take(&mut self.buf)
    }
}

/// A write to a PTY that has already ended is expected (the exec shell
/// exited, the pane is closing) and not worth a warning, as in Vue.
pub fn is_quiet_write_error(err: &str) -> bool {
    err.contains("not found")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn container(name: &str, running: bool, ports: &str) -> Container {
        Container {
            id: "abc123".into(),
            name: name.into(),
            image: "nginx:latest".into(),
            status: "Up 2 hours".into(),
            ports: ports.into(),
            created: String::new(),
            state: if running { "running" } else { "exited" }.into(),
            running,
        }
    }

    #[test]
    fn problems_come_from_the_sentinels() {
        assert_eq!(
            problem_of("Error: DOCKER_NOT_INSTALLED"),
            Some(Problem::NotInstalled)
        );
        assert_eq!(
            problem_of("DOCKER_PERMISSION_DENIED: got permission denied"),
            Some(Problem::PermissionDenied)
        );
        assert_eq!(
            problem_of("DOCKER_DAEMON_NOT_RUNNING"),
            Some(Problem::DaemonNotRunning)
        );
        assert_eq!(problem_of("No exec session for host 3"), None);
        // Template order: not installed is shown before the others.
        assert_eq!(
            problem_of("DOCKER_PERMISSION_DENIED DOCKER_NOT_INSTALLED"),
            Some(Problem::NotInstalled)
        );
    }

    #[test]
    fn action_errors_use_the_vue_wording_and_order() {
        assert_eq!(
            action_error_message("DOCKER_PERMISSION_DENIED DOCKER_NOT_INSTALLED", "Stop"),
            "Docker permission denied. Add user to docker group: sudo usermod -aG docker $USER"
        );
        assert_eq!(
            action_error_message("DOCKER_DAEMON_NOT_RUNNING", "Start"),
            "Docker daemon is not running. Start it with: sudo systemctl start docker"
        );
        assert_eq!(
            action_error_message("DOCKER_NOT_INSTALLED", "Exec"),
            "Docker is not installed on this host"
        );
        assert_eq!(
            action_error_message("boom", "Restart"),
            "Restart failed: boom"
        );
    }

    #[test]
    fn the_list_view_follows_the_template_chain() {
        assert_eq!(
            list_view(true, Some(Problem::NotInstalled), 3, false),
            ListView::Loading
        );
        assert_eq!(
            list_view(false, Some(Problem::DaemonNotRunning), 0, true),
            ListView::Problem(Problem::DaemonNotRunning)
        );
        assert_eq!(
            list_view(false, None, 0, false),
            ListView::Empty { show_all: false }
        );
        assert_eq!(
            list_view(false, None, 0, true),
            ListView::Empty { show_all: true }
        );
        assert_eq!(list_view(false, None, 2, false), ListView::List);
    }

    #[test]
    fn labels() {
        assert_eq!(count_label(0), "");
        assert_eq!(count_label(4), "4 containers");
        assert_eq!(status_line(&container("web", true, "")), "Up 2 hours");
        assert_eq!(
            status_line(&container("web", true, "0.0.0.0:80->80/tcp")),
            "Up 2 hours \u{b7} 0.0.0.0:80->80/tcp"
        );
        assert_eq!(pane_title(PaneKind::Logs, "web"), "Logs: web");
        assert_eq!(pane_title(PaneKind::Exec, "web"), "Exec: web");
    }

    #[test]
    fn optimistic_updates_and_confirmations() {
        let mut c = container("web", false, "");
        Action::Start.apply(&mut c);
        assert!(c.running);
        assert_eq!(c.status, "Starting...");
        Action::Restart.apply(&mut c);
        assert!(c.running);
        assert_eq!(c.status, "Restarting...");
        Action::Stop.apply(&mut c);
        assert!(!c.running);
        assert_eq!(c.status, "Stopping...");

        assert_eq!(Action::Start.confirmation("web"), None);
        assert_eq!(
            Action::Stop.confirmation("web"),
            Some(("Stop Container", "Stop \"web\"?".to_string()))
        );
        assert_eq!(
            Action::Restart.confirmation("web"),
            Some(("Restart Container", "Restart \"web\"?".to_string()))
        );
        assert_eq!(Action::Stop.verb(), "Stop");
    }

    #[test]
    fn commands_quote_the_name_and_narrow_the_shell() {
        assert_eq!(logs_command("web", true), "docker logs -f --tail 200 web");
        assert_eq!(logs_command("web", false), "docker logs --tail 200 web");
        assert_eq!(
            logs_command("a b;rm", false),
            format!("docker logs --tail 200 {}", shell_escape("a b;rm"))
        );
        assert!(!logs_command("a b;rm", false).contains("a b;rm "));
        assert_eq!(exec_command("web", "bash"), "docker exec -it web bash");
        assert_eq!(exec_command("web", "bash\n"), "docker exec -it web bash");
        assert_eq!(exec_command("web", "sh"), "docker exec -it web sh");
        assert_eq!(
            exec_command("web", "zsh; rm -rf /"),
            "docker exec -it web sh"
        );
    }

    #[test]
    fn pane_messages_are_red_lines() {
        assert_eq!(pty_error_text("eof"), "\r\n\x1b[31mError: eof\x1b[0m\r\n");
        assert_eq!(
            start_failed_text("no route"),
            "\r\n\x1b[31mFailed to start: no route\x1b[0m\r\n"
        );
    }

    #[test]
    fn pane_height_stays_in_bounds() {
        assert_eq!(clamp_pane_height(10.0), PANE_MIN);
        assert_eq!(clamp_pane_height(1000.0), PANE_MAX);
        assert_eq!(clamp_pane_height(250.0), 250.0);
        assert_eq!(clamp_pane_height(f32::NAN), PANE_HEIGHT);
    }

    #[test]
    fn the_key_batch_arms_once_per_flush() {
        let mut b = KeyBatch::default();
        assert!(b.push(b"l"), "the first key arms the timer");
        assert!(!b.push(b"s"), "later keys ride the same timer");
        assert_eq!(b.take(), b"ls");
        assert!(b.take().is_empty());
        assert!(b.push(b"\r"), "a flush disarms it");
    }

    #[test]
    fn writes_to_a_gone_pty_are_quiet() {
        assert!(is_quiet_write_error("PTY session not found"));
        assert!(!is_quiet_write_error("channel closed"));
    }
}
