use ssh2::Session;
use std::path::Path;
use std::time::Duration;

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

/// Repeat a non-blocking ssh2 call until it stops returning WouldBlock,
/// sleeping 10ms between attempts. Any other error is prefixed with `label`.
pub(crate) fn retry_would_block<T>(
    label: &str,
    mut op: impl FnMut() -> Result<T, ssh2::Error>,
) -> Result<T, String> {
    loop {
        match op() {
            Ok(v) => return Ok(v),
            Err(e) => {
                let io_err: std::io::Error = e.into();
                if io_err.kind() == std::io::ErrorKind::WouldBlock {
                    std::thread::sleep(Duration::from_millis(10));
                    continue;
                }
                return Err(format!("{}: {}", label, io_err));
            }
        }
    }
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

pub fn create_session(
    host: &str,
    port: u16,
    username: &str,
    password: Option<&str>,
    key_path: Option<&str>,
) -> Result<Session, String> {
    let tcp = resolve_and_connect(host, port)?;

    tcp.set_nonblocking(true)
        .map_err(|e| format!("set_nonblocking: {}", e))?;

    let mut session = Session::new().map_err(|e| format!("session: {}", e))?;

    session.set_tcp_stream(tcp);
    session.set_blocking(false);

    // Retry handshake in non-blocking mode
    retry_would_block("handshake", || session.handshake())?;

    // Retry auth in non-blocking mode
    if let Some(key_path) = key_path {
        let expanded = expand_key_path(key_path);
        retry_would_block("key auth", || {
            session.userauth_pubkey_file(username, None, &expanded, None)
        })?;
    } else if let Some(password) = password {
        retry_would_block("auth", || session.userauth_password(username, password))?;
    } else {
        return Err("no credentials provided".to_string());
    }

    Ok(session)
}

/// Creates a new SSH session (blocking mode) for exec or SFTP reuse.
pub fn create_exec_session(
    host: &str,
    port: u16,
    username: &str,
    password: Option<&str>,
    key_path: Option<&str>,
) -> Result<Session, String> {
    let tcp = resolve_and_connect(host, port)?;
    let mut session = Session::new().map_err(|e| format!("session: {}", e))?;
    session.set_tcp_stream(tcp);
    session
        .handshake()
        .map_err(|e| format!("handshake: {}", e))?;

    if let Some(key_path) = key_path {
        let expanded = expand_key_path(key_path);
        session
            .userauth_pubkey_file(username, None, &expanded, None)
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
