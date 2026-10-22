use ssh2::Channel as SshChannel;
use std::io::{Read, Write};
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

/// Decode as much of `bytes` as forms whole characters.
///
/// Returns the text and how many bytes it consumed; whatever is left is an
/// **incomplete trailing sequence** whose remaining bytes are still in flight,
/// and the caller must keep it for the next read.
///
/// `String::from_utf8_lossy` per read was the bug this replaces. A read
/// boundary falls wherever the network put it, so a multi-byte character split
/// across two reads became two replacement characters. It only ever affected
/// the fallback path -- but that is precisely the path that runs *before* the
/// frontend registers its channel, which is when a login banner or MOTD full of
/// box-drawing characters arrives.
///
/// Genuinely invalid bytes are still replaced rather than carried: no byte
/// arriving later can make them valid, so holding them would stall the stream.
fn decode_complete(bytes: &[u8]) -> (String, usize) {
    let mut text = String::new();
    let mut at = 0;

    loop {
        match std::str::from_utf8(&bytes[at..]) {
            Ok(rest) => {
                text.push_str(rest);
                return (text, bytes.len());
            }
            Err(e) => {
                let valid = e.valid_up_to();
                if let Ok(good) = std::str::from_utf8(&bytes[at..at + valid]) {
                    text.push_str(good);
                }
                match e.error_len() {
                    // Malformed, and no later byte can fix it.
                    Some(bad) => {
                        text.push(char::REPLACEMENT_CHARACTER);
                        at += valid + bad;
                    }
                    // Truncated: the rest of this character is in the next read.
                    None => return (text, at + valid),
                }
            }
        }
    }
}

/// Flush output buffer if it exceeds this size (bytes).
const OUTPUT_BATCH_SIZE: usize = 4096;
/// Flush output buffer at this interval during heavy output (ms).
const OUTPUT_FLUSH_INTERVAL_MS: u64 = 16;

/// The most output gathered into one data-channel message.
pub const COALESCE_MAX_BYTES: usize = 64 * 1024;
/// The longest output waits to be sent while more keeps arriving.
pub const COALESCE_MAX_AGE: Duration = Duration::from_millis(4);

/// The UI's data channel for a terminal. In the Tauri app it is a
/// `Channel<InvokeResponseBody>` fed `Raw` bytes: Tauri serialises a `Vec<u8>`
/// as a JSON array of decimal numbers, while a Raw payload of 1 KB or more
/// crosses as a real `ArrayBuffer` (smaller ones are inlined either way, which
/// is why output is coalesced before it is sent).
pub type TerminalChannel = crate::events::ByteSlot;

/// Where terminal output goes: gathered into as few messages as the stream
/// allows, then sent on the data channel, or -- before the frontend has
/// opened one -- decoded and batched for the JSON event fallback.
///
/// Coalescing is what keeps a fast short-line producer usable. `seq` writes
/// one line per syscall, every read used to become its own IPC message
/// (~175k messages of under 100 bytes for 17 MB), and the webview froze for
/// up to 10 s draining them. A lone keystroke echo is not delayed: the read
/// after it would block, which flushes at once.
pub struct OutputSink<B, F>
where
    B: FnMut(Vec<u8>) -> Result<(), Vec<u8>>,
    F: FnMut(&str),
{
    /// Hands bytes to the binary channel, or gives them back when there is none.
    send_binary: B,
    on_data_fallback: F,
    pending: Vec<u8>,
    pending_since: Option<Instant>,
    /// Bytes of a character whose remainder has not arrived yet. Bounded by the
    /// longest UTF-8 sequence, so this never grows past three bytes.
    partial: Vec<u8>,
    output_buf: String,
    last_flush: Instant,
}

impl<B, F> OutputSink<B, F>
where
    B: FnMut(Vec<u8>) -> Result<(), Vec<u8>>,
    F: FnMut(&str),
{
    pub fn new(send_binary: B, on_data_fallback: F) -> Self {
        OutputSink {
            send_binary,
            on_data_fallback,
            pending: Vec::with_capacity(COALESCE_MAX_BYTES),
            pending_since: None,
            partial: Vec::new(),
            output_buf: String::with_capacity(8192),
            last_flush: Instant::now(),
        }
    }

    /// Take freshly read output. Sends only once a size or age cap is hit;
    /// otherwise the caller flushes when the stream pauses.
    pub fn push(&mut self, bytes: &[u8]) {
        if self.pending.is_empty() {
            self.pending_since = Some(Instant::now());
        }
        self.pending.extend_from_slice(bytes);
        let aged = self
            .pending_since
            .is_some_and(|t| t.elapsed() >= COALESCE_MAX_AGE);
        if self.pending.len() >= COALESCE_MAX_BYTES || aged {
            self.flush();
        }
    }

    /// Gather output without the size and age caps; for a loop that times its
    /// own flushes rather than polling.
    pub fn append(&mut self, bytes: &[u8]) {
        if self.pending.is_empty() {
            self.pending_since = Some(Instant::now());
        }
        self.pending.extend_from_slice(bytes);
    }

    pub fn pending_len(&self) -> usize {
        self.pending.len()
    }

    /// Nothing is waiting to be sent on either path.
    pub fn is_empty(&self) -> bool {
        self.pending.is_empty() && self.output_buf.is_empty()
    }

    /// Send whatever is gathered.
    pub fn flush(&mut self) {
        self.pending_since = None;
        if self.pending.is_empty() {
            return;
        }
        let bytes = std::mem::replace(&mut self.pending, Vec::with_capacity(COALESCE_MAX_BYTES));
        // Fallback text queued before the channel opened goes first, so the
        // order on screen is the order it was read.
        if !self.output_buf.is_empty() {
            self.flush_fallback();
        }
        if let Err(bytes) = (self.send_binary)(bytes) {
            self.push_fallback(&bytes);
        }
    }

    fn push_fallback(&mut self, bytes: &[u8]) {
        self.partial.extend_from_slice(bytes);
        let (text, consumed) = decode_complete(&self.partial);
        self.partial.drain(..consumed);
        self.output_buf.push_str(&text);
        if self.output_buf.len() >= OUTPUT_BATCH_SIZE
            || self.last_flush.elapsed().as_millis() as u64 >= OUTPUT_FLUSH_INTERVAL_MS
        {
            self.flush_fallback();
        }
    }

    fn flush_fallback(&mut self) {
        if !self.output_buf.is_empty() {
            (self.on_data_fallback)(&self.output_buf);
            self.output_buf.clear();
        }
        self.last_flush = Instant::now();
    }

    /// The stream has gone quiet: send gathered output, and let fallback text
    /// that has waited its batch interval go too.
    pub fn idle(&mut self) {
        self.flush();
        if !self.output_buf.is_empty()
            && self.last_flush.elapsed().as_millis() as u64 >= OUTPUT_FLUSH_INTERVAL_MS
        {
            self.flush_fallback();
        }
    }

    /// The stream has ended: send everything.
    pub fn finish(&mut self) {
        self.flush();
        self.flush_fallback();
    }
}

/// An `OutputSink` sender for the frontend's data channel.
pub fn channel_sender(data_channel: TerminalChannel) -> impl FnMut(Vec<u8>) -> Result<(), Vec<u8>> {
    move |bytes| match data_channel.lock() {
        Ok(mut lock) => match lock.as_mut() {
            Some(ch) => {
                // A receiver that has gone is no reason to fall back to the
                // event path, which is what the Tauri channel's ignored send
                // error always meant: the tab is closing and nobody would see it.
                let _ = ch.send(bytes);
                crate::dev_perf::mark("sent");
                Ok(())
            }
            None => Err(bytes),
        },
        Err(_) => Err(bytes),
    }
}

/// The byte stream behind a terminal: a remote SSH channel or a local PTY.
///
/// `read` must be **non-blocking** -- return `WouldBlock` when nothing is
/// waiting -- because `run_loop` polls the write, resize and disconnect queues
/// between reads on a single thread. `Ok(0)` means the other end is gone.
pub trait PtyIo: Read + Write {
    fn resize(&mut self, cols: u32, rows: u32);
    /// Tear the stream down once the loop has finished with it.
    fn shutdown(&mut self);
}

impl PtyIo for SshChannel {
    fn resize(&mut self, cols: u32, rows: u32) {
        loop {
            match self.request_pty_size(cols, rows, None, None) {
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

    fn shutdown(&mut self) {
        let _ = self.send_eof();
        let _ = self.wait_eof();
        let _ = self.close();
        let _ = self.wait_close();
    }
}

/// Run the main I/O loop for a PTY channel.
/// Reads from `channel`, writes from `write_rx`, handles disconnect and,
/// when `resize_rx` is given, terminal resize requests.
pub fn run_loop(
    mut channel: impl PtyIo,
    mut write_rx: mpsc::UnboundedReceiver<String>,
    mut disconnect_rx: mpsc::UnboundedReceiver<()>,
    mut resize_rx: Option<mpsc::UnboundedReceiver<(u32, u32)>>,
    data_channel: TerminalChannel,
    on_data_fallback: impl FnMut(&str),
    mut on_disconnect: impl FnMut(),
) {
    let mut buf = vec![0u8; 16384];
    let mut sink = OutputSink::new(channel_sender(data_channel), on_data_fallback);
    let mut intentional_disconnect = false;

    loop {
        if disconnect_rx.try_recv().is_ok() {
            intentional_disconnect = true;
            break;
        }

        // Handle resize requests
        if let Some(resize_rx) = resize_rx.as_mut() {
            while let Ok((cols, rows)) = resize_rx.try_recv() {
                channel.resize(cols, rows);
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

        // Read incoming data. A successful read loops straight back for more
        // without sleeping, so a burst is gathered into one message; the
        // stream pausing (WouldBlock) is what sends it.
        match channel.read(&mut buf) {
            Ok(0) => {
                sink.finish();
                break;
            }
            Ok(n) => sink.push(&buf[..n]),
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                sink.idle();
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(_) => {
                sink.finish();
                break;
            }
        }
    }

    channel.shutdown();

    if !intentional_disconnect {
        on_disconnect();
    }
}

#[cfg(test)]
mod tests {
    use super::decode_complete;

    #[test]
    fn a_character_split_across_two_reads_survives_whole() {
        // The bug. "é" is two bytes; a read boundary falls wherever the network
        // put it, and decoding each read on its own turned this into two
        // replacement characters.
        let whole = "café".as_bytes();
        let (first, consumed) = decode_complete(&whole[..4]);
        assert_eq!(first, "caf", "the truncated character must be held back");
        assert_eq!(consumed, 3, "its first byte stays for the next read");

        // The next read brings the rest.
        let mut carried = whole[consumed..4].to_vec();
        carried.extend_from_slice(&whole[4..]);
        let (second, consumed) = decode_complete(&carried);
        assert_eq!(second, "é");
        assert_eq!(consumed, carried.len());
    }

    #[test]
    fn a_three_byte_character_split_at_either_point_survives() {
        // Box-drawing characters are three bytes and are exactly what a MOTD
        // is full of, so both split points matter.
        let line = "─┤".as_bytes();
        for split in 1..line.len() {
            let (head, consumed) = decode_complete(&line[..split]);
            let mut carried = line[consumed..split].to_vec();
            carried.extend_from_slice(&line[split..]);
            let (tail, _) = decode_complete(&carried);
            assert_eq!(
                format!("{}{}", head, tail),
                "─┤",
                "lost a character when split at byte {}",
                split
            );
        }
    }

    #[test]
    fn complete_input_is_consumed_entirely() {
        let (text, consumed) = decode_complete("hello".as_bytes());
        assert_eq!(text, "hello");
        assert_eq!(consumed, 5);

        let (text, consumed) = decode_complete(&[]);
        assert_eq!(text, "");
        assert_eq!(consumed, 0);
    }

    #[test]
    fn genuinely_invalid_bytes_are_replaced_rather_than_held() {
        // A lone 0xFF can never become valid, so carrying it would stall the
        // stream waiting for a byte that means nothing.
        let (text, consumed) = decode_complete(&[b'a', 0xFF, b'b']);
        assert_eq!(text, "a\u{FFFD}b");
        assert_eq!(consumed, 3, "nothing is left pending");
    }

    #[test]
    fn what_is_held_back_is_never_more_than_a_character() {
        // The pending buffer is bounded by the longest UTF-8 sequence, which is
        // what keeps it from growing without limit on a hostile stream.
        for bytes in [
            vec![0xF0],             // start of a 4-byte sequence
            vec![0xF0, 0x9F],       // ...two of four
            vec![0xF0, 0x9F, 0x98], // ...three of four
        ] {
            let (_, consumed) = decode_complete(&bytes);
            assert!(
                bytes.len() - consumed < 4,
                "held {} bytes back",
                bytes.len() - consumed
            );
        }
    }
}

#[cfg(test)]
mod loop_tests {
    use super::*;
    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex};

    /// A PTY whose reads are scripted: `Some(bytes)` is data, `None` is
    /// `WouldBlock`, and running out is EOF.
    struct FakePty {
        reads: VecDeque<Option<Vec<u8>>>,
    }

    impl Read for FakePty {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            match self.reads.pop_front() {
                Some(Some(bytes)) => {
                    buf[..bytes.len()].copy_from_slice(&bytes);
                    Ok(bytes.len())
                }
                Some(None) => Err(std::io::ErrorKind::WouldBlock.into()),
                None => Ok(0),
            }
        }
    }

    impl Write for FakePty {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl PtyIo for FakePty {
        fn resize(&mut self, _: u32, _: u32) {}
        fn shutdown(&mut self) {}
    }

    struct Outcome {
        messages: Vec<Vec<u8>>,
        fallback: String,
        disconnected: bool,
    }

    fn run(reads: Vec<Option<Vec<u8>>>, with_channel: bool, disconnect_first: bool) -> Outcome {
        let sent = Arc::new(Mutex::new(Vec::new()));
        let channel: TerminalChannel = Arc::new(Mutex::new(None));
        if with_channel {
            let sent = sent.clone();
            *channel.lock().unwrap() = Some(Box::new(crate::events::FnSink(move |bytes| {
                sent.lock().unwrap().push(bytes);
                true
            })));
        }
        let (_write_tx, write_rx) = mpsc::unbounded_channel();
        let (disconnect_tx, disconnect_rx) = mpsc::unbounded_channel();
        if disconnect_first {
            disconnect_tx.send(()).unwrap();
        }
        let mut fallback = String::new();
        let mut disconnected = false;
        run_loop(
            FakePty {
                reads: reads.into(),
            },
            write_rx,
            disconnect_rx,
            None,
            channel,
            |text| fallback.push_str(text),
            || disconnected = true,
        );
        let messages = sent.lock().unwrap().clone();
        Outcome {
            messages,
            fallback,
            disconnected,
        }
    }

    #[test]
    fn a_burst_of_short_lines_becomes_one_message_with_every_byte() {
        // The bug this fixes: one message per read, ~175k of them for `seq`.
        let reads: Vec<_> = (0..500)
            .map(|i| Some(format!("{}\n", i).into_bytes()))
            .collect();
        let expected: Vec<u8> = (0..500)
            .flat_map(|i| format!("{}\n", i).into_bytes())
            .collect();
        let out = run(reads, true, false);
        assert!(out.messages.len() <= 2, "{} messages", out.messages.len());
        assert_eq!(out.messages.concat(), expected);
        assert!(out.disconnected, "EOF reports the disconnect");
    }

    #[test]
    fn a_keystroke_echo_is_sent_when_the_stream_pauses_not_held() {
        let out = run(
            vec![Some(b"x".to_vec()), None, Some(b"y".to_vec())],
            true,
            false,
        );
        assert_eq!(out.messages, vec![b"x".to_vec(), b"y".to_vec()]);
    }

    #[test]
    fn messages_are_capped_and_nothing_is_lost() {
        let chunk = vec![b'a'; 16384];
        let reads: Vec<_> = (0..10).map(|_| Some(chunk.clone())).collect();
        let out = run(reads, true, false);
        assert!(out.messages.len() >= 3, "a 160 KB burst is split");
        assert!(out
            .messages
            .iter()
            .all(|m| m.len() <= COALESCE_MAX_BYTES + 16384));
        assert_eq!(out.messages.concat().len(), 10 * 16384);
    }

    #[test]
    fn without_a_channel_output_falls_back_to_text_and_keeps_split_characters() {
        let bytes = "café ─┤".as_bytes();
        let reads = vec![Some(bytes[..4].to_vec()), None, Some(bytes[4..].to_vec())];
        let out = run(reads, false, false);
        assert!(out.messages.is_empty());
        assert_eq!(out.fallback, "café ─┤");
    }

    #[test]
    fn an_intentional_disconnect_sends_nothing_and_reports_nothing() {
        let out = run(vec![Some(b"late output".to_vec())], true, true);
        assert!(out.messages.is_empty());
        assert!(!out.disconnected);
    }
}
