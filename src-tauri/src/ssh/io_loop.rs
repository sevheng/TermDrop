use ssh2::Channel;
use std::io::{Read, Write};
use std::time::Duration;
use tokio::sync::mpsc;

/// Run the main I/O loop for a PTY channel.
/// Reads from `channel`, writes from `write_rx`, handles disconnect and resize.
pub fn run_io_loop(
    mut channel: Channel,
    mut write_rx: mpsc::UnboundedReceiver<String>,
    mut disconnect_rx: mpsc::UnboundedReceiver<()>,
    mut resize_rx: mpsc::UnboundedReceiver<(u32, u32)>,
    mut on_data: impl FnMut(&str),
    mut on_disconnect: impl FnMut(),
) {
    let mut buf = vec![0u8; 16384];
    let mut intentional_disconnect = false;

    loop {
        if disconnect_rx.try_recv().is_ok() {
            intentional_disconnect = true;
            break;
        }

        // Handle resize requests
        while let Ok((cols, rows)) = resize_rx.try_recv() {
            loop {
                match channel.request_pty_size(cols, rows, None, None) {
                    Ok(()) => break,
                    Err(e) => {
                        let io_err: std::io::Error = e.into();
                        if io_err.kind() == std::io::ErrorKind::WouldBlock {
                            std::thread::sleep(Duration::from_millis(5));
                            continue;
                        }
                        break;
                    }
                }
            }
        }

        // Handle outgoing data (keystrokes)
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

        // Read incoming data
        match channel.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                let data = String::from_utf8_lossy(&buf[..n]);
                on_data(&data);
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
        on_disconnect();
    }
}

/// Run the main I/O loop for an exec PTY channel (no resize support).
pub fn run_exec_pty_loop(
    mut channel: Channel,
    mut write_rx: mpsc::UnboundedReceiver<String>,
    mut disconnect_rx: mpsc::UnboundedReceiver<()>,
    mut on_data: impl FnMut(&str),
    mut on_disconnect: impl FnMut(),
) {
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
                on_data(&data);
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
        on_disconnect();
    }
}
