use ssh2::Session;
use std::io::{Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

#[allow(dead_code)]
pub struct ActiveForward {
    pub rule_id: i64,
    pub shutdown: Arc<AtomicBool>,
    pub thread_handle: Option<thread::JoinHandle<()>>,
}

pub struct ForwardManager {
    pub active: Mutex<std::collections::HashMap<i64, ActiveForward>>,
}

impl ForwardManager {
    pub fn new() -> Self {
        Self {
            active: Mutex::new(std::collections::HashMap::new()),
        }
    }

    pub fn start_local(
        &self,
        rule_id: i64,
        ssh_host: String,
        ssh_port: u16,
        username: String,
        password: Option<String>,
        key_path: Option<String>,
        local_host: String,
        local_port: u16,
        remote_host: String,
        remote_port: u16,
    ) -> Result<(), String> {
        let addr = format!("{}:{}", local_host, local_port);
        let listener = TcpListener::bind(&addr).map_err(|e| format!("bind {}: {}", addr, e))?;
        listener.set_nonblocking(true).map_err(|e| e.to_string())?;

        let shutdown = Arc::new(AtomicBool::new(false));
        let shutdown_clone = shutdown.clone();

        let handle = thread::spawn(move || loop {
            if shutdown_clone.load(Ordering::Relaxed) {
                break;
            }
            match listener.accept() {
                Ok((client, _)) => {
                    let h = ssh_host.clone();
                    let p = ssh_port;
                    let u = username.clone();
                    let pw = password.clone();
                    let k = key_path.clone();
                    let rh = remote_host.clone();
                    let rp = remote_port;
                    thread::spawn(move || {
                        if let Err(e) = handle_local_connection(h, p, u, pw, k, client, rh, rp) {
                            eprintln!("[forward {}] connection error: {}", rule_id, e);
                        }
                    });
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(100));
                }
                Err(e) => {
                    eprintln!("[forward {}] listener error: {}", rule_id, e);
                    break;
                }
            }
        });

        let mut active = self.active.lock().map_err(|e| e.to_string())?;
        active.insert(
            rule_id,
            ActiveForward {
                rule_id,
                shutdown,
                thread_handle: Some(handle),
            },
        );

        Ok(())
    }

    pub fn start_dynamic(
        &self,
        rule_id: i64,
        ssh_host: String,
        ssh_port: u16,
        username: String,
        password: Option<String>,
        key_path: Option<String>,
        local_host: String,
        local_port: u16,
    ) -> Result<(), String> {
        let addr = format!("{}:{}", local_host, local_port);
        let listener = TcpListener::bind(&addr).map_err(|e| format!("bind {}: {}", addr, e))?;
        listener.set_nonblocking(true).map_err(|e| e.to_string())?;

        let shutdown = Arc::new(AtomicBool::new(false));
        let shutdown_clone = shutdown.clone();

        let handle = thread::spawn(move || loop {
            if shutdown_clone.load(Ordering::Relaxed) {
                break;
            }
            match listener.accept() {
                Ok((client, _)) => {
                    let h = ssh_host.clone();
                    let p = ssh_port;
                    let u = username.clone();
                    let pw = password.clone();
                    let k = key_path.clone();
                    thread::spawn(move || {
                        if let Err(e) = handle_socks_connection(h, p, u, pw, k, client) {
                            eprintln!("[forward {}] socks error: {}", rule_id, e);
                        }
                    });
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(100));
                }
                Err(e) => {
                    eprintln!("[forward {}] listener error: {}", rule_id, e);
                    break;
                }
            }
        });

        let mut active = self.active.lock().map_err(|e| e.to_string())?;
        active.insert(
            rule_id,
            ActiveForward {
                rule_id,
                shutdown,
                thread_handle: Some(handle),
            },
        );

        Ok(())
    }

    pub fn stop(&self, rule_id: i64) {
        let mut active = self.active.lock().unwrap();
        if let Some(fw) = active.remove(&rule_id) {
            fw.shutdown.store(true, Ordering::Relaxed);
        }
    }

    pub fn is_active(&self, rule_id: i64) -> bool {
        let active = self.active.lock().unwrap();
        active.contains_key(&rule_id)
    }
}

fn create_ssh_session(
    host: &str,
    port: u16,
    username: &str,
    password: Option<&str>,
    key_path: Option<&str>,
) -> Result<Session, String> {
    let tcp = crate::ssh::session::resolve_and_connect(host, port)
        .map_err(|e| format!("connect: {}", e))?;
    let mut session = Session::new().map_err(|e| format!("session: {}", e))?;
    session.set_tcp_stream(tcp);
    session
        .handshake()
        .map_err(|e| format!("handshake: {}", e))?;

    if let Some(key_path) = key_path {
        let expanded = if key_path.starts_with("~/") {
            dirs::home_dir()
                .map(|h| h.join(&key_path[2..]))
                .unwrap_or_else(|| Path::new(key_path).to_path_buf())
        } else {
            Path::new(key_path).to_path_buf()
        };
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

fn handle_local_connection(
    ssh_host: String,
    ssh_port: u16,
    username: String,
    password: Option<String>,
    key_path: Option<String>,
    mut client: TcpStream,
    remote_host: String,
    remote_port: u16,
) -> Result<(), String> {
    let pw = password.as_deref();
    let kp = key_path.as_deref();
    let session = create_ssh_session(&ssh_host, ssh_port, &username, pw, kp)?;

    let mut channel = session
        .channel_direct_tcpip(&remote_host, remote_port, Some(("127.0.0.1", 0)))
        .map_err(|e| format!("direct_tcpip: {}", e))?;

    // Switch to non-blocking mode so a single thread can poll both directions
    session.set_blocking(false);
    client.set_nonblocking(true).map_err(|e| e.to_string())?;

    pipe_bidirectional_nb(&mut client, &mut channel)?;
    let _ = client.shutdown(Shutdown::Both);
    Ok(())
}

fn handle_socks_connection(
    ssh_host: String,
    ssh_port: u16,
    username: String,
    password: Option<String>,
    key_path: Option<String>,
    mut client: TcpStream,
) -> Result<(), String> {
    // SOCKS5 greeting
    let mut greet = [0u8; 2];
    client
        .read_exact(&mut greet)
        .map_err(|e| format!("socks greet: {}", e))?;
    if greet[0] != 0x05 {
        return Err("unsupported SOCKS version".to_string());
    }
    let nmethods = greet[1] as usize;
    let mut methods = vec![0u8; nmethods];
    client
        .read_exact(&mut methods)
        .map_err(|e| format!("socks methods: {}", e))?;

    // Respond: no auth required
    client
        .write_all(&[0x05, 0x00])
        .map_err(|e| format!("socks auth resp: {}", e))?;

    // Request
    let mut req = [0u8; 4];
    client
        .read_exact(&mut req)
        .map_err(|e| format!("socks req: {}", e))?;
    if req[0] != 0x05 || req[1] != 0x01 {
        client
            .write_all(&[0x05, 0x07, 0x00, 0x01, 0, 0, 0, 0, 0, 0])
            .ok();
        return Err("unsupported SOCKS command".to_string());
    }

    let dst = match req[3] {
        0x01 => {
            // IPv4
            let mut addr = [0u8; 4];
            client
                .read_exact(&mut addr)
                .map_err(|e| format!("socks ipv4: {}", e))?;
            let mut port = [0u8; 2];
            client
                .read_exact(&mut port)
                .map_err(|e| format!("socks port: {}", e))?;
            let port = u16::from_be_bytes(port);
            (
                format!("{}.{}", addr[0], addr[1]),
                format!("{}.{}", addr[2], addr[3]),
                port,
            )
        }
        0x03 => {
            // Domain
            let mut len = [0u8; 1];
            client
                .read_exact(&mut len)
                .map_err(|e| format!("socks domain len: {}", e))?;
            let mut domain = vec![0u8; len[0] as usize];
            client
                .read_exact(&mut domain)
                .map_err(|e| format!("socks domain: {}", e))?;
            let mut port = [0u8; 2];
            client
                .read_exact(&mut port)
                .map_err(|e| format!("socks port: {}", e))?;
            let port = u16::from_be_bytes(port);
            let host = String::from_utf8_lossy(&domain).to_string();
            (host.clone(), host, port)
        }
        _ => {
            client
                .write_all(&[0x05, 0x08, 0x00, 0x01, 0, 0, 0, 0, 0, 0])
                .ok();
            return Err("unsupported address type".to_string());
        }
    };

    let pw = password.as_deref();
    let kp = key_path.as_deref();
    let session = create_ssh_session(&ssh_host, ssh_port, &username, pw, kp)?;

    let mut channel = session
        .channel_direct_tcpip(&dst.0, dst.2, Some(("127.0.0.1", 0)))
        .map_err(|e| format!("direct_tcpip: {}", e))?;

    // Respond success
    client
        .write_all(&[0x05, 0x00, 0x00, 0x01, 0, 0, 0, 0, 0, 0])
        .map_err(|e| format!("socks success resp: {}", e))?;

    // Switch to non-blocking mode so a single thread can poll both directions
    session.set_blocking(false);
    client.set_nonblocking(true).map_err(|e| e.to_string())?;

    pipe_bidirectional_nb(&mut client, &mut channel)?;
    let _ = client.shutdown(Shutdown::Both);
    Ok(())
}

fn pipe_bidirectional_nb(
    client: &mut TcpStream,
    channel: &mut ssh2::Channel,
) -> Result<(), String> {
    let mut buf_c2s = [0u8; 8192];
    let mut buf_s2c = [0u8; 8192];

    // Pending data when a non-blocking write only accepts part of the buffer
    let mut pending_c2s: Vec<u8> = Vec::new();
    let mut pending_s2c: Vec<u8> = Vec::new();

    loop {
        let mut progress = false;

        // --- Client -> Server (SSH channel) ---
        if pending_c2s.is_empty() {
            match client.read(&mut buf_c2s) {
                Ok(0) => break,
                Ok(n) => {
                    pending_c2s.extend_from_slice(&buf_c2s[..n]);
                    progress = true;
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(e) => {
                    eprintln!("port-forward client read error: {}", e);
                    break;
                }
            }
        }

        if !pending_c2s.is_empty() {
            match channel.write(&pending_c2s) {
                Ok(0) => {
                    eprintln!("port-forward channel write returned 0");
                    break;
                }
                Ok(n) => {
                    pending_c2s.drain(..n);
                    progress = true;
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(e) => {
                    eprintln!("port-forward channel write error: {}", e);
                    break;
                }
            }
        }

        // --- Server (SSH channel) -> Client ---
        if pending_s2c.is_empty() {
            match channel.read(&mut buf_s2c) {
                Ok(0) => break,
                Ok(n) => {
                    pending_s2c.extend_from_slice(&buf_s2c[..n]);
                    progress = true;
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(e) => {
                    eprintln!("port-forward channel read error: {}", e);
                    break;
                }
            }
        }

        if !pending_s2c.is_empty() {
            match client.write(&pending_s2c) {
                Ok(0) => {
                    eprintln!("port-forward client write returned 0");
                    break;
                }
                Ok(n) => {
                    pending_s2c.drain(..n);
                    progress = true;
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(e) => {
                    eprintln!("port-forward client write error: {}", e);
                    break;
                }
            }
        }

        // Prevent busy-waiting when both directions are idle
        if !progress {
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    Ok(())
}
