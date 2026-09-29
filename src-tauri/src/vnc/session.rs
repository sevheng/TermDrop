//! The per-session thread: one OS thread owning one RFB connection.
//!
//! The shape mirrors `ssh/mod.rs`: a handle of channel senders lives in
//! `AppState`, and the thread does all the blocking work. What differs is the
//! pacing, which is the load-bearing part of this module.
//!
//! **Two threads, and exactly one of them writes.** The reader owns the `Conn`
//! and never writes; the writer owns the cloned write half and is the only
//! thing that touches the socket to send. Input and framebuffer requests alike
//! go through one queue, so no two writes can interleave a partial message.
//!
//! That split is what keeps a keystroke from waiting on a frame. On one thread
//! an outgoing event waited for the current read to time out — up to 20ms —
//! and, if a frame happened to be arriving, for the *entire* frame to be read,
//! which on a 2560x1440 desktop is 14MB.
//!
//! **Flow control.** RFB sends nothing until the client asks, so a frame is
//! requested only after the previous one has been painted. Without that, a
//! server on a busy desktop produces frames faster than a canvas can draw them
//! and the IPC queue grows without bound. The renderer acknowledges a frame by
//! sequence number and only then is the next one requested.
//!
//! An update with **nothing to draw** sends no frame, so no acknowledgement can
//! follow it and waiting for one would stall the session for the whole ack
//! timeout — see [`Pacing`], which exists because that bug shipped.
//!
//! The rule is **always read the socket; gate only the request.** Blocking the
//! read while waiting for an acknowledgement would stall a `Bell` or a
//! clipboard update behind a frame the user cannot see. Since the server will
//! not send another update unasked, the socket simply goes quiet instead.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tauri::ipc::{Channel, InvokeResponseBody};
use tauri::{Emitter, Window};
use tokio::sync::mpsc;

use super::conn::{handshake, Conn, Writer, CANCELLED};
use super::encoding::{decode_rect, Decoders, RectOutcome};
use super::frame::FrameBuilder;
use super::pixel::PixelFormat;
use super::proto::{self, server_message, RectHeader};

/// How long to wait for the renderer before assuming the acknowledgement is
/// lost and asking for another frame anyway.
///
/// A dropped acknowledgement must degrade the frame rate, never freeze the
/// session — a hidden tab, a slow paint or a reload should all recover on their
/// own rather than needing a reconnect.
const ACK_TIMEOUT: Duration = Duration::from_millis(250);

/// A cut-text payload larger than this is not a clipboard.
const MAX_CUT_TEXT: usize = 1024 * 1024;

/// Everything the commands need to drive a live session.
pub struct VncSessionHandle {
    pub host_id: i64,
    /// Encoded client messages, written by the thread in arrival order.
    input_tx: mpsc::UnboundedSender<Outgoing>,
    /// One entry per painted frame.
    ack_tx: mpsc::UnboundedSender<u32>,
    shutdown: Arc<AtomicBool>,
    /// Enforced here rather than in the UI: a view-only switch that only hides
    /// buttons is not a safety feature.
    view_only: Arc<AtomicBool>,
    /// Set when the next request must be a full refresh instead of incremental.
    needs_full: Arc<AtomicBool>,
    /// Registered by the panel once it is ready to paint. The thread requests
    /// no frames at all until this is set, which removes the dropped-first-frame
    /// race rather than papering over it.
    channel: Arc<Mutex<Option<Channel<InvokeResponseBody>>>>,
}

impl VncSessionHandle {
    pub fn send_input(&self, bytes: Vec<u8>) {
        if self.view_only.load(Ordering::Relaxed) {
            return;
        }
        let _ = self.input_tx.send(Outgoing::Bytes {
            queued: Instant::now(),
            bytes,
        });
    }

    pub fn ack(&self, seq: u32) {
        let _ = self.ack_tx.send(seq);
    }

    pub fn refresh(&self) {
        self.needs_full.store(true, Ordering::Relaxed);
    }

    pub fn set_view_only(&self, value: bool) {
        self.view_only.store(value, Ordering::Relaxed);
    }

    pub fn is_view_only(&self) -> bool {
        self.view_only.load(Ordering::Relaxed)
    }

    pub fn attach_channel(&self, channel: Channel<InvokeResponseBody>) {
        if let Ok(mut slot) = self.channel.lock() {
            *slot = Some(channel);
        }
    }

    pub fn disconnect(&self) {
        self.shutdown.store(true, Ordering::Relaxed);
    }
}

impl Drop for VncSessionHandle {
    fn drop(&mut self) {
        // Losing the handle without the thread noticing would leak a thread and
        // a socket for the lifetime of the app.
        self.shutdown.store(true, Ordering::Relaxed);
    }
}

/// What a completed handshake reports back to the frontend.
#[derive(Debug, Clone, serde::Serialize)]
pub struct VncSessionInfo {
    pub session_id: String,
    pub width: u16,
    pub height: u16,
    pub desktop_name: String,
    pub tunnelled: bool,
    pub security: String,
    pub view_only: bool,
}

/// Connect, authenticate, and start the session thread.
///
/// The handshake runs here rather than on the thread so that a wrong password
/// or an unreachable host is the return value of `vnc_connect`, not a
/// disconnect event arriving moments later with no context.
#[allow(clippy::too_many_arguments)]
pub fn start(
    window: Window,
    session_id: String,
    host_id: i64,
    address: &str,
    port: u16,
    password: Option<&str>,
    username: Option<&str>,
    tunnelled: bool,
    view_only: bool,
) -> Result<(VncSessionHandle, VncSessionInfo), String> {
    let shutdown = Arc::new(AtomicBool::new(false));
    let mut conn = Conn::connect(address, port, shutdown.clone())?;
    let hs = handshake(&mut conn, password, username)?;

    // One queue for everything leaving this socket -- input and frame
    // requests alike -- so the writer thread is the only thing that writes.
    let (input_tx, outgoing_rx) = mpsc::unbounded_channel();
    let writer = conn.split_writer()?;
    let write_error = Arc::new(Mutex::new(None));
    let (ack_tx, ack_rx) = mpsc::unbounded_channel();
    let channel = Arc::new(Mutex::new(None));
    let needs_full = Arc::new(AtomicBool::new(true));
    let size = Arc::new(Mutex::new((hs.server_init.width, hs.server_init.height)));

    let info = VncSessionInfo {
        session_id: session_id.clone(),
        width: hs.server_init.width,
        height: hs.server_init.height,
        desktop_name: hs.server_init.name.clone(),
        tunnelled,
        security: format!("{:?}", hs.security),
        view_only,
    };

    let handle = VncSessionHandle {
        host_id,
        input_tx: input_tx.clone(),
        ack_tx,
        shutdown: shutdown.clone(),
        view_only: Arc::new(AtomicBool::new(view_only)),
        needs_full: needs_full.clone(),
        channel: channel.clone(),
    };

    let writer_thread = {
        let shutdown = shutdown.clone();
        let write_error = write_error.clone();
        std::thread::Builder::new()
            .name("vnc-writer".into())
            .spawn(move || run_writer(writer, outgoing_rx, shutdown, write_error))
            .map_err(|e| format!("could not start the VNC writer thread: {}", e))?
    };

    let thread_state = LoopState {
        window,
        session_id,
        conn,
        channel,
        needs_full,
        size,
        shutdown,
        outgoing_tx: input_tx,
        write_error,
        writer: Some(writer_thread),
        last_frame_bytes: 0,
    };
    std::thread::Builder::new()
        .name("vnc-session".into())
        .spawn(move || run_loop(thread_state, ack_rx))
        .map_err(|e| format!("could not start the VNC session thread: {}", e))?;

    Ok((handle, info))
}

/// How long outgoing messages sat in the queue, summarised once a second.
///
/// Worth keeping: "it feels slow" is how this feature shipped with a 250ms
/// stall in it, and this is the number that answers it next time. Read it with
/// `RUST_LOG=termdrop::vnc=debug`.
#[derive(Default)]
struct QueueWaits {
    count: u32,
    total: Duration,
    max: Duration,
    since: Option<Instant>,
}

impl QueueWaits {
    fn record(&mut self, wait: Duration) {
        let since = *self.since.get_or_insert_with(Instant::now);
        self.count += 1;
        self.total += wait;
        self.max = self.max.max(wait);

        if since.elapsed() >= Duration::from_secs(1) {
            tracing::debug!(
                target: "termdrop::vnc",
                messages = self.count,
                avg_us = self.total.as_micros() as u64 / self.count.max(1) as u64,
                max_us = self.max.as_micros() as u64,
                "VNC write queue wait"
            );
            *self = Self::default();
        }
    }
}

/// Where a frame's time actually goes, summarised once a second.
///
/// Split deliberately: `server` is the wait between asking for an update and
/// the first byte of the answer — the remote's own encoding time plus the
/// network, none of which this codebase can shorten. `read` is everything
/// after that: pulling the bytes off the socket and decoding them, which is
/// ours. Without the split, "it is slow" has no answerable form.
#[derive(Default)]
struct FrameTimes {
    frames: u32,
    server: Duration,
    read: Duration,
    bytes: u64,
    since: Option<Instant>,
}

impl FrameTimes {
    fn record(&mut self, server: Duration, read: Duration, bytes: u64) {
        let since = *self.since.get_or_insert_with(Instant::now);
        self.frames += 1;
        self.server += server;
        self.read += read;
        self.bytes += bytes;

        if since.elapsed() >= Duration::from_secs(1) {
            let n = self.frames.max(1) as u64;
            tracing::debug!(
                target: "termdrop::vnc",
                fps = self.frames,
                server_ms = self.server.as_millis() as u64 / n,
                read_ms = self.read.as_millis() as u64 / n,
                kb_per_frame = self.bytes / n / 1024,
                "VNC frame timing"
            );
            *self = Self::default();
        }
    }
}

/// A message on its way out of this session.
enum Outgoing {
    /// A client message: input, or the reader's framebuffer update request.
    ///
    /// `queued` is when it was accepted, so the writer can report how long it
    /// waited. That number is the whole point of this thread existing: on the
    /// reader it averaged ~10ms and spiked to the length of a whole frame read.
    Bytes { queued: Instant, bytes: Vec<u8> },
    /// The reader has finished.
    ///
    /// Without this the writer can outlive it: dropping the reader's sender is
    /// not enough while the handle still holds one, so the writer would park
    /// on `blocking_recv` forever, holding the cloned descriptor open and
    /// leaking both the thread and the socket.
    Stop,
}

/// Drain the outgoing queue onto the socket, one message at a time.
///
/// The whole point of a second thread: `blocking_recv` sleeps until there is
/// something to send, so a keystroke goes out the instant it is queued. On the
/// reader thread it would have waited for the current `poll_byte` to time out
/// — up to 20ms — and, if a frame happened to be arriving, for the *entire*
/// frame to be read, which on a 2560x1440 desktop is 14MB.
///
/// This thread owns the write half outright, so no two writes can interleave.
fn run_writer(
    mut writer: Writer,
    mut outgoing_rx: mpsc::UnboundedReceiver<Outgoing>,
    shutdown: Arc<AtomicBool>,
    write_error: Arc<Mutex<Option<String>>>,
) {
    // `blocking_recv` is usable here only because this is a real OS thread.
    // From inside the tokio runtime it panics, so this must never become a
    // `spawn` or a `spawn_blocking`.
    let mut waits = QueueWaits::default();

    while let Some(message) = outgoing_rx.blocking_recv() {
        let (queued, bytes) = match message {
            Outgoing::Stop => break,
            Outgoing::Bytes { queued, bytes } => (queued, bytes),
        };
        if shutdown.load(Ordering::Relaxed) {
            break;
        }
        waits.record(queued.elapsed());
        if let Err(e) = writer.write_all(&bytes) {
            // A failed write means the socket is gone. Hand the reason to the
            // reader, which owns the error reporting, and let it notice within
            // one read timeout.
            if let Ok(mut slot) = write_error.lock() {
                *slot = Some(e);
            }
            shutdown.store(true, Ordering::Relaxed);
            break;
        }
    }
}

struct LoopState {
    window: Window,
    session_id: String,
    conn: Conn,
    channel: Arc<Mutex<Option<Channel<InvokeResponseBody>>>>,
    needs_full: Arc<AtomicBool>,
    size: Arc<Mutex<(u16, u16)>>,
    shutdown: Arc<AtomicBool>,
    /// Requests go out through the writer thread like any other message, so
    /// that the socket keeps exactly one writer.
    outgoing_tx: mpsc::UnboundedSender<Outgoing>,
    write_error: Arc<Mutex<Option<String>>>,
    writer: Option<std::thread::JoinHandle<()>>,
    /// Size of the frame most recently handed to the renderer, for the timing
    /// log. Not shared state: only the reader thread touches it.
    last_frame_bytes: usize,
}

/// Whether an update actually produced a frame for the renderer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Update {
    /// A frame went to the channel, so an acknowledgement will follow.
    Sent,
    /// The server had nothing to draw. **No acknowledgement can ever arrive**,
    /// so waiting for one would stall the session for the whole ack timeout.
    Empty,
}

/// The request/acknowledge pacing state.
///
/// A struct rather than two loose booleans in the loop because the rule that
/// binds them is easy to get wrong and impossible to see from the outside when
/// it is: an empty update used to arm the acknowledgement timer, and since no
/// frame had been sent no acknowledgement could ever come, so every idle poll
/// cost the full 250ms timeout. On a mostly-static desktop that was most of
/// the session, and it read to the user as mouse lag.
/// How many painted-but-unacknowledged frames may be outstanding.
///
/// One means the request for the next frame waits behind the IPC hop, the
/// paint and the acknowledgement of the current one — measured at ~20-30ms of
/// a 74ms round trip against macOS. Two lets the server encode the next frame
/// while the renderer is still finishing this one, and is still a hard bound:
/// the worst case is two frames of memory, which matters only during a resize
/// when frames are full-screen.
const MAX_UNACKED: u32 = 2;

#[derive(Debug, Default)]
struct Pacing {
    request_in_flight: bool,
    /// Frames sent to the renderer that have not been acknowledged.
    unacked: u32,
    /// When the oldest unacknowledged frame went out.
    oldest_unacked: Option<Instant>,
}

impl Pacing {
    /// Whether another update may be requested now.
    fn may_request(&self, channel_ready: bool) -> bool {
        !self.request_in_flight && self.unacked < MAX_UNACKED && channel_ready
    }

    fn requested(&mut self) {
        self.request_in_flight = true;
    }

    /// Fold in the outcome of an update that has just been read.
    fn update_arrived(&mut self, update: Update, now: Instant) {
        self.request_in_flight = false;
        if update == Update::Sent {
            self.unacked += 1;
            self.oldest_unacked.get_or_insert(now);
        }
    }

    fn acked(&mut self, now: Instant) {
        self.unacked = self.unacked.saturating_sub(1);
        self.oldest_unacked = (self.unacked > 0).then_some(now);
    }

    /// Give up on a lost acknowledgement, reporting whether one was dropped.
    ///
    /// A renderer that never answers must cost frame rate, never the session.
    fn expire_ack(&mut self, now: Instant, timeout: Duration) -> bool {
        match self.oldest_unacked {
            Some(at) if now.duration_since(at) > timeout => {
                // Give up on all of them: a renderer that has stopped
                // answering will not answer the older ones either.
                self.unacked = 0;
                self.oldest_unacked = None;
                true
            }
            _ => false,
        }
    }
}

fn run_loop(mut state: LoopState, mut ack_rx: mpsc::UnboundedReceiver<u32>) {
    let format = PixelFormat::PREFERRED;
    let seq = AtomicU32::new(0);
    // Lives as long as the connection: see the note on `Decoders`. It also
    // owns the reusable pixel buffers, so a full-screen rectangle costs no
    // allocation after the first frame.
    let mut decoders = Decoders::default();

    let mut pacing = Pacing::default();
    let mut times = FrameTimes::default();
    // When the most recent update request went onto the wire.
    let mut requested_at = Instant::now();

    let outcome = loop {
        if state.shutdown.load(Ordering::Relaxed) {
            break Ok(());
        }

        // Drain acknowledgements; only the most recent matters.
        while ack_rx.try_recv().is_ok() {
            pacing.acked(Instant::now());
        }
        if pacing.expire_ack(Instant::now(), ACK_TIMEOUT) {
            tracing::debug!(
                session = %state.session_id,
                "no frame acknowledgement within {}ms; continuing anyway",
                ACK_TIMEOUT.as_millis()
            );
        }

        // Gate only the request, never the read.
        if pacing.may_request(channel_ready(&state.channel)) {
            let (width, height) = *state.size.lock().unwrap_or_else(|e| e.into_inner());
            let full = state.needs_full.swap(false, Ordering::Relaxed);
            let request = proto::encode_framebuffer_update_request(!full, 0, 0, width, height);
            // Queued rather than written here, so there stays exactly one
            // writer. The queue is FIFO, so a request posted behind a burst of
            // input goes out after it -- the right priority, since input is
            // what the user is waiting on.
            let queued = Instant::now();
            if state
                .outgoing_tx
                .send(Outgoing::Bytes {
                    queued,
                    bytes: request,
                })
                .is_err()
            {
                break Ok(());
            }
            requested_at = Instant::now();
            pacing.requested();
        }

        match state.conn.poll_byte() {
            Ok(None) => continue,
            Err(e) => break Err(e),
            Ok(Some(message_type)) => match message_type {
                server_message::FRAMEBUFFER_UPDATE => {
                    // The first byte has just arrived, so everything before
                    // now was the remote thinking.
                    let first_byte = Instant::now();
                    let server = first_byte.saturating_duration_since(requested_at);

                    let next = seq.fetch_add(1, Ordering::Relaxed);
                    match read_update(&mut state, &format, next, &mut decoders) {
                        Ok(update) => {
                            let now = Instant::now();
                            times.record(
                                server,
                                now.saturating_duration_since(first_byte),
                                state.last_frame_bytes as u64,
                            );
                            pacing.update_arrived(update, now);
                        }
                        Err(e) => break Err(e),
                    }
                }
                // BELL and SERVER_CUT_TEXT deliberately leave the request
                // outstanding: the server sent them *alongside* an update we
                // are still waiting for, so the request really is in flight.
                server_message::BELL => {
                    let _ = state.window.emit("vnc-bell", &state.session_id);
                }
                server_message::SERVER_CUT_TEXT => {
                    if let Err(e) = read_cut_text(&mut state) {
                        break Err(e);
                    }
                }
                server_message::SET_COLOUR_MAP_ENTRIES => {
                    if let Err(e) = skip_colour_map(&mut state) {
                        break Err(e);
                    }
                }
                other => {
                    break Err(format!("the server sent unknown message type {}", other));
                }
            },
        }
    };

    state.shutdown.store(true, Ordering::Relaxed);

    // Stop the writer and wait for it before reading its error slot: without
    // the join there is a race where a write failure has not been recorded yet
    // and the session is reported as a clean disconnect. The write timeout is
    // what bounds this wait.
    let _ = state.outgoing_tx.send(Outgoing::Stop);
    if let Some(writer) = state.writer.take() {
        let _ = writer.join();
    }

    // A write failure is the more specific story: the writer setting the
    // shutdown flag is *precisely* what makes the reader report `CANCELLED`,
    // so without this every dead socket would look like the user closing the
    // tab. A genuine reader error still wins.
    let write_error = state.write_error.lock().ok().and_then(|mut e| e.take());
    let outcome = match (outcome, write_error) {
        // A real read failure says more than "the write end gave up".
        (Err(e), _) if e != CANCELLED => Err(e),
        (_, Some(write_error)) => Err(write_error),
        (outcome, None) => outcome,
    };

    match outcome {
        Ok(()) => {
            let _ = state.window.emit("vnc-disconnected", &state.session_id);
        }
        Err(e) if e == CANCELLED => {
            let _ = state.window.emit("vnc-disconnected", &state.session_id);
        }
        Err(e) => {
            tracing::warn!(session = %state.session_id, "VNC session ended: {}", e);
            let _ = state.window.emit(
                "vnc-error",
                serde_json::json!({ "sessionId": state.session_id, "error": e }),
            );
        }
    }
}

fn channel_ready(slot: &Arc<Mutex<Option<Channel<InvokeResponseBody>>>>) -> bool {
    slot.lock().map(|s| s.is_some()).unwrap_or(false)
}

/// Read one `FramebufferUpdate` and hand the decoded frame to the renderer.
fn read_update(
    state: &mut LoopState,
    format: &PixelFormat,
    seq: u32,
    decoders: &mut Decoders,
) -> Result<Update, String> {
    let mut head = [0u8; 3];
    state.conn.read_exact(&mut head)?;
    let count = u16::from_be_bytes([head[1], head[2]]);

    let mut frame = FrameBuilder::new(seq);
    let mut resized: Option<(u16, u16)> = None;

    for _ in 0..count {
        let mut raw = [0u8; RectHeader::WIRE_LEN];
        state.conn.read_exact(&mut raw)?;
        let header = RectHeader::parse(&raw)?;

        match decode_rect(&mut state.conn, &header, format, &mut frame, decoders)? {
            RectOutcome::Painted => {}
            RectOutcome::Last => break,
            RectOutcome::Resized { width, height } => {
                resized = Some((width, height));
                // Deliberately not pushed here: resizing the canvas clears it,
                // so the marker has to be the frame's last rectangle or the
                // pixels after it would be painted and thrown away.
            }
        }
    }

    if let Some((width, height)) = resized {
        *state.size.lock().unwrap_or_else(|e| e.into_inner()) = (width, height);
        // A cleared canvas has nothing to update incrementally against.
        state.needs_full.store(true, Ordering::Relaxed);
        frame.push_desktop_size(width, height);
        let _ = state.window.emit(
            "vnc-resize",
            serde_json::json!({ "sessionId": state.session_id, "width": width, "height": height }),
        );
    }

    if frame.is_empty() {
        // The server had nothing to draw. Saying so is what lets the caller
        // ask again immediately instead of waiting out an acknowledgement that
        // is never coming.
        return Ok(Update::Empty);
    }
    let bytes = frame.finish();
    state.last_frame_bytes = bytes.len();
    send_frame(state, bytes);
    Ok(Update::Sent)
}

fn send_frame(state: &LoopState, bytes: Vec<u8>) {
    if let Ok(slot) = state.channel.lock() {
        if let Some(channel) = slot.as_ref() {
            // Raw, not Vec<u8>: Tauri's blanket Serialize impl would turn a
            // Vec<u8> into a JSON array of decimal numbers, which for an 8MB
            // framebuffer is both enormous and slow to parse.
            let _ = channel.send(InvokeResponseBody::Raw(bytes));
        }
    }
}

fn read_cut_text(state: &mut LoopState) -> Result<(), String> {
    let mut head = [0u8; 7];
    state.conn.read_exact(&mut head)?;
    let len = u32::from_be_bytes([head[3], head[4], head[5], head[6]]) as usize;
    let bytes = state.conn.read_vec(len, MAX_CUT_TEXT, "clipboard")?;
    let text = proto::decode_server_cut_text(&bytes);
    let _ = state.window.emit(
        "vnc-clipboard",
        serde_json::json!({ "sessionId": state.session_id, "text": text }),
    );
    Ok(())
}

/// We always negotiate true colour, so a colour map is not ours to use — but
/// its bytes still have to leave the stream or every message after it is
/// misread.
fn skip_colour_map(state: &mut LoopState) -> Result<(), String> {
    let mut head = [0u8; 5];
    state.conn.read_exact(&mut head)?;
    let count = u16::from_be_bytes([head[3], head[4]]) as usize;
    state.conn.skip(count * 6)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_update_does_not_wait_for_an_acknowledgement() {
        // The bug this type exists for. An update with nothing to draw sends
        // no frame, so no acknowledgement can ever arrive -- and arming the
        // timer anyway cost the full 250ms before the next request went out.
        // A static desktop answers most incremental requests this way, so the
        // session spent most of its time stalled and it read as mouse lag.
        let mut pacing = Pacing::default();
        pacing.requested();
        assert!(!pacing.may_request(true), "a request is outstanding");

        pacing.update_arrived(Update::Empty, Instant::now());
        assert!(
            pacing.may_request(true),
            "an empty update must free the pump immediately"
        );
        assert_eq!(pacing.unacked, 0, "nothing was sent, so nothing is owed");
    }

    #[test]
    fn one_frame_may_be_in_flight_while_the_renderer_paints_the_last() {
        // Requesting only after the previous frame was *painted* put the IPC
        // hop, the paint and the acknowledgement on the critical path -- about
        // 20-30ms of a measured 74ms round trip against macOS. Overlapping one
        // frame lets the server encode while the canvas is still drawing.
        let now = Instant::now();
        let mut pacing = Pacing::default();
        pacing.requested();
        pacing.update_arrived(Update::Sent, now);
        assert!(pacing.may_request(true), "one unacked frame must not block");

        pacing.requested();
        pacing.update_arrived(Update::Sent, now);
        assert!(
            !pacing.may_request(true),
            "two unacked frames is the cap: the queue must stay bounded"
        );

        pacing.acked(now);
        assert!(pacing.may_request(true));
    }

    #[test]
    fn a_lost_acknowledgement_costs_frame_rate_not_the_session() {
        // A renderer that never answers -- a hidden tab, a slow paint, a
        // reload -- must degrade rather than freeze.
        let start = Instant::now();
        let mut pacing = Pacing::default();
        pacing.requested();
        pacing.update_arrived(Update::Sent, start);

        assert!(!pacing.expire_ack(start, ACK_TIMEOUT), "not yet");

        // Fill the pipeline so the cap, not the request flag, is what blocks.
        pacing.requested();
        pacing.update_arrived(Update::Sent, start);
        assert!(!pacing.may_request(true), "at the cap");

        let late = start + ACK_TIMEOUT + Duration::from_millis(1);
        assert!(
            pacing.expire_ack(late, ACK_TIMEOUT),
            "reports the drop once"
        );
        assert!(
            pacing.may_request(true),
            "a silent renderer must not wedge the session"
        );
        assert!(!pacing.expire_ack(late, ACK_TIMEOUT), "only once");
    }

    #[test]
    fn nothing_is_requested_before_the_renderer_has_a_channel() {
        // The backend asks for no frame at all until the panel registers its
        // channel, which is what removes the dropped-first-frame race rather
        // than papering over it with a retry.
        let pacing = Pacing::default();
        assert!(!pacing.may_request(false));
        assert!(pacing.may_request(true));
    }
}
