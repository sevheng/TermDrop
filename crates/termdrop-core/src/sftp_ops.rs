//! The file panel's operations: what the SFTP panel asks of a tab's SFTP
//! session, plus the two local-file steps around it (where downloads land,
//! writing a file the user picked).
//!
//! These were the bodies of the Tauri commands of the same names. They live
//! here so the native app runs exactly the same steps; error text is
//! unchanged, because the Vue frontend shows it as is. Every blocking SFTP
//! call goes through [`sftp_blocking`], which runs it off the async thread on
//! the session the id names.
//!
//! Transfer progress goes out as `CoreEvent::SftpProgress` on the `events`
//! a transfer is given.

use crate::events::Events;
use crate::sftp::{self, SftpFile, SftpStat};
use crate::state::{exec_session, sftp_blocking, sftp_handle};
use crate::Core;
use std::path::{Path, PathBuf};

/// Quote `s` for a POSIX shell, leaving plain paths as they are.
pub fn shell_escape(s: &str) -> String {
    if s.is_empty() {
        return "''".to_string();
    }
    if s.chars()
        .all(|c| c.is_alphanumeric() || "_-./:@".contains(c))
    {
        return s.to_string();
    }
    format!("'{}'", s.replace('\'', "'\"'\"'"))
}

/// Where downloads land: the user's Downloads folder, `~/Downloads` when the
/// platform does not name one, the temp dir as a last resort.
pub fn download_dir() -> PathBuf {
    dirs::download_dir()
        .or_else(|| dirs::home_dir().map(|h| h.join("Downloads")))
        .unwrap_or_else(std::env::temp_dir)
}

/// The last component of a remote path, or `fallback` for `/` and the like.
fn file_name_or(remote_path: &str, fallback: &str) -> String {
    Path::new(remote_path)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| fallback.to_string())
}

/// Write `content` to a local file the user chose (the editor's "save a
/// copy"). Not an SFTP call: it never touches the remote.
pub fn write_file(path: &str, content: &str) -> Result<(), String> {
    std::fs::write(path, content).map_err(|e| e.to_string())
}

impl Core {
    pub async fn sftp_list(
        &self,
        sftp_session_id: &str,
        path: String,
    ) -> Result<Vec<SftpFile>, String> {
        sftp_blocking(self, sftp_session_id, move |handle| {
            sftp::sftp_list(handle, &path)
        })
        .await
    }

    /// Upload a local file, reporting progress as `SftpProgress` events keyed
    /// by `sftp_session_id` and the remote path.
    pub async fn sftp_upload(
        &self,
        events: Events,
        sftp_session_id: &str,
        local_path: String,
        remote_path: String,
    ) -> Result<(), String> {
        let session_id = sftp_session_id.to_string();
        sftp_blocking(self, sftp_session_id, move |handle| {
            sftp::sftp_upload(events, &session_id, handle, &local_path, &remote_path)
        })
        .await
    }

    /// Download a remote file into [`download_dir`] under its own name,
    /// returning the local path. Progress as for [`Self::sftp_upload`].
    pub async fn sftp_download(
        &self,
        events: Events,
        sftp_session_id: &str,
        remote_path: String,
    ) -> Result<String, String> {
        let local_path = download_dir().join(file_name_or(&remote_path, "download"));
        let local_path = local_path.to_string_lossy().to_string();
        self.sftp_download_to(events, sftp_session_id, remote_path, local_path.clone())
            .await?;
        Ok(local_path)
    }

    /// As [`Self::sftp_download`], to a local path the caller chose.
    pub async fn sftp_download_to(
        &self,
        events: Events,
        sftp_session_id: &str,
        remote_path: String,
        local_path: String,
    ) -> Result<(), String> {
        let session_id = sftp_session_id.to_string();
        sftp_blocking(self, sftp_session_id, move |handle| {
            sftp::sftp_download(events, &session_id, handle, &remote_path, &local_path)
        })
        .await
    }

    /// Download a remote directory: tar it on the host (over the host's
    /// shared exec session), fetch the archive over SFTP, unpack it into
    /// [`download_dir`] and clean up both copies. Returns the local
    /// directory. Needs a terminal open on the host, since that is what keeps
    /// the exec session alive.
    pub async fn sftp_download_dir(
        &self,
        sftp_session_id: &str,
        remote_path: String,
    ) -> Result<String, String> {
        // Get SFTP handle and host_id
        let handle = sftp_handle(self, sftp_session_id)?;
        let host_id = handle.host_id;

        // Get exec session for tar command
        let exec_session = exec_session(self, host_id, "No exec session for this host")?;

        let folder_name = file_name_or(&remote_path, "archive");
        let parent_path = Path::new(&remote_path)
            .parent()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|| "/".to_string());

        let remote_temp = format!("/tmp/termdrop-{}.tar.gz", uuid::Uuid::new_v4());

        // Create tar.gz on remote
        let remote_temp_clone = remote_temp.clone();
        let folder_name_clone = folder_name.clone();
        tokio::task::spawn_blocking(move || {
            let session = exec_session.lock().map_err(|e| e.to_string())?;
            let mut channel = session.channel_session().map_err(|e| e.to_string())?;
            let cmd = format!(
                "tar -czf {} -C {} {}",
                shell_escape(&remote_temp_clone),
                shell_escape(&parent_path),
                shell_escape(&folder_name_clone)
            );
            channel.exec(&cmd).map_err(|e| e.to_string())?;
            let mut stdout = String::new();
            use std::io::Read;
            channel
                .read_to_string(&mut stdout)
                .map_err(|e| e.to_string())?;
            let mut stderr = String::new();
            channel
                .stderr()
                .read_to_string(&mut stderr)
                .map_err(|e| e.to_string())?;
            channel.wait_close().ok();
            let status = channel.exit_status().unwrap_or(0);
            if status != 0 {
                return Err(format!(
                    "tar failed: {}",
                    if stderr.is_empty() { stdout } else { stderr }
                ));
            }
            Ok(())
        })
        .await
        .map_err(|e| e.to_string())??;

        // Download the archive (blocking I/O off the async thread)
        let download_dir = download_dir();
        let local_archive = download_dir.join(format!("{}.tar.gz", folder_name));
        let local_archive_str = local_archive.to_string_lossy().to_string();
        let remote_temp_clone = remote_temp.clone();
        let handle_clone = handle.clone();
        let local_archive_str_for_dl = local_archive_str.clone();

        tokio::task::spawn_blocking(move || {
            sftp::sftp_download_simple(&handle_clone, &remote_temp_clone, &local_archive_str_for_dl)
        })
        .await
        .map_err(|e| format!("download task failed: {}", e))?
        .map_err(|e| format!("download failed: {}", e))?;

        // Extract locally
        let extract_dir = download_dir.join(&folder_name);
        let extract_dir_str = extract_dir.to_string_lossy().to_string();
        let extract_dir_str_clone = extract_dir_str.clone();
        tokio::task::spawn_blocking(move || {
            std::fs::create_dir_all(&extract_dir_str_clone)
                .map_err(|e| format!("create dir: {}", e))?;
            let output = std::process::Command::new("tar")
                .args([
                    "-xzf",
                    &local_archive_str,
                    "-C",
                    &extract_dir_str_clone,
                    "--strip-components=1",
                ])
                .output()
                .map_err(|e| format!("extract: {}", e))?;
            if !output.status.success() {
                let err = String::from_utf8_lossy(&output.stderr);
                return Err(format!("extract failed: {}", err));
            }
            Ok(())
        })
        .await
        .map_err(|e| e.to_string())??;

        // Clean up local archive and remote temp (best effort)
        let _ = tokio::task::spawn_blocking(move || {
            let _ = std::fs::remove_file(&local_archive);
            let _ = sftp::sftp_delete(&handle, &remote_temp);
        })
        .await;

        Ok(extract_dir_str)
    }

    pub async fn sftp_write_file(
        &self,
        sftp_session_id: &str,
        remote_path: String,
        content: String,
    ) -> Result<(), String> {
        sftp_blocking(self, sftp_session_id, move |handle| {
            sftp::sftp_write_file(handle, &remote_path, &content)
        })
        .await
    }

    /// Size and mtime of a remote file, used to detect that it changed while
    /// an editor had it open.
    pub async fn sftp_stat_file(
        &self,
        sftp_session_id: &str,
        remote_path: String,
    ) -> Result<SftpStat, String> {
        sftp_blocking(self, sftp_session_id, move |handle| {
            sftp::sftp_stat_file(handle, &remote_path)
        })
        .await
    }

    pub async fn sftp_realpath(
        &self,
        sftp_session_id: &str,
        remote_path: String,
    ) -> Result<String, String> {
        sftp_blocking(self, sftp_session_id, move |handle| {
            sftp::sftp_realpath(handle, &remote_path)
        })
        .await
    }

    pub async fn sftp_delete(
        &self,
        sftp_session_id: &str,
        remote_path: String,
    ) -> Result<(), String> {
        sftp_blocking(self, sftp_session_id, move |handle| {
            sftp::sftp_delete(handle, &remote_path)
        })
        .await
    }

    pub async fn sftp_rename(
        &self,
        sftp_session_id: &str,
        old_path: String,
        new_path: String,
    ) -> Result<(), String> {
        sftp_blocking(self, sftp_session_id, move |handle| {
            sftp::sftp_rename(handle, &old_path, &new_path)
        })
        .await
    }

    pub async fn sftp_mkdir(
        &self,
        sftp_session_id: &str,
        remote_path: String,
    ) -> Result<(), String> {
        sftp_blocking(self, sftp_session_id, move |handle| {
            sftp::sftp_mkdir(handle, &remote_path)
        })
        .await
    }

    pub async fn sftp_rmdir(
        &self,
        sftp_session_id: &str,
        remote_path: String,
    ) -> Result<(), String> {
        sftp_blocking(self, sftp_session_id, move |handle| {
            sftp::sftp_rmdir(handle, &remote_path)
        })
        .await
    }

    pub async fn sftp_read_file(
        &self,
        sftp_session_id: &str,
        remote_path: String,
    ) -> Result<String, String> {
        sftp_blocking(self, sftp_session_id, move |handle| {
            sftp::sftp_read_file(handle, &remote_path)
        })
        .await
    }

    pub async fn sftp_read_file_base64(
        &self,
        sftp_session_id: &str,
        remote_path: String,
    ) -> Result<String, String> {
        sftp_blocking(self, sftp_session_id, move |handle| {
            sftp::sftp_read_file_base64(handle, &remote_path)
        })
        .await
    }

    /// Drop a tab's SFTP session. Unknown ids are not an error.
    pub fn sftp_disconnect(&self, sftp_session_id: &str) -> Result<(), String> {
        let mut sftp_sessions = self.sftp_sessions.lock().map_err(|e| e.to_string())?;
        sftp_sessions.remove(sftp_session_id);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_escape_leaves_plain_paths_and_quotes_the_rest() {
        assert_eq!(shell_escape(""), "''");
        assert_eq!(
            shell_escape("/var/log/app-1.2_x:y@z"),
            "/var/log/app-1.2_x:y@z"
        );
        assert_eq!(shell_escape("a b"), "'a b'");
        assert_eq!(shell_escape("it's"), "'it'\"'\"'s'");
        assert_eq!(shell_escape("$(rm -rf /)"), "'$(rm -rf /)'");
    }

    #[test]
    fn file_name_falls_back_for_a_root_path() {
        assert_eq!(file_name_or("/srv/data", "archive"), "data");
        assert_eq!(file_name_or("/", "archive"), "archive");
    }

    #[test]
    fn write_file_writes_and_reports_errors_as_text() {
        let dir = std::env::temp_dir().join(format!("td-sftp-ops-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("f.txt");
        write_file(path.to_str().unwrap(), "hello").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "hello");
        let missing = dir.join("no/such/dir/f.txt");
        assert!(write_file(missing.to_str().unwrap(), "x").is_err());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn sftp_calls_on_an_unknown_session_say_so() {
        let core = crate::state::tests_support::memory_core();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let err = rt.block_on(core.sftp_list("nope", "/".into())).unwrap_err();
        assert_eq!(err, "SFTP session not found");
        let err = rt
            .block_on(core.sftp_download_dir("nope", "/x".into()))
            .unwrap_err();
        assert_eq!(err, "SFTP session not found");
        assert!(core.sftp_disconnect("nope").is_ok());
    }
}
