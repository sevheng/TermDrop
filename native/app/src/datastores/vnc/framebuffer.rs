//! The remote screen as the native renderer holds it: one BGRA buffer the
//! size of the remote desktop, updated in place, and a grid of tiles that
//! records which parts changed.
//!
//! This is what the Vue panel's canvas backing store was. It is BGRA rather
//! than RGBA because that is what a GPUI `RenderImage` uploads, so a tile
//! goes to the GPU as a row copy with no conversion; the swizzle happens
//! once, here, when a raw rectangle is written.
//!
//! GPUI cannot update part of a texture, so the screen is drawn as a grid of
//! [`TILE`]-pixel images and only the tiles a frame touched are rebuilt. On a
//! typing or pointer update that is one or two 64 KB tiles instead of the
//! whole desktop (8 MB at 1920x1080). 256-pixel tiles were tried first: a
//! desktop settling after login sent ~260 small updates a second, each
//! rebuilding about three tiles, 750 KB of copying per 2 KB update.

/// Side of a tile, in remote pixels.
pub const TILE: usize = 128;

/// A decoded JPEG, as RGB rows.
pub struct Rgb<'a> {
    pub width: usize,
    pub height: usize,
    pub pixels: &'a [u8],
}

pub struct Framebuffer {
    width: usize,
    height: usize,
    bgra: Vec<u8>,
    cols: usize,
    rows: usize,
    dirty: Vec<bool>,
}

impl Framebuffer {
    /// Black and opaque, as `prepareCanvas` filled the canvas, with every
    /// tile dirty.
    pub fn new(width: u16, height: u16) -> Self {
        let (width, height) = (width as usize, height as usize);
        let mut bgra = vec![0u8; width * height * 4];
        for px in bgra.as_chunks_mut::<4>().0 {
            px[3] = 0xff;
        }
        let cols = width.div_ceil(TILE);
        let rows = height.div_ceil(TILE);
        Self {
            width,
            height,
            bgra,
            cols,
            rows,
            dirty: vec![true; cols * rows],
        }
    }

    pub fn width(&self) -> usize {
        self.width
    }

    pub fn height(&self) -> usize {
        self.height
    }

    /// Tiles across and down.
    pub fn grid(&self) -> (usize, usize) {
        (self.cols, self.rows)
    }

    #[cfg(test)]
    pub fn pixel(&self, x: usize, y: usize) -> [u8; 4] {
        let at = (y * self.width + x) * 4;
        [
            self.bgra[at],
            self.bgra[at + 1],
            self.bgra[at + 2],
            self.bgra[at + 3],
        ]
    }

    /// Clip a rectangle to the screen; `None` when nothing is left.
    fn clip(&self, x: usize, y: usize, w: usize, h: usize) -> Option<(usize, usize, usize, usize)> {
        if x >= self.width || y >= self.height {
            return None;
        }
        let w = w.min(self.width - x);
        let h = h.min(self.height - y);
        (w > 0 && h > 0).then_some((x, y, w, h))
    }

    fn mark(&mut self, x: usize, y: usize, w: usize, h: usize) {
        for row in y / TILE..=(y + h - 1) / TILE {
            for col in x / TILE..=(x + w - 1) / TILE {
                self.dirty[row * self.cols + col] = true;
            }
        }
    }

    /// `putImageData`: RGBA rows, clipped to the screen.
    pub fn put_rgba(&mut self, x: u16, y: u16, w: u16, h: u16, rgba: &[u8]) {
        let (src_w, src_h) = (w as usize, h as usize);
        if rgba.len() < src_w * src_h * 4 {
            return;
        }
        let Some((x, y, w, h)) = self.clip(x as usize, y as usize, src_w, src_h) else {
            return;
        };
        for row in 0..h {
            let src = &rgba[row * src_w * 4..][..w * 4];
            let at = ((y + row) * self.width + x) * 4;
            let dst = &mut self.bgra[at..at + w * 4];
            for (d, s) in dst
                .as_chunks_mut::<4>()
                .0
                .iter_mut()
                .zip(src.as_chunks::<4>().0)
            {
                d[0] = s[2];
                d[1] = s[1];
                d[2] = s[0];
                d[3] = s[3];
            }
        }
        self.mark(x, y, w, h);
    }

    /// `fillRect` with the rectangle's colour (RGBA; alpha is opaque).
    pub fn fill(&mut self, x: u16, y: u16, w: u16, h: u16, rgba: [u8; 4]) {
        let Some((x, y, w, h)) = self.clip(x as usize, y as usize, w as usize, h as usize) else {
            return;
        };
        let colour = [rgba[2], rgba[1], rgba[0], 0xff];
        for row in y..y + h {
            let at = (row * self.width + x) * 4;
            for px in self.bgra[at..at + w * 4].as_chunks_mut::<4>().0 {
                px.copy_from_slice(&colour);
            }
        }
        self.mark(x, y, w, h);
    }

    /// The canvas self-blit: copy `w` x `h` from `(sx, sy)` to `(x, y)`.
    /// Source and destination may overlap (dragging a window does exactly
    /// that), so rows are walked in the direction that never reads a row
    /// already overwritten, and each row is a `copy_within` (a memmove).
    pub fn copy_rect(&mut self, sx: u16, sy: u16, x: u16, y: u16, w: u16, h: u16) {
        let (sx, sy, x, y) = (sx as usize, sy as usize, x as usize, y as usize);
        let (w, h) = (w as usize, h as usize);
        // Clip both ends together, so the copy stays the same shape.
        if sx >= self.width || sy >= self.height || x >= self.width || y >= self.height {
            return;
        }
        let w = w.min(self.width - sx).min(self.width - x);
        let h = h.min(self.height - sy).min(self.height - y);
        if w == 0 || h == 0 {
            return;
        }
        let stride = self.width * 4;
        let mut copy_row = |row: usize| {
            let from = (sy + row) * stride + sx * 4;
            let to = (y + row) * stride + x * 4;
            self.bgra.copy_within(from..from + w * 4, to);
        };
        if y > sy {
            (0..h).rev().for_each(&mut copy_row);
        } else {
            (0..h).for_each(&mut copy_row);
        }
        self.mark(x, y, w, h);
    }

    /// `drawImage(bitmap, x, y)`: a decoded JPEG at its own size, clipped to
    /// the screen.
    pub fn put_rgb(&mut self, x: u16, y: u16, image: &Rgb<'_>) {
        if image.pixels.len() < image.width * image.height * 3 {
            return;
        }
        let Some((x, y, w, h)) = self.clip(x as usize, y as usize, image.width, image.height)
        else {
            return;
        };
        for row in 0..h {
            let src = &image.pixels[row * image.width * 3..][..w * 3];
            let at = ((y + row) * self.width + x) * 4;
            for (d, s) in self.bgra[at..at + w * 4]
                .as_chunks_mut::<4>()
                .0
                .iter_mut()
                .zip(src.as_chunks::<3>().0)
            {
                d[0] = s[2];
                d[1] = s[1];
                d[2] = s[0];
                d[3] = 0xff;
            }
        }
        self.mark(x, y, w, h);
    }

    /// The tiles changed since the last call, in row-major order; clears
    /// the marks.
    pub fn take_dirty(&mut self) -> Vec<usize> {
        let dirty = self
            .dirty
            .iter()
            .enumerate()
            .filter_map(|(i, d)| d.then_some(i))
            .collect();
        self.dirty.fill(false);
        dirty
    }

    /// Tile `index`'s place on the screen: `(x, y, width, height)`.
    pub fn tile_rect(&self, index: usize) -> (usize, usize, usize, usize) {
        tile_rect(self.width, self.height, self.cols, index)
    }

    /// A copy of tile `index`'s pixels, BGRA rows. The one copy a changed
    /// tile costs on its way to the GPU.
    pub fn tile_bgra(&self, index: usize) -> Vec<u8> {
        let (x, y, w, h) = self.tile_rect(index);
        let mut out = Vec::with_capacity(w * h * 4);
        for row in y..y + h {
            let at = (row * self.width + x) * 4;
            out.extend_from_slice(&self.bgra[at..at + w * 4]);
        }
        out
    }
}

/// Tile `index`'s rectangle on a `width` x `height` screen `cols` tiles
/// across. Edge tiles are cut to the screen.
pub fn tile_rect(
    width: usize,
    height: usize,
    cols: usize,
    index: usize,
) -> (usize, usize, usize, usize) {
    let (col, row) = (index % cols.max(1), index / cols.max(1));
    let (x, y) = (col * TILE, row * TILE);
    (
        x,
        y,
        TILE.min(width.saturating_sub(x)),
        TILE.min(height.saturating_sub(y)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const BLACK: [u8; 4] = [0, 0, 0, 0xff];
    const T: u16 = TILE as u16;
    /// Three tiles across and two down, the last of each cut short.
    const W: u16 = 2 * T + 88;
    const H: u16 = T + 44;

    #[test]
    fn starts_black_and_wholly_dirty() {
        let mut fb = Framebuffer::new(W, H);
        assert_eq!(fb.grid(), (3, 2));
        assert_eq!(fb.pixel(W as usize - 1, H as usize - 1), BLACK);
        assert_eq!(fb.take_dirty(), vec![0, 1, 2, 3, 4, 5]);
        assert!(fb.take_dirty().is_empty(), "the marks are cleared");
    }

    #[test]
    fn raw_pixels_are_swizzled_to_bgra_and_mark_only_their_tiles() {
        let mut fb = Framebuffer::new(W, H);
        fb.take_dirty();
        fb.put_rgba(T + 44, 10, 1, 1, &[1, 2, 3, 4]);
        assert_eq!(fb.pixel(TILE + 44, 10), [3, 2, 1, 4]);
        assert_eq!(fb.take_dirty(), vec![1]);
    }

    #[test]
    fn a_rectangle_across_a_tile_edge_marks_both() {
        let mut fb = Framebuffer::new(W, H);
        fb.take_dirty();
        fb.fill(T - 6, T - 6, 10, 10, [9, 8, 7, 255]);
        assert_eq!(fb.take_dirty(), vec![0, 1, 3, 4]);
        assert_eq!(fb.pixel(TILE - 1, TILE - 1), [7, 8, 9, 255]);
    }

    #[test]
    fn writes_are_clipped_to_the_screen() {
        // A server rectangle hanging off the edge must not panic or wrap.
        let mut fb = Framebuffer::new(10, 10);
        fb.take_dirty();
        fb.fill(8, 8, 50, 50, [1, 1, 1, 255]);
        fb.put_rgba(9, 0, 2, 1, &[5, 5, 5, 255, 6, 6, 6, 255]);
        fb.fill(20, 20, 5, 5, [2, 2, 2, 255]);
        assert_eq!(fb.pixel(9, 9), [1, 1, 1, 255]);
        assert_eq!(fb.pixel(9, 0), [5, 5, 5, 255]);
        assert_eq!(fb.pixel(7, 7), BLACK);
    }

    #[test]
    fn a_short_raw_payload_is_ignored() {
        let mut fb = Framebuffer::new(4, 4);
        fb.take_dirty();
        fb.put_rgba(0, 0, 2, 2, &[1, 2, 3]);
        assert!(fb.take_dirty().is_empty());
    }

    #[test]
    fn copy_rect_handles_overlap_in_both_directions() {
        // Dragging a window down or right overlaps source and destination.
        let mut fb = Framebuffer::new(4, 4);
        for y in 0..4u16 {
            for x in 0..4u16 {
                fb.put_rgba(x, y, 1, 1, &[(y * 4 + x) as u8, 0, 0, 255]);
            }
        }
        let red = |fb: &Framebuffer, x, y| fb.pixel(x, y)[2];
        // Down by one: rows 0..3 to 1..4.
        fb.copy_rect(0, 0, 0, 1, 4, 3);
        assert_eq!([red(&fb, 0, 1), red(&fb, 0, 2), red(&fb, 0, 3)], [0, 4, 8]);
        // Left by one, the other direction.
        fb.copy_rect(1, 0, 0, 0, 3, 1);
        assert_eq!([red(&fb, 0, 0), red(&fb, 1, 0), red(&fb, 2, 0)], [1, 2, 3]);
    }

    #[test]
    fn a_jpeg_lands_at_its_own_size() {
        let mut fb = Framebuffer::new(4, 4);
        let pixels = [10, 20, 30, 40, 50, 60];
        fb.put_rgb(
            3,
            1,
            &Rgb {
                width: 2,
                height: 1,
                pixels: &pixels,
            },
        );
        assert_eq!(fb.pixel(3, 1), [30, 20, 10, 255]);
        // The second pixel was off the screen.
        assert_eq!(fb.pixel(2, 1), BLACK);
    }

    #[test]
    fn a_tile_copy_is_its_rows() {
        let mut fb = Framebuffer::new(T + 44, 2);
        fb.fill(T, 0, 44, 2, [1, 2, 3, 255]);
        assert_eq!(fb.tile_rect(1), (TILE, 0, 44, 2));
        let tile = fb.tile_bgra(1);
        assert_eq!(tile.len(), 44 * 2 * 4);
        assert!(tile.as_chunks::<4>().0.iter().all(|p| *p == [3, 2, 1, 255]));
    }

    #[test]
    fn edge_tiles_are_cut_to_the_screen() {
        let (w, h) = (W as usize, H as usize);
        assert_eq!(tile_rect(w, h, 3, 5), (2 * TILE, TILE, 88, 44));
        assert_eq!(tile_rect(w, h, 3, 0), (0, 0, TILE, TILE));
    }
}
