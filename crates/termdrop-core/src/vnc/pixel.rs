//! RFB pixel formats.
//!
//! The client asks for a format with `SetPixelFormat` and the server is
//! supposed to honour it, so in practice every pixel we decode is
//! [`PixelFormat::PREFERRED`]. This type stays general anyway for two reasons:
//! a handful of servers ignore `SetPixelFormat` and keep sending their native
//! format, and Tight's TPIXEL is a *different* width from the negotiated
//! `bits_per_pixel` under exactly the conditions our preferred format meets —
//! see [`PixelFormat::tpixel_bytes`].

/// A `PIXEL_FORMAT` as it appears on the wire: 16 bytes, big-endian.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PixelFormat {
    pub bits_per_pixel: u8,
    pub depth: u8,
    pub big_endian: bool,
    pub true_colour: bool,
    pub red_max: u16,
    pub green_max: u16,
    pub blue_max: u16,
    pub red_shift: u8,
    pub green_shift: u8,
    pub blue_shift: u8,
}

impl PixelFormat {
    /// The format we ask every server for.
    ///
    /// The shifts are the whole point: on a little-endian machine, shifts of
    /// R=0 / G=8 / B=16 put red in byte 0, green in byte 1 and blue in byte 2,
    /// which is exactly the order a canvas `ImageData` wants. Converting a row
    /// is then a strided copy that writes `0xFF` into the alpha lane, with no
    /// per-pixel arithmetic.
    ///
    /// The mirror-image choice (R=16, G=8, B=0) is the one that looks natural
    /// written down, and it yields BGRA — correct, but it costs a swizzle on
    /// every pixel of every frame.
    pub const PREFERRED: Self = Self {
        bits_per_pixel: 32,
        depth: 24,
        big_endian: false,
        true_colour: true,
        red_max: 255,
        green_max: 255,
        blue_max: 255,
        red_shift: 0,
        green_shift: 8,
        blue_shift: 16,
    };

    /// The wire size of a `PIXEL_FORMAT`.
    pub const WIRE_LEN: usize = 16;

    pub fn parse(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() < Self::WIRE_LEN {
            return Err(format!(
                "pixel format needs {} bytes, got {}",
                Self::WIRE_LEN,
                bytes.len()
            ));
        }
        Ok(Self {
            bits_per_pixel: bytes[0],
            depth: bytes[1],
            big_endian: bytes[2] != 0,
            true_colour: bytes[3] != 0,
            red_max: u16::from_be_bytes([bytes[4], bytes[5]]),
            green_max: u16::from_be_bytes([bytes[6], bytes[7]]),
            blue_max: u16::from_be_bytes([bytes[8], bytes[9]]),
            red_shift: bytes[10],
            green_shift: bytes[11],
            blue_shift: bytes[12],
            // bytes[13..16] are padding.
        })
    }

    pub fn encode(&self) -> [u8; Self::WIRE_LEN] {
        let mut out = [0u8; Self::WIRE_LEN];
        out[0] = self.bits_per_pixel;
        out[1] = self.depth;
        out[2] = u8::from(self.big_endian);
        out[3] = u8::from(self.true_colour);
        out[4..6].copy_from_slice(&self.red_max.to_be_bytes());
        out[6..8].copy_from_slice(&self.green_max.to_be_bytes());
        out[8..10].copy_from_slice(&self.blue_max.to_be_bytes());
        out[10] = self.red_shift;
        out[11] = self.green_shift;
        out[12] = self.blue_shift;
        out
    }

    /// How many bytes one pixel occupies *inside a Tight rect*.
    ///
    /// This is the single most common Tight bug. The Tight encoding defines a
    /// "TPIXEL" which is normally `bits_per_pixel / 8`, **except** when the
    /// format is 32bpp, depth 24 and all three maxima are 255 — in which case
    /// the always-zero fourth byte is omitted and a pixel is **3 bytes**.
    ///
    /// Our preferred format meets that condition exactly, so a decoder that
    /// assumes 4 reads every Tight rect one byte per pixel out of step.
    pub fn tpixel_bytes(&self) -> usize {
        if self.bits_per_pixel == 32
            && self.depth == 24
            && self.red_max == 255
            && self.green_max == 255
            && self.blue_max == 255
        {
            3
        } else {
            (self.bits_per_pixel as usize).div_ceil(8)
        }
    }

    /// Read one pixel's raw integer value, honouring the format's width and
    /// byte order.
    fn raw_value(&self, bytes: &[u8]) -> u32 {
        let width = (self.bits_per_pixel as usize).div_ceil(8).min(4);
        let mut value = 0u32;
        if self.big_endian {
            for &b in &bytes[..width] {
                value = (value << 8) | b as u32;
            }
        } else {
            for (i, &b) in bytes[..width].iter().enumerate() {
                value |= (b as u32) << (8 * i);
            }
        }
        value
    }

    /// Extract one channel, scaled to 0-255.
    ///
    /// The scaling matters for the 16bpp formats a few servers insist on: a
    /// 5-bit channel has a maximum of 31, and shifting it left by three would
    /// make pure white come out as 0xF8 — a visible grey cast across the whole
    /// desktop. Multiplying by 255 and dividing by the maximum is exact at both
    /// ends of the range.
    fn channel(value: u32, shift: u8, max: u16) -> u8 {
        if max == 0 {
            return 0;
        }
        let raw = (value >> shift) & max as u32;
        ((raw * 255 + max as u32 / 2) / max as u32) as u8
    }

    /// Convert one pixel of this format to RGB.
    pub fn pixel_to_rgb(&self, bytes: &[u8]) -> [u8; 3] {
        let value = self.raw_value(bytes);
        [
            Self::channel(value, self.red_shift, self.red_max),
            Self::channel(value, self.green_shift, self.green_max),
            Self::channel(value, self.blue_shift, self.blue_max),
        ]
    }

    /// Convert one Tight TPIXEL to RGB.
    ///
    /// In the compact 3-byte form the bytes are **red, green, blue in that
    /// order** by specification, with no reference to the format's shifts —
    /// which is why this cannot just defer to [`PixelFormat::pixel_to_rgb`].
    pub fn tpixel_to_rgb(&self, bytes: &[u8]) -> [u8; 3] {
        if self.tpixel_bytes() == 3 {
            [bytes[0], bytes[1], bytes[2]]
        } else {
            self.pixel_to_rgb(bytes)
        }
    }

    /// Convert a run of pixels into RGBA8888, appending to `dst`.
    ///
    /// The alpha lane is always written as opaque: RFB has no transparency, and
    /// leaving whatever the server put in the fourth byte would make the canvas
    /// composite against the page behind it.
    pub fn to_rgba(self, src: &[u8], pixel_count: usize, dst: &mut [u8]) -> Result<(), String> {
        let stride = (self.bits_per_pixel as usize).div_ceil(8);
        let needed = pixel_count
            .checked_mul(stride)
            .ok_or_else(|| "pixel run overflowed".to_string())?;
        if src.len() < needed {
            return Err(format!(
                "pixel run needs {} bytes, got {}",
                needed,
                src.len()
            ));
        }
        if dst.len() < pixel_count * 4 {
            return Err(format!(
                "destination needs {} bytes, got {}",
                pixel_count * 4,
                dst.len()
            ));
        }

        // Writes into a caller-owned slice rather than appending to a `Vec`:
        // at 2560x1440 this runs over 14MB, and `extend_from_slice` per pixel
        // meant 3.7 million capacity checks and 3.7 million four-byte memcpy
        // calls. Zipping two chunk iterators lets the bounds checks fall out
        // and each pixel become a single 32-bit store.
        let out = dst[..pixel_count * 4].as_chunks_mut::<4>().0;

        // The fast path is the one that actually runs: we ask every server for
        // `PREFERRED`, whose channel bytes are already in canvas order, so the
        // only work is dropping the unused fourth byte and forcing alpha.
        if self == Self::PREFERRED {
            for (dst_px, src_px) in out.iter_mut().zip(src[..needed].as_chunks::<4>().0) {
                *dst_px = [src_px[0], src_px[1], src_px[2], 0xFF];
            }
            return Ok(());
        }

        for (dst_px, src_px) in out.iter_mut().zip(src[..needed].chunks_exact(stride)) {
            let [r, g, b] = self.pixel_to_rgb(src_px);
            *dst_px = [r, g, b, 0xFF];
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preferred_format_round_trips_through_the_wire_form() {
        let encoded = PixelFormat::PREFERRED.encode();
        assert_eq!(
            PixelFormat::parse(&encoded).unwrap(),
            PixelFormat::PREFERRED
        );
    }

    #[test]
    fn preferred_format_encodes_the_documented_bytes() {
        // Pinned rather than derived, so a change to the shifts has to be
        // deliberate: these three bytes are what make a row copy instead of a
        // per-pixel swizzle.
        let e = PixelFormat::PREFERRED.encode();
        assert_eq!(e[0], 32, "bits per pixel");
        assert_eq!(e[1], 24, "depth");
        assert_eq!(e[2], 0, "little-endian");
        assert_eq!(e[3], 1, "true colour");
        assert_eq!(&e[4..10], &[0, 255, 0, 255, 0, 255], "maxima");
        assert_eq!((e[10], e[11], e[12]), (0, 8, 16), "R, G, B shifts");
        assert_eq!(&e[13..16], &[0, 0, 0], "padding");
    }

    #[test]
    fn parse_rejects_a_short_buffer_instead_of_panicking() {
        // The bytes come off a socket, so a truncated read must be an error
        // and not an index-out-of-bounds.
        assert!(PixelFormat::parse(&[0u8; 15]).is_err());
    }

    #[test]
    fn parse_ignores_the_three_padding_bytes() {
        let mut bytes = PixelFormat::PREFERRED.encode().to_vec();
        bytes[13..16].copy_from_slice(&[0xAA, 0xBB, 0xCC]);
        assert_eq!(PixelFormat::parse(&bytes).unwrap(), PixelFormat::PREFERRED);
    }

    #[test]
    fn tpixel_is_three_bytes_for_our_own_format() {
        // The condition that triggers the compact form is precisely the format
        // we negotiate, so this is the case that actually runs in production.
        assert_eq!(PixelFormat::PREFERRED.tpixel_bytes(), 3);
    }

    #[test]
    fn tpixel_is_the_full_width_when_any_condition_fails() {
        // Depth 32 (an alpha channel the server actually uses) disqualifies it.
        let deep = PixelFormat {
            depth: 32,
            ..PixelFormat::PREFERRED
        };
        assert_eq!(deep.tpixel_bytes(), 4);

        // So does a reduced colour range, even at 32bpp.
        let narrow = PixelFormat {
            red_max: 31,
            ..PixelFormat::PREFERRED
        };
        assert_eq!(narrow.tpixel_bytes(), 4);

        // 16bpp is two bytes and never compacts.
        let high_colour = PixelFormat {
            bits_per_pixel: 16,
            depth: 16,
            red_max: 31,
            green_max: 63,
            blue_max: 31,
            red_shift: 11,
            green_shift: 5,
            blue_shift: 0,
            ..PixelFormat::PREFERRED
        };
        assert_eq!(high_colour.tpixel_bytes(), 2);
    }

    // -- conversion ---------------------------------------------------------

    /// Convert into an exactly-sized buffer, the way the decoder does.
    fn convert(format: PixelFormat, src: &[u8], pixels: usize) -> Vec<u8> {
        let mut dst = vec![0u8; pixels * 4];
        format.to_rgba(src, pixels, &mut dst).unwrap();
        dst
    }

    #[test]
    fn the_preferred_format_converts_by_copying_and_forcing_alpha() {
        // Byte 3 is whatever the server left there; a canvas would composite
        // against it if we passed it through.
        let src = [10, 20, 30, 0x00, 40, 50, 60, 0x7F];
        assert_eq!(
            convert(PixelFormat::PREFERRED, &src, 2),
            vec![10, 20, 30, 0xFF, 40, 50, 60, 0xFF]
        );
    }

    #[test]
    fn a_byte_swapped_format_is_unscrambled_rather_than_copied() {
        // The mirror-image shifts: the same bytes must come out reversed.
        let bgra = PixelFormat {
            red_shift: 16,
            green_shift: 8,
            blue_shift: 0,
            ..PixelFormat::PREFERRED
        };
        assert_eq!(
            convert(bgra, &[10, 20, 30, 0], 1),
            vec![30, 20, 10, 0xFF],
            "channels were not swapped"
        );
    }

    #[test]
    fn narrow_channels_scale_to_full_range_instead_of_shifting() {
        // RGB565. Shifting a 5-bit channel left by three makes pure white come
        // out as 0xF8 -- a grey cast over the entire desktop.
        let rgb565 = PixelFormat {
            bits_per_pixel: 16,
            depth: 16,
            red_max: 31,
            green_max: 63,
            blue_max: 31,
            red_shift: 11,
            green_shift: 5,
            blue_shift: 0,
            ..PixelFormat::PREFERRED
        };
        assert_eq!(
            convert(rgb565, &[0xFF, 0xFF], 1),
            vec![255, 255, 255, 0xFF],
            "white must be pure white"
        );
        assert_eq!(
            convert(rgb565, &[0x00, 0x00], 1),
            vec![0, 0, 0, 0xFF],
            "black must be pure black"
        );
    }

    #[test]
    fn big_endian_pixels_are_read_in_the_other_order() {
        let be = PixelFormat {
            big_endian: true,
            ..PixelFormat::PREFERRED
        };
        // Shifts still say R=0, so red is the *last* byte when big-endian.
        assert_eq!(convert(be, &[0x00, 30, 20, 10], 1), vec![10, 20, 30, 0xFF]);
    }

    #[test]
    fn the_bulk_path_and_the_general_path_agree_byte_for_byte() {
        // The fast path skips the shift arithmetic entirely, so the two could
        // drift apart silently -- and only on the format that actually runs in
        // production, where nothing else would notice.
        let src: Vec<u8> = (0..64 * 4).map(|i| (i * 7 % 251) as u8).collect();
        let fast = convert(PixelFormat::PREFERRED, &src, 64);

        // Same layout, but declared so it misses the `== PREFERRED` check.
        let general = PixelFormat {
            depth: 25,
            ..PixelFormat::PREFERRED
        };
        let slow = convert(general, &src, 64);
        assert_eq!(fast, slow);
    }

    #[test]
    fn to_rgba_rejects_a_short_run_instead_of_reading_past_it() {
        // Both lengths come from a rect header the server sent, so a wrong one
        // must be an error rather than an out-of-bounds read.
        let mut dst = vec![0u8; 8];
        assert!(PixelFormat::PREFERRED
            .to_rgba(&[0; 7], 2, &mut dst)
            .is_err());

        // ...and a destination too small for the run it was given.
        let mut small = vec![0u8; 4];
        assert!(PixelFormat::PREFERRED
            .to_rgba(&[0; 8], 2, &mut small)
            .is_err());
    }

    #[test]
    fn a_compact_tpixel_is_red_green_blue_regardless_of_the_shifts() {
        // The specification fixes this order for the 3-byte form, which is why
        // it cannot defer to the shift-based path -- and our preferred format
        // is exactly the one that triggers it.
        assert_eq!(
            PixelFormat::PREFERRED.tpixel_to_rgb(&[10, 20, 30]),
            [10, 20, 30]
        );

        let bgra = PixelFormat {
            red_shift: 16,
            green_shift: 8,
            blue_shift: 0,
            ..PixelFormat::PREFERRED
        };
        assert_eq!(
            bgra.tpixel_to_rgb(&[10, 20, 30]),
            [10, 20, 30],
            "the compact form ignores the shifts"
        );
    }

    #[test]
    fn a_non_compact_tpixel_falls_back_to_the_shift_based_path() {
        let deep = PixelFormat {
            depth: 32,
            ..PixelFormat::PREFERRED
        };
        assert_eq!(deep.tpixel_bytes(), 4);
        assert_eq!(deep.tpixel_to_rgb(&[10, 20, 30, 0]), [10, 20, 30]);
    }
}
