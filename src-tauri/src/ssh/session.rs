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

#[allow(clippy::too_many_arguments)]
pub fn create_session(
    host: &str,
    port: u16,
    username: &str,
    password: Option<&str>,
    key_path: Option<&str>,
    passphrase: Option<&str>,
    accept_host_key: Option<&str>,
) -> Result<Session, String> {
    let tcp = resolve_and_connect(host, port)?;

    tcp.set_nonblocking(true)
        .map_err(|e| format!("set_nonblocking: {}", e))?;

    let mut session = Session::new().map_err(|e| format!("session: {}", e))?;

    session.set_tcp_stream(tcp);
    session.set_blocking(false);

    // Retry handshake in non-blocking mode
    retry_would_block("handshake", || session.handshake())?;

    // Before any credential leaves this machine.
    trust::verify(&session, host, port, accept_host_key).map_err(refusal_to_string)?;

    // Retry auth in non-blocking mode
    if let Some(key_path) = key_path {
        let expanded = expand_key_path(key_path);
        retry_would_block("key auth", || {
            session.userauth_pubkey_file(username, None, &expanded, passphrase)
        })?;
    } else if let Some(password) = password {
        retry_would_block("auth", || session.userauth_password(username, password))?;
    } else {
        return Err("no credentials provided".to_string());
    }

    Ok(session)
}

/// Creates a new SSH session (blocking mode) for exec or SFTP reuse.
///
/// `accept_host_key` is the fingerprint the user agreed to, and only
/// `ssh_connect` passes one: it opens this session *before* the terminal, so
/// the key is on file by the time the PTY thread checks it. Every other caller
/// passes `None`, which refuses a key that is not already on file.
pub fn create_exec_session(
    host: &str,
    port: u16,
    username: &str,
    password: Option<&str>,
    key_path: Option<&str>,
    passphrase: Option<&str>,
    accept_host_key: Option<&str>,
) -> Result<Session, String> {
    let tcp = resolve_and_connect(host, port)?;
    let mut session = Session::new().map_err(|e| format!("session: {}", e))?;
    session.set_tcp_stream(tcp);
    session
        .handshake()
        .map_err(|e| format!("handshake: {}", e))?;

    trust::verify(&session, host, port, accept_host_key).map_err(refusal_to_string)?;

    if let Some(key_path) = key_path {
        let expanded = expand_key_path(key_path);
        session
            .userauth_pubkey_file(username, None, &expanded, passphrase)
            .map_err(|e| format!("key auth: {}", e))?;
    } else if let Some(password) = password {
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
