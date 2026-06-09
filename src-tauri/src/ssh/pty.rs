use ssh2::{Session, Channel};
use std::time::Duration;

/// Open a channel session, request a PTY, and start a shell.
pub fn create_pty_channel(
    session: &Session,
    cols: u32,
    rows: u32,
) -> Result<Channel, String> {
    let mut channel = loop {
        match session.channel_session() {
            Ok(c) => break c,
            Err(e) => {
                let io_err: std::io::Error = e.into();
                if io_err.kind() == std::io::ErrorKind::WouldBlock {
                    std::thread::sleep(Duration::from_millis(10));
                    continue;
                }
                return Err(format!("channel: {}", io_err));
            }
        }
    };

    loop {
        match channel.request_pty("xterm-256color", None, Some((cols, rows, 0, 0))) {
            Ok(()) => break,
            Err(e) => {
                let io_err: std::io::Error = e.into();
                if io_err.kind() == std::io::ErrorKind::WouldBlock {
                    std::thread::sleep(Duration::from_millis(10));
                    continue;
                }
                return Err(format!("pty: {}", io_err));
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
                return Err(format!("shell: {}", io_err));
            }
        }
    }

    Ok(channel)
}

/// Open a channel session, request a PTY, and execute a command.
pub fn create_exec_pty_channel(
    session: &Session,
    command: &str,
) -> Result<Channel, String> {
    let mut channel = loop {
        match session.channel_session() {
            Ok(c) => break c,
            Err(e) => {
                let io_err: std::io::Error = e.into();
                if io_err.kind() == std::io::ErrorKind::WouldBlock {
                    std::thread::sleep(Duration::from_millis(10));
                    continue;
                }
                return Err(format!("channel: {}", io_err));
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
                return Err(format!("pty: {}", io_err));
            }
        }
    }

    loop {
        match channel.exec(command) {
            Ok(()) => break,
            Err(e) => {
                let io_err: std::io::Error = e.into();
                if io_err.kind() == std::io::ErrorKind::WouldBlock {
                    std::thread::sleep(Duration::from_millis(10));
                    continue;
                }
                return Err(format!("exec: {}", io_err));
            }
        }
    }

    Ok(channel)
}
