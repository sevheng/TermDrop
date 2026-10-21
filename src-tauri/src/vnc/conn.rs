//! The TCP connection and the RFB handshake.
//!
//! Blocking sockets on a dedicated thread, matching how `ssh/mod.rs` runs one
//! thread per PTY. The alternative — async — would mean enabling `tokio`'s
//! `net` and `io-util` features for this one module, and would put CPU-bound
//! pixel decoding on the shared runtime where it would compete with every
//! database call in the app.
//!
//! The awkwardness a read timeout buys is concentrated in
//! [`Conn::read_exact`]: a timeout can land in the middle of a message, so
//! every read retries until it has all it asked for. What it buys is a session
//! that notices [`Conn::shutdown`] within one timeout rather than hanging on a
//! dead socket until the OS gives up.

use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use super::auth;
use super::pixel::PixelFormat;
use super::proto::{self, ProtocolVersion, SecurityType, ServerInit};

/// How long a read blocks before the loop re-checks the shutdown flag.
const READ_TIMEOUT: Duration = Duration::from_millis(20);
/// How long to wait for the TCP connection itself.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// How long a write may block before the socket is treated as dead.
const WRITE_TIMEOUT: Duration = Duration::from_secs(5);

/// The error text a cancelled session reports.
///
/// Matched by the caller rather than being a variant, which is the same
/// arrangement `mongodb.rs` uses for its `cancelled` string.
pub const CANCELLED: &str = "cancelled";

pub struct Conn {
    stream: TcpStream,
    pub shutdown: Arc<AtomicBool>,
}

impl Conn {
    pub fn connect(host: &str, port: u16, shutdown: Arc<AtomicBool>) -> Result<Self, String> {
        let addr = (host, port)
            .to_socket_addrs()
            .map_err(|e| format!("could not resolve {}: {}", host, e))?
            .next()
            .ok_or_else(|| format!("{} resolved to no addresses", host))?;

        let stream = TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT)
            .map_err(|e| format!("could not connect to {}:{}: {}", host, port, e))?;

        // Nagle batches small writes for up to 40ms. RFB is request/response,
        // so every one of our writes is the thing the other end is waiting
        // for, and that delay lands squarely on the frame rate.
        let _ = stream.set_nodelay(true);
        stream
            .set_read_timeout(Some(READ_TIMEOUT))
            .map_err(|e| format!("could not configure the socket: {}", e))?;

        Ok(Self { stream, shutdown })
    }

    fn cancelled(&self) -> bool {
        self.shutdown.load(Ordering::Relaxed)
    }

    /// Fill `buf` completely, or fail.
    ///
    /// Retries around the read timeout, which is not an error here — it is the
    /// mechanism by which a blocked read stays interruptible.
    pub fn read_exact(&mut self, buf: &mut [u8]) -> Result<(), String> {
        let mut filled = 0;
        while filled < buf.len() {
            if self.cancelled() {
                return Err(CANCELLED.to_string());
            }
            match self.stream.read(&mut buf[filled..]) {
                Ok(0) => return Err("the server closed the connection".to_string()),
                Ok(n) => filled += n,
                Err(e)
                    if matches!(
                        e.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    ) => {}
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                Err(e) => return Err(format!("read failed: {}", e)),
            }
        }
        Ok(())
    }

    /// Read exactly `len` bytes into a fresh buffer.
    ///
    /// `len` always comes from something the server said, so it is capped: a
    /// hostile or confused length would otherwise be a single allocation large
    /// enough to take the app down.
    pub fn read_vec(&mut self, len: usize, cap: usize, what: &str) -> Result<Vec<u8>, String> {
        if len > cap {
            return Err(format!(
                "the server sent an implausible {} length: {} bytes",
                what, len
            ));
        }
        let mut buf = vec![0u8; len];
        self.read_exact(&mut buf)?;
        Ok(buf)
    }

    /// Try to read a single byte, reporting `Ok(None)` when nothing arrived.
    ///
    /// This is the one place a timeout is not retried: the loop uses it to stay
    /// responsive to input and shutdown while the server has nothing to say,
    /// which is most of the time in a request-driven protocol.
    pub fn poll_byte(&mut self) -> Result<Option<u8>, String> {
        let mut byte = [0u8; 1];
        match self.stream.read(&mut byte) {
            Ok(0) => Err("the server closed the connection".to_string()),
            Ok(_) => Ok(Some(byte[0])),
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock
                        | std::io::ErrorKind::TimedOut
                        | std::io::ErrorKind::Interrupted
                ) =>
            {
                Ok(None)
            }
            Err(e) => Err(format!("read failed: {}", e)),
        }
    }

    /// [`Conn::poll_byte`], blocking for at most `timeout` instead of the
    /// usual read timeout.
    ///
    /// Only the read timeout is changed, never non-blocking mode: the writer
    /// thread holds a clone of this socket, and non-blocking mode is shared
    /// with it, so its writes would start failing with `WouldBlock`. The read
    /// timeout is shared too, but only this side ever reads.
    pub fn poll_byte_within(&mut self, timeout: Duration) -> Result<Option<u8>, String> {
        self.stream
            .set_read_timeout(Some(timeout))
            .map_err(|e| format!("could not configure the socket: {}", e))?;
        let result = self.poll_byte();
        self.stream
            .set_read_timeout(Some(READ_TIMEOUT))
            .map_err(|e| format!("could not configure the socket: {}", e))?;
        result
    }

    /// A second handle on the same socket, for the writer thread.
    ///
    /// Reads and writes are split across two threads so that an outgoing
    /// keystroke never waits on an incoming frame. The clone is handed to the
    /// writer and **this side never writes again** — two threads writing one
    /// socket could interleave a partial write mid-message and corrupt the RFB
    /// stream, so there is exactly one writer by construction.
    pub fn split_writer(&self) -> Result<Writer, String> {
        let stream = self
            .stream
            .try_clone()
            .map_err(|e| format!("could not split the VNC socket: {}", e))?;
        // `try_clone` dups the file descriptor, so both handles refer to the
        // *same* socket and every socket option is shared. Clearing the read
        // timeout here would clear it for the reader too, which would leave it
        // blocked in `read` forever and unable to notice the shutdown flag.
        //
        // A write timeout is set instead, and is deliberately fatal: a timed
        // out write may have gone out in part, which would corrupt the message
        // stream. There is no recovering from that, only tearing the session
        // down -- so this must never be "fixed" into a retry.
        let _ = stream.set_write_timeout(Some(WRITE_TIMEOUT));
        Ok(Writer { stream })
    }

    /// Read and discard `len` bytes, for a message we parse but do not use.
    /// Write during the handshake, before the socket is split.
    ///
    /// After [`Conn::split_writer`] **exactly one thread writes this socket**
    /// and it is the writer thread. This exists only for the handshake, which
    /// runs to completion on one thread before either session thread starts.
    pub fn handshake_write(&mut self, bytes: &[u8]) -> Result<(), String> {
        self.stream
            .write_all(bytes)
            .map_err(|e| format!("write failed: {}", e))
    }

    pub fn skip(&mut self, len: usize) -> Result<(), String> {
        let mut remaining = len;
        let mut scratch = [0u8; 4096];
        while remaining > 0 {
            let take = remaining.min(scratch.len());
            self.read_exact(&mut scratch[..take])?;
            remaining -= take;
        }
        Ok(())
    }
}

/// The write half of a VNC connection, owned by exactly one thread.
pub struct Writer {
    stream: TcpStream,
}

impl Writer {
    pub fn write_all(&mut self, bytes: &[u8]) -> Result<(), String> {
        self.stream
            .write_all(bytes)
            .map_err(|e| format!("write failed: {}", e))
    }
}

/// What the handshake settled on.
#[derive(Debug)]
pub struct Handshake {
    pub server_init: ServerInit,
    pub security: SecurityType,
}

/// A desktop name longer than this is not a desktop name.
const MAX_NAME_LEN: usize = 4096;

/// Run the full RFB handshake, leaving the connection ready for updates.
///
/// Done synchronously before the session thread starts, so that a wrong
/// password or an unsupported server is reported by `vnc_connect` itself
/// rather than arriving later as a disconnect event with no context.
pub fn handshake(
    conn: &mut Conn,
    password: Option<&str>,
    username: Option<&str>,
) -> Result<Handshake, String> {
    // 1. Version.
    let mut greeting = [0u8; ProtocolVersion::WIRE_LEN];
    conn.read_exact(&mut greeting)?;
    let version = ProtocolVersion::negotiate(&greeting)?;
    conn.handshake_write(&version.bytes())?;

    // 2. Security type. The two versions disagree about the shape of this
    // entirely: 3.8 sends a counted list, 3.3 sends one bare u32.
    let offered = read_security_types(conn, version)?;
    let has_username = username.is_some_and(|u| !u.trim().is_empty());
    let security = auth::select_security_type(&offered, has_username)?;
    if version == ProtocolVersion::Rfb38 {
        conn.handshake_write(&[security.to_u8()])?;
    }

    // 3. Answer the challenge.
    match security {
        SecurityType::None => {}
        SecurityType::VncAuth => {
            let password = password.unwrap_or_default();
            let mut challenge = [0u8; 16];
            conn.read_exact(&mut challenge)?;
            conn.handshake_write(&auth::vnc_auth_response(password, &challenge))?;
        }
        SecurityType::Ard => {
            let mut header = [0u8; auth::ArdParams::HEADER_LEN];
            conn.read_exact(&mut header)?;
            let key_len = auth::ArdParams::key_len(&header)?;
            let body = conn.read_vec(key_len * 2, 4096, "ARD key")?;
            let params = auth::ArdParams::parse(&header, &body)?;
            let response = auth::ard_response(
                &params,
                username.unwrap_or_default(),
                password.unwrap_or_default(),
            )?;
            conn.handshake_write(&response)?;
        }
    }

    // 4. SecurityResult — but only when this version sends one. Reading four
    // bytes an RFB 3.3 server never sent would hang until the read timeout,
    // with nothing in the error to say why.
    if version.has_security_result(security) {
        let mut result = [0u8; 4];
        conn.read_exact(&mut result)?;
        if !auth::security_result_ok(&result)? {
            return Err(failure_reason(conn, version, security));
        }
    }

    // 5. ClientInit / ServerInit.
    conn.handshake_write(&proto::encode_client_init(true))?;
    let mut fixed = [0u8; ServerInit::FIXED_LEN];
    conn.read_exact(&mut fixed)?;
    let name_len = ServerInit::name_len(&fixed)?;
    let name = conn.read_vec(name_len, MAX_NAME_LEN, "desktop name")?;
    let server_init = ServerInit::parse(&fixed, &name)?;

    // 6. Ask for the format and encodings we can actually decode. From here
    // the server sends nothing until we request an update.
    conn.handshake_write(&proto::encode_set_pixel_format(&PixelFormat::PREFERRED))?;
    conn.handshake_write(&proto::encode_set_encodings(&proto::preferred_encodings()))?;

    Ok(Handshake {
        server_init,
        security,
    })
}

fn read_security_types(conn: &mut Conn, version: ProtocolVersion) -> Result<Vec<u8>, String> {
    if version == ProtocolVersion::Rfb33 {
        let mut raw = [0u8; 4];
        conn.read_exact(&mut raw)?;
        let value = u32::from_be_bytes(raw);
        // 0 means the server refused us outright and a reason follows.
        if value == 0 {
            return Err(read_reason(conn).unwrap_or_else(|| {
                "the server refused the connection during the handshake".to_string()
            }));
        }
        return Ok(vec![value as u8]);
    }

    let mut count = [0u8; 1];
    conn.read_exact(&mut count)?;
    if count[0] == 0 {
        return Err(
            read_reason(conn).unwrap_or_else(|| "the server offered no security types".to_string())
        );
    }
    conn.read_vec(count[0] as usize, 256, "security type list")
}

/// The server's own words for why authentication failed.
///
/// Worth reading: a server that is rate-limiting after too many bad passwords
/// says so here, and "authentication failed" would send the user on retrying
/// the very thing that is causing it.
fn failure_reason(conn: &mut Conn, version: ProtocolVersion, security: SecurityType) -> String {
    let generic = match security {
        SecurityType::VncAuth => "the VNC password was rejected",
        SecurityType::Ard => "the user name or password was rejected",
        SecurityType::None => "the server refused the connection",
    };
    if version != ProtocolVersion::Rfb38 {
        return generic.to_string();
    }
    match read_reason(conn) {
        Some(reason) => format!("{}: {}", generic, reason),
        None => generic.to_string(),
    }
}

/// A length-prefixed failure string, if one is there to be read.
fn read_reason(conn: &mut Conn) -> Option<String> {
    let mut len = [0u8; 4];
    conn.read_exact(&mut len).ok()?;
    let len = u32::from_be_bytes(len) as usize;
    let bytes = conn.read_vec(len, 4096, "failure reason").ok()?;
    let text = String::from_utf8_lossy(&bytes).trim().to_string();
    if text.is_empty() {
        None
    } else {
        Some(text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vnc::proto::client_message;
    use std::net::TcpListener;
    use std::sync::mpsc;

    /// A scripted RFB server on loopback.
    ///
    /// The handshake is the part of this module with no pure-function seam --
    /// it is a conversation, and the order and the conditional reads are
    /// exactly what goes wrong. A real socket is the only honest way to test
    /// it, and a loopback listener costs a millisecond.
    struct FakeServer {
        port: u16,
        done: mpsc::Receiver<Result<Vec<u8>, String>>,
    }

    impl FakeServer {
        /// `script` plays the server's half and returns everything the client
        /// sent, so a test can assert on both directions.
        fn spawn(script: impl FnOnce(&mut Server) -> Result<(), String> + Send + 'static) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
            let port = listener.local_addr().expect("addr").port();
            let (tx, done) = mpsc::channel();

            std::thread::spawn(move || {
                let result = (|| {
                    let (stream, _) = listener.accept().map_err(|e| e.to_string())?;
                    stream
                        .set_read_timeout(Some(Duration::from_secs(5)))
                        .map_err(|e| e.to_string())?;
                    let mut server = Server {
                        stream,
                        received: Vec::new(),
                    };
                    script(&mut server)?;
                    Ok(server.received)
                })();
                let _ = tx.send(result);
            });

            Self { port, done }
        }

        fn connect(&self) -> Conn {
            Conn::connect("127.0.0.1", self.port, Arc::new(AtomicBool::new(false)))
                .expect("connect to the fake server")
        }

        /// Everything the client wrote, once the script has finished.
        fn client_wrote(self) -> Vec<u8> {
            self.done
                .recv_timeout(Duration::from_secs(5))
                .expect("the server script never finished")
                .expect("the server script failed")
        }
    }

    struct Server {
        stream: TcpStream,
        received: Vec<u8>,
    }

    impl Server {
        fn send(&mut self, bytes: &[u8]) -> Result<(), String> {
            self.stream.write_all(bytes).map_err(|e| e.to_string())
        }

        /// Read exactly `n` bytes from the client, recording them.
        fn expect(&mut self, n: usize) -> Result<Vec<u8>, String> {
            let mut buf = vec![0u8; n];
            self.stream
                .read_exact(&mut buf)
                .map_err(|e| format!("expected {} bytes from the client: {}", n, e))?;
            self.received.extend_from_slice(&buf);
            Ok(buf)
        }

        /// The fixed part of a ServerInit for a 1280x800 desktop.
        fn send_server_init(&mut self, name: &str) -> Result<(), String> {
            let mut init = Vec::new();
            init.extend_from_slice(&1280u16.to_be_bytes());
            init.extend_from_slice(&800u16.to_be_bytes());
            init.extend_from_slice(&PixelFormat::PREFERRED.encode());
            init.extend_from_slice(&(name.len() as u32).to_be_bytes());
            init.extend_from_slice(name.as_bytes());
            self.send(&init)
        }

        /// Consume the SetPixelFormat and SetEncodings that end the handshake.
        fn expect_setup(&mut self) -> Result<(), String> {
            self.expect(20)?; // SetPixelFormat
            let head = self.expect(4)?; // SetEncodings header
            let count = u16::from_be_bytes([head[2], head[3]]) as usize;
            self.expect(count * 4)?;
            Ok(())
        }
    }

    #[test]
    fn a_full_rfb_38_handshake_with_no_authentication() {
        let server = FakeServer::spawn(|s| {
            s.send(b"RFB 003.008\n")?;
            s.expect(12)?;
            s.send(&[1, 1])?; // one security type: None
            s.expect(1)?; // the client's choice
            s.send(&[0, 0, 0, 0])?; // SecurityResult: OK
            s.expect(1)?; // ClientInit
            s.send_server_init("kiosk-1")?;
            s.expect_setup()
        });

        let mut conn = server.connect();
        let hs = handshake(&mut conn, None, None).expect("handshake");
        assert_eq!(hs.server_init.width, 1280);
        assert_eq!(hs.server_init.height, 800);
        assert_eq!(hs.server_init.name, "kiosk-1");
        assert_eq!(hs.security, SecurityType::None);

        let wrote = server.client_wrote();
        assert_eq!(&wrote[..12], b"RFB 003.008\n", "answered with 3.8");
        assert_eq!(wrote[12], SecurityType::NONE, "chose None");
        assert_eq!(wrote[13], 1, "asked to share the desktop");
        assert_eq!(wrote[14], client_message::SET_PIXEL_FORMAT);
    }

    #[test]
    fn rfb_33_with_none_sends_no_security_result() {
        // The asymmetry that hangs a client which assumes 3.8's shape. Under
        // 3.3 the security type is a bare u32 and `None` goes straight to
        // ClientInit -- so a client reading four bytes here would block until
        // the read timeout and report nothing useful.
        let server = FakeServer::spawn(|s| {
            s.send(b"RFB 003.003\n")?;
            s.expect(12)?;
            s.send(&1u32.to_be_bytes())?; // security type None, no list, no result
            s.expect(1)?; // ClientInit -- straight through
            s.send_server_init("old-server")?;
            s.expect_setup()
        });

        let mut conn = server.connect();
        let hs = handshake(&mut conn, None, None).expect("handshake");
        assert_eq!(hs.server_init.name, "old-server");

        let wrote = server.client_wrote();
        assert_eq!(&wrote[..12], b"RFB 003.003\n", "answered in kind");
        // No security-type byte under 3.3: the server chose for us.
        assert_eq!(wrote[12], 1, "ClientInit follows immediately");
    }

    #[test]
    fn a_vnc_password_challenge_is_answered_with_the_mangled_des_key() {
        let challenge: [u8; 16] = std::array::from_fn(|i| (i as u8).wrapping_mul(7));
        let server = FakeServer::spawn(move |s| {
            s.send(b"RFB 003.008\n")?;
            s.expect(12)?;
            s.send(&[1, SecurityType::VNC_AUTH])?;
            s.expect(1)?;
            s.send(&challenge)?;
            s.expect(16)?; // the response
            s.send(&[0, 0, 0, 0])?;
            s.expect(1)?;
            s.send_server_init("secured")?;
            s.expect_setup()
        });

        let mut conn = server.connect();
        handshake(&mut conn, Some("hunter2"), None).expect("handshake");

        let wrote = server.client_wrote();
        // Bytes 13..29 are the challenge response. It must match what the
        // pure function produces, which is what ties the socket path to the
        // bit-reversal the auth tests pin.
        let expected = crate::vnc::auth::vnc_auth_response("hunter2", &challenge);
        assert_eq!(&wrote[13..29], &expected, "wrong challenge response");
    }

    #[test]
    fn a_rejected_password_reports_the_servers_own_reason() {
        // "authentication failed" would send a user retrying the very thing
        // causing a rate limit; the server usually says so here.
        let server = FakeServer::spawn(|s| {
            s.send(b"RFB 003.008\n")?;
            s.expect(12)?;
            s.send(&[1, SecurityType::VNC_AUTH])?;
            s.expect(1)?;
            s.send(&[0u8; 16])?;
            s.expect(16)?;
            s.send(&1u32.to_be_bytes())?; // SecurityResult: failed
            let reason = b"Too many authentication failures";
            s.send(&(reason.len() as u32).to_be_bytes())?;
            s.send(reason)
        });

        let mut conn = server.connect();
        let err = handshake(&mut conn, Some("wrong"), None).unwrap_err();
        assert!(err.contains("password was rejected"), "got: {}", err);
        assert!(
            err.contains("Too many authentication failures"),
            "the server's reason was dropped: {}",
            err
        );
    }

    #[test]
    fn a_server_offering_only_unsupported_security_names_what_it_offered() {
        let server = FakeServer::spawn(|s| {
            s.send(b"RFB 003.008\n")?;
            s.expect(12)?;
            s.send(&[2, 5, 6])?; // RA2 and RA2ne only
            Ok(())
        });

        let mut conn = server.connect();
        let err = handshake(&mut conn, Some("pw"), None).unwrap_err();
        assert!(err.contains("RA2"), "should name the method: {}", err);
    }

    #[test]
    fn something_that_is_not_a_vnc_server_is_reported_as_such() {
        // Pointing a VNC tab at an HTTP port is an easy mistake to make.
        let server = FakeServer::spawn(|s| s.send(b"HTTP/1.1 400 Bad Request\r\n"));
        let mut conn = server.connect();
        let err = handshake(&mut conn, None, None).unwrap_err();
        assert!(err.contains("not a VNC server"), "got: {}", err);
    }

    #[test]
    fn a_shutdown_flag_interrupts_a_read_that_is_waiting() {
        // The reason every read retries around the timeout: a session must
        // close promptly even when the server has gone quiet, rather than
        // hanging until the OS gives up on the socket.
        let server = FakeServer::spawn(|s| {
            s.send(b"RFB 003.008\n")?;
            // Then say nothing at all.
            std::thread::sleep(Duration::from_millis(300));
            Ok(())
        });

        let shutdown = Arc::new(AtomicBool::new(false));
        let mut conn = Conn::connect("127.0.0.1", server.port, shutdown.clone()).unwrap();
        let flag = shutdown.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(50));
            flag.store(true, Ordering::Relaxed);
        });

        let mut greeting = [0u8; 12];
        conn.read_exact(&mut greeting)
            .expect("the greeting arrives");
        let mut never = [0u8; 8];
        assert_eq!(conn.read_exact(&mut never).unwrap_err(), CANCELLED);
    }

    #[test]
    fn a_split_writer_sends_while_the_reader_is_blocked_on_a_read() {
        // The whole reason for two threads. On one thread an outgoing
        // keystroke waited for the current read to time out -- up to
        // READ_TIMEOUT -- and if a frame was arriving, for the entire frame.
        // Here the reader is deliberately parked in a read that will never
        // complete, and the write still has to land promptly.
        let (tx, rx) = mpsc::channel();
        let server = FakeServer::spawn(move |s| {
            s.send(b"RFB 003.008\n")?;
            // Say nothing more, so the client's read blocks.
            let got = s.expect(4)?;
            let _ = tx.send(got);
            Ok(())
        });

        let mut conn = server.connect();
        let mut writer = conn.split_writer().expect("split");

        let mut greeting = [0u8; 12];
        conn.read_exact(&mut greeting).unwrap();

        // Park the reader in a read that cannot be satisfied.
        let reading = std::thread::spawn(move || {
            let mut never = [0u8; 64];
            let _ = conn.read_exact(&mut never);
        });

        let sent_at = std::time::Instant::now();
        writer.write_all(&[1, 2, 3, 4]).expect("write");
        let echoed = rx
            .recv_timeout(Duration::from_secs(5))
            .expect("the server never saw the write");

        assert_eq!(echoed, vec![1, 2, 3, 4]);
        assert!(
            sent_at.elapsed() < READ_TIMEOUT,
            "the write waited on the read: {:?}",
            sent_at.elapsed()
        );

        server.done.recv_timeout(Duration::from_secs(5)).ok();
        reading.join().ok();
    }

    #[test]
    fn an_implausible_length_is_refused_before_anything_is_allocated() {
        // Lengths come off the wire; a 4GB desktop name must be an error, not
        // an allocation.
        let server = FakeServer::spawn(|s| s.send(b"RFB 003.008\n"));
        let mut conn = server.connect();
        let err = conn.read_vec(usize::MAX, 4096, "desktop name").unwrap_err();
        assert!(err.contains("implausible"), "got: {}", err);
    }
}
