use std::io::{Read, Write};
use std::time::Duration;
use ssh2::Session;
use tauri::{Emitter, Window};
use tokio::sync::mpsc;

pub struct SshSessionHandle {
    pub write_tx: mpsc::UnboundedSender<String>,
    pub disconnect_tx: mpsc::UnboundedSender<()>,
}

pub fn connect(
    window: Window,
    session_id: String,
    host: String,
    port: u16,
    username: String,
    password: String,
) -> Result<SshSessionHandle, String> {
    let (write_tx, mut write_rx) = mpsc::unbounded_channel::<String>();
    let (disconnect_tx, mut disconnect_rx) = mpsc::unbounded_channel::<()>();

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

        if let Err(e) = session.handshake() {
            let payload = serde_json::json!({"session_id": &session_id, "error": format!("handshake: {}", e)});
            let _ = window.emit("ssh-error", payload);
            return;
        }

        if let Err(e) = session.userauth_password(&username, &password) {
            let payload = serde_json::json!({"session_id": &session_id, "error": format!("auth: {}", e)});
            let _ = window.emit("ssh-error", payload);
            return;
        }

        let mut channel = match session.channel_session() {
            Ok(c) => c,
            Err(e) => {
                let payload = serde_json::json!({"session_id": &session_id, "error": format!("channel: {}", e)});
                let _ = window.emit("ssh-error", payload);
                return;
            }
        };

        if let Err(e) = channel.request_pty("xterm-256color", None, None) {
            let payload = serde_json::json!({"session_id": &session_id, "error": format!("pty: {}", e)});
            let _ = window.emit("ssh-error", payload);
            return;
        }

        if let Err(e) = channel.shell() {
            let payload = serde_json::json!({"session_id": &session_id, "error": format!("shell: {}", e)});
            let _ = window.emit("ssh-error", payload);
            return;
        }

        let _ = window.emit("ssh-connected", session_id.clone());

        let mut buf = [0u8; 4096];
        loop {
            if disconnect_rx.try_recv().is_ok() {
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
        let _ = window.emit("ssh-disconnected", session_id);
    });

    Ok(SshSessionHandle { write_tx, disconnect_tx })
}
