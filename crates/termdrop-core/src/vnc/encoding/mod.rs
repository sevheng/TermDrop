//! Rectangle decoders.
//!
//! Raw and CopyRect live here because they are a handful of lines each; ZRLE
//! and Tight get their own files, being respectively ten and forty times the
//! size.
//!
//! One rule governs the whole module: **a rectangle we cannot decode ends the
//! session.** It is tempting to skip an unknown encoding and carry on, and it
//! is wrong — Tight and ZRLE carry zlib streams whose state spans the entire
//! connection, so a skipped rectangle desyncs every rectangle after it. The
//! failure would surface as coloured garbage minutes later rather than as an
//! error here.

pub mod tight;
pub mod zrle;

use super::conn::Conn;
use super::frame::FrameBuilder;
use super::pixel::PixelFormat;
use super::proto::{encoding, RectHeader};

/// The largest rectangle payload we will allocate for, ~8K by 8K of RGBA.
const MAX_RECT_BYTES: usize = 256 * 1024 * 1024;

/// A cursor over an in-memory buffer that cannot read past the end.
///
/// Every length inside a compressed rectangle comes from the server, so the
/// decoders need bounds checking on each read rather than at the edges. Sharing
/// one reader keeps that from being re-implemented per encoding, each time with
/// its own chance of an off-by-one that reads into the next tile.
pub struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    pub fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, at: 0 }
    }

    pub fn u8(&mut self) -> Result<u8, String> {
        let byte = *self
            .bytes
            .get(self.at)
            .ok_or_else(|| format!("rectangle data ended at byte {}", self.at))?;
        self.at += 1;
        Ok(byte)
    }

    pub fn take(&mut self, n: usize) -> Result<&'a [u8], String> {
        let end = self
            .at
            .checked_add(n)
            .ok_or_else(|| "rectangle read overflowed".to_string())?;
        if end > self.bytes.len() {
            return Err(format!(
                "rectangle wanted {} bytes at {} but only {} remain",
                n,
                self.at,
                self.bytes.len().saturating_sub(self.at)
            ));
        }
        let slice = &self.bytes[self.at..end];
        self.at = end;
        Ok(slice)
    }

    /// How much is left. Used by the decoders' tests to assert that a rectangle
    /// consumed exactly its own bytes and not one more.
    #[cfg(test)]
    pub fn remaining(&self) -> usize {
        self.bytes.len().saturating_sub(self.at)
    }
}

/// Decoder state that outlives a single rectangle.
///
/// ZRLE and Tight both carry zlib streams whose dictionaries span the entire
/// connection. Recreating them per rectangle works on the first frame and then
/// silently corrupts every one after it, so they live here and are threaded
/// through every decode call.
#[derive(Default)]
pub struct Decoders {
    pub zrle: zrle::ZrleStream,
    pub tight: tight::TightStreams,
    /// A raw rectangle's bytes, straight off the socket.
    raw_src: Vec<u8>,
    /// The same pixels converted to RGBA.
    raw_rgba: Vec<u8>,
}

/// Grow a reused buffer to at least `len`, without re-zeroing what it already
/// holds.
///
/// `clear()` then `resize()` looks equivalent and is not: it memsets the whole
/// buffer every time, which for a full-screen rectangle is 14MB of zeros that
/// the very next read overwrites. Since the framebuffer size barely changes,
/// growing only when it must means no memset at all after the first frame.
fn ensure(buf: &mut Vec<u8>, len: usize) -> &mut [u8] {
    if buf.len() < len {
        buf.resize(len, 0);
    }
    &mut buf[..len]
}

/// What a decoded rectangle means to the session loop.
pub enum RectOutcome {
    /// Pixels were appended to the frame.
    Painted,
    /// A `DesktopSize` pseudo-rectangle: the framebuffer is now this size.
    Resized { width: u16, height: u16 },
    /// The `LastRect` marker: stop reading rectangles from this update.
    Last,
}

/// Which decoder an encoding number selects.
///
/// Split out from [`decode_rect`] so the routing can be tested without a
/// socket, the way `security.rs` keeps its `classify_*` functions pure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dispatch {
    Raw,
    CopyRect,
    Zrle,
    Tight,
    Cursor,
    DesktopSize,
    LastRect,
    /// An encoding we never advertised. Fatal, deliberately — see the module
    /// documentation.
    Unsupported(i32),
}

pub fn classify(encoding_value: i32) -> Dispatch {
    match encoding_value {
        encoding::RAW => Dispatch::Raw,
        encoding::COPY_RECT => Dispatch::CopyRect,
        encoding::ZRLE => Dispatch::Zrle,
        encoding::TIGHT => Dispatch::Tight,
        encoding::PSEUDO_CURSOR => Dispatch::Cursor,
        encoding::PSEUDO_DESKTOP_SIZE => Dispatch::DesktopSize,
        encoding::PSEUDO_LAST_RECT => Dispatch::LastRect,
        other => Dispatch::Unsupported(other),
    }
}

/// The message shown when a server uses something we cannot decode.
pub fn unsupported_message(encoding_value: i32) -> String {
    format!(
        "the server used encoding {}, which TermDrop did not ask for and cannot \
         decode. The connection has to end here: skipping a rectangle would \
         corrupt every one after it.",
        encoding_value
    )
}

/// Decode one rectangle into `frame`.
pub fn decode_rect(
    conn: &mut Conn,
    header: &RectHeader,
    format: &PixelFormat,
    frame: &mut FrameBuilder,
    decoders: &mut Decoders,
) -> Result<RectOutcome, String> {
    match classify(header.encoding) {
        Dispatch::Raw => {
            raw(conn, header, format, frame, decoders)?;
            Ok(RectOutcome::Painted)
        }
        Dispatch::CopyRect => {
            copy_rect(conn, header, frame)?;
            Ok(RectOutcome::Painted)
        }
        Dispatch::Zrle => {
            // Length-prefixed, so the compressed bytes can be lifted off the
            // socket in one go and handed to the connection-long stream.
            let mut len = [0u8; 4];
            conn.read_exact(&mut len)?;
            let len = u32::from_be_bytes(len) as usize;
            let compressed = conn.read_vec(len, MAX_RECT_BYTES, "ZRLE rectangle")?;
            zrle::decode(&mut decoders.zrle, &compressed, header, format, frame)?;
            Ok(RectOutcome::Painted)
        }
        Dispatch::Tight => {
            // Tight has no overall length: the control byte decides how much
            // follows, so the body is read piece by piece off the socket.
            tight_rect(conn, header, format, frame, decoders)?;
            Ok(RectOutcome::Painted)
        }
        Dispatch::Cursor => {
            cursor(conn, header, format, frame)?;
            Ok(RectOutcome::Painted)
        }
        Dispatch::DesktopSize => Ok(RectOutcome::Resized {
            width: header.width,
            height: header.height,
        }),
        Dispatch::LastRect => Ok(RectOutcome::Last),
        Dispatch::Unsupported(value) => Err(unsupported_message(value)),
    }
}

/// Read a Tight rectangle off the socket and decode it.
///
/// Tight is the one encoding with no length up front: how many bytes follow
/// depends on the control byte, the filter, and whether the payload was small
/// enough to skip zlib. So the pieces are read in order rather than lifted off
/// in one go.
fn tight_rect(
    conn: &mut Conn,
    header: &RectHeader,
    format: &PixelFormat,
    frame: &mut FrameBuilder,
    decoders: &mut Decoders,
) -> Result<(), String> {
    let mut control = [0u8; 1];
    conn.read_exact(&mut control)?;
    let body = tight::read_body(conn, control[0], header, format)?;
    let mut reader = Reader::new(&body);
    tight::decode(
        &mut decoders.tight,
        control[0],
        &mut reader,
        header,
        format,
        frame,
    )
}

/// Uncompressed pixels, row by row.
fn raw(
    conn: &mut Conn,
    header: &RectHeader,
    format: &PixelFormat,
    frame: &mut FrameBuilder,
    decoders: &mut Decoders,
) -> Result<(), String> {
    let pixels = (header.width as usize) * (header.height as usize);
    let stride = (format.bits_per_pixel as usize).div_ceil(8);
    let len = pixels
        .checked_mul(stride)
        .ok_or_else(|| "raw rectangle size overflowed".to_string())?;
    if len > MAX_RECT_BYTES {
        return Err(format!("implausible raw rectangle: {} bytes", len));
    }

    // Both buffers live in `Decoders` and grow only. A full-screen rectangle is
    // 14MB at 2560x1440; allocating that per frame meant an mmap and thousands
    // of page faults every time, on the same thread that has to service input.
    let src = ensure(&mut decoders.raw_src, len);
    conn.read_exact(src)?;

    let rgba = ensure(&mut decoders.raw_rgba, pixels * 4);
    format.to_rgba(&decoders.raw_src[..len], pixels, rgba)?;
    frame.push_raw(
        header.x,
        header.y,
        header.width,
        header.height,
        &decoders.raw_rgba[..pixels * 4],
    );
    Ok(())
}

/// The remote pointer's shape, to be drawn locally.
///
/// Worth having: without it the local cursor is a generic arrow wherever the
/// remote is showing a text beam or a resize handle, and the lag between moving
/// the mouse and seeing the remote pointer follow is the thing that makes a
/// remote desktop feel remote.
///
/// The colour data and the 1-bit mask are composited here rather than sent on
/// separately, because the frontend wants one RGBA image for a CSS cursor and
/// has no use for the mask on its own.
fn cursor(
    conn: &mut Conn,
    header: &RectHeader,
    format: &PixelFormat,
    frame: &mut FrameBuilder,
) -> Result<(), String> {
    let width = header.width as usize;
    let height = header.height as usize;

    // A zero-sized cursor means the remote is hiding it.
    if width == 0 || height == 0 {
        frame.push_cursor(header.x, header.y, 0, 0, &[]);
        return Ok(());
    }

    let stride = (format.bits_per_pixel as usize).div_ceil(8);
    let colours = conn.read_vec(width * height * stride, MAX_RECT_BYTES, "cursor image")?;
    // The mask is one bit per pixel, and each *row* is padded to a byte.
    let mask_row = width.div_ceil(8);
    let mask = conn.read_vec(mask_row * height, MAX_RECT_BYTES, "cursor mask")?;

    let mut rgba = Vec::with_capacity(width * height * 4);
    for y in 0..height {
        for x in 0..width {
            let [r, g, b] = format.pixel_to_rgb(&colours[(y * width + x) * stride..]);
            let bit = mask[y * mask_row + x / 8] >> (7 - (x % 8)) & 1;
            // Transparent where the mask is clear: a cursor is rarely
            // rectangular, and an opaque block around it looks broken.
            rgba.extend_from_slice(&[r, g, b, if bit == 1 { 0xFF } else { 0x00 }]);
        }
    }
    frame.push_cursor(header.x, header.y, header.width, header.height, &rgba);
    Ok(())
}

/// A region the client already has, somewhere else on screen.
///
/// Four bytes on the wire however large the rectangle, which is why dragging a
/// window is nearly free and why this is worth having before any compressed
/// encoding.
fn copy_rect(conn: &mut Conn, header: &RectHeader, frame: &mut FrameBuilder) -> Result<(), String> {
    let mut src = [0u8; 4];
    conn.read_exact(&mut src)?;
    frame.push_copy_rect(
        header.x,
        header.y,
        header.width,
        header.height,
        u16::from_be_bytes([src[0], src[1]]),
        u16::from_be_bytes([src[2], src[3]]),
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vnc::proto::supported_encodings;

    #[test]
    fn every_encoding_we_advertise_has_a_decoder() {
        // A server only sends what we asked for, so these two lists drifting
        // apart is the one way an undecodable rectangle can arrive. Better a
        // failing test than a session that dies mid-frame.
        for value in supported_encodings() {
            assert_ne!(
                classify(value),
                Dispatch::Unsupported(value),
                "advertised encoding {} has no decoder",
                value
            );
        }
    }

    #[test]
    fn an_unadvertised_encoding_is_fatal_rather_than_skipped() {
        // Hextile, RRE, CoRRE and TRLE are never advertised, so no decoder
        // for them is ever needed. Skipping
        // one would desync the zlib streams Tight and ZRLE carry across the
        // whole connection, so garbage would appear minutes later instead of
        // an error appearing now.
        for value in [2, 4, 5, 15] {
            assert_eq!(classify(value), Dispatch::Unsupported(value));
        }
        let message = unsupported_message(5);
        assert!(message.contains('5'), "the encoding should be named");
        assert!(
            message.contains("corrupt"),
            "the message should say why it cannot simply continue"
        );
    }

    #[test]
    fn the_pseudo_rectangles_route_to_markers_not_decoders() {
        // Neither carries a payload, so reaching a decoder that tried to read
        // one would consume the next rectangle's bytes.
        assert_eq!(
            classify(encoding::PSEUDO_DESKTOP_SIZE),
            Dispatch::DesktopSize
        );
        assert_eq!(classify(encoding::PSEUDO_LAST_RECT), Dispatch::LastRect);
    }

    #[test]
    fn raw_and_copy_rect_route_to_their_own_decoders() {
        assert_eq!(classify(encoding::RAW), Dispatch::Raw);
        assert_eq!(classify(encoding::COPY_RECT), Dispatch::CopyRect);
    }
}
