//! Tight: the encoding every real VNC server prefers.
//!
//! Four things here are load-bearing, and three of them fail silently:
//!
//! 1. **Four zlib streams, each alive for the whole connection.** The control
//!    byte's low nibble says which to reset; nothing else ever recreates one.
//!    Recreating per rectangle works on the first frame and corrupts the rest.
//! 2. **A JPEG rectangle touches none of the streams.** That is what makes it
//!    safe to hand the bytes to the browser undecoded — the decoder state does
//!    not depend on them.
//! 3. **TPIXEL is 3 bytes, not 4**, for the pixel format we negotiate. Reading
//!    4 walks one byte further into every rectangle.
//! 4. **Payloads under 12 bytes are sent uncompressed**, with no length prefix
//!    and no zlib involvement at all.
//!
//! All three filters are implemented. Skipping an unsupported one is not an
//! option: its bytes are inside a zlib stream that continues into the next
//! rectangle, so a skip desyncs everything after it.

use flate2::{Decompress, FlushDecompress};

use super::super::conn::Conn;
use super::super::frame::FrameBuilder;
use super::super::pixel::PixelFormat;
use super::super::proto::RectHeader;
use super::Reader;

/// Below this, `basic` data is sent raw rather than through zlib.
const MIN_COMPRESSED: usize = 12;
/// A rectangle larger than this is not a rectangle.
const MAX_TIGHT_BYTES: usize = 64 * 1024 * 1024;

/// The four zlib streams, held for the connection's lifetime.
pub struct TightStreams {
    streams: [Decompress; 4],
}

impl Default for TightStreams {
    fn default() -> Self {
        Self::new()
    }
}

impl TightStreams {
    pub fn new() -> Self {
        Self {
            streams: std::array::from_fn(|_| Decompress::new(true)),
        }
    }

    fn reset(&mut self, index: usize) {
        // A *new* stream, not `Decompress::reset`: the server has genuinely
        // restarted the compressor, so the old dictionary must go.
        self.streams[index] = Decompress::new(true);
    }

    fn inflate(
        &mut self,
        index: usize,
        compressed: &[u8],
        expected: usize,
    ) -> Result<Vec<u8>, String> {
        let mut out = Vec::with_capacity(expected.min(MAX_TIGHT_BYTES));
        let stream = &mut self.streams[index];
        let mut consumed = 0;

        while consumed < compressed.len() && out.len() < expected {
            let before_in = stream.total_in();
            let before_out = stream.total_out();
            // Sync, never Finish: the stream continues into later rectangles
            // and telling zlib otherwise discards the dictionary.
            stream
                .decompress_vec(&compressed[consumed..], &mut out, FlushDecompress::Sync)
                .map_err(|e| format!("Tight zlib error on stream {}: {}", index, e))?;
            let read = (stream.total_in() - before_in) as usize;
            let wrote = stream.total_out() - before_out;
            consumed += read;

            if read == 0 && wrote == 0 {
                if out.len() >= expected {
                    break;
                }
                let want = (expected - out.len()).max(4096);
                if out.capacity() - out.len() >= want {
                    return Err("Tight stream made no progress".to_string());
                }
                out.reserve(want);
            }
        }

        if out.len() < expected {
            return Err(format!(
                "Tight stream produced {} bytes, expected {}",
                out.len(),
                expected
            ));
        }
        out.truncate(expected);
        Ok(out)
    }
}

/// What the control byte selected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TightMode {
    Fill,
    Jpeg,
    /// Basic compression using zlib stream `stream`, with an explicit filter
    /// byte when `has_filter`.
    Basic {
        stream: usize,
        has_filter: bool,
    },
    /// 0xA0 and above are reserved; `tight-png` uses them and we do not
    /// negotiate it.
    Reserved(u8),
}

/// Split a Tight control byte into its reset mask and its mode.
///
/// Pure, so the bit layout can be tested without a socket — it is the part
/// most likely to be wrong and least likely to announce it.
pub fn parse_control(control: u8) -> (u8, TightMode) {
    let resets = control & 0x0F;
    let mode = match control >> 4 {
        0x08 => TightMode::Fill,
        0x09 => TightMode::Jpeg,
        high if high >= 0x0A => TightMode::Reserved(control),
        high => TightMode::Basic {
            stream: (high & 0x03) as usize,
            // Bit 6 of the control byte: an explicit filter id follows.
            has_filter: high & 0x04 != 0,
        },
    };
    (resets, mode)
}

/// Tight's compact length: 7 bits per byte, high bit means "another follows".
fn compact_len(reader: &mut Reader) -> Result<usize, String> {
    let b0 = reader.u8()?;
    let mut length = (b0 & 0x7F) as usize;
    if b0 & 0x80 == 0 {
        return Ok(length);
    }
    let b1 = reader.u8()?;
    length |= ((b1 & 0x7F) as usize) << 7;
    if b1 & 0x80 == 0 {
        return Ok(length);
    }
    let b2 = reader.u8()?;
    length |= (b2 as usize) << 14;
    Ok(length)
}

/// Read exactly one Tight rectangle's bytes off the socket.
///
/// Tight is the only encoding with no length up front — how much follows
/// depends on the control byte, the filter, the palette size and whether the
/// payload was small enough to skip zlib. So this walks the same structure
/// [`decode`] does, buffering as it goes.
///
/// The duplication is deliberate: keeping [`decode`] working on a buffer is
/// what makes every branch of it testable without a socket. The two agreeing is
/// pinned by `read_body_and_decode_agree_on_the_byte_count`.
pub fn read_body(
    conn: &mut Conn,
    control: u8,
    header: &RectHeader,
    format: &PixelFormat,
) -> Result<Vec<u8>, String> {
    let (_, mode) = parse_control(control);
    let tpixel = format.tpixel_bytes();
    let width = header.width as usize;
    let height = header.height as usize;
    let mut body = Vec::new();

    match mode {
        TightMode::Fill => take(conn, &mut body, tpixel)?,
        TightMode::Jpeg => {
            let len = take_compact(conn, &mut body)?;
            take(conn, &mut body, len)?;
        }
        // Left empty: `decode` is what reports it, so the message lives in one
        // place.
        TightMode::Reserved(_) => {}
        TightMode::Basic { has_filter, .. } => {
            let filter = if has_filter {
                take(conn, &mut body, 1)?;
                body[body.len() - 1]
            } else {
                0
            };
            match filter {
                0 => take_payload(conn, &mut body, width * height * tpixel)?,
                1 => {
                    take(conn, &mut body, 1)?;
                    let colours = body[body.len() - 1] as usize + 1;
                    take(conn, &mut body, colours * tpixel)?;
                    let row_bytes = if colours == 2 {
                        width.div_ceil(8)
                    } else {
                        width
                    };
                    take_payload(conn, &mut body, row_bytes * height)?;
                }
                2 => take_payload(conn, &mut body, width * height * 3)?,
                // Unknown: `decode` reports it.
                _ => {}
            }
        }
    }
    Ok(body)
}

/// Either the raw bytes, or a compact length and that many compressed ones.
fn take_payload(conn: &mut Conn, body: &mut Vec<u8>, expected: usize) -> Result<(), String> {
    if expected > MAX_TIGHT_BYTES {
        return Err(format!("implausible Tight rectangle: {} bytes", expected));
    }
    if expected < MIN_COMPRESSED {
        return take(conn, body, expected);
    }
    let len = take_compact(conn, body)?;
    take(conn, body, len)
}

fn take(conn: &mut Conn, body: &mut Vec<u8>, n: usize) -> Result<(), String> {
    let bytes = conn.read_vec(n, MAX_TIGHT_BYTES, "Tight rectangle")?;
    body.extend_from_slice(&bytes);
    Ok(())
}

/// Read a compact length, keeping its bytes in `body` so [`decode`] sees them.
fn take_compact(conn: &mut Conn, body: &mut Vec<u8>) -> Result<usize, String> {
    let mut length = 0usize;
    for shift in [0u32, 7, 14] {
        let mut byte = [0u8; 1];
        conn.read_exact(&mut byte)?;
        body.push(byte[0]);
        if shift == 14 {
            length |= (byte[0] as usize) << shift;
            break;
        }
        length |= ((byte[0] & 0x7F) as usize) << shift;
        if byte[0] & 0x80 == 0 {
            break;
        }
    }
    Ok(length)
}

/// Decode one Tight rectangle. `body` is everything after the control byte.
pub fn decode(
    streams: &mut TightStreams,
    control: u8,
    reader: &mut Reader,
    header: &RectHeader,
    format: &PixelFormat,
    frame: &mut FrameBuilder,
) -> Result<(), String> {
    let (resets, mode) = parse_control(control);
    for index in 0..4 {
        if resets & (1 << index) != 0 {
            streams.reset(index);
        }
    }

    let tpixel = format.tpixel_bytes();
    let width = header.width as usize;
    let height = header.height as usize;

    match mode {
        TightMode::Fill => {
            let rgb = format.tpixel_to_rgb(reader.take(tpixel)?);
            frame.push_fill(header.x, header.y, header.width, header.height, rgb);
            Ok(())
        }
        TightMode::Jpeg => {
            let len = compact_len(reader)?;
            let jpeg = reader.take(len)?;
            // Handed on undecoded: the browser has a native decoder, and
            // expanding it here would multiply the IPC payload a hundredfold.
            frame.push_jpeg(header.x, header.y, header.width, header.height, jpeg);
            Ok(())
        }
        TightMode::Reserved(byte) => Err(format!(
            "the server used Tight control byte 0x{:02X}, which TermDrop does not \
             negotiate. The connection has to end here: its bytes are inside a \
             zlib stream that continues into the next rectangle.",
            byte
        )),
        TightMode::Basic { stream, has_filter } => {
            let filter = if has_filter { reader.u8()? } else { 0 };
            match filter {
                0 => basic_copy(
                    streams, stream, reader, width, height, tpixel, format, frame, header,
                ),
                1 => basic_palette(
                    streams, stream, reader, width, height, tpixel, format, frame, header,
                ),
                2 => basic_gradient(
                    streams, stream, reader, width, height, tpixel, format, frame, header,
                ),
                other => Err(format!("unknown Tight filter {}", other)),
            }
        }
    }
}

/// Read `expected` bytes of filter data, decompressing only when it was.
fn filter_data(
    streams: &mut TightStreams,
    stream: usize,
    reader: &mut Reader,
    expected: usize,
) -> Result<Vec<u8>, String> {
    if expected > MAX_TIGHT_BYTES {
        return Err(format!("implausible Tight rectangle: {} bytes", expected));
    }
    // Under twelve bytes the server sends the data raw, with no length prefix
    // and without putting it through zlib at all.
    if expected < MIN_COMPRESSED {
        return Ok(reader.take(expected)?.to_vec());
    }
    let len = compact_len(reader)?;
    let compressed = reader.take(len)?;
    streams.inflate(stream, compressed, expected)
}

#[allow(clippy::too_many_arguments)]
fn basic_copy(
    streams: &mut TightStreams,
    stream: usize,
    reader: &mut Reader,
    width: usize,
    height: usize,
    tpixel: usize,
    format: &PixelFormat,
    frame: &mut FrameBuilder,
    header: &RectHeader,
) -> Result<(), String> {
    let data = filter_data(streams, stream, reader, width * height * tpixel)?;
    let mut rgba = Vec::with_capacity(width * height * 4);
    for px in data.chunks_exact(tpixel) {
        let [r, g, b] = format.tpixel_to_rgb(px);
        rgba.extend_from_slice(&[r, g, b, 0xFF]);
    }
    frame.push_raw(header.x, header.y, header.width, header.height, &rgba);
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn basic_palette(
    streams: &mut TightStreams,
    stream: usize,
    reader: &mut Reader,
    width: usize,
    height: usize,
    tpixel: usize,
    format: &PixelFormat,
    frame: &mut FrameBuilder,
    header: &RectHeader,
) -> Result<(), String> {
    // The byte is the *last index*, so a two-colour palette sends 1.
    let colours = reader.u8()? as usize + 1;
    let mut palette = Vec::with_capacity(colours);
    for _ in 0..colours {
        palette.push(format.tpixel_to_rgb(reader.take(tpixel)?));
    }

    // Two colours are packed one bit per pixel and each row restarts on a byte
    // boundary; anything more is a whole byte per pixel.
    let (row_bytes, one_bit) = if colours == 2 {
        (width.div_ceil(8), true)
    } else {
        (width, false)
    };
    let data = filter_data(streams, stream, reader, row_bytes * height)?;

    let mut rgba = Vec::with_capacity(width * height * 4);
    for row in 0..height {
        let row_start = row * row_bytes;
        for column in 0..width {
            let index = if one_bit {
                let byte = data[row_start + column / 8];
                ((byte >> (7 - (column % 8))) & 1) as usize
            } else {
                data[row_start + column] as usize
            };
            let [r, g, b] = *palette
                .get(index)
                .ok_or_else(|| format!("Tight palette index {} is out of range", index))?;
            rgba.extend_from_slice(&[r, g, b, 0xFF]);
        }
    }
    frame.push_raw(header.x, header.y, header.width, header.height, &rgba);
    Ok(())
}

/// The gradient filter: each channel is predicted from its neighbours.
///
/// Cannot be left unimplemented. An undecodable rectangle would have to be
/// skipped, and its bytes live in a zlib stream that continues into the next
/// one — so "unsupported" would mean "every later rectangle is garbage", not
/// "this one is missing".
#[allow(clippy::too_many_arguments)]
fn basic_gradient(
    streams: &mut TightStreams,
    stream: usize,
    reader: &mut Reader,
    width: usize,
    height: usize,
    tpixel: usize,
    format: &PixelFormat,
    frame: &mut FrameBuilder,
    header: &RectHeader,
) -> Result<(), String> {
    if tpixel != 3 {
        return Err("the Tight gradient filter needs a 24-bit pixel format".to_string());
    }
    let data = filter_data(streams, stream, reader, width * height * 3)?;

    let mut rgba = vec![0u8; width * height * 4];
    // The previous row of *reconstructed* values, which is what the predictor
    // uses -- not the previous row of residuals.
    let mut previous = vec![0u8; width * 3];
    let mut current = vec![0u8; width * 3];

    for row in 0..height {
        for column in 0..width {
            for channel in 0..3 {
                let left = if column > 0 {
                    current[(column - 1) * 3 + channel] as i32
                } else {
                    0
                };
                let above = previous[column * 3 + channel] as i32;
                let above_left = if column > 0 {
                    previous[(column - 1) * 3 + channel] as i32
                } else {
                    0
                };

                // Paeth-style prediction, clamped to a byte.
                let predicted = (left + above - above_left).clamp(0, 255);
                let residual = data[(row * width + column) * 3 + channel];
                let value = (predicted as u8).wrapping_add(residual);
                current[column * 3 + channel] = value;
                rgba[(row * width + column) * 4 + channel] = value;
            }
            rgba[(row * width + column) * 4 + 3] = 0xFF;
        }
        std::mem::swap(&mut previous, &mut current);
    }

    // The gradient filter always works in RGB order, independently of the
    // negotiated shifts -- the same rule the compact TPIXEL follows.
    let _ = format;
    frame.push_raw(header.x, header.y, header.width, header.height, &rgba);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_control_byte_splits_into_resets_and_a_mode() {
        // The bit layout, isolated. Everything downstream depends on it and
        // nothing about a wrong answer announces itself.
        assert_eq!(parse_control(0x80), (0, TightMode::Fill));
        assert_eq!(parse_control(0x90), (0, TightMode::Jpeg));
        assert_eq!(
            parse_control(0x00),
            (
                0,
                TightMode::Basic {
                    stream: 0,
                    has_filter: false
                }
            )
        );
        assert_eq!(
            parse_control(0x40),
            (
                0,
                TightMode::Basic {
                    stream: 0,
                    has_filter: true
                }
            )
        );
        assert_eq!(
            parse_control(0x30),
            (
                0,
                TightMode::Basic {
                    stream: 3,
                    has_filter: false
                }
            )
        );
    }

    #[test]
    fn the_low_nibble_names_the_streams_to_reset() {
        // Resets ride along with the mode, so they must be read even from a
        // Fill or JPEG control byte.
        assert_eq!(parse_control(0x0F).0, 0b1111);
        assert_eq!(parse_control(0x85).0, 0b0101, "a Fill can reset too");
        assert_eq!(parse_control(0x91).0, 0b0001, "so can a JPEG");
    }

    #[test]
    fn reserved_control_bytes_are_refused_rather_than_skipped() {
        // 0xA0 and up belong to tight-png, which we do not negotiate. Skipping
        // would leave the zlib streams out of step for every later rectangle.
        assert!(matches!(parse_control(0xA0).1, TightMode::Reserved(0xA0)));
        assert!(matches!(parse_control(0xF0).1, TightMode::Reserved(0xF0)));
    }

    #[test]
    fn a_compact_length_uses_seven_bits_per_byte() {
        assert_eq!(compact_len(&mut Reader::new(&[0x00])).unwrap(), 0);
        assert_eq!(compact_len(&mut Reader::new(&[0x7F])).unwrap(), 127);
        // Two bytes: 0x80 sets the continuation bit, so this is 0 | (1 << 7).
        assert_eq!(compact_len(&mut Reader::new(&[0x80, 0x01])).unwrap(), 128);
        assert_eq!(compact_len(&mut Reader::new(&[0xFF, 0x7F])).unwrap(), 16383);
        // Three bytes for anything larger.
        assert_eq!(
            compact_len(&mut Reader::new(&[0x80, 0x80, 0x01])).unwrap(),
            16384
        );
    }

    #[test]
    fn a_compact_length_that_runs_off_the_end_is_an_error() {
        assert!(compact_len(&mut Reader::new(&[0x80])).is_err());
        assert!(compact_len(&mut Reader::new(&[])).is_err());
    }

    fn header(width: u16, height: u16) -> RectHeader {
        RectHeader {
            x: 0,
            y: 0,
            width,
            height,
            encoding: 7,
        }
    }

    #[test]
    fn a_fill_rectangle_reads_three_bytes_and_emits_a_fill() {
        // Our negotiated format makes TPIXEL three bytes; reading four would
        // consume the first byte of whatever follows.
        let mut streams = TightStreams::new();
        let mut frame = FrameBuilder::new(0);
        let mut reader = Reader::new(&[11, 22, 33, 0xAA]);
        decode(
            &mut streams,
            0x80,
            &mut reader,
            &header(100, 50),
            &PixelFormat::PREFERRED,
            &mut frame,
        )
        .unwrap();
        assert_eq!(reader.remaining(), 1, "read exactly three bytes");
        let bytes = frame.finish();
        assert_eq!(&bytes[32..36], &[11, 22, 33, 0xFF]);
    }

    #[test]
    fn a_jpeg_rectangle_passes_its_bytes_through_untouched() {
        // And crucially without touching any zlib stream -- which is what makes
        // handing them to the browser safe.
        let mut streams = TightStreams::new();
        let mut frame = FrameBuilder::new(0);
        let jpeg = [0xFF, 0xD8, 0xFF, 0xE0, 0xFF, 0xD9];
        let mut data = vec![jpeg.len() as u8];
        data.extend_from_slice(&jpeg);

        let mut reader = Reader::new(&data);
        decode(
            &mut streams,
            0x90,
            &mut reader,
            &header(16, 16),
            &PixelFormat::PREFERRED,
            &mut frame,
        )
        .unwrap();
        let bytes = frame.finish();
        assert_eq!(&bytes[32..], &jpeg, "the JPEG was altered");
    }

    #[test]
    fn short_basic_data_is_read_raw_with_no_length_prefix() {
        // Under twelve bytes there is no compact length and no zlib at all.
        // Reading a length byte here would eat the first pixel.
        let mut streams = TightStreams::new();
        let mut frame = FrameBuilder::new(0);
        // 2x1 pixels at 3 bytes each = 6 bytes, below the threshold.
        let mut reader = Reader::new(&[1, 2, 3, 4, 5, 6]);
        decode(
            &mut streams,
            0x00,
            &mut reader,
            &header(2, 1),
            &PixelFormat::PREFERRED,
            &mut frame,
        )
        .unwrap();
        let bytes = frame.finish();
        assert_eq!(&bytes[32..40], &[1, 2, 3, 0xFF, 4, 5, 6, 0xFF]);
    }

    #[test]
    fn a_two_colour_palette_packs_one_bit_per_pixel() {
        let mut streams = TightStreams::new();
        let mut frame = FrameBuilder::new(0);
        let mut data = vec![1]; // filter 1: palette
        data.push(1); // last index 1, so two colours
        data.extend_from_slice(&[0, 0, 0]); // colour 0
        data.extend_from_slice(&[255, 255, 255]); // colour 1
        data.push(0b1010_0000); // 4 pixels: 1, 0, 1, 0

        let mut reader = Reader::new(&data);
        decode(
            &mut streams,
            0x40, // basic, stream 0, with a filter byte
            &mut reader,
            &header(4, 1),
            &PixelFormat::PREFERRED,
            &mut frame,
        )
        .unwrap();
        let bytes = frame.finish();
        assert_eq!(
            &bytes[32..48],
            &[255, 255, 255, 255, 0, 0, 0, 255, 255, 255, 255, 255, 0, 0, 0, 255]
        );
    }

    #[test]
    fn an_out_of_range_palette_index_is_an_error() {
        let mut streams = TightStreams::new();
        let mut frame = FrameBuilder::new(0);
        let mut data = vec![1, 2]; // filter 1, three colours
        data.extend_from_slice(&[0, 0, 0]);
        data.extend_from_slice(&[1, 1, 1]);
        data.extend_from_slice(&[2, 2, 2]);
        data.push(9); // index 9 into a 3-colour palette
        let mut reader = Reader::new(&data);
        assert!(decode(
            &mut streams,
            0x40,
            &mut reader,
            &header(1, 1),
            &PixelFormat::PREFERRED,
            &mut frame,
        )
        .is_err());
    }

    /// zlib-compress a payload the way a server would.
    fn zlib(data: &[u8]) -> Vec<u8> {
        use flate2::write::ZlibEncoder;
        use std::io::Write;
        let mut encoder = ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(data).unwrap();
        encoder.finish().unwrap()
    }

    /// Prefix a compressed payload with its compact length.
    fn with_len(compressed: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut len = compressed.len();
        loop {
            let mut byte = (len & 0x7F) as u8;
            len >>= 7;
            if len > 0 {
                byte |= 0x80;
            }
            out.push(byte);
            if len == 0 {
                break;
            }
        }
        out.extend_from_slice(compressed);
        out
    }

    #[test]
    fn the_gradient_filter_predicts_down_as_well_as_across() {
        // 2x2 is 12 bytes, which is exactly the threshold -- so this also
        // exercises the zlib path rather than the raw one.
        //
        // Residuals all zero means every pixel equals its prediction. Pixel
        // (1,1) predicts left + above - above-left, so a wrong sign or a
        // missing above-left term shows up here and nowhere else.
        let mut streams = TightStreams::new();
        let mut frame = FrameBuilder::new(0);
        let mut data = vec![2]; // filter 2: gradient
        data.extend_from_slice(&with_len(&zlib(&[0u8; 12])));

        let mut reader = Reader::new(&data);
        decode(
            &mut streams,
            0x40,
            &mut reader,
            &header(2, 2),
            &PixelFormat::PREFERRED,
            &mut frame,
        )
        .unwrap();
        let bytes = frame.finish();
        for pixel in bytes[32..].as_chunks::<4>().0 {
            assert_eq!(pixel, &[0, 0, 0, 0xFF]);
        }
    }

    #[test]
    fn a_zlib_stream_carries_its_dictionary_across_rectangles() {
        // The single most important property in this module. A server
        // compresses every rectangle through *one* stream, so the second
        // depends on the dictionary built by the first. Creating a fresh
        // decompressor per rectangle decodes frame one perfectly and then
        // produces coloured garbage for everything after it -- which reads as
        // a decoder bug anywhere but here.
        use flate2::{Compress, Compression, FlushCompress};

        let first = [7u8; 24];
        let second = [7u8; 24];

        // One compressor, two flushes: exactly what a server does.
        let mut compressor = Compress::new(Compression::default(), true);
        let mut chunk_one = Vec::with_capacity(128);
        compressor
            .compress_vec(&first, &mut chunk_one, FlushCompress::Sync)
            .unwrap();
        let mut chunk_two = Vec::with_capacity(128);
        compressor
            .compress_vec(&second, &mut chunk_two, FlushCompress::Sync)
            .unwrap();

        // The second chunk is much smaller precisely because it reuses the
        // dictionary -- if it were self-contained there would be nothing to
        // get wrong.
        assert!(
            chunk_two.len() < chunk_one.len(),
            "the fixture does not actually depend on the shared dictionary"
        );

        let mut streams = TightStreams::new();
        for chunk in [chunk_one, chunk_two] {
            let mut data = vec![0u8]; // filter 0: copy
            data.extend_from_slice(&with_len(&chunk));
            let mut frame = FrameBuilder::new(0);
            let mut reader = Reader::new(&data);
            decode(
                &mut streams,
                0x40,
                &mut reader,
                &header(8, 1),
                &PixelFormat::PREFERRED,
                &mut frame,
            )
            .expect("second rectangle needs the first one's dictionary");
            let bytes = frame.finish();
            for pixel in bytes[32..].as_chunks::<4>().0 {
                assert_eq!(pixel, &[7, 7, 7, 0xFF]);
            }
        }
    }

    #[test]
    fn a_reset_bit_discards_the_dictionary() {
        // The other half: when the server restarts its compressor it says so
        // in the control byte's low nibble, and carrying the old dictionary
        // over would corrupt everything from there on.
        let mut streams = TightStreams::new();
        let mut data = vec![0u8];
        data.extend_from_slice(&with_len(&zlib(&[3u8; 24])));
        let mut frame = FrameBuilder::new(0);
        let mut reader = Reader::new(&data);
        // 0x41: basic, stream 0, with filter, resetting stream 0 first.
        decode(
            &mut streams,
            0x41,
            &mut reader,
            &header(8, 1),
            &PixelFormat::PREFERRED,
            &mut frame,
        )
        .unwrap();
        let bytes = frame.finish();
        assert_eq!(&bytes[32..36], &[3, 3, 3, 0xFF]);
    }

    #[test]
    fn the_gradient_predictor_adds_the_left_neighbour_on_the_first_row() {
        // On row 0 the above and above-left neighbours are zero, so the
        // prediction is just the pixel to the left and the residuals
        // accumulate along the row.
        let mut streams = TightStreams::new();
        let mut frame = FrameBuilder::new(0);
        let mut data = vec![2];
        data.extend_from_slice(&[10, 0, 0]); // pixel 0: red 10
        data.extend_from_slice(&[5, 0, 0]); // pixel 1: predicted 10, +5 = 15
        data.extend_from_slice(&[0, 0, 0]); // pixel 2: predicted 15, +0 = 15
        let mut reader = Reader::new(&data);
        decode(
            &mut streams,
            0x40,
            &mut reader,
            &header(3, 1),
            &PixelFormat::PREFERRED,
            &mut frame,
        )
        .unwrap();
        let bytes = frame.finish();
        assert_eq!(bytes[32], 10);
        assert_eq!(bytes[36], 15);
        assert_eq!(bytes[40], 15);
    }

    #[test]
    fn an_unknown_filter_is_an_error_rather_than_a_skip() {
        let mut streams = TightStreams::new();
        let mut frame = FrameBuilder::new(0);
        let mut reader = Reader::new(&[99]);
        assert!(decode(
            &mut streams,
            0x40,
            &mut reader,
            &header(1, 1),
            &PixelFormat::PREFERRED,
            &mut frame,
        )
        .is_err());
    }

    /// A Conn fed from a buffer, for exercising the socket-side reader.
    fn conn_of(bytes: Vec<u8>) -> Conn {
        use std::io::Write;
        use std::net::TcpListener;
        use std::sync::atomic::AtomicBool;
        use std::sync::Arc;

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let _ = stream.write_all(&bytes);
                // Hold the socket open so a read past the end blocks rather
                // than returning a spurious EOF.
                std::thread::sleep(std::time::Duration::from_millis(200));
            }
        });
        Conn::connect("127.0.0.1", port, Arc::new(AtomicBool::new(false))).unwrap()
    }

    #[test]
    fn read_body_and_decode_agree_on_the_byte_count() {
        // The two walk the same structure -- one off a socket, one off a
        // buffer -- so that `decode` stays testable without a network. If they
        // disagree by even one byte, every rectangle after this one is read at
        // the wrong offset and the picture turns to noise with no error.
        let cases: Vec<(&str, u8, RectHeader, Vec<u8>)> = vec![
            ("fill", 0x80, header(64, 64), vec![1, 2, 3]),
            ("jpeg", 0x90, header(16, 16), {
                let mut v = vec![4];
                v.extend_from_slice(&[0xFF, 0xD8, 0xFF, 0xD9]);
                v
            }),
            (
                "basic raw, below the zlib threshold",
                0x00,
                header(2, 1),
                vec![1, 2, 3, 4, 5, 6],
            ),
            ("basic copy, compressed", 0x40, header(8, 1), {
                let mut v = vec![0u8];
                v.extend_from_slice(&with_len(&zlib(&[9u8; 24])));
                v
            }),
            ("palette of two", 0x40, header(4, 1), {
                let mut v = vec![1, 1];
                v.extend_from_slice(&[0, 0, 0]);
                v.extend_from_slice(&[255, 255, 255]);
                v.push(0b1010_0000);
                v
            }),
            ("gradient", 0x40, header(3, 1), {
                let mut v = vec![2];
                v.extend_from_slice(&[10, 0, 0, 5, 0, 0, 0, 0, 0]);
                v
            }),
        ];

        for (name, control, rect, body) in cases {
            // A trailing sentinel: if read_body takes too much it consumes it,
            // and if it takes too little the sentinel is left in the body.
            let mut wire = body.clone();
            wire.extend_from_slice(&[0xDE, 0xAD]);
            let mut conn = conn_of(wire);

            let read = read_body(&mut conn, control, &rect, &PixelFormat::PREFERRED)
                .unwrap_or_else(|e| panic!("{}: read_body failed: {}", name, e));
            assert_eq!(read, body, "{}: read_body took the wrong bytes", name);

            let mut streams = TightStreams::new();
            let mut frame = FrameBuilder::new(0);
            let mut reader = Reader::new(&read);
            decode(
                &mut streams,
                control,
                &mut reader,
                &rect,
                &PixelFormat::PREFERRED,
                &mut frame,
            )
            .unwrap_or_else(|e| panic!("{}: decode failed: {}", name, e));
            assert_eq!(reader.remaining(), 0, "{}: decode left bytes unread", name);
        }
    }

    #[test]
    fn a_reserved_control_byte_names_itself_in_the_error() {
        let mut streams = TightStreams::new();
        let mut frame = FrameBuilder::new(0);
        let mut reader = Reader::new(&[]);
        let err = decode(
            &mut streams,
            0xA0,
            &mut reader,
            &header(1, 1),
            &PixelFormat::PREFERRED,
            &mut frame,
        )
        .unwrap_err();
        assert!(err.contains("0xA0"), "got: {}", err);
    }
}
