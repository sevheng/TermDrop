//! The binary frame format that carries decoded rectangles to the canvas.
//!
//! One message is one complete `FramebufferUpdate`, so the renderer always gets
//! an atomic unit and never paints a torn frame.
//!
//! **Little-endian, unlike everything in [`super::proto`].** RFB is big-endian
//! on the wire; the frontend reads this with a `DataView` on a machine that is
//! little-endian, and converting once here is cheaper than making JavaScript
//! byte-swap every field of every rect. The two orders are kept in separate
//! modules so the distinction is impossible to lose track of.
//!
//! A note on how this reaches the frontend: it must be handed to a
//! `tauri::ipc::Channel<tauri::ipc::InvokeResponseBody>` as
//! `InvokeResponseBody::Raw`. A `Channel<Vec<u8>>` looks like the obvious
//! choice and is a trap — Tauri has a blanket `impl<T: Serialize> IpcResponse`,
//! so it would JSON-encode every frame as an array of decimal numbers. That is
//! survivable for the SSH terminal's 4KB flushes and emphatically not for an
//! 8MB framebuffer.

/// Spells "TDVB" when the little-endian bytes are read in order.
///
/// A desync in the frame stream otherwise shows up as a garbled picture
/// hundreds of frames later; this turns it into an error on the first one.
pub const MAGIC: u32 = u32::from_le_bytes(*b"TDVB");

/// Bumped only for a change the frontend cannot parse without knowing.
pub const VERSION: u16 = 1;

pub const HEADER_LEN: usize = 16;
#[cfg(test)]
pub const RECT_HEADER_LEN: usize = 16;

/// What the renderer should do with a rectangle.
pub mod op {
    /// `w * h * 4` bytes of RGBA, straight into `putImageData`.
    pub const RAW_RGBA: u16 = 1;
    /// 4 bytes: source x and y. A canvas self-blit.
    pub const COPY_RECT: u16 = 2;
    /// A JPEG, decoded by the browser rather than by us.
    pub const JPEG: u16 = 3;
    /// 4 bytes of RGBA covering the whole rectangle.
    pub const FILL: u16 = 4;
    /// No payload; `width` and `height` are the new desktop size.
    pub const DESKTOP_SIZE: u16 = 5;
    /// `w * h * 4` RGBA cursor image; `x` and `y` are the hotspot.
    pub const CURSOR: u16 = 6;
}

/// Accumulates one frame.
///
/// The header is written up front with placeholder counts and patched in
/// [`FrameBuilder::finish`], so rectangles can be appended as they decode
/// without knowing ahead of time how many there will be — which matters
/// because a server using `LastRect` does not tell us.
pub struct FrameBuilder {
    buf: Vec<u8>,
    rect_count: u16,
}

impl FrameBuilder {
    pub fn new(seq: u32) -> Self {
        let mut buf = Vec::with_capacity(HEADER_LEN + 64 * 1024);
        buf.extend_from_slice(&MAGIC.to_le_bytes());
        buf.extend_from_slice(&VERSION.to_le_bytes());
        buf.extend_from_slice(&0u16.to_le_bytes()); // rect_count, patched later
        buf.extend_from_slice(&seq.to_le_bytes());
        buf.extend_from_slice(&0u32.to_le_bytes()); // payload_len, patched later
        debug_assert_eq!(buf.len(), HEADER_LEN);
        Self { buf, rect_count: 0 }
    }

    fn push_header(&mut self, x: u16, y: u16, width: u16, height: u16, op: u16, len: usize) {
        self.buf.extend_from_slice(&x.to_le_bytes());
        self.buf.extend_from_slice(&y.to_le_bytes());
        self.buf.extend_from_slice(&width.to_le_bytes());
        self.buf.extend_from_slice(&height.to_le_bytes());
        self.buf.extend_from_slice(&op.to_le_bytes());
        // Padding, so every payload starts 4-byte aligned from the frame start.
        self.buf.extend_from_slice(&0u16.to_le_bytes());
        self.buf.extend_from_slice(&(len as u32).to_le_bytes());
        self.rect_count = self.rect_count.saturating_add(1);
    }

    pub fn push_raw(&mut self, x: u16, y: u16, width: u16, height: u16, rgba: &[u8]) {
        self.push_header(x, y, width, height, op::RAW_RGBA, rgba.len());
        self.buf.extend_from_slice(rgba);
    }

    /// A solid rectangle, as 4 bytes rather than `w * h * 4`.
    ///
    /// Not an optimisation to add later: Tight's fill subencoding and ZRLE's
    /// solid tiles cover most of a real desktop, and sending a megabyte of
    /// identical pixels for a window background is the difference between a
    /// usable viewer and an unusable one.
    pub fn push_fill(&mut self, x: u16, y: u16, width: u16, height: u16, rgb: [u8; 3]) {
        self.push_header(x, y, width, height, op::FILL, 4);
        self.buf.extend_from_slice(&[rgb[0], rgb[1], rgb[2], 0xFF]);
    }

    pub fn push_copy_rect(
        &mut self,
        x: u16,
        y: u16,
        width: u16,
        height: u16,
        src_x: u16,
        src_y: u16,
    ) {
        self.push_header(x, y, width, height, op::COPY_RECT, 4);
        self.buf.extend_from_slice(&src_x.to_le_bytes());
        self.buf.extend_from_slice(&src_y.to_le_bytes());
    }

    /// Hand a Tight JPEG rectangle's bytes through undecoded.
    ///
    /// The browser has a native JPEG decoder and we would otherwise need one in
    /// Rust *and* pay a hundredfold IPC amplification — a 30KB rect covering
    /// 1024x768 becomes 3MB once expanded to RGBA.
    ///
    /// This is safe only because a Tight JPEG rectangle is self-contained: by
    /// specification it touches none of the four zlib streams, so passing it on
    /// cannot desync the decoder. A Tight *basic* rectangle does, which is why
    /// only this subencoding gets the treatment.
    pub fn push_jpeg(&mut self, x: u16, y: u16, width: u16, height: u16, jpeg: &[u8]) {
        self.push_header(x, y, width, height, op::JPEG, jpeg.len());
        self.buf.extend_from_slice(jpeg);
    }

    /// The desktop changed size.
    ///
    /// Must be the **last** rectangle in its frame: resizing a canvas clears
    /// it, so anything appended after this would be painted and then thrown
    /// away. The caller follows it with a non-incremental update request.
    pub fn push_desktop_size(&mut self, width: u16, height: u16) {
        self.push_header(0, 0, width, height, op::DESKTOP_SIZE, 0);
    }

    pub fn push_cursor(&mut self, hot_x: u16, hot_y: u16, width: u16, height: u16, rgba: &[u8]) {
        self.push_header(hot_x, hot_y, width, height, op::CURSOR, rgba.len());
        self.buf.extend_from_slice(rgba);
    }

    #[cfg(test)]
    pub fn rect_count(&self) -> u16 {
        self.rect_count
    }

    pub fn is_empty(&self) -> bool {
        self.rect_count == 0
    }

    /// Patch the header and hand back the finished frame.
    pub fn finish(mut self) -> Vec<u8> {
        let payload_len = (self.buf.len() - HEADER_LEN) as u32;
        self.buf[6..8].copy_from_slice(&self.rect_count.to_le_bytes());
        self.buf[12..16].copy_from_slice(&payload_len.to_le_bytes());
        self.buf
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn u16_at(buf: &[u8], at: usize) -> u16 {
        u16::from_le_bytes([buf[at], buf[at + 1]])
    }
    fn u32_at(buf: &[u8], at: usize) -> u32 {
        u32::from_le_bytes([buf[at], buf[at + 1], buf[at + 2], buf[at + 3]])
    }

    #[test]
    fn the_magic_spells_tdvb_on_the_wire() {
        // The frontend matches these four bytes in order, so the constant and
        // the byte order have to agree.
        assert_eq!(MAGIC.to_le_bytes(), *b"TDVB");
    }

    #[test]
    fn an_empty_frame_is_just_a_header() {
        let frame = FrameBuilder::new(7).finish();
        assert_eq!(frame.len(), HEADER_LEN);
        assert_eq!(u32_at(&frame, 0), MAGIC);
        assert_eq!(u16_at(&frame, 4), VERSION);
        assert_eq!(u16_at(&frame, 6), 0, "rect count");
        assert_eq!(u32_at(&frame, 8), 7, "sequence number");
        assert_eq!(u32_at(&frame, 12), 0, "payload length");
    }

    #[test]
    fn the_header_is_patched_with_the_real_count_and_length() {
        // Both are unknown when the header is written — a server using
        // LastRect never tells us the count — so the patch is what makes the
        // frame parseable at all.
        let mut f = FrameBuilder::new(1);
        f.push_fill(0, 0, 10, 10, [1, 2, 3]);
        f.push_fill(10, 0, 10, 10, [4, 5, 6]);
        assert_eq!(f.rect_count(), 2);
        let frame = f.finish();

        assert_eq!(u16_at(&frame, 6), 2);
        assert_eq!(u32_at(&frame, 12), (frame.len() - HEADER_LEN) as u32);
        assert_eq!(frame.len(), HEADER_LEN + 2 * (RECT_HEADER_LEN + 4));
    }

    #[test]
    fn a_raw_rectangle_carries_its_pixels_verbatim() {
        let rgba: Vec<u8> = (0..2 * 3 * 4).map(|i| i as u8).collect();
        let mut f = FrameBuilder::new(0);
        f.push_raw(5, 9, 2, 3, &rgba);
        let frame = f.finish();

        let r = HEADER_LEN;
        assert_eq!(u16_at(&frame, r), 5, "x");
        assert_eq!(u16_at(&frame, r + 2), 9, "y");
        assert_eq!(u16_at(&frame, r + 4), 2, "width");
        assert_eq!(u16_at(&frame, r + 6), 3, "height");
        assert_eq!(u16_at(&frame, r + 8), op::RAW_RGBA);
        assert_eq!(u16_at(&frame, r + 10), 0, "padding");
        assert_eq!(u32_at(&frame, r + 12), rgba.len() as u32);
        assert_eq!(&frame[r + RECT_HEADER_LEN..], &rgba[..]);
    }

    #[test]
    fn a_fill_is_four_bytes_however_large_the_rectangle() {
        // The whole point: a full-screen background must not cost 8MB.
        let mut f = FrameBuilder::new(0);
        f.push_fill(0, 0, 1920, 1080, [0x22, 0x44, 0x66]);
        let frame = f.finish();

        assert_eq!(frame.len(), HEADER_LEN + RECT_HEADER_LEN + 4);
        let r = HEADER_LEN;
        assert_eq!(u16_at(&frame, r + 8), op::FILL);
        assert_eq!(u32_at(&frame, r + 12), 4);
        assert_eq!(
            &frame[r + RECT_HEADER_LEN..],
            &[0x22, 0x44, 0x66, 0xFF],
            "alpha is forced opaque"
        );
    }

    #[test]
    fn a_copy_rect_carries_only_its_source_position() {
        let mut f = FrameBuilder::new(0);
        f.push_copy_rect(100, 200, 50, 60, 10, 20);
        let frame = f.finish();

        let r = HEADER_LEN;
        assert_eq!(u16_at(&frame, r + 8), op::COPY_RECT);
        assert_eq!(u32_at(&frame, r + 12), 4);
        assert_eq!(u16_at(&frame, r + RECT_HEADER_LEN), 10, "source x");
        assert_eq!(u16_at(&frame, r + RECT_HEADER_LEN + 2), 20, "source y");
    }

    #[test]
    fn a_jpeg_rectangle_is_passed_through_undecoded() {
        // Byte-identical: the browser decodes it, so anything we did to it here
        // would be both wasted work and a chance to corrupt it.
        let jpeg = [0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10, 0xFF, 0xD9];
        let mut f = FrameBuilder::new(0);
        f.push_jpeg(0, 0, 16, 16, &jpeg);
        let frame = f.finish();

        let r = HEADER_LEN;
        assert_eq!(u16_at(&frame, r + 8), op::JPEG);
        assert_eq!(&frame[r + RECT_HEADER_LEN..], &jpeg);
    }

    #[test]
    fn desktop_size_has_no_payload_and_carries_the_new_size() {
        let mut f = FrameBuilder::new(0);
        f.push_desktop_size(1280, 720);
        let frame = f.finish();

        let r = HEADER_LEN;
        assert_eq!(u16_at(&frame, r + 4), 1280, "width");
        assert_eq!(u16_at(&frame, r + 6), 720, "height");
        assert_eq!(u16_at(&frame, r + 8), op::DESKTOP_SIZE);
        assert_eq!(u32_at(&frame, r + 12), 0);
        assert_eq!(frame.len(), HEADER_LEN + RECT_HEADER_LEN);
    }

    #[test]
    fn a_cursor_puts_its_hotspot_in_the_position_fields() {
        let rgba = vec![0u8; 4 * 4 * 4];
        let mut f = FrameBuilder::new(0);
        f.push_cursor(2, 3, 4, 4, &rgba);
        let frame = f.finish();

        let r = HEADER_LEN;
        assert_eq!(u16_at(&frame, r), 2, "hotspot x");
        assert_eq!(u16_at(&frame, r + 2), 3, "hotspot y");
        assert_eq!(u16_at(&frame, r + 8), op::CURSOR);
    }

    #[test]
    fn every_payload_starts_four_byte_aligned() {
        // What the two padding bytes in the rect header buy: the frontend can
        // take a Uint32Array view straight onto a raw rect's pixels.
        let mut f = FrameBuilder::new(0);
        f.push_fill(0, 0, 1, 1, [0, 0, 0]);
        f.push_raw(0, 0, 1, 1, &[1, 2, 3, 4]);
        f.push_copy_rect(0, 0, 1, 1, 0, 0);
        let frame = f.finish();

        assert_eq!(HEADER_LEN % 4, 0);
        assert_eq!(RECT_HEADER_LEN % 4, 0);
        assert_eq!(frame.len() % 4, 0);
    }

    #[test]
    fn mixed_rectangles_stay_in_the_order_they_were_pushed() {
        // Wire order is apply order — a CopyRect that overtook the JPEG it
        // depends on would paint stale pixels.
        let mut f = FrameBuilder::new(3);
        f.push_jpeg(0, 0, 8, 8, &[0xFF, 0xD8]);
        f.push_copy_rect(8, 0, 8, 8, 0, 0);
        f.push_fill(16, 0, 8, 8, [9, 9, 9]);
        let frame = f.finish();

        let mut at = HEADER_LEN;
        let mut seen = Vec::new();
        for _ in 0..u16_at(&frame, 6) {
            seen.push(u16_at(&frame, at + 8));
            at += RECT_HEADER_LEN + u32_at(&frame, at + 12) as usize;
        }
        assert_eq!(seen, vec![op::JPEG, op::COPY_RECT, op::FILL]);
        assert_eq!(
            at,
            frame.len(),
            "walking the rects consumed the frame exactly"
        );
    }
}
