//! The thread that turns frames into textures.
//!
//! The session thread in the core hands each finished frame to a
//! [`ByteSink`]; here that sink is a channel into one thread per tab, which
//! parses the frame ([`super::frame`]), decodes its JPEG rectangles (the core
//! passes them through undecoded, as the browser decoded them for the Vue
//! panel), applies every rectangle in wire order to the [`Framebuffer`], and
//! builds a GPUI image for each tile the frame touched. The UI thread only
//! swaps those images in and acknowledges the frame, so neither decoding nor
//! pixel copying ever runs on it.
//!
//! Where the bytes go, per frame: the core's message (one allocation, made
//! by the core) -> the framebuffer, written in place (raw pixels swizzled to
//! BGRA, fills and copy-rects without any allocation, a JPEG decoded into a
//! scratch RGB buffer and then written in) -> one new buffer per changed
//! tile, which is what GPUI uploads to its texture atlas. Nothing is
//! allocated at the size of the screen except on a resize.

use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui_kit::RenderImage;
use termdrop_core::ByteSink;

use super::frame::{Op, parse_frame};
use super::framebuffer::{Framebuffer, Rgb};

/// One applied frame, for the UI thread.
pub struct Update {
    pub seq: u32,
    /// The framebuffer's size after this frame.
    pub size: (u16, u16),
    /// Tiles across and down.
    pub grid: (usize, usize),
    /// The grid was rebuilt (the first frame, or a desktop resize): every
    /// tile the UI holds is stale and `tiles` covers the whole screen.
    pub reset: bool,
    /// The changed tiles' new images, by index in the grid.
    pub tiles: Vec<(usize, Arc<RenderImage>)>,
    /// The message's size, for the stats line.
    pub bytes: usize,
    /// Parse, decode and apply, off the UI thread.
    pub apply: Duration,
}

pub enum Message {
    Frame(Update),
    /// The frame could not be parsed: the stream is out of step and the
    /// session must end (`Corrupt frame from the server: ...`).
    Corrupt(String),
}

/// Start the thread for a `width` x `height` desktop. Returns the sink to
/// attach with `vnc_open_frame_channel` and the stream of applied frames.
/// The thread ends when the session drops the sink or the UI drops the
/// stream.
pub fn start(width: u16, height: u16) -> (Box<dyn ByteSink>, async_channel::Receiver<Message>) {
    let (frames_tx, frames_rx) = std::sync::mpsc::channel::<Vec<u8>>();
    let (out_tx, out_rx) = async_channel::unbounded();
    let spawned = std::thread::Builder::new()
        .name("vnc-render".into())
        .spawn(move || run(width, height, frames_rx, out_tx));
    if let Err(e) = spawned {
        tracing::error!("could not start the VNC render thread: {e}");
    }
    (Box::new(frames_tx), out_rx)
}

fn run(
    width: u16,
    height: u16,
    frames: std::sync::mpsc::Receiver<Vec<u8>>,
    out: async_channel::Sender<Message>,
) {
    let mut fb = Framebuffer::new(width, height);
    let mut first = true;
    let mut stats = Stats::default();
    while let Ok(bytes) = frames.recv() {
        let started = Instant::now();
        let message = match apply(&mut fb, &bytes) {
            Ok(Applied {
                seq,
                resized,
                jpegs,
            }) => {
                let reset = std::mem::take(&mut first) || resized;
                let dirty = fb.take_dirty();
                let tiles: Vec<_> = dirty.iter().map(|&i| (i, tile_image(&fb, i))).collect();
                let apply = started.elapsed();
                stats.record(apply, bytes.len(), jpegs, &tiles, &fb);
                Message::Frame(Update {
                    seq,
                    size: (fb.width() as u16, fb.height() as u16),
                    grid: fb.grid(),
                    reset,
                    tiles,
                    bytes: bytes.len(),
                    apply,
                })
            }
            Err(e) => Message::Corrupt(e),
        };
        let corrupt = matches!(message, Message::Corrupt(_));
        if out.send_blocking(message).is_err() || corrupt {
            return;
        }
    }
}

/// What applying a frame did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Applied {
    pub seq: u32,
    /// The desktop was resized (its last rectangle).
    pub resized: bool,
    /// JPEG rectangles it carried, for the debug log.
    pub jpegs: usize,
}

/// Apply one message to `fb`.
pub fn apply(fb: &mut Framebuffer, bytes: &[u8]) -> Result<Applied, String> {
    let frame = parse_frame(bytes)?;
    // Every JPEG first, so the frame is then applied in one pass in wire
    // order; a later CopyRect may read what a JPEG wrote.
    let jpegs = decode_jpegs(&frame.rects);
    let mut resized = false;
    for (index, rect) in frame.rects.iter().enumerate() {
        match rect.op {
            Op::RawRgba => fb.put_rgba(rect.x, rect.y, rect.width, rect.height, rect.payload),
            Op::Fill(rgba) => fb.fill(rect.x, rect.y, rect.width, rect.height, rgba),
            Op::CopyRect { src_x, src_y } => {
                fb.copy_rect(src_x, src_y, rect.x, rect.y, rect.width, rect.height)
            }
            Op::Jpeg => {
                // A JPEG that will not decode is skipped, as the Vue
                // renderer's failed `createImageBitmap` was. The core marked
                // its area unknown in its shadow, so the next update that
                // touches it sends the pixels again.
                if let Some((w, h, rgb)) = jpegs
                    .iter()
                    .find(|j| j.0 == index)
                    .and_then(|j| j.1.as_ref())
                {
                    fb.put_rgb(
                        rect.x,
                        rect.y,
                        &Rgb {
                            width: *w,
                            height: *h,
                            pixels: rgb,
                        },
                    );
                }
            }
            Op::DesktopSize => {
                // Always the last rectangle: the new screen starts black.
                *fb = Framebuffer::new(rect.width, rect.height);
                resized = true;
            }
            // The Vue panel made the remote cursor a CSS cursor; GPUI can
            // only choose among the system's, so the local arrow stays.
            Op::Cursor | Op::Unknown(_) => {}
        }
    }
    Ok(Applied {
        seq: frame.seq,
        resized,
        jpegs: jpegs.len(),
    })
}

type Decoded = (usize, Option<(usize, usize, Vec<u8>)>);

/// Decode a frame's JPEG rectangles, in parallel when there are several
/// (the Vue panel started every `createImageBitmap` at once).
fn decode_jpegs(rects: &[super::frame::Rect<'_>]) -> Vec<Decoded> {
    let jobs: Vec<(usize, &[u8])> = rects
        .iter()
        .enumerate()
        .filter(|(_, r)| r.op == Op::Jpeg)
        .map(|(i, r)| (i, r.payload))
        .collect();
    if jobs.len() <= 1 {
        return jobs.into_iter().map(|(i, p)| (i, decode_jpeg(p))).collect();
    }
    std::thread::scope(|scope| {
        let handles: Vec<_> = jobs
            .into_iter()
            .map(|(i, p)| (i, scope.spawn(move || decode_jpeg(p))))
            .collect();
        handles
            .into_iter()
            .map(|(i, h)| (i, h.join().ok().flatten()))
            .collect()
    })
}

fn decode_jpeg(bytes: &[u8]) -> Option<(usize, usize, Vec<u8>)> {
    let image = image::load_from_memory_with_format(bytes, image::ImageFormat::Jpeg)
        .map_err(|e| tracing::debug!(target: "termdrop::vnc", "JPEG rectangle not decoded: {e}"))
        .ok()?
        .into_rgb8();
    let (w, h) = (image.width() as usize, image.height() as usize);
    Some((w, h, image.into_raw()))
}

/// Tile `index` as a GPUI image (BGRA, which `RenderImage` expects).
fn tile_image(fb: &Framebuffer, index: usize) -> Arc<RenderImage> {
    let (_, _, w, h) = fb.tile_rect(index);
    let buffer = image::RgbaImage::from_raw(w as u32, h as u32, fb.tile_bgra(index))
        .expect("a tile's buffer is its width times its height");
    Arc::new(RenderImage::new(vec![image::Frame::new(buffer)]))
}

/// What the renderer did in the last second, logged under
/// `RUST_LOG=termdrop::vnc=debug` beside the core's own frame timing.
#[derive(Default)]
struct Stats {
    frames: u32,
    apply: Duration,
    bytes: usize,
    jpegs: usize,
    tiles: usize,
    tile_bytes: usize,
    since: Option<Instant>,
}

impl Stats {
    fn record(
        &mut self,
        apply: Duration,
        bytes: usize,
        jpegs: usize,
        tiles: &[(usize, Arc<RenderImage>)],
        fb: &Framebuffer,
    ) {
        let since = *self.since.get_or_insert_with(Instant::now);
        self.frames += 1;
        self.apply += apply;
        self.bytes += bytes;
        self.jpegs += jpegs;
        self.tiles += tiles.len();
        self.tile_bytes += tiles
            .iter()
            .map(|(i, _)| {
                let (_, _, w, h) = fb.tile_rect(*i);
                w * h * 4
            })
            .sum::<usize>();
        if since.elapsed() >= Duration::from_secs(1) {
            let n = self.frames.max(1);
            tracing::debug!(
                target: "termdrop::vnc",
                frames = self.frames,
                apply_us = self.apply.as_micros() as u64 / n as u64,
                kb_per_frame = self.bytes / n as usize / 1024,
                jpegs = self.jpegs,
                tiles_per_frame = self.tiles / n as usize,
                upload_kb_per_frame = self.tile_bytes / n as usize / 1024,
                "VNC native render"
            );
            *self = Self::default();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::framebuffer::TILE;
    use super::*;
    use termdrop_core::vnc::frame::FrameBuilder;

    /// A size `n` tiles and a bit across.
    const fn span(n: usize) -> u16 {
        (n * TILE + 44) as u16
    }

    #[test]
    fn a_frame_is_applied_in_wire_order() {
        let mut fb = Framebuffer::new(span(1), 4);
        fb.take_dirty();
        let mut b = FrameBuilder::new(5);
        b.push_fill(0, 0, 2, 2, [255, 0, 0]);
        // Copies the fill that came before it.
        b.push_copy_rect(TILE as u16 + 4, 0, 2, 2, 0, 0);
        let applied = apply(&mut fb, &b.finish()).unwrap();
        assert_eq!(
            applied,
            Applied {
                seq: 5,
                resized: false,
                jpegs: 0
            }
        );
        assert_eq!(fb.pixel(TILE + 5, 1), [0, 0, 255, 255]);
        assert_eq!(fb.take_dirty(), vec![0, 1]);
    }

    #[test]
    fn a_resize_starts_a_new_black_screen() {
        let mut fb = Framebuffer::new(10, 10);
        let mut b = FrameBuilder::new(1);
        b.push_fill(0, 0, 10, 10, [9, 9, 9]);
        b.push_desktop_size(20, 5);
        assert!(apply(&mut fb, &b.finish()).unwrap().resized);
        assert_eq!((fb.width(), fb.height()), (20, 5));
        assert_eq!(fb.pixel(0, 0), [0, 0, 0, 255]);
    }

    #[test]
    fn a_jpeg_is_decoded_into_place_and_a_bad_one_is_skipped() {
        let mut jpeg = Vec::new();
        let img = image::RgbImage::from_pixel(4, 4, image::Rgb([200, 100, 50]));
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, 100)
            .encode_image(&img)
            .unwrap();
        let mut fb = Framebuffer::new(8, 8);
        let mut b = FrameBuilder::new(0);
        b.push_jpeg(2, 2, 4, 4, &jpeg);
        b.push_jpeg(0, 0, 2, 2, &[0xff, 0xd8, 0x00]);
        assert_eq!(apply(&mut fb, &b.finish()).unwrap().jpegs, 2);
        let [blue, green, red, alpha] = fb.pixel(3, 3);
        // JPEG is lossy; near enough is the decoder working.
        assert!(red.abs_diff(200) < 8 && green.abs_diff(100) < 8 && blue.abs_diff(50) < 8);
        assert_eq!(alpha, 255);
        assert_eq!(fb.pixel(0, 0), [0, 0, 0, 255], "the bad one drew nothing");
    }

    #[test]
    fn several_jpegs_decode_in_parallel_and_land_in_order() {
        let encode = |rgb: [u8; 3]| {
            let mut out = Vec::new();
            let img = image::RgbImage::from_pixel(2, 2, image::Rgb(rgb));
            image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 100)
                .encode_image(&img)
                .unwrap();
            out
        };
        let mut fb = Framebuffer::new(4, 2);
        let mut b = FrameBuilder::new(0);
        b.push_jpeg(0, 0, 2, 2, &encode([255, 255, 255]));
        b.push_jpeg(2, 0, 2, 2, &encode([0, 0, 0]));
        apply(&mut fb, &b.finish()).unwrap();
        assert!(fb.pixel(0, 0)[0] > 240);
        assert!(fb.pixel(3, 0)[0] < 15);
    }

    #[test]
    fn a_corrupt_frame_is_an_error() {
        let mut fb = Framebuffer::new(1, 1);
        assert_eq!(
            apply(&mut fb, &[1, 2, 3]).unwrap_err(),
            "frame too short: 3 bytes"
        );
    }

    #[test]
    fn the_thread_reports_the_first_frame_as_a_reset_then_only_changes() {
        let (mut sink, rx) = start(span(1), span(1));
        let mut b = FrameBuilder::new(1);
        b.push_fill(0, 0, 1, 1, [1, 2, 3]);
        assert!(sink.send(b.finish()));
        let Message::Frame(first) = rx.recv_blocking().unwrap() else {
            panic!("a good frame");
        };
        assert!(first.reset);
        assert_eq!(first.tiles.len(), 4, "the whole 2x2 grid");
        let mut b = FrameBuilder::new(2);
        b.push_fill(span(1) - 1, span(1) - 1, 1, 1, [1, 2, 3]);
        assert!(sink.send(b.finish()));
        let Message::Frame(second) = rx.recv_blocking().unwrap() else {
            panic!("a good frame");
        };
        assert!(!second.reset);
        assert_eq!(second.seq, 2);
        assert_eq!(
            second.tiles.iter().map(|t| t.0).collect::<Vec<_>>(),
            vec![3]
        );
        assert!(sink.send(vec![0; 4]));
        assert!(matches!(rx.recv_blocking().unwrap(), Message::Corrupt(_)));
    }
}
