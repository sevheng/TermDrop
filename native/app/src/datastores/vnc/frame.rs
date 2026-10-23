//! Reading the binary frames the VNC session sends: the port of
//! `src/utils/vncFrame.js`, over the format `termdrop_core::vnc::frame`
//! writes.
//!
//! One message is one complete framebuffer update, so a frame is applied
//! whole or not at all. Little-endian throughout, the opposite of RFB on the
//! wire. Payloads are borrowed from the message, never copied: a full-screen
//! raw rectangle is 8 MB.

use termdrop_core::vnc::frame::{HEADER_LEN, MAGIC, VERSION, op};

const RECT_HEADER_LEN: usize = 16;

/// What the renderer does with a rectangle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    /// `width * height * 4` bytes of RGBA.
    RawRgba,
    /// A self-blit from `(src_x, src_y)`.
    CopyRect { src_x: u16, src_y: u16 },
    /// A JPEG the core passed through undecoded.
    Jpeg,
    /// One RGBA colour over the whole rectangle (alpha always opaque).
    Fill([u8; 4]),
    /// The desktop is now `width` x `height`; always the frame's last rect.
    DesktopSize,
    /// An RGBA cursor image; `x`/`y` are the hotspot.
    Cursor,
    /// An op this build does not know: skipped, as the Vue renderer does.
    Unknown(u16),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rect<'a> {
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
    pub op: Op,
    pub payload: &'a [u8],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame<'a> {
    pub seq: u32,
    pub rects: Vec<Rect<'a>>,
}

fn u16_at(buf: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([buf[at], buf[at + 1]])
}

fn u32_at(buf: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([buf[at], buf[at + 1], buf[at + 2], buf[at + 3]])
}

/// Parse a frame into its rectangles, in wire order (which is apply order:
/// a CopyRect that overtook the JPEG it copies from would paint stale
/// pixels). Errors are `parseFrame`'s wording; every length came off a
/// socket, so a wrong one is an error and never an out-of-range slice.
pub fn parse_frame(buf: &[u8]) -> Result<Frame<'_>, String> {
    if buf.len() < HEADER_LEN {
        return Err(format!("frame too short: {} bytes", buf.len()));
    }
    let magic = u32_at(buf, 0);
    if magic != MAGIC {
        return Err(format!("bad frame magic 0x{magic:x}"));
    }
    let version = u16_at(buf, 4);
    if version != VERSION {
        return Err(format!("unsupported frame version {version}"));
    }
    let count = u16_at(buf, 6);
    let seq = u32_at(buf, 8);
    let payload_len = u32_at(buf, 12) as usize;
    if HEADER_LEN + payload_len > buf.len() {
        return Err("frame payload is shorter than its header claims".into());
    }

    let mut rects = Vec::with_capacity(count as usize);
    let mut at = HEADER_LEN;
    for _ in 0..count {
        if at + RECT_HEADER_LEN > buf.len() {
            return Err(format!("frame ended mid-rectangle at {at}"));
        }
        let (x, y) = (u16_at(buf, at), u16_at(buf, at + 2));
        let (width, height) = (u16_at(buf, at + 4), u16_at(buf, at + 6));
        let code = u16_at(buf, at + 8);
        // at + 10 is padding, which keeps every payload 4-byte aligned.
        let len = u32_at(buf, at + 12) as usize;
        let start = at + RECT_HEADER_LEN;
        if start + len > buf.len() {
            return Err(format!("rectangle payload runs past the frame at {start}"));
        }
        let payload = &buf[start..start + len];
        let op = match code {
            op::RAW_RGBA => Op::RawRgba,
            op::COPY_RECT if payload.len() >= 4 => Op::CopyRect {
                src_x: u16_at(payload, 0),
                src_y: u16_at(payload, 2),
            },
            op::JPEG => Op::Jpeg,
            op::FILL if payload.len() >= 4 => {
                Op::Fill([payload[0], payload[1], payload[2], payload[3]])
            }
            op::DESKTOP_SIZE => Op::DesktopSize,
            op::CURSOR => Op::Cursor,
            other => Op::Unknown(other),
        };
        rects.push(Rect {
            x,
            y,
            width,
            height,
            op,
            payload,
        });
        at = start + len;
    }
    Ok(Frame { seq, rects })
}

#[cfg(test)]
mod tests {
    use super::*;
    use termdrop_core::vnc::frame::FrameBuilder;

    /// A frame built by hand, so malformed ones can be made (the core's
    /// builder only writes good ones).
    /// x, y, width, height, op, payload.
    type RawRect<'a> = (u16, u16, u16, u16, u16, &'a [u8]);

    fn frame(seq: u32, rects: &[RawRect<'_>]) -> Vec<u8> {
        let payload: usize = rects.iter().map(|r| 16 + r.5.len()).sum();
        let mut buf = Vec::new();
        buf.extend_from_slice(&MAGIC.to_le_bytes());
        buf.extend_from_slice(&VERSION.to_le_bytes());
        buf.extend_from_slice(&(rects.len() as u16).to_le_bytes());
        buf.extend_from_slice(&seq.to_le_bytes());
        buf.extend_from_slice(&(payload as u32).to_le_bytes());
        for (x, y, w, h, op, p) in rects {
            for v in [*x, *y, *w, *h, *op, 0] {
                buf.extend_from_slice(&v.to_le_bytes());
            }
            buf.extend_from_slice(&(p.len() as u32).to_le_bytes());
            buf.extend_from_slice(p);
        }
        buf
    }

    #[test]
    fn reads_the_header_and_every_rectangle_in_wire_order() {
        let buf = frame(
            42,
            &[
                (0, 0, 8, 8, op::JPEG, &[0xff, 0xd8]),
                (8, 0, 0, 0, op::COPY_RECT, &[1, 0, 2, 0]),
                (0, 0, 4, 4, op::FILL, &[9, 8, 7, 255]),
            ],
        );
        let parsed = parse_frame(&buf).unwrap();
        assert_eq!(parsed.seq, 42);
        let ops: Vec<Op> = parsed.rects.iter().map(|r| r.op).collect();
        assert_eq!(
            ops,
            [
                Op::Jpeg,
                Op::CopyRect { src_x: 1, src_y: 2 },
                Op::Fill([9, 8, 7, 255])
            ]
        );
        assert_eq!(parsed.rects[1].x, 8);
    }

    #[test]
    fn payloads_borrow_the_message() {
        let buf = frame(0, &[(0, 0, 1, 1, op::RAW_RGBA, &[1, 2, 3, 4])]);
        let parsed = parse_frame(&buf).unwrap();
        let payload = parsed.rects[0].payload;
        assert_eq!(payload, &[1, 2, 3, 4]);
        // The same memory, not a copy.
        assert!(std::ptr::eq(
            payload.as_ptr(),
            buf[HEADER_LEN + 16..].as_ptr()
        ));
    }

    #[test]
    fn accepts_a_frame_with_no_rectangles() {
        assert!(parse_frame(&frame(7, &[])).unwrap().rects.is_empty());
    }

    #[test]
    fn reads_what_the_core_writes() {
        // The two ends of the format, tested against each other.
        let mut b = FrameBuilder::new(9);
        b.push_fill(1, 2, 3, 4, [10, 20, 30]);
        b.push_copy_rect(5, 6, 7, 8, 9, 10);
        b.push_raw(0, 0, 1, 1, &[1, 2, 3, 4]);
        b.push_jpeg(0, 0, 2, 2, &[0xff, 0xd8, 0xff]);
        b.push_cursor(3, 4, 1, 1, &[5, 6, 7, 8]);
        b.push_desktop_size(800, 600);
        let buf = b.finish();
        let f = parse_frame(&buf).unwrap();
        assert_eq!(f.seq, 9);
        assert_eq!(f.rects.len(), 6);
        assert_eq!(f.rects[0].op, Op::Fill([10, 20, 30, 255]));
        assert_eq!((f.rects[0].x, f.rects[0].y), (1, 2));
        assert_eq!(
            f.rects[1].op,
            Op::CopyRect {
                src_x: 9,
                src_y: 10
            }
        );
        assert_eq!(f.rects[2].payload, &[1, 2, 3, 4]);
        assert_eq!(f.rects[3].op, Op::Jpeg);
        assert_eq!(f.rects[3].payload, &[0xff, 0xd8, 0xff]);
        assert_eq!(f.rects[4].op, Op::Cursor);
        assert_eq!((f.rects[4].x, f.rects[4].y), (3, 4));
        assert_eq!(f.rects[5].op, Op::DesktopSize);
        assert_eq!((f.rects[5].width, f.rects[5].height), (800, 600));
    }

    #[test]
    fn rejects_a_frame_that_is_not_one_of_ours() {
        let mut buf = frame(0, &[]);
        buf[0..4].copy_from_slice(&0xdeadbeef_u32.to_le_bytes());
        assert_eq!(parse_frame(&buf).unwrap_err(), "bad frame magic 0xdeadbeef");
    }

    #[test]
    fn rejects_a_version_it_cannot_parse() {
        let mut buf = frame(0, &[]);
        buf[4..6].copy_from_slice(&99u16.to_le_bytes());
        assert_eq!(
            parse_frame(&buf).unwrap_err(),
            "unsupported frame version 99"
        );
    }

    #[test]
    fn refuses_to_read_past_the_end() {
        assert_eq!(
            parse_frame(&[0; 8]).unwrap_err(),
            "frame too short: 8 bytes"
        );

        let mut buf = frame(0, &[(0, 0, 1, 1, op::RAW_RGBA, &[1, 2, 3, 4])]);
        buf[28..32].copy_from_slice(&9999u32.to_le_bytes()); // the rect's length
        assert!(parse_frame(&buf).unwrap_err().contains("past the frame"));

        let mut short = frame(0, &[(0, 0, 1, 1, op::FILL, &[1, 2, 3, 4])]);
        short[6..8].copy_from_slice(&5u16.to_le_bytes()); // claim five rects
        assert!(parse_frame(&short).unwrap_err().contains("mid-rectangle"));

        let mut lying = frame(0, &[]);
        lying[12..16].copy_from_slice(&100u32.to_le_bytes());
        assert_eq!(
            parse_frame(&lying).unwrap_err(),
            "frame payload is shorter than its header claims"
        );
    }

    #[test]
    fn an_unknown_op_is_carried_not_fatal() {
        let buf = frame(0, &[(0, 0, 1, 1, 77, &[])]);
        assert_eq!(parse_frame(&buf).unwrap().rects[0].op, Op::Unknown(77));
    }
}
