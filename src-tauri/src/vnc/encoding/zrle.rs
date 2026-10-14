//! ZRLE: zlib-compressed tiles of run-length-encoded palette indices.
//!
//! **The zlib stream spans the whole connection, not the rectangle.** One
//! [`flate2::Decompress`] is created when the session starts and fed every ZRLE
//! rectangle in order. Creating a fresh one per rectangle appears to work on
//! the first frame and then produces garbage, because the compressor on the
//! other end keeps its dictionary across rectangles. That state is the single
//! most important thing about this module.
//!
//! Inside the decompressed stream the rectangle is 64x64 tiles, left to right
//! and top to bottom, each with its own one-byte subencoding.

use flate2::{Decompress, FlushDecompress};

use super::super::frame::FrameBuilder;
use super::super::pixel::PixelFormat;
use super::super::proto::RectHeader;
use super::Reader;

/// ZRLE tiles are always 64x64, except where they are clipped by the edge.
const TILE: u16 = 64;

/// Resize `buf` to hold `pixels` RGBA pixels and hand back a pixel-wise view.
///
/// Returning `&mut [[u8; 4]]` is what lets the decoders write a whole pixel as
/// one store, with the bounds check hoisted out of the loop.
fn sized(buf: &mut Vec<u8>, pixels: usize) -> &mut [[u8; 4]] {
    buf.clear();
    // 0xFF rather than 0 so the alpha lane is already opaque and the decoders
    // never have to write it.
    buf.resize(pixels * 4, 0xFF);
    buf.as_chunks_mut::<4>().0
}

/// A ZRLE stream, alive for the lifetime of the connection.
pub struct ZrleStream {
    inflate: Decompress,
    /// Reused across rectangles so a 1080p update does not reallocate 8MB.
    plain: Vec<u8>,
    tile: TileBuf,
}

/// Scratch for one tile, reused across every tile of every rectangle.
///
/// A 2560x1440 screen is 900 tiles, and these were two fresh allocations each
/// -- roughly 1800 per full frame, on the thread that also has to service
/// input. They are cleared rather than reallocated, so after the first frame
/// this costs nothing.
#[derive(Default)]
struct TileBuf {
    rgba: Vec<u8>,
    palette: Vec<[u8; 3]>,
}

impl Default for ZrleStream {
    fn default() -> Self {
        Self::new()
    }
}

impl ZrleStream {
    pub fn new() -> Self {
        Self {
            // `true`: zlib wrapper, not a raw deflate stream.
            inflate: Decompress::new(true),
            plain: Vec::new(),
            tile: TileBuf::default(),
        }
    }

    /// Inflate one rectangle's compressed payload.
    ///
    /// `FlushDecompress::Sync` rather than `Finish`: the stream does not end at
    /// the rectangle boundary, and telling zlib it does would discard the
    /// dictionary the next rectangle depends on.
    fn inflate(&mut self, compressed: &[u8], expected: usize) -> Result<(), String> {
        self.plain.clear();
        self.plain.reserve(expected);
        let mut consumed = 0;

        while consumed < compressed.len() {
            let before_in = self.inflate.total_in();
            let before_out = self.inflate.total_out();
            self.inflate
                .decompress_vec(
                    &compressed[consumed..],
                    &mut self.plain,
                    FlushDecompress::Sync,
                )
                .map_err(|e| format!("ZRLE zlib error: {}", e))?;
            let read = (self.inflate.total_in() - before_in) as usize;
            let wrote = self.inflate.total_out() - before_out;
            consumed += read;

            if read == 0 && wrote == 0 {
                // No progress with input still pending means the output buffer
                // is the constraint; give it room and go round again.
                self.plain.reserve(self.plain.len().max(8192));
                if self.plain.capacity() == self.plain.len() {
                    return Err("ZRLE stream made no progress".to_string());
                }
            }
        }
        Ok(())
    }
}

/// Decode one ZRLE rectangle.
pub fn decode(
    stream: &mut ZrleStream,
    compressed: &[u8],
    header: &RectHeader,
    format: &PixelFormat,
    frame: &mut FrameBuilder,
) -> Result<(), String> {
    let pixels = (header.width as usize) * (header.height as usize);
    stream.inflate(compressed, pixels * 4)?;

    // Taken out so the tile loop can borrow the stream's buffer immutably
    // while writing into its own output.
    // `plain` is taken out so the tile buffers can be borrowed mutably
    // alongside it; disjoint fields would not be enough through the `&mut`.
    let plain = std::mem::take(&mut stream.plain);
    let result = decode_tiles(&plain, &mut stream.tile, header, format, frame);
    stream.plain = plain;
    result
}

fn decode_tiles(
    plain: &[u8],
    tile: &mut TileBuf,
    header: &RectHeader,
    format: &PixelFormat,
    frame: &mut FrameBuilder,
) -> Result<(), String> {
    let mut reader = Reader::new(plain);
    let cpixel = format.tpixel_bytes();

    let mut tile_y = 0u16;
    while tile_y < header.height {
        let h = TILE.min(header.height - tile_y);
        let mut tile_x = 0u16;
        while tile_x < header.width {
            let w = TILE.min(header.width - tile_x);
            let x = header.x + tile_x;
            let y = header.y + tile_y;

            let subencoding = reader.u8()?;
            let palette_size = (subencoding & 0x7F) as usize;
            let run_length_encoded = subencoding & 0x80 != 0;

            match (run_length_encoded, palette_size) {
                // Raw pixels, one after another.
                (false, 0) => {
                    // One bounds-checked take for the whole tile, then 32-bit
                    // stores into a pre-sized slice. Per-pixel `take` plus
                    // `extend_from_slice` cost a bounds check, a capacity check
                    // and a 4-byte memcpy each, and a 2560x1440 update is 3.7
                    // million pixels of it.
                    let count = w as usize * h as usize;
                    let src = reader.take(count * cpixel)?;
                    let out = sized(&mut tile.rgba, count);
                    for (dst, px) in out.iter_mut().zip(src.chunks_exact(cpixel)) {
                        let [r, g, b] = format.tpixel_to_rgb(px);
                        *dst = [r, g, b, 0xFF];
                    }
                    frame.push_raw(x, y, w, h, &tile.rgba);
                }
                // A single colour over the whole tile: four bytes on the wire
                // instead of up to sixteen kilobytes.
                (false, 1) => {
                    let rgb = format.tpixel_to_rgb(reader.take(cpixel)?);
                    frame.push_fill(x, y, w, h, rgb);
                }
                // Packed palette indices, 1, 2 or 4 bits each.
                (false, size) => {
                    read_palette(&mut reader, size, cpixel, format, &mut tile.palette)?;
                    let bits = packed_bits(size);
                    unpack_indices(&mut reader, w, h, bits, &tile.palette, &mut tile.rgba)?;
                    frame.push_raw(x, y, w, h, &tile.rgba);
                }
                // Plain RLE: runs of (pixel, length).
                (true, 0) => {
                    plain_rle(&mut reader, w, h, cpixel, format, &mut tile.rgba)?;
                    frame.push_raw(x, y, w, h, &tile.rgba);
                }
                (true, 1) => return Err("ZRLE subencoding 129 is not defined".to_string()),
                // Palette RLE: runs of (index, length).
                (true, size) => {
                    // `size` is already `subencoding - 128`, which is the
                    // colour count; subtracting again drops the last colour
                    // and puts every index out by one.
                    read_palette(&mut reader, size, cpixel, format, &mut tile.palette)?;
                    palette_rle(&mut reader, w, h, &tile.palette, &mut tile.rgba)?;
                    frame.push_raw(x, y, w, h, &tile.rgba);
                }
            }

            tile_x += w;
        }
        tile_y += h;
    }
    Ok(())
}

fn read_palette(
    reader: &mut Reader,
    size: usize,
    cpixel: usize,
    format: &PixelFormat,
    out: &mut Vec<[u8; 3]>,
) -> Result<(), String> {
    out.clear();
    for _ in 0..size {
        out.push(format.tpixel_to_rgb(reader.take(cpixel)?));
    }
    Ok(())
}

/// Bits per index for a packed palette of `size` colours.
fn packed_bits(size: usize) -> u32 {
    match size {
        2 => 1,
        3..=4 => 2,
        _ => 4,
    }
}

/// Unpack bit-packed palette indices. **Each row restarts on a byte boundary.**
fn unpack_indices(
    reader: &mut Reader,
    w: u16,
    h: u16,
    bits: u32,
    palette: &[[u8; 3]],
    rgba: &mut Vec<u8>,
) -> Result<(), String> {
    let per_byte = 8 / bits;
    let bytes_per_row = (w as u32).div_ceil(per_byte) as usize;
    let out = sized(rgba, w as usize * h as usize);
    let mut at = 0usize;

    for _ in 0..h {
        let row = reader.take(bytes_per_row)?;
        let mut produced = 0u16;
        'row: for byte in row {
            for slot in 0..per_byte {
                if produced == w {
                    break 'row;
                }
                let shift = 8 - bits * (slot + 1);
                let index = ((byte >> shift) & ((1 << bits) - 1) as u8) as usize;
                out[at] = indexed(palette, index)?;
                at += 1;
                produced += 1;
            }
        }
    }
    Ok(())
}

fn plain_rle(
    reader: &mut Reader,
    w: u16,
    h: u16,
    cpixel: usize,
    format: &PixelFormat,
    rgba: &mut Vec<u8>,
) -> Result<(), String> {
    let total = w as usize * h as usize;
    let out = sized(rgba, total);
    let mut at = 0usize;

    while at < total {
        let [r, g, b] = format.tpixel_to_rgb(reader.take(cpixel)?);
        let run = read_run_length(reader)?;
        if at + run > total {
            return Err("ZRLE run overran its tile".to_string());
        }
        // A run is a memset of one repeated pixel, which is the whole point of
        // the encoding -- filling it one `extend_from_slice` at a time threw
        // that away.
        out[at..at + run].fill([r, g, b, 0xFF]);
        at += run;
    }
    Ok(())
}

fn palette_rle(
    reader: &mut Reader,
    w: u16,
    h: u16,
    palette: &[[u8; 3]],
    rgba: &mut Vec<u8>,
) -> Result<(), String> {
    let total = w as usize * h as usize;
    let out = sized(rgba, total);
    let mut at = 0usize;

    while at < total {
        let byte = reader.u8()?;
        // The high bit says a run length follows; without it the index stands
        // for exactly one pixel.
        let index = (byte & 0x7F) as usize;
        let run = if byte & 0x80 != 0 {
            read_run_length(reader)?
        } else {
            1
        };
        if at + run > total {
            return Err("ZRLE run overran its tile".to_string());
        }
        out[at..at + run].fill(indexed(palette, index)?);
        at += run;
    }
    Ok(())
}

fn indexed(palette: &[[u8; 3]], index: usize) -> Result<[u8; 4], String> {
    let [r, g, b] = *palette
        .get(index)
        .ok_or_else(|| format!("ZRLE palette index {} is out of range", index))?;
    Ok([r, g, b, 0xFF])
}

/// A run length: 255 means "add 255 and keep reading".
fn read_run_length(reader: &mut Reader) -> Result<usize, String> {
    let mut length = 1usize;
    loop {
        let byte = reader.u8()?;
        length += byte as usize;
        if byte != 255 {
            return Ok(length);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fmt() -> PixelFormat {
        PixelFormat::PREFERRED
    }

    /// Run the tile decoder over already-inflated bytes.
    fn tiles(plain: &[u8], width: u16, height: u16) -> Result<Vec<u8>, String> {
        let header = RectHeader {
            x: 0,
            y: 0,
            width,
            height,
            encoding: 16,
        };
        let mut frame = FrameBuilder::new(0);
        let mut tile = TileBuf::default();
        decode_tiles(plain, &mut tile, &header, &fmt(), &mut frame)?;
        Ok(frame.finish())
    }

    #[test]
    fn a_solid_tile_becomes_a_fill_not_a_bitmap() {
        // Subencoding 1 over a 64x64 tile is 4 bytes on the wire; expanding it
        // to 16KB of identical pixels before sending it on would waste most of
        // the bandwidth this encoding exists to save.
        let frame = tiles(&[1, 10, 20, 30], 64, 64).unwrap();
        // header(16) + rect header(16) + 4 bytes of colour
        assert_eq!(frame.len(), 36);
        assert_eq!(&frame[32..36], &[10, 20, 30, 0xFF]);
    }

    #[test]
    fn a_raw_tile_expands_every_pixel() {
        // Subencoding 0, a 2x1 tile of two CPIXELs.
        let frame = tiles(&[0, 1, 2, 3, 4, 5, 6], 2, 1).unwrap();
        assert_eq!(&frame[32..40], &[1, 2, 3, 0xFF, 4, 5, 6, 0xFF]);
    }

    #[test]
    fn a_packed_palette_unpacks_the_right_number_of_bits() {
        // Two colours means one bit per pixel: 0b1010_0000 over a 4x1 tile is
        // indices 1, 0, 1, 0.
        let mut data = vec![2]; // subencoding: palette of 2
        data.extend_from_slice(&[0, 0, 0]); // colour 0: black
        data.extend_from_slice(&[255, 255, 255]); // colour 1: white
        data.push(0b1010_0000);
        let frame = tiles(&data, 4, 1).unwrap();
        assert_eq!(
            &frame[32..48],
            &[255, 255, 255, 255, 0, 0, 0, 255, 255, 255, 255, 255, 0, 0, 0, 255]
        );
    }

    #[test]
    fn each_packed_row_restarts_on_a_byte_boundary() {
        // The trap: a 4-pixel-wide 1-bit tile uses four bits of each byte, and
        // the other four are padding. Packing rows continuously instead would
        // shear the image progressively down the tile.
        let mut data = vec![2, 0, 0, 0, 255, 255, 255];
        data.push(0b1111_0000); // row 0: white, white, white, white
        data.push(0b0000_0000); // row 1: black x4
        let frame = tiles(&data, 4, 2).unwrap();
        assert_eq!(&frame[32..36], &[255, 255, 255, 255], "row 0 is white");
        assert_eq!(&frame[48..52], &[0, 0, 0, 255], "row 1 is black");
    }

    #[test]
    fn plain_rle_expands_runs() {
        // Subencoding 128: a pixel then a length. Length 0 means one pixel,
        // so 2 here means three.
        let data = vec![128, 7, 8, 9, 2];
        let frame = tiles(&data, 3, 1).unwrap();
        assert_eq!(&frame[32..44], &[7, 8, 9, 255, 7, 8, 9, 255, 7, 8, 9, 255]);
    }

    #[test]
    fn a_run_length_of_255_continues_into_the_next_byte() {
        // 255 means "add 255 and keep reading", so a long run is not capped at
        // 256 pixels -- truncating it would leave the tail of every long run
        // unpainted. One 64x4 tile is 256 pixels, and 1 + 255 + 0 is 256.
        let data = vec![128, 1, 2, 3, 255, 0];
        let frame = tiles(&data, 64, 4).unwrap();
        assert_eq!(frame.len(), 16 + 16 + 256 * 4);
        assert_eq!(&frame[32..36], &[1, 2, 3, 255]);
        assert_eq!(&frame[frame.len() - 4..], &[1, 2, 3, 255]);
    }

    #[test]
    fn palette_rle_reads_a_length_only_when_the_high_bit_is_set() {
        // Without the high bit an index stands for exactly one pixel and no
        // length byte follows -- reading one anyway would consume the next
        // index and shift everything after it.
        let mut data = vec![130]; // 128 + 2: palette RLE, 2 colours
        data.extend_from_slice(&[0, 0, 0]);
        data.extend_from_slice(&[9, 9, 9]);
        data.push(0x81); // index 1, with a run
        data.push(1); // run of 2
        data.push(0x00); // index 0, single pixel
        let frame = tiles(&data, 3, 1).unwrap();
        assert_eq!(&frame[32..44], &[9, 9, 9, 255, 9, 9, 9, 255, 0, 0, 0, 255]);
    }

    #[test]
    fn a_rectangle_is_split_into_sixty_four_pixel_tiles() {
        // Three solid tiles across a 130-wide rectangle: 64, 64 and 2.
        let data = vec![1, 1, 1, 1, 1, 2, 2, 2, 1, 3, 3, 3];
        let frame = tiles(&data, 130, 1).unwrap();
        assert_eq!(u16::from_le_bytes([frame[6], frame[7]]), 3, "three tiles");
        // The last tile is clipped to the rectangle's edge, not a full 64.
        let third = 16 + 2 * 20;
        assert_eq!(
            u16::from_le_bytes([frame[third], frame[third + 1]]),
            128,
            "x"
        );
        assert_eq!(
            u16::from_le_bytes([frame[third + 4], frame[third + 5]]),
            2,
            "clipped width"
        );
    }

    #[test]
    fn an_out_of_range_palette_index_is_an_error_not_a_panic() {
        // The bytes came off a socket; a bad index must not index out of
        // bounds.
        let data = vec![130, 0, 0, 0, 9, 9, 9, 0x7F];
        assert!(tiles(&data, 1, 1).is_err());
    }

    #[test]
    fn a_truncated_tile_is_an_error_not_a_panic() {
        assert!(tiles(&[0, 1, 2], 4, 4).is_err());
        assert!(tiles(&[], 1, 1).is_err());
    }

    #[test]
    fn the_undefined_subencoding_is_rejected() {
        // 129 has no meaning; continuing past it would desync the stream.
        assert!(tiles(&[129], 1, 1).is_err());
    }
}
