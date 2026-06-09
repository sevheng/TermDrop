use std::io::{Read, Write};
use std::path::Path;
use std::time::Duration;
use ssh2::Session;
use tauri::{Emitter, Window};
use tokio::sync::mpsc;

pub struct SshSessionHandle {
    pub host_id: i64,
    pub write_tx: mpsc::UnboundedSender<String>,
    pub disconnect_tx: mpsc::UnboundedSender<()>,
    pub resize_tx: mpsc::UnboundedSender<(u32, u32)>,
}

pub struct ExecPtyHandle {
    pub write_tx: mpsc::UnboundedSender<String>,
    pub disconnect_tx: mpsc::UnboundedSender<()>,
}

/// Creates a new SSH session (blocking mode) for exec or SFTP reuse.
pub fn create_exec_session(
    host: &str,
    port: u16,
    username: &str,
    password: Option<&str>,
    key_path: Option<&str>,
) -> Result<Session, String> {
    let addr = format!("{}:{}", host, port);
    let tcp = std::net::TcpStream::connect(&addr)
        .map_err(|e| format!("connect: {}", e))?;
    let mut session = Session::new()
        .map_err(|e| format!("session: {}", e))?;
    session.set_tcp_stream(tcp);
    session.handshake()
        .map_err(|e| format!("handshake: {}", e))?;

    if let Some(key_path) = key_path {
        let expanded = expand_key_path(key_path);
        session.userauth_pubkey_file(username, None, &expanded, None)
            .map_err(|e| format!("key auth: {}", e))?;
    } else if let Some(password) = password {
        session.userauth_password(username, password)
            .map_err(|e| format!("auth: {}", e))?;
    } else {
        return Err("no credentials provided".to_string());
    }

    Ok(session)
}

/// Run a command on an existing SSH session (reuses connection).
pub fn exec_with_session(
    session: &Session,
    command: &str,
) -> Result<String, String> {
    let mut channel = session.channel_session()
        .map_err(|e| format!("channel: {}", e))?;
    channel.exec(command)
        .map_err(|e| format!("exec: {}", e))?;

    let mut output = String::new();
    channel.read_to_string(&mut output)
        .map_err(|e| format!("read: {}", e))?;

    channel.wait_close().ok();
    Ok(output.trim().to_string())
}

fn expand_key_path(key_path: &str) -> std::path::PathBuf {
    if key_path.starts_with("~/") {
        dirs::home_dir()
            .map(|h| h.join(&key_path[2..]))
            .unwrap_or_else(|| Path::new(key_path).to_path_buf())
    } else {
        Path::new(key_path).to_path_buf()
    }
}

pub fn connect(
    window: Window,
    session_id: String,
    host_id: i64,
    host: String,
    port: u16,
    username: String,
    password: Option<String>,
    key_path: Option<String>,
) -> Result<SshSessionHandle, String> {
    let (write_tx, mut write_rx) = mpsc::unbounded_channel::<String>();
    let (disconnect_tx, mut disconnect_rx) = mpsc::unbounded_channel::<()>();
    let (resize_tx, mut resize_rx) = mpsc::unbounded_channel::<(u32, u32)>();

    std::thread::spawn(move || {
        let addr = format!("{}:{}", host, port);
        let tcp = match std::net::TcpStream::connect(&addr) {
            Ok(t) => t,
            Err(e) => {
                let payload = serde_json::json!({"session_id": &session_id, "error": format!("connect: {}", e)});
                let _ = window.emit("ssh-error", payload);
                return;
            }
        };

        if let Err(e) = tcp.set_nonblocking(true) {
            let payload = serde_json::json!({"session_id": &session_id, "error": format!("set_nonblocking: {}", e)});
            let _ = window.emit("ssh-error", payload);
            return;
        }

        let mut session = match Session::new() {
            Ok(s) => s,
            Err(e) => {
                let payload = serde_json::json!({"session_id": &session_id, "error": format!("session: {}", e)});
                let _ = window.emit("ssh-error", payload);
                return;
            }
        };

        session.set_tcp_stream(tcp);
        session.set_blocking(false);

        // Retry handshake in non-blocking mode
        loop {
            match session.handshake() {
                Ok(()) => break,
                Err(e) => {
                    let io_err: std::io::Error = e.into();
                    if io_err.kind() == std::io::ErrorKind::WouldBlock {
                        std::thread::sleep(Duration::from_millis(10));
                        continue;
                    }
                    let payload = serde_json::json!({"session_id": &session_id, "error": format!("handshake: {}", io_err)});
                    let _ = window.emit("ssh-error", payload);
                    return;
                }
            }
        }

        // Retry auth in non-blocking mode
        if let Some(key_path) = key_path {
            let expanded = expand_key_path(&key_path);
            loop {
                match session.userauth_pubkey_file(&username, None, &expanded, None) {
                    Ok(()) => break,
                    Err(e) => {
                        let io_err: std::io::Error = e.into();
                        if io_err.kind() == std::io::ErrorKind::WouldBlock {
                            std::thread::sleep(Duration::from_millis(10));
                            continue;
                        }
                        let payload = serde_json::json!({"session_id": &session_id, "error": format!("key auth: {}", io_err)});
                        let _ = window.emit("ssh-error", payload);
                        return;
                    }
                }
            }
        } else if let Some(password) = password {
            loop {
                match session.userauth_password(&username, &password) {
                    Ok(()) => break,
                    Err(e) => {
                        let io_err: std::io::Error = e.into();
                        if io_err.kind() == std::io::ErrorKind::WouldBlock {
                            std::thread::sleep(Duration::from_millis(10));
                            continue;
                        }
                        let payload = serde_json::json!({"session_id": &session_id, "error": format!("auth: {}", io_err)});
                        let _ = window.emit("ssh-error", payload);
                        return;
                    }
                }
            }
        } else {
            let payload = serde_json::json!({"session_id": &session_id, "error": "no credentials provided"});
            let _ = window.emit("ssh-error", payload);
            return;
        }

        // Retry channel session in non-blocking mode
        let mut channel = loop {
            match session.channel_session() {
                Ok(c) => break c,
                Err(e) => {
                    let io_err: std::io::Error = e.into();
                    if io_err.kind() == std::io::ErrorKind::WouldBlock {
                        std::thread::sleep(Duration::from_millis(10));
                        continue;
                    }
                    let payload = serde_json::json!({"session_id": &session_id, "error": format!("channel: {}", io_err)});
                    let _ = window.emit("ssh-error", payload);
                    return;
                }
            }
        };

        loop {
            match channel.request_pty("xterm-256color", None, None) {
                Ok(()) => break,
                Err(e) => {
                    let io_err: std::io::Error = e.into();
                    if io_err.kind() == std::io::ErrorKind::WouldBlock {
                        std::thread::sleep(Duration::from_millis(10));
                        continue;
                    }
                    let payload = serde_json::json!({"session_id": &session_id, "error": format!("pty: {}", io_err)});
                    let _ = window.emit("ssh-error", payload);
                    return;
                }
            }
        }

        loop {
            match channel.shell() {
                Ok(()) => break,
                Err(e) => {
                    let io_err: std::io::Error = e.into();
                    if io_err.kind() == std::io::ErrorKind::WouldBlock {
                        std::thread::sleep(Duration::from_millis(10));
                        continue;
                    }
                    let payload = serde_json::json!({"session_id": &session_id, "error": format!("shell: {}", io_err)});
                    let _ = window.emit("ssh-error", payload);
                    return;
                }
            }
        }

        let _ = window.emit("ssh-connected", session_id.clone());

        let mut buf = vec![0u8; 16384];
        let mut intentional_disconnect = false;
        loop {
            if disconnect_rx.try_recv().is_ok() {
                intentional_disconnect = true;
                break;
            }

            while let Ok((cols, rows)) = resize_rx.try_recv() {
                let _ = channel.request_pty_size(cols, rows, None, None);
            }

            while let Ok(data) = write_rx.try_recv() {
                let mut written = 0;
                while written < data.len() {
                    match channel.write(&data.as_bytes()[written..]) {
                        Ok(n) => written += n,
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                            std::thread::sleep(Duration::from_millis(5));
                        }
                        Err(_) => break,
                    }
                }
            }

            match channel.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    let data = String::from_utf8_lossy(&buf[..n]);
                    let payload = serde_json::json!({
                        "session_id": &session_id,
                        "data": data.to_string(),
                    });
                    let _ = window.emit("ssh-data", payload);
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(_) => break,
            }
        }

        let _ = channel.send_eof();
        let _ = channel.wait_eof();
        let _ = channel.close();
        let _ = channel.wait_close();
        if !intentional_disconnect {
            let _ = window.emit("ssh-disconnected", session_id);
        }
    });

    Ok(SshSessionHandle { host_id, write_tx, disconnect_tx, resize_tx })
}

pub fn exec_pty_connect(
    window: Window,
    pty_session_id: String,
    host: String,
    port: u16,
    username: String,
    password: Option<String>,
    key_path: Option<String>,
    command: String,
) -> Result<ExecPtyHandle, String> {
    let (write_tx, mut write_rx) = mpsc::unbounded_channel::<String>();
    let (disconnect_tx, mut disconnect_rx) = mpsc::unbounded_channel::<()>();

    std::thread::spawn(move || {
        let addr = format!("{}:{}", host, port);
        let tcp = match std::net::TcpStream::connect(&addr) {
            Ok(t) => t,
            Err(e) => {
                let payload = serde_json::json!({"pty_session_id": &pty_session_id, "error": format!("connect: {}", e)});
                let _ = window.emit("exec-pty-error", payload);
                return;
            }
        };

        if let Err(e) = tcp.set_nonblocking(true) {
            let payload = serde_json::json!({"pty_session_id": &pty_session_id, "error": format!("set_nonblocking: {}", e)});
            let _ = window.emit("exec-pty-error", payload);
            return;
        }

        let mut session = match Session::new() {
            Ok(s) => s,
            Err(e) => {
                let payload = serde_json::json!({"pty_session_id": &pty_session_id, "error": format!("session: {}", e)});
                let _ = window.emit("exec-pty-error", payload);
                return;
            }
        };

        session.set_tcp_stream(tcp);
        session.set_blocking(false);

        // Retry handshake in non-blocking mode
        loop {
            match session.handshake() {
                Ok(()) => break,
                Err(e) => {
                    let io_err: std::io::Error = e.into();
                    if io_err.kind() == std::io::ErrorKind::WouldBlock {
                        std::thread::sleep(Duration::from_millis(10));
                        continue;
                    }
                    let payload = serde_json::json!({"pty_session_id": &pty_session_id, "error": format!("handshake: {}", io_err)});
                    let _ = window.emit("exec-pty-error", payload);
                    return;
                }
            }
        }

        // Retry auth in non-blocking mode
        if let Some(key_path) = key_path {
            let expanded = expand_key_path(&key_path);
            loop {
                match session.userauth_pubkey_file(&username, None, &expanded, None) {
                    Ok(()) => break,
                    Err(e) => {
                        let io_err: std::io::Error = e.into();
                        if io_err.kind() == std::io::ErrorKind::WouldBlock {
                            std::thread::sleep(Duration::from_millis(10));
                            continue;
                        }
                        let payload = serde_json::json!({"pty_session_id": &pty_session_id, "error": format!("key auth: {}", io_err)});
                        let _ = window.emit("exec-pty-error", payload);
                        return;
                    }
                }
            }
        } else if let Some(password) = password {
            loop {
                match session.userauth_password(&username, &password) {
                    Ok(()) => break,
                    Err(e) => {
                        let io_err: std::io::Error = e.into();
                        if io_err.kind() == std::io::ErrorKind::WouldBlock {
                            std::thread::sleep(Duration::from_millis(10));
                            continue;
                        }
                        let payload = serde_json::json!({"pty_session_id": &pty_session_id, "error": format!("auth: {}", io_err)});
                        let _ = window.emit("exec-pty-error", payload);
                        return;
                    }
                }
            }
        } else {
            let payload = serde_json::json!({"pty_session_id": &pty_session_id, "error": "no credentials provided"});
            let _ = window.emit("exec-pty-error", payload);
            return;
        }

        // Retry channel session in non-blocking mode
        let mut channel = loop {
            match session.channel_session() {
                Ok(c) => break c,
                Err(e) => {
                    let io_err: std::io::Error = e.into();
                    if io_err.kind() == std::io::ErrorKind::WouldBlock {
                        std::thread::sleep(Duration::from_millis(10));
                        continue;
                    }
                    let payload = serde_json::json!({"pty_session_id": &pty_session_id, "error": format!("channel: {}", io_err)});
                    let _ = window.emit("exec-pty-error", payload);
                    return;
                }
            }
        };

        loop {
            match channel.request_pty("xterm-256color", None, None) {
                Ok(()) => break,
                Err(e) => {
                    let io_err: std::io::Error = e.into();
                    if io_err.kind() == std::io::ErrorKind::WouldBlock {
                        std::thread::sleep(Duration::from_millis(10));
                        continue;
                    }
                    let payload = serde_json::json!({"pty_session_id": &pty_session_id, "error": format!("pty: {}", io_err)});
                    let _ = window.emit("exec-pty-error", payload);
                    return;
                }
            }
        }

        loop {
            match channel.exec(&command) {
                Ok(()) => break,
                Err(e) => {
                    let io_err: std::io::Error = e.into();
                    if io_err.kind() == std::io::ErrorKind::WouldBlock {
                        std::thread::sleep(Duration::from_millis(10));
                        continue;
                    }
                    let payload = serde_json::json!({"pty_session_id": &pty_session_id, "error": format!("exec: {}", io_err)});
                    let _ = window.emit("exec-pty-error", payload);
                    return;
                }
            }
        }

        let _ = window.emit("exec-pty-connected", pty_session_id.clone());

        let mut buf = vec![0u8; 16384];
        let mut intentional_disconnect = false;
        loop {
            if disconnect_rx.try_recv().is_ok() {
                intentional_disconnect = true;
                break;
            }

            while let Ok(data) = write_rx.try_recv() {
                let mut written = 0;
                while written < data.len() {
                    match channel.write(&data.as_bytes()[written..]) {
                        Ok(n) => written += n,
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                            std::thread::sleep(Duration::from_millis(5));
                        }
                        Err(_) => break,
                    }
                }
            }

            match channel.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    let data = String::from_utf8_lossy(&buf[..n]);
                    let payload = serde_json::json!({
                        "pty_session_id": &pty_session_id,
                        "data": data.to_string(),
                    });
                    let _ = window.emit("exec-pty-data", payload);
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(_) => break,
            }
        }

        let _ = channel.send_eof();
        let _ = channel.wait_eof();
        let _ = channel.close();
        let _ = channel.wait_close();
        if !intentional_disconnect {
            let _ = window.emit("exec-pty-disconnected", pty_session_id);
        }
    });

    Ok(ExecPtyHandle { write_tx, disconnect_tx })
}
