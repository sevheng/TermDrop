use std::io::{Read, Write};
use std::path::Path;
use std::time::{Duration, Instant};
use ssh2::Session;
use serde::Serialize;
use tauri::{Emitter, Window};
use base64::Engine;

const SFTP_BUF_SIZE: usize = 65536; // 64KB
const PROGRESS_MIN_INTERVAL: Duration = Duration::from_millis(100);
const PROGRESS_MIN_PERCENT_DELTA: f64 = 1.0; // at least 1% change

#[derive(Debug, Serialize, Clone)]
pub struct SftpFile {
    pub name: String,
    pub path: String,
    pub size: u64,
    pub is_dir: bool,
    pub modified: Option<u64>,
    pub permissions: Option<u32>,
    pub uid: Option<u32>,
    pub gid: Option<u32>,
}

pub struct SftpSessionHandle {
    pub session: std::sync::Mutex<Session>,
    pub host_id: i64,
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

pub fn sftp_connect(
    host: String,
    port: u16,
    username: String,
    password: Option<String>,
    key_path: Option<String>,
    host_id: i64,
) -> Result<SftpSessionHandle, String> {
    let addr = format!("{}:{}", host, port);
    let tcp = std::net::TcpStream::connect(&addr)
        .map_err(|e| format!("connect: {}", e))?;
    let mut session = Session::new()
        .map_err(|e| format!("session: {}", e))?;
    session.set_tcp_stream(tcp);
    session.handshake()
        .map_err(|e| format!("handshake: {}", e))?;

    if let Some(key_path) = key_path {
        let expanded = expand_key_path(&key_path);
        session.userauth_pubkey_file(&username, None, &expanded, None)
            .map_err(|e| format!("key auth: {}", e))?;
    } else if let Some(password) = password {
        session.userauth_password(&username, &password)
            .map_err(|e| format!("auth: {}", e))?;
    } else {
        return Err("no credentials provided".to_string());
    }

    Ok(SftpSessionHandle {
        session: std::sync::Mutex::new(session),
        host_id,
    })
}

pub fn sftp_list(
    handle: &SftpSessionHandle,
    path: &str,
) -> Result<Vec<SftpFile>, String> {
    let session = handle.session.lock().map_err(|e| e.to_string())?;
    let sftp = session.sftp().map_err(|e| format!("sftp: {}", e))?;
    let entries = sftp.readdir(Path::new(path))
        .map_err(|e| format!("readdir: {}", e))?;

    let mut files = Vec::new();
    for (pathbuf, stat) in entries {
        let name = pathbuf.file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        if name == "." || name == ".." {
            continue;
        }
        files.push(SftpFile {
            path: pathbuf.to_string_lossy().to_string(),
            name,
            size: stat.size.unwrap_or(0),
            is_dir: stat.file_type().is_dir(),
            modified: stat.mtime.map(|t| t as u64),
            permissions: stat.perm,
            uid: stat.uid,
            gid: stat.gid,
        });
    }

    files.sort_by(|a, b| {
        if a.is_dir == b.is_dir {
            a.name.cmp(&b.name)
        } else if a.is_dir {
            std::cmp::Ordering::Less
        } else {
            std::cmp::Ordering::Greater
        }
    });

    Ok(files)
}

pub fn sftp_upload(
    window: Window,
    handle: &SftpSessionHandle,
    local_path: &str,
    remote_path: &str,
) -> Result<(), String> {
    let local_file = std::fs::File::open(local_path)
        .map_err(|e| format!("open local: {}", e))?;
    let metadata = local_file.metadata()
        .map_err(|e| format!("metadata: {}", e))?;
    let total = metadata.len();

    let session = handle.session.lock().map_err(|e| e.to_string())?;
    let sftp = session.sftp().map_err(|e| format!("sftp: {}", e))?;
    let mut remote_file = sftp.create(Path::new(remote_path))
        .map_err(|e| format!("create remote: {}", e))?;

    let mut buf = vec![0u8; SFTP_BUF_SIZE];
    let mut transferred = 0u64;
    let mut last_emit = Instant::now();
    let mut last_percent = 0.0;
    let mut reader = std::io::BufReader::new(local_file);

    loop {
        let n = reader.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 { break; }
        remote_file.write_all(&buf[..n]).map_err(|e| e.to_string())?;
        transferred += n as u64;

        let percent = if total > 0 {
            (transferred as f64 / total as f64) * 100.0
        } else {
            100.0
        };
        let elapsed = last_emit.elapsed();
        let percent_delta = percent - last_percent;

        if elapsed >= PROGRESS_MIN_INTERVAL || percent_delta >= PROGRESS_MIN_PERCENT_DELTA || transferred >= total {
            let payload = serde_json::json!({
                "file": remote_path,
                "bytes_transferred": transferred,
                "total_bytes": total,
            });
            let _ = window.emit("sftp-progress", payload);
            last_emit = Instant::now();
            last_percent = percent;
        }
    }

    // Final progress event
    let payload = serde_json::json!({
        "file": remote_path,
        "bytes_transferred": transferred,
        "total_bytes": total,
    });
    let _ = window.emit("sftp-progress", payload);

    Ok(())
}

pub fn sftp_download(
    window: Window,
    handle: &SftpSessionHandle,
    remote_path: &str,
    local_path: &str,
) -> Result<(), String> {
    let session = handle.session.lock().map_err(|e| e.to_string())?;
    let sftp = session.sftp().map_err(|e| format!("sftp: {}", e))?;
    let mut remote_file = sftp.open(Path::new(remote_path))
        .map_err(|e| format!("open remote: {}", e))?;
    let total = remote_file.stat().map(|s| s.size.unwrap_or(0)).unwrap_or(0);

    if let Some(parent) = Path::new(local_path).parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("create dir: {}", e))?;
    }
    let mut local_file = std::fs::File::create(local_path)
        .map_err(|e| format!("create local: {}", e))?;

    let mut buf = vec![0u8; SFTP_BUF_SIZE];
    let mut transferred = 0u64;
    let mut last_emit = Instant::now();
    let mut last_percent = 0.0;

    loop {
        let n = remote_file.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 { break; }
        local_file.write_all(&buf[..n]).map_err(|e| e.to_string())?;
        transferred += n as u64;

        let percent = if total > 0 {
            (transferred as f64 / total as f64) * 100.0
        } else {
            100.0
        };
        let elapsed = last_emit.elapsed();
        let percent_delta = percent - last_percent;

        if elapsed >= PROGRESS_MIN_INTERVAL || percent_delta >= PROGRESS_MIN_PERCENT_DELTA || transferred >= total {
            let payload = serde_json::json!({
                "file": remote_path,
                "bytes_transferred": transferred,
                "total_bytes": total,
            });
            let _ = window.emit("sftp-progress", payload);
            last_emit = Instant::now();
            last_percent = percent;
        }
    }

    // Final progress event
    let payload = serde_json::json!({
        "file": remote_path,
        "bytes_transferred": transferred,
        "total_bytes": total,
    });
    let _ = window.emit("sftp-progress", payload);

    Ok(())
}

pub fn sftp_download_simple(
    handle: &SftpSessionHandle,
    remote_path: &str,
    local_path: &str,
) -> Result<(), String> {
    let session = handle.session.lock().map_err(|e| e.to_string())?;
    let sftp = session.sftp().map_err(|e| format!("sftp: {}", e))?;
    let mut remote_file = sftp.open(Path::new(remote_path))
        .map_err(|e| format!("open remote: {}", e))?;

    if let Some(parent) = Path::new(local_path).parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("create dir: {}", e))?;
    }
    let mut local_file = std::fs::File::create(local_path)
        .map_err(|e| format!("create local: {}", e))?;

    let mut buf = vec![0u8; SFTP_BUF_SIZE];
    loop {
        let n = remote_file.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 { break; }
        local_file.write_all(&buf[..n]).map_err(|e| e.to_string())?;
    }
    Ok(())
}

pub fn sftp_realpath(
    handle: &SftpSessionHandle,
    remote_path: &str,
) -> Result<String, String> {
    let session = handle.session.lock().map_err(|e| e.to_string())?;
    let sftp = session.sftp().map_err(|e| format!("sftp: {}", e))?;
    let resolved = sftp.realpath(Path::new(remote_path))
        .map_err(|e| format!("realpath: {}", e))?;
    Ok(resolved.to_string_lossy().to_string())
}

pub fn sftp_delete(
    handle: &SftpSessionHandle,
    remote_path: &str,
) -> Result<(), String> {
    let session = handle.session.lock().map_err(|e| e.to_string())?;
    let sftp = session.sftp().map_err(|e| format!("sftp: {}", e))?;
    sftp.unlink(Path::new(remote_path))
        .map_err(|e| format!("delete: {}", e))?;
    Ok(())
}

pub fn sftp_rename(
    handle: &SftpSessionHandle,
    old_path: &str,
    new_path: &str,
) -> Result<(), String> {
    let session = handle.session.lock().map_err(|e| e.to_string())?;
    let sftp = session.sftp().map_err(|e| format!("sftp: {}", e))?;
    sftp.rename(Path::new(old_path), Path::new(new_path), None)
        .map_err(|e| format!("rename: {}", e))?;
    Ok(())
}

pub fn sftp_mkdir(
    handle: &SftpSessionHandle,
    remote_path: &str,
) -> Result<(), String> {
    let session = handle.session.lock().map_err(|e| e.to_string())?;
    let sftp = session.sftp().map_err(|e| format!("sftp: {}", e))?;
    sftp.mkdir(Path::new(remote_path), 0o755)
        .map_err(|e| format!("mkdir: {}", e))?;
    Ok(())
}

fn sftp_rmdir_recursive(
    sftp: &ssh2::Sftp,
    path: &Path,
) -> Result<(), String> {
    let entries = sftp.readdir(path)
        .map_err(|e| format!("readdir: {}", e))?;
    for (entry_path, stat) in entries {
        let name = entry_path.file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        if name == "." || name == ".." {
            continue;
        }
        if stat.file_type().is_dir() {
            sftp_rmdir_recursive(sftp, &entry_path)?;
            sftp.rmdir(&entry_path)
                .map_err(|e| format!("rmdir: {}", e))?;
        } else {
            sftp.unlink(&entry_path)
                .map_err(|e| format!("unlink: {}", e))?;
        }
    }
    sftp.rmdir(path)
        .map_err(|e| format!("rmdir: {}", e))?;
    Ok(())
}

pub fn sftp_read_file(
    handle: &SftpSessionHandle,
    remote_path: &str,
) -> Result<String, String> {
    const MAX_SIZE: u64 = 5 * 1024 * 1024; // 5MB
    let session = handle.session.lock().map_err(|e| e.to_string())?;
    let sftp = session.sftp().map_err(|e| format!("sftp: {}", e))?;
    let mut remote_file = sftp.open(Path::new(remote_path))
        .map_err(|e| format!("open: {}", e))?;
    let stat = remote_file.stat()
        .map_err(|e| format!("stat: {}", e))?;
    let size = stat.size.unwrap_or(0);
    if size > MAX_SIZE {
        return Err(format!("File too large: {} bytes (max {})", size, MAX_SIZE));
    }
    let mut content = Vec::with_capacity(size as usize);
    use std::io::Read;
    remote_file.read_to_end(&mut content)
        .map_err(|e| format!("read: {}", e))?;
    String::from_utf8(content)
        .map_err(|_| "File contains non-UTF-8 data".to_string())
}

pub fn sftp_read_file_base64(
    handle: &SftpSessionHandle,
    remote_path: &str,
) -> Result<String, String> {
    const MAX_SIZE: u64 = 10 * 1024 * 1024; // 10MB for images
    let session = handle.session.lock().map_err(|e| e.to_string())?;
    let sftp = session.sftp().map_err(|e| format!("sftp: {}", e))?;
    let mut remote_file = sftp.open(Path::new(remote_path))
        .map_err(|e| format!("open: {}", e))?;
    let stat = remote_file.stat()
        .map_err(|e| format!("stat: {}", e))?;
    let size = stat.size.unwrap_or(0);
    if size > MAX_SIZE {
        return Err(format!("File too large: {} bytes (max {})", size, MAX_SIZE));
    }
    let mut content = Vec::with_capacity(size as usize);
    use std::io::Read;
    remote_file.read_to_end(&mut content)
        .map_err(|e| format!("read: {}", e))?;
    Ok(base64::engine::general_purpose::STANDARD.encode(&content))
}

pub fn sftp_rmdir(
    handle: &SftpSessionHandle,
    remote_path: &str,
) -> Result<(), String> {
    let session = handle.session.lock().map_err(|e| e.to_string())?;
    let sftp = session.sftp().map_err(|e| format!("sftp: {}", e))?;
    sftp_rmdir_recursive(&sftp, Path::new(remote_path))
}

pub fn sftp_write_file(
    handle: &SftpSessionHandle,
    remote_path: &str,
    content: &str,
) -> Result<(), String> {
    let session = handle.session.lock().map_err(|e| e.to_string())?;
    let sftp = session.sftp().map_err(|e| format!("sftp: {}", e))?;
    let mut remote_file = sftp.create(Path::new(remote_path))
        .map_err(|e| format!("create remote: {}", e))?;
    remote_file.write_all(content.as_bytes())
        .map_err(|e| format!("write: {}", e))?;
    Ok(())
}
