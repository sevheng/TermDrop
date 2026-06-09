use ssh2::Channel;
use std::io::{Read, Write};
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

/// Flush output buffer if it exceeds this size (bytes).
const OUTPUT_BATCH_SIZE: usize = 4096;
/// Flush output buffer at this interval during heavy output (ms).
const OUTPUT_FLUSH_INTERVAL_MS: u64 = 16;

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
    let mut output_buf = String::with_capacity(8192);
    let mut last_flush = Instant::now();
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
            Ok(0) => {
                if !output_buf.is_empty() {
                    on_data(&output_buf);
                    output_buf.clear();
                }
                break;
            }
            Ok(n) => {
                output_buf.push_str(&String::from_utf8_lossy(&buf[..n]));
                // Flush immediately if buffer is small and idle, or if buffer is large
                let now = Instant::now();
                let elapsed = now.duration_since(last_flush).as_millis() as u64;
                if output_buf.len() >= OUTPUT_BATCH_SIZE || elapsed >= OUTPUT_FLUSH_INTERVAL_MS {
                    on_data(&output_buf);
                    output_buf.clear();
                    last_flush = now;
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                // Flush any pending output before sleeping
                if !output_buf.is_empty() {
                    let elapsed = Instant::now().duration_since(last_flush).as_millis() as u64;
                    if elapsed >= OUTPUT_FLUSH_INTERVAL_MS {
                        on_data(&output_buf);
                        output_buf.clear();
                        last_flush = Instant::now();
                    }
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(_) => {
                if !output_buf.is_empty() {
                    on_data(&output_buf);
                    output_buf.clear();
                }
                break;
            }
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
    let mut output_buf = String::with_capacity(8192);
    let mut last_flush = Instant::now();
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
            Ok(0) => {
                if !output_buf.is_empty() {
                    on_data(&output_buf);
                    output_buf.clear();
                }
                break;
            }
            Ok(n) => {
                output_buf.push_str(&String::from_utf8_lossy(&buf[..n]));
                let now = Instant::now();
                let elapsed = now.duration_since(last_flush).as_millis() as u64;
                if output_buf.len() >= OUTPUT_BATCH_SIZE || elapsed >= OUTPUT_FLUSH_INTERVAL_MS {
                    on_data(&output_buf);
                    output_buf.clear();
                    last_flush = now;
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                if !output_buf.is_empty() {
                    let elapsed = Instant::now().duration_since(last_flush).as_millis() as u64;
                    if elapsed >= OUTPUT_FLUSH_INTERVAL_MS {
                        on_data(&output_buf);
                        output_buf.clear();
                        last_flush = Instant::now();
                    }
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(_) => {
                if !output_buf.is_empty() {
                    on_data(&output_buf);
                    output_buf.clear();
                }
                break;
            }
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
