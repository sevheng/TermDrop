use super::session::retry_would_block;
use ssh2::{Channel, Session};

/// Open a channel session, request a PTY, and start a shell.
pub fn create_pty_channel(session: &Session, cols: u32, rows: u32) -> Result<Channel, String> {
    let mut channel = retry_would_block("channel", || session.channel_session())?;
    retry_would_block("pty", || {
        channel.request_pty("xterm-256color", None, Some((cols, rows, 0, 0)))
    })?;
    retry_would_block("shell", || channel.shell())?;
    Ok(channel)
}

/// Open a channel session, request a PTY, and execute a command.
pub fn create_exec_pty_channel(session: &Session, command: &str) -> Result<Channel, String> {
    let mut channel = retry_would_block("channel", || session.channel_session())?;
    retry_would_block("pty", || channel.request_pty("xterm-256color", None, None))?;
    retry_would_block("exec", || channel.exec(command))?;
    Ok(channel)
}
