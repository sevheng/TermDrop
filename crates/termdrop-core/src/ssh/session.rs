use super::trust::{self, Refusal};
use ssh2::Session;
use std::path::Path;
use std::time::{Duration, Instant};

/// Expand `~/` prefix to the user's home directory.
pub fn expand_key_path(key_path: &str) -> std::path::PathBuf {
    if key_path.starts_with("~/") {
        dirs::home_dir()
            .map(|h| h.join(&key_path[2..]))
            .unwrap_or_else(|| Path::new(key_path).to_path_buf())
    } else {
        Path::new(key_path).to_path_buf()
    }
}

/// How long a non-blocking call may keep answering WouldBlock before it is
/// treated as a stalled server.
///
/// Matches `CONNECT_TIMEOUT`, which covers only the TCP connect -- the
/// handshake and the authentication that follow it are what this bounds.
const WOULD_BLOCK_TIMEOUT: Duration = Duration::from_secs(30);

/// Repeat a non-blocking ssh2 call until it stops returning WouldBlock,
/// sleeping 10ms between attempts. Any other error is prefixed with `label`.
///
/// **Bounded.** This used to loop forever: a server that stalls at EAGAIN --
/// mid-handshake, or during authentication -- would spin a thread at 100Hz for
/// the lifetime of the process, with nothing above it to notice.
pub(crate) fn retry_would_block<T>(
    label: &str,
    op: impl FnMut() -> Result<T, ssh2::Error>,
) -> Result<T, String> {
    retry_would_block_for(label, WOULD_BLOCK_TIMEOUT, op)
}

/// The body of [`retry_would_block`], with the budget as a parameter so a test
/// can exercise the timeout without waiting out the real one.
pub(crate) fn retry_would_block_for<T>(
    label: &str,
    budget: Duration,
    mut op: impl FnMut() -> Result<T, ssh2::Error>,
) -> Result<T, String> {
    let deadline = Instant::now() + budget;
    loop {
        match op() {
            Ok(v) => return Ok(v),
            Err(e) => {
                let io_err: std::io::Error = e.into();
                if io_err.kind() == std::io::ErrorKind::WouldBlock {
                    if Instant::now() >= deadline {
                        return Err(format!(
                            "{}: the server stopped responding after {}s",
                            label,
                            budget.as_secs()
                        ));
                    }
                    std::thread::sleep(Duration::from_millis(10));
                    continue;
                }
                return Err(format!("{}: {}", label, io_err));
            }
        }
    }
}

/// Turn a refusal into the error string the frontend parses.
fn refusal_to_string(refusal: Refusal) -> String {
    trust::refusal_payload(&refusal)
}

/// Create a TCP connection, perform SSH handshake, and authenticate.
/// Returns a ready-to-use `Session` in **non-blocking** mode.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);

pub fn resolve_and_connect(host: &str, port: u16) -> Result<std::net::TcpStream, String> {
    let addr = format!("{}:{}", host, port);
    // Try to parse as SocketAddr first (IP address)
    if let Ok(socket_addr) = addr.parse::<std::net::SocketAddr>() {
        return std::net::TcpStream::connect_timeout(&socket_addr, CONNECT_TIMEOUT)
            .map_err(|e| format!("connect: {}", e));
    }
    // Otherwise resolve hostname
    let addrs =
        std::net::ToSocketAddrs::to_socket_addrs(&addr).map_err(|e| format!("resolve: {}", e))?;
    let mut last_err = None;
    for socket_addr in addrs {
        match std::net::TcpStream::connect_timeout(&socket_addr, CONNECT_TIMEOUT) {
            Ok(tcp) => return Ok(tcp),
            Err(e) => last_err = Some(e),
        }
    }
    Err(format!(
        "connect: {}",
        last_err.unwrap_or_else(|| std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "no addresses resolved"
        ))
    ))
}

/// Everything needed to open an SSH session to one host, including how to
/// reach it.
///
/// One struct rather than a growing list of arguments: `jump` is the reason it
/// exists, and threading a second host's six fields through every call site
/// as loose parameters is how one of them gets forgotten.
#[derive(Clone, Debug, Default)]
pub struct SshTarget {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: Option<String>,
    pub key_path: Option<String>,
    /// For an encrypted private key.
    pub passphrase: Option<String>,
    /// The saved host to go through, when this one is not directly reachable.
    /// One hop: the resolver refuses a jump host that has its own.
    pub jump: Option<Box<SshTarget>>,
}

/// Host key fingerprints the user explicitly accepted, one per hop.
///
/// A fingerprint rather than a yes, so an acceptance cannot let a different
/// server through on the retry. Kept per hop so accepting the bastion's key
/// says nothing about the target's.
#[derive(Clone, Debug, Default)]
pub struct Accept {
    pub target: Option<String>,
    pub jump: Option<String>,
}

impl Accept {
    /// Refuse any key that is not already on file.
    pub fn none() -> Self {
        Self::default()
    }
}

/// Open the socket a session runs over: straight to the host, or through its
/// jump host.
fn dial(target: &SshTarget, accept: &Accept) -> Result<std::net::TcpStream, String> {
    match &target.jump {
        None => resolve_and_connect(&target.host, target.port),
        Some(jump) => {
            super::jump::open_via(jump, &target.host, target.port, accept.jump.as_deref())
        }
    }
}

/// Create a session in **non-blocking** mode, for an interactive PTY.
pub fn create_session(target: &SshTarget, accept: &Accept) -> Result<Session, String> {
    let tcp = dial(target, accept)?;

    tcp.set_nonblocking(true)
        .map_err(|e| format!("set_nonblocking: {}", e))?;

    let mut session = Session::new().map_err(|e| format!("session: {}", e))?;

    session.set_tcp_stream(tcp);
    session.set_blocking(false);

    // Retry handshake in non-blocking mode
    retry_would_block("handshake", || session.handshake())?;

    // Before any credential leaves this machine. Always the target's own name,
    // never the loopback address a jump host delivers it on.
    trust::verify(
        &session,
        &target.host,
        target.port,
        accept.target.as_deref(),
    )
    .map_err(refusal_to_string)?;

    let username = target.username.as_str();
    let passphrase = target.passphrase.as_deref();
    if let Some(key_path) = &target.key_path {
        let expanded = expand_key_path(key_path);
        retry_would_block("key auth", || {
            session.userauth_pubkey_file(username, None, &expanded, passphrase)
        })?;
    } else if let Some(password) = &target.password {
        retry_would_block("auth", || session.userauth_password(username, password))?;
    } else {
        return Err("no credentials provided".to_string());
    }

    Ok(session)
}

/// Creates a new SSH session (blocking mode) for exec or SFTP reuse.
///
/// Only `ssh_connect` passes an acceptance: it opens this session *before* the
/// terminal, so the key is on file by the time the PTY thread checks it. Every
/// other caller passes [`Accept::none`], which refuses a key not already on
/// file.
pub fn create_exec_session(target: &SshTarget, accept: &Accept) -> Result<Session, String> {
    let tcp = dial(target, accept)?;
    let mut session = Session::new().map_err(|e| format!("session: {}", e))?;
    session.set_tcp_stream(tcp);
    session
        .handshake()
        .map_err(|e| format!("handshake: {}", e))?;

    trust::verify(
        &session,
        &target.host,
        target.port,
        accept.target.as_deref(),
    )
    .map_err(refusal_to_string)?;

    let username = target.username.as_str();
    if let Some(key_path) = &target.key_path {
        let expanded = expand_key_path(key_path);
        session
            .userauth_pubkey_file(username, None, &expanded, target.passphrase.as_deref())
            .map_err(|e| format!("key auth: {}", e))?;
    } else if let Some(password) = &target.password {
        session
            .userauth_password(username, password)
            .map_err(|e| format!("auth: {}", e))?;
    } else {
        return Err("no credentials provided".to_string());
    }

    Ok(session)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The EAGAIN a non-blocking ssh2 call answers with while it waits.
    fn would_block() -> ssh2::Error {
        ssh2::Error::from_errno(ssh2::ErrorCode::Session(-37))
    }

    #[test]
    fn a_stalled_server_gives_up_instead_of_spinning_forever() {
        // The bug: this loop had no cap and no deadline, so a server stuck at
        // EAGAIN mid-handshake spun a thread at 100Hz for the life of the
        // process, with nothing above it to notice.
        let mut attempts = 0;
        let result: Result<(), String> =
            retry_would_block_for("handshake", Duration::from_millis(50), || {
                attempts += 1;
                Err(would_block())
            });

        let err = result.unwrap_err();
        assert!(err.starts_with("handshake:"), "unlabelled: {}", err);
        assert!(err.contains("stopped responding"), "unhelpful: {}", err);
        assert!(attempts > 1, "should have retried before giving up");
    }

    #[test]
    fn it_retries_until_the_call_succeeds() {
        // The behaviour the bound must not break: WouldBlock is normal, and
        // the overwhelming majority of calls answer it once or twice first.
        let mut attempts = 0;
        let result = retry_would_block_for("auth", Duration::from_secs(5), || {
            attempts += 1;
            if attempts < 3 {
                Err(would_block())
            } else {
                Ok(42)
            }
        });
        assert_eq!(result.unwrap(), 42);
        assert_eq!(attempts, 3);
    }

    #[test]
    fn any_other_error_returns_at_once_and_carries_its_label() {
        // Only WouldBlock is worth retrying; everything else is final, and
        // retrying it would turn an instant failure into a 30s hang.
        let mut attempts = 0;
        let result: Result<(), String> =
            retry_would_block_for("auth", Duration::from_secs(5), || {
                attempts += 1;
                Err(ssh2::Error::from_errno(ssh2::ErrorCode::Session(-18)))
            });
        assert!(result.unwrap_err().starts_with("auth:"));
        assert_eq!(attempts, 1, "a final error must not be retried");
    }

    #[test]
    fn expand_key_path_handles_the_tilde_forms() {
        // Only a leading `~/` expands: a tilde anywhere else is an ordinary
        // character in a filename.
        let home = dirs::home_dir().expect("a home directory");
        assert_eq!(
            expand_key_path("~/.ssh/id_ed25519"),
            home.join(".ssh/id_ed25519")
        );
        assert_eq!(expand_key_path("/etc/keys/id"), Path::new("/etc/keys/id"));
        assert_eq!(expand_key_path("keys/~backup"), Path::new("keys/~backup"));
        assert_eq!(
            expand_key_path("~"),
            Path::new("~"),
            "a bare tilde is not a path"
        );
    }
}
