use std::io::{Read, Write};
use std::path::Path;
use ssh2::Session;
use serde::Serialize;
use tauri::{Emitter, Window};

#[derive(Debug, Serialize, Clone)]
pub struct SftpFile {
    pub name: String,
    pub path: String,
    pub size: u64,
    pub is_dir: bool,
    pub modified: Option<u64>,
}

pub struct SftpSessionHandle {
    pub session: std::sync::Mutex<Session>,
}

pub fn sftp_connect(
    host: String,
    port: u16,
    username: String,
    password: String,
) -> Result<SftpSessionHandle, String> {
    let addr = format!("{}:{}", host, port);
    let tcp = std::net::TcpStream::connect(&addr)
        .map_err(|e| format!("connect: {}", e))?;
    let mut session = Session::new()
        .map_err(|e| format!("session: {}", e))?;
    session.set_tcp_stream(tcp);
    session.handshake()
        .map_err(|e| format!("handshake: {}", e))?;
    session.userauth_password(&username, &password)
        .map_err(|e| format!("auth: {}", e))?;
    Ok(SftpSessionHandle {
        session: std::sync::Mutex::new(session),
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

    let mut buf = [0u8; 8192];
    let mut transferred = 0u64;
    let mut reader = std::io::BufReader::new(local_file);

    loop {
        let n = reader.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 { break; }
        remote_file.write_all(&buf[..n]).map_err(|e| e.to_string())?;
        transferred += n as u64;

        let payload = serde_json::json!({
            "file": remote_path,
            "bytes_transferred": transferred,
            "total_bytes": total,
        });
        let _ = window.emit("sftp-progress", payload);
    }

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

    let mut local_file = std::fs::File::create(local_path)
        .map_err(|e| format!("create local: {}", e))?;

    let mut buf = [0u8; 8192];
    let mut transferred = 0u64;

    loop {
        let n = remote_file.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 { break; }
        local_file.write_all(&buf[..n]).map_err(|e| e.to_string())?;
        transferred += n as u64;

        let payload = serde_json::json!({
            "file": remote_path,
            "bytes_transferred": transferred,
            "total_bytes": total,
        });
        let _ = window.emit("sftp-progress", payload);
    }

    Ok(())
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
