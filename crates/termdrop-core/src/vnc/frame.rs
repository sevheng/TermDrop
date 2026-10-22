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

/// Side of the square a raw rectangle is compared in against the shadow.
///
/// ZRLE already sends 64x64 tiles; a Tight rectangle can be any size, and
/// splitting it the same way means one changed pixel costs one tile rather
/// than the whole rectangle.
const DIFF_TILE: usize = 64;

/// What the canvas is showing, kept so that pixels it already has are not
/// sent to it again.
///
/// **Measured against macOS Screen Sharing, 96-98% of the pixels in an
/// incremental update were identical to the screen.** The server re-encodes
/// large regions around every change, so a 2560x1440 desktop was shipping
/// 1-6MB of RGBA per frame across IPC, nearly all of it a repaint of what was
/// already there. Comparing a decoded tile with this copy costs a `memcmp`.
///
/// RGBA, and **alpha 0 means "unknown"**: decoded framebuffer pixels are
/// always opaque, so an unknown pixel can never compare equal to one the
/// server sends. Everything starts unknown, a JPEG rectangle (which we do not
/// decode) makes its area unknown, and a full refresh makes all of it unknown
/// again, which is what makes the Refresh button still repaint everything.
pub struct Shadow {
    width: usize,
    height: usize,
    rgba: Vec<u8>,
}

impl Shadow {
    pub fn new(width: u16, height: u16) -> Self {
        let (width, height) = (width as usize, height as usize);
        Self {
            width,
            height,
            rgba: vec![0; width * height * 4],
        }
    }

    /// Forget everything: the next frame is sent whole.
    pub fn invalidate(&mut self) {
        self.rgba.fill(0);
    }

    /// Whether a rectangle lies wholly on the screen. One that does not is
    /// passed through untouched and never recorded.
    fn contains(&self, x: usize, y: usize, width: usize, height: usize) -> bool {
        x + width <= self.width && y + height <= self.height
    }

    fn row(&self, x: usize, y: usize, width: usize) -> &[u8] {
        let at = (y * self.width + x) * 4;
        &self.rgba[at..at + width * 4]
    }

    fn row_mut(&mut self, x: usize, y: usize, width: usize) -> &mut [u8] {
        let at = (y * self.width + x) * 4;
        &mut self.rgba[at..at + width * 4]
    }

    fn mark_unknown(&mut self, x: usize, y: usize, width: usize, height: usize) {
        let (width, height) = (
            width.min(self.width.saturating_sub(x)),
            height.min(self.height.saturating_sub(y)),
        );
        for row in y..y + height {
            for pixel in self.row_mut(x, row, width).as_chunks_mut::<4>().0 {
                pixel[3] = 0;
            }
        }
    }
}

/// Accumulates one frame.
///
/// The header is written up front with placeholder counts and patched in
/// [`FrameBuilder::finish`], so rectangles can be appended as they decode
/// without knowing ahead of time how many there will be — which matters
/// because a server using `LastRect` does not tell us.
///
/// With a [`Shadow`] attached, pixels the canvas already shows are dropped
/// here, so the decoders need not know about it.
pub struct FrameBuilder {
    buf: Vec<u8>,
    rect_count: u16,
    shadow: Option<Shadow>,
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
        Self {
            buf,
            rect_count: 0,
            shadow: None,
        }
    }

    /// Drop pixels `shadow` says the canvas already shows. Hand it back with
    /// [`FrameBuilder::take_shadow`] before finishing.
    pub fn with_shadow(mut self, shadow: Option<Shadow>) -> Self {
        self.shadow = shadow;
        self
    }

    pub fn take_shadow(&mut self) -> Option<Shadow> {
        self.shadow.take()
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
        let (ux, uy, w, h) = (x as usize, y as usize, width as usize, height as usize);
        let Some(mut shadow) = self
            .shadow
            .take_if(|s| rgba.len() == w * h * 4 && s.contains(ux, uy, w, h))
        else {
            self.push_header(x, y, width, height, op::RAW_RGBA, rgba.len());
            self.buf.extend_from_slice(rgba);
            return;
        };

        for ty in (0..h).step_by(DIFF_TILE) {
            for tx in (0..w).step_by(DIFF_TILE) {
                let (tw, th) = (DIFF_TILE.min(w - tx), DIFF_TILE.min(h - ty));
                let source = |row: usize| {
                    let at = ((ty + row) * w + tx) * 4;
                    &rgba[at..at + tw * 4]
                };
                if (0..th).all(|row| shadow.row(ux + tx, uy + ty + row, tw) == source(row)) {
                    continue;
                }
                // Rows are copied straight from the source rectangle, so a
                // tile costs no intermediate buffer.
                self.push_header(
                    (ux + tx) as u16,
                    (uy + ty) as u16,
                    tw as u16,
                    th as u16,
                    op::RAW_RGBA,
                    tw * th * 4,
                );
                for row in 0..th {
                    let pixels = source(row);
                    self.buf.extend_from_slice(pixels);
                    shadow
                        .row_mut(ux + tx, uy + ty + row, tw)
                        .copy_from_slice(pixels);
                }
            }
        }
        self.shadow = Some(shadow);
    }

    /// A solid rectangle, as 4 bytes rather than `w * h * 4`.
    ///
    /// Not an optimisation to add later: Tight's fill subencoding and ZRLE's
    /// solid tiles cover most of a real desktop, and sending a megabyte of
    /// identical pixels for a window background is the difference between a
    /// usable viewer and an unusable one.
    pub fn push_fill(&mut self, x: u16, y: u16, width: u16, height: u16, rgb: [u8; 3]) {
        let colour = [rgb[0], rgb[1], rgb[2], 0xFF];
        let (ux, uy, w, h) = (x as usize, y as usize, width as usize, height as usize);
        if let Some(shadow) = self.shadow.as_mut().filter(|s| s.contains(ux, uy, w, h)) {
            let same = (uy..uy + h).all(|row| {
                shadow
                    .row(ux, row, w)
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .all(|p| *p == colour)
            });
            if same {
                return;
            }
            for row in uy..uy + h {
                for pixel in shadow.row_mut(ux, row, w).as_chunks_mut::<4>().0 {
                    *pixel = colour;
                }
            }
        }
        self.push_header(x, y, width, height, op::FILL, 4);
        self.buf.extend_from_slice(&colour);
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
        // The canvas blits from itself, so the shadow does the same. Through a
        // copy, because source and destination may overlap.
        let (w, h) = (width as usize, height as usize);
        let (dx, dy, sx, sy) = (x as usize, y as usize, src_x as usize, src_y as usize);
        if let Some(shadow) = self.shadow.as_mut() {
            if shadow.contains(dx, dy, w, h) && shadow.contains(sx, sy, w, h) {
                let block: Vec<u8> = (sy..sy + h)
                    .flat_map(|row| shadow.row(sx, row, w).to_vec())
                    .collect();
                for (i, row) in (dy..dy + h).enumerate() {
                    shadow
                        .row_mut(dx, row, w)
                        .copy_from_slice(&block[i * w * 4..(i + 1) * w * 4]);
                }
            } else {
                shadow.invalidate();
            }
        }
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
        // The browser decodes it, so we never learn its pixels.
        if let Some(shadow) = self.shadow.as_mut() {
            shadow.mark_unknown(x as usize, y as usize, width as usize, height as usize);
        }
        self.push_header(x, y, width, height, op::JPEG, jpeg.len());
        self.buf.extend_from_slice(jpeg);
    }

    /// The desktop changed size.
    ///
    /// Must be the **last** rectangle in its frame: resizing a canvas clears
    /// it, so anything appended after this would be painted and then thrown
    /// away. The caller follows it with a non-incremental update request.
    pub fn push_desktop_size(&mut self, width: u16, height: u16) {
        // Resizing clears the canvas.
        if self.shadow.is_some() {
            self.shadow = Some(Shadow::new(width, height));
        }
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

    /// (x, y, width, height, op) of one rectangle in a finished frame.
    type Rect = (u16, u16, u16, u16, u16);

    fn rects_of(frame: &[u8]) -> Vec<Rect> {
        let mut at = HEADER_LEN;
        let mut out = Vec::new();
        for _ in 0..u16_at(frame, 6) {
            out.push((
                u16_at(frame, at),
                u16_at(frame, at + 2),
                u16_at(frame, at + 4),
                u16_at(frame, at + 6),
                u16_at(frame, at + 8),
            ));
            at += RECT_HEADER_LEN + u32_at(frame, at + 12) as usize;
        }
        out
    }

    fn solid(width: usize, height: usize, rgb: [u8; 3]) -> Vec<u8> {
        [rgb[0], rgb[1], rgb[2], 0xFF].repeat(width * height)
    }

    /// Push through a shadow and return the frame's rects plus the shadow.
    fn through(shadow: Shadow, push: impl FnOnce(&mut FrameBuilder)) -> (Vec<Rect>, Shadow) {
        let mut f = FrameBuilder::new(0).with_shadow(Some(shadow));
        push(&mut f);
        let shadow = f.take_shadow().unwrap();
        (rects_of(&f.finish()), shadow)
    }

    #[test]
    fn pixels_the_canvas_already_shows_are_not_sent_again() {
        // The measured case: macOS resends large unchanged regions, and every
        // byte of them used to cross IPC.
        let tile = solid(64, 64, [10, 20, 30]);
        let (first, shadow) = through(Shadow::new(256, 256), |f| f.push_raw(0, 0, 64, 64, &tile));
        assert_eq!(first.len(), 1, "unknown pixels are always sent");

        let (again, _) = through(shadow, |f| f.push_raw(0, 0, 64, 64, &tile));
        assert!(again.is_empty(), "an identical tile is dropped");
    }

    #[test]
    fn only_the_changed_tile_of_a_large_rectangle_is_sent() {
        let mut big = solid(128, 128, [1, 1, 1]);
        let (_, shadow) = through(Shadow::new(256, 256), |f| f.push_raw(0, 0, 128, 128, &big));

        // One pixel in the bottom-right 64x64 tile.
        let at = (100 * 128 + 100) * 4;
        big[at] = 99;
        let (rects, _) = through(shadow, |f| f.push_raw(0, 0, 128, 128, &big));
        assert_eq!(rects, vec![(64, 64, 64, 64, op::RAW_RGBA)]);
    }

    #[test]
    fn a_tile_is_sent_with_its_own_pixels() {
        // The tile's payload is gathered row by row from the wider source.
        let mut big = solid(128, 64, [0, 0, 0]);
        for row in 0..64 {
            let at = (row * 128 + 64) * 4;
            big[at..at + 4].copy_from_slice(&[row as u8, 2, 3, 0xFF]);
        }
        let (_, shadow) = through(Shadow::new(128, 64), |f| {
            f.push_raw(0, 0, 128, 64, &solid(128, 64, [0, 0, 0]))
        });
        let mut f = FrameBuilder::new(0).with_shadow(Some(shadow));
        f.push_raw(0, 0, 128, 64, &big);
        let frame = f.finish();
        assert_eq!(rects_of(&frame), vec![(64, 0, 64, 64, op::RAW_RGBA)]);
        let payload = &frame[HEADER_LEN + RECT_HEADER_LEN..];
        for row in 0..64 {
            assert_eq!(
                &payload[row * 64 * 4..row * 64 * 4 + 4],
                &[row as u8, 2, 3, 0xFF]
            );
        }
    }

    #[test]
    fn a_fill_the_canvas_already_has_is_dropped() {
        let (_, shadow) = through(Shadow::new(64, 64), |f| {
            f.push_fill(0, 0, 32, 32, [5, 6, 7])
        });
        let (same, shadow) = through(shadow, |f| f.push_fill(0, 0, 32, 32, [5, 6, 7]));
        assert!(same.is_empty());
        let (other, _) = through(shadow, |f| f.push_fill(0, 0, 32, 32, [5, 6, 8]));
        assert_eq!(other.len(), 1);
    }

    #[test]
    fn a_jpeg_makes_its_area_unknown_so_the_next_pixels_are_sent() {
        // We never decode a JPEG, so the canvas there no longer matches the
        // shadow. Skipping a "same as before" tile would leave the JPEG's
        // pixels on screen.
        let tile = solid(64, 64, [1, 2, 3]);
        let (_, shadow) = through(Shadow::new(64, 64), |f| f.push_raw(0, 0, 64, 64, &tile));
        let (_, shadow) = through(shadow, |f| f.push_jpeg(16, 16, 8, 8, &[0xFF, 0xD8]));
        let (rects, _) = through(shadow, |f| f.push_raw(0, 0, 64, 64, &tile));
        assert_eq!(rects.len(), 1);
    }

    #[test]
    fn invalidating_sends_everything_again() {
        // What the Refresh button relies on.
        let tile = solid(64, 64, [1, 2, 3]);
        let (_, mut shadow) = through(Shadow::new(64, 64), |f| f.push_raw(0, 0, 64, 64, &tile));
        shadow.invalidate();
        let (rects, _) = through(shadow, |f| f.push_raw(0, 0, 64, 64, &tile));
        assert_eq!(rects.len(), 1);
    }

    #[test]
    fn a_copy_rect_moves_pixels_in_the_shadow_too() {
        let red = solid(64, 64, [255, 0, 0]);
        let (_, shadow) = through(Shadow::new(128, 64), |f| {
            f.push_raw(0, 0, 64, 64, &red);
            f.push_raw(64, 0, 64, 64, &solid(64, 64, [0, 0, 255]));
        });
        let (rects, shadow) = through(shadow, |f| f.push_copy_rect(64, 0, 64, 64, 0, 0));
        assert_eq!(rects, vec![(64, 0, 64, 64, op::COPY_RECT)], "always sent");
        let (rects, _) = through(shadow, |f| f.push_raw(64, 0, 64, 64, &red));
        assert!(
            rects.is_empty(),
            "the destination now holds the copied pixels"
        );
    }

    #[test]
    fn an_overlapping_copy_rect_reads_the_source_before_writing() {
        // Scrolling by one row: source and destination overlap.
        let mut rows = Vec::new();
        for row in 0..4u8 {
            rows.extend([row, 0, 0, 0xFF].repeat(4));
        }
        let (_, shadow) = through(Shadow::new(4, 4), |f| f.push_raw(0, 0, 4, 4, &rows));
        let (_, shadow) = through(shadow, |f| f.push_copy_rect(0, 0, 4, 3, 0, 1));
        let mut expected = Vec::new();
        for row in [1u8, 2, 3, 3] {
            expected.extend([row, 0, 0, 0xFF].repeat(4));
        }
        let (rects, _) = through(shadow, |f| f.push_raw(0, 0, 4, 4, &expected));
        assert!(rects.is_empty(), "shadow matches a correct scroll");
    }

    #[test]
    fn a_rectangle_off_the_screen_passes_through_untouched() {
        // A confused server must not panic the shadow or be silently dropped.
        let tile = solid(64, 64, [1, 1, 1]);
        let (rects, _) = through(Shadow::new(32, 32), |f| f.push_raw(0, 0, 64, 64, &tile));
        assert_eq!(rects, vec![(0, 0, 64, 64, op::RAW_RGBA)]);
    }

    #[test]
    fn a_desktop_resize_starts_the_shadow_over() {
        let tile = solid(64, 64, [1, 2, 3]);
        let (_, shadow) = through(Shadow::new(64, 64), |f| f.push_raw(0, 0, 64, 64, &tile));
        let (_, shadow) = through(shadow, |f| f.push_desktop_size(128, 128));
        let (rects, _) = through(shadow, |f| f.push_raw(0, 0, 64, 64, &tile));
        assert_eq!(rects.len(), 1, "the resized canvas is blank");
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
