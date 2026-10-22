//! RFB message framing: the handshake, the client messages we send, and the
//! headers of the server messages we read.
//!
//! Everything here is pure — bytes in, bytes out — so the whole protocol
//! surface is testable without a socket. RFB is **big-endian** on the wire,
//! which is the opposite of both our host machines and the IPC frame format in
//! [`super::frame`], so the two conversions are kept strictly apart: this
//! module never emits a little-endian integer.

use super::pixel::PixelFormat;

// ---------------------------------------------------------------------------
// Protocol version
// ---------------------------------------------------------------------------

/// The protocol version the two ends agreed on.
///
/// Only these two matter. 3.7 exists but is identical to 3.8 for everything we
/// do except that it omits the `SecurityResult` after a successful `None`
/// handshake — the same quirk 3.3 has, so it is folded into the 3.3 arm.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProtocolVersion {
    /// RFB 3.3: a single `u32` security type, and no `SecurityResult` for `None`.
    Rfb33,
    /// RFB 3.8: a counted list of security types, and a `SecurityResult` always.
    Rfb38,
}

impl ProtocolVersion {
    pub const WIRE_LEN: usize = 12;

    /// Parse the server's greeting and pick the version we will answer with.
    ///
    /// We never speak a version *newer* than the server's, which is what the
    /// specification requires, and we never speak one older than 3.3.
    pub fn negotiate(greeting: &[u8]) -> Result<Self, String> {
        if greeting.len() < Self::WIRE_LEN {
            return Err(format!(
                "truncated protocol greeting: {} bytes",
                greeting.len()
            ));
        }
        let text = std::str::from_utf8(&greeting[..Self::WIRE_LEN])
            .map_err(|_| "protocol greeting is not ASCII; is this a VNC server?".to_string())?;
        let rest = text
            .strip_prefix("RFB ")
            .and_then(|r| r.strip_suffix('\n'))
            .ok_or_else(|| {
                format!(
                    "not a VNC server: expected a greeting like \"RFB 003.008\", got {:?}",
                    text.trim_end()
                )
            })?;
        let (major, minor) = rest
            .split_once('.')
            .ok_or_else(|| format!("malformed protocol version {:?}", rest))?;
        let major: u32 = major
            .trim()
            .parse()
            .map_err(|_| format!("malformed protocol major version {:?}", major))?;
        let minor: u32 = minor
            .trim()
            .parse()
            .map_err(|_| format!("malformed protocol minor version {:?}", minor))?;

        if major != 3 {
            return Err(format!(
                "unsupported RFB protocol version {}.{}; TermDrop speaks 3.3 and 3.8",
                major, minor
            ));
        }
        // 3.7 behaves like 3.8 for the counted security list but, like 3.3,
        // sends no SecurityResult after `None`. Treating it as 3.3 would break
        // the list parse, so it is deliberately not supported as its own arm;
        // servers that announce 3.7 are answered with 3.3, which every one of
        // them accepts.
        Ok(if minor >= 8 {
            ProtocolVersion::Rfb38
        } else {
            ProtocolVersion::Rfb33
        })
    }

    pub fn bytes(&self) -> [u8; Self::WIRE_LEN] {
        match self {
            ProtocolVersion::Rfb33 => *b"RFB 003.003\n",
            ProtocolVersion::Rfb38 => *b"RFB 003.008\n",
        }
    }

    /// Whether a `SecurityResult` follows a successful handshake.
    ///
    /// Carried as state rather than re-derived at the call site: under 3.3 a
    /// `None` handshake goes **straight** to ClientInit, and reading four bytes
    /// that were never sent hangs the connection until the read timeout with no
    /// useful error.
    pub fn has_security_result(&self, security: SecurityType) -> bool {
        match self {
            ProtocolVersion::Rfb38 => true,
            ProtocolVersion::Rfb33 => security != SecurityType::None,
        }
    }
}

// ---------------------------------------------------------------------------
// Security types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecurityType {
    /// No authentication at all.
    None,
    /// The classic DES challenge-response.
    VncAuth,
    /// Apple Remote Desktop / macOS Screen Sharing: Diffie-Hellman + AES.
    Ard,
}

impl SecurityType {
    pub const NONE: u8 = 1;
    pub const VNC_AUTH: u8 = 2;
    pub const ARD: u8 = 30;

    pub fn to_u8(self) -> u8 {
        match self {
            Self::None => Self::NONE,
            Self::VncAuth => Self::VNC_AUTH,
            Self::Ard => Self::ARD,
        }
    }
}

/// A human name for a security type we do *not* implement.
///
/// Worth the table: "authentication failed" against a RealVNC server that only
/// offers RA2 sends the user hunting for a wrong password forever, when the
/// real answer is that TermDrop cannot speak to it at all.
pub fn security_type_name(value: u8) -> &'static str {
    match value {
        0 => "Invalid",
        1 => "None",
        2 => "VNC password",
        5 => "RA2 (RealVNC proprietary)",
        6 => "RA2ne (RealVNC proprietary)",
        16 => "Tight",
        18 => "TLS",
        19 => "VeNCrypt",
        30 => "Apple Screen Sharing",
        35 => "Apple Screen Sharing (variant)",
        _ => "unknown",
    }
}

// ---------------------------------------------------------------------------
// Encodings
// ---------------------------------------------------------------------------

pub mod encoding {
    pub const RAW: i32 = 0;
    pub const COPY_RECT: i32 = 1;
    pub const TIGHT: i32 = 7;
    pub const ZRLE: i32 = 16;

    pub const PSEUDO_DESKTOP_SIZE: i32 = -223;
    pub const PSEUDO_CURSOR: i32 = -239;
    pub const PSEUDO_LAST_RECT: i32 = -224;

    /// JPEG quality 0-9 maps to -32..=-23. Higher is prettier and slower for
    /// the *server* to produce.
    pub const fn jpeg_quality(level: u8) -> i32 {
        -32 + (level as i32)
    }
    /// zlib compression 0-9 maps to -256..=-247. This is the remote's CPU
    /// budget per rectangle, not ours.
    pub const fn compress_level(level: u8) -> i32 {
        -256 + (level as i32)
    }
}

/// True encodings, which a rectangle can actually arrive as.
///
/// **Every entry must have a decoder**, because a server only ever sends what
/// the client advertised — that is the whole reason Hextile, RRE, CoRRE and
/// TRLE are absent, and it removes four decoders from the codebase at no
/// compatibility cost. Raw is mandatory for every server and is the universal
/// fallback.
///
/// `encoding::tests::every_encoding_we_advertise_has_a_decoder` pins the pairing.
pub fn supported_encodings() -> Vec<i32> {
    vec![
        encoding::TIGHT,
        encoding::ZRLE,
        encoding::COPY_RECT,
        encoding::RAW,
    ]
}

/// Pseudo-encodings: markers and settings rather than pixel payloads.
///
/// `DesktopSize` and `LastRect` do arrive in the rectangle stream and are
/// handled there; the quality and compression hints only tell the server how to
/// encode and never come back.
///
/// **The two levels are tuned for latency, not for bandwidth.** They are the
/// remote's CPU knobs: `compress_level` is the zlib effort it spends on every
/// ZRLE and Tight rectangle, and `jpeg_quality` the effort on every photographic
/// one. Measured against macOS Screen Sharing, a window drag spent ~300ms per
/// frame inside the server before a single byte came back — so asking it to
/// compress harder buys smaller frames at the cost of the thing that is
/// actually scarce.
///
/// Level 1 rather than 6, and quality 6 rather than 8. On a local network the
/// extra bytes cost nothing measurable; over a slow link the trade reverses,
/// which is what a per-host quality control would be for.
pub fn pseudo_encodings() -> Vec<i32> {
    vec![
        encoding::PSEUDO_DESKTOP_SIZE,
        encoding::PSEUDO_CURSOR,
        encoding::PSEUDO_LAST_RECT,
        encoding::jpeg_quality(6),
        encoding::compress_level(1),
    ]
}

/// What we advertise, in descending order of preference.
pub fn preferred_encodings() -> Vec<i32> {
    let mut all = supported_encodings();
    all.extend(pseudo_encodings());
    all
}

// ---------------------------------------------------------------------------
// ServerInit
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerInit {
    pub width: u16,
    pub height: u16,
    pub pixel_format: PixelFormat,
    pub name: String,
}

impl ServerInit {
    /// Everything but the desktop name, which is length-prefixed and so needs a
    /// second read.
    pub const FIXED_LEN: usize = 24;

    /// The byte length of the desktop name that follows the fixed part.
    pub fn name_len(fixed: &[u8]) -> Result<usize, String> {
        if fixed.len() < Self::FIXED_LEN {
            return Err(format!("truncated ServerInit: {} bytes", fixed.len()));
        }
        Ok(u32::from_be_bytes([fixed[20], fixed[21], fixed[22], fixed[23]]) as usize)
    }

    pub fn parse(fixed: &[u8], name: &[u8]) -> Result<Self, String> {
        if fixed.len() < Self::FIXED_LEN {
            return Err(format!("truncated ServerInit: {} bytes", fixed.len()));
        }
        Ok(Self {
            width: u16::from_be_bytes([fixed[0], fixed[1]]),
            height: u16::from_be_bytes([fixed[2], fixed[3]]),
            pixel_format: PixelFormat::parse(&fixed[4..20])?,
            // Nominally ISO-8859-1. Servers are inconsistent and many send
            // UTF-8, so try that first and fall back rather than mangling a
            // name that was already correct.
            name: match std::str::from_utf8(name) {
                Ok(s) => s.to_string(),
                Err(_) => name.iter().map(|&b| b as char).collect(),
            },
        })
    }
}

// ---------------------------------------------------------------------------
// Client messages
// ---------------------------------------------------------------------------

pub mod client_message {
    pub const SET_PIXEL_FORMAT: u8 = 0;
    pub const SET_ENCODINGS: u8 = 2;
    pub const FRAMEBUFFER_UPDATE_REQUEST: u8 = 3;
    pub const KEY_EVENT: u8 = 4;
    pub const POINTER_EVENT: u8 = 5;
    pub const CLIENT_CUT_TEXT: u8 = 6;
}

pub mod server_message {
    pub const FRAMEBUFFER_UPDATE: u8 = 0;
    pub const SET_COLOUR_MAP_ENTRIES: u8 = 1;
    pub const BELL: u8 = 2;
    pub const SERVER_CUT_TEXT: u8 = 3;
}

/// `ClientInit`. The flag asks the server to let other clients stay connected;
/// we always share, because kicking someone off their own desktop without
/// asking is not a thing a terminal app should do by default.
pub fn encode_client_init(shared: bool) -> Vec<u8> {
    vec![u8::from(shared)]
}

pub fn encode_set_pixel_format(format: &PixelFormat) -> Vec<u8> {
    let mut out = Vec::with_capacity(20);
    out.push(client_message::SET_PIXEL_FORMAT);
    out.extend_from_slice(&[0, 0, 0]); // padding
    out.extend_from_slice(&format.encode());
    out
}

pub fn encode_set_encodings(encodings: &[i32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(4 + encodings.len() * 4);
    out.push(client_message::SET_ENCODINGS);
    out.push(0); // padding
    out.extend_from_slice(&(encodings.len() as u16).to_be_bytes());
    for e in encodings {
        out.extend_from_slice(&e.to_be_bytes());
    }
    out
}

pub fn encode_framebuffer_update_request(
    incremental: bool,
    x: u16,
    y: u16,
    width: u16,
    height: u16,
) -> Vec<u8> {
    let mut out = Vec::with_capacity(10);
    out.push(client_message::FRAMEBUFFER_UPDATE_REQUEST);
    out.push(u8::from(incremental));
    out.extend_from_slice(&x.to_be_bytes());
    out.extend_from_slice(&y.to_be_bytes());
    out.extend_from_slice(&width.to_be_bytes());
    out.extend_from_slice(&height.to_be_bytes());
    out
}

pub fn encode_key_event(down: bool, keysym: u32) -> Vec<u8> {
    let mut out = Vec::with_capacity(8);
    out.push(client_message::KEY_EVENT);
    out.push(u8::from(down));
    out.extend_from_slice(&[0, 0]); // padding
    out.extend_from_slice(&keysym.to_be_bytes());
    out
}

pub fn encode_pointer_event(button_mask: u8, x: u16, y: u16) -> Vec<u8> {
    let mut out = Vec::with_capacity(6);
    out.push(client_message::POINTER_EVENT);
    out.push(button_mask);
    out.extend_from_slice(&x.to_be_bytes());
    out.extend_from_slice(&y.to_be_bytes());
    out
}

/// Encode text for `ClientCutText`, reporting whether anything was lost.
///
/// The base protocol's cut text is **ISO-8859-1**, full stop — there is no
/// length-prefixed UTF-8 form without the extended-clipboard pseudo-encoding,
/// which we do not negotiate. Rather than silently mangling, the caller is told
/// so it can say so once.
pub fn encode_client_cut_text(text: &str) -> (Vec<u8>, bool) {
    let mut lossy = false;
    let latin1: Vec<u8> = text
        .chars()
        // A bare CR would arrive as a second line break on the remote side;
        // the protocol specifies LF-only line endings for cut text.
        .filter(|&c| c != '\r')
        .map(|c| {
            if (c as u32) <= 0xFF {
                c as u8
            } else {
                lossy = true;
                b'?'
            }
        })
        .collect();

    let mut out = Vec::with_capacity(8 + latin1.len());
    out.push(client_message::CLIENT_CUT_TEXT);
    out.extend_from_slice(&[0, 0, 0]); // padding
    out.extend_from_slice(&(latin1.len() as u32).to_be_bytes());
    out.extend_from_slice(&latin1);
    (out, lossy)
}

/// Decode `ServerCutText`, which is ISO-8859-1 in the other direction.
pub fn decode_server_cut_text(bytes: &[u8]) -> String {
    bytes.iter().map(|&b| b as char).collect()
}

// ---------------------------------------------------------------------------
// Server messages
// ---------------------------------------------------------------------------

/// The header of one rectangle inside a `FramebufferUpdate`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RectHeader {
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
    pub encoding: i32,
}

impl RectHeader {
    pub const WIRE_LEN: usize = 12;

    pub fn parse(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() < Self::WIRE_LEN {
            return Err(format!("truncated rectangle header: {} bytes", bytes.len()));
        }
        Ok(Self {
            x: u16::from_be_bytes([bytes[0], bytes[1]]),
            y: u16::from_be_bytes([bytes[2], bytes[3]]),
            width: u16::from_be_bytes([bytes[4], bytes[5]]),
            height: u16::from_be_bytes([bytes[6], bytes[7]]),
            encoding: i32::from_be_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // -- protocol version ---------------------------------------------------

    #[test]
    fn negotiates_the_common_server_greetings() {
        for (greeting, expected) in [
            (b"RFB 003.008\n", ProtocolVersion::Rfb38),
            (b"RFB 003.889\n", ProtocolVersion::Rfb38), // Apple's odd minor
            (b"RFB 003.007\n", ProtocolVersion::Rfb33),
            (b"RFB 003.003\n", ProtocolVersion::Rfb33),
        ] {
            assert_eq!(
                ProtocolVersion::negotiate(greeting).unwrap(),
                expected,
                "greeting {:?}",
                std::str::from_utf8(greeting).unwrap()
            );
        }
    }

    #[test]
    fn a_non_vnc_server_is_named_as_such_rather_than_failing_obscurely() {
        // Pointing a VNC tab at an HTTP port is an easy mistake, and "invalid
        // UTF-8" or "parse error" would send the user looking in the wrong
        // place entirely.
        let err = ProtocolVersion::negotiate(b"HTTP/1.1 200").unwrap_err();
        assert!(err.contains("not a VNC server"), "unhelpful error: {}", err);

        let err = ProtocolVersion::negotiate(b"RFB 004.000\n").unwrap_err();
        assert!(err.contains("unsupported"), "unhelpful error: {}", err);
    }

    #[test]
    fn a_truncated_greeting_is_an_error_not_a_panic() {
        assert!(ProtocolVersion::negotiate(b"RFB 003").is_err());
        assert!(ProtocolVersion::negotiate(b"").is_err());
    }

    #[test]
    fn rfb_33_sends_no_security_result_for_none_but_does_for_a_password() {
        // The trap: reading four bytes that were never sent hangs until the
        // read timeout, with nothing in the error to say why.
        let v33 = ProtocolVersion::Rfb33;
        assert!(!v33.has_security_result(SecurityType::None));
        assert!(v33.has_security_result(SecurityType::VncAuth));

        // 3.8 always sends one.
        let v38 = ProtocolVersion::Rfb38;
        assert!(v38.has_security_result(SecurityType::None));
        assert!(v38.has_security_result(SecurityType::VncAuth));
    }

    #[test]
    fn version_bytes_are_exactly_twelve_and_round_trip() {
        for v in [ProtocolVersion::Rfb33, ProtocolVersion::Rfb38] {
            let bytes = v.bytes();
            assert_eq!(bytes.len(), ProtocolVersion::WIRE_LEN);
            assert_eq!(ProtocolVersion::negotiate(&bytes).unwrap(), v);
        }
    }

    // -- security types -----------------------------------------------------

    #[test]
    fn security_types_have_the_numbers_the_protocol_assigns() {
        // These go on the wire verbatim. A wrong one is answered with a
        // challenge of the wrong shape, which the server reports as an
        // authentication failure -- sending the user after their password.
        assert_eq!(SecurityType::None.to_u8(), 1);
        assert_eq!(SecurityType::VncAuth.to_u8(), 2);
        assert_eq!(SecurityType::Ard.to_u8(), 30);
    }

    #[test]
    fn unsupported_security_types_have_names_a_user_can_act_on() {
        assert_eq!(security_type_name(5), "RA2 (RealVNC proprietary)");
        assert_eq!(security_type_name(19), "VeNCrypt");
        assert_eq!(security_type_name(200), "unknown");
    }

    // -- encodings ----------------------------------------------------------

    #[test]
    fn preferred_encodings_omit_the_ones_we_have_no_decoder_for() {
        let list = preferred_encodings();
        // A server only sends what we advertise, so these absences are what
        // keep four unwritten decoders from ever being needed.
        for (value, name) in [(2, "RRE"), (4, "CoRRE"), (5, "Hextile"), (15, "TRLE")] {
            assert!(
                !list.contains(&value),
                "{} must not be advertised without a decoder",
                name
            );
        }
        assert!(
            list.contains(&encoding::RAW),
            "Raw is the mandatory fallback"
        );
    }

    #[test]
    fn the_two_halves_of_the_advertised_list_do_not_overlap() {
        // Only `supported_encodings` needs a decoder; conflating the two would
        // make that check either vacuous or wrong.
        for pseudo in pseudo_encodings() {
            assert!(
                !supported_encodings().contains(&pseudo),
                "{} is in both lists",
                pseudo
            );
        }
        assert!(pseudo_encodings().iter().all(|&e| e < 0));
        assert!(supported_encodings().iter().all(|&e| e >= 0));
    }

    #[test]
    fn pseudo_encoding_levels_land_in_their_specified_ranges() {
        assert_eq!(encoding::jpeg_quality(0), -32);
        assert_eq!(encoding::jpeg_quality(9), -23);
        assert_eq!(encoding::compress_level(0), -256);
        assert_eq!(encoding::compress_level(9), -247);
    }

    // -- client messages ----------------------------------------------------

    #[test]
    fn set_pixel_format_is_twenty_bytes_with_the_padding_in_place() {
        let msg = encode_set_pixel_format(&PixelFormat::PREFERRED);
        assert_eq!(msg.len(), 20);
        assert_eq!(msg[0], client_message::SET_PIXEL_FORMAT);
        assert_eq!(&msg[1..4], &[0, 0, 0], "three padding bytes");
        assert_eq!(&msg[4..], &PixelFormat::PREFERRED.encode());
    }

    #[test]
    fn set_encodings_writes_a_big_endian_count_then_signed_values() {
        let msg = encode_set_encodings(&[encoding::TIGHT, encoding::PSEUDO_CURSOR]);
        assert_eq!(msg.len(), 4 + 2 * 4);
        assert_eq!(msg[0], client_message::SET_ENCODINGS);
        assert_eq!(msg[1], 0, "padding");
        assert_eq!(&msg[2..4], &2u16.to_be_bytes(), "count is big-endian");
        assert_eq!(&msg[4..8], &7i32.to_be_bytes());
        // The negative pseudo-encoding must survive as two's complement.
        assert_eq!(&msg[8..12], &(-239i32).to_be_bytes());
    }

    #[test]
    fn framebuffer_update_request_encodes_incremental_and_the_rectangle() {
        let msg = encode_framebuffer_update_request(true, 0, 0, 1920, 1080);
        assert_eq!(msg.len(), 10);
        assert_eq!(msg[0], client_message::FRAMEBUFFER_UPDATE_REQUEST);
        assert_eq!(msg[1], 1, "incremental");
        assert_eq!(&msg[6..8], &1920u16.to_be_bytes());
        assert_eq!(&msg[8..10], &1080u16.to_be_bytes());

        // A full refresh differs only in that byte, and is what we send after a
        // desktop resize.
        assert_eq!(encode_framebuffer_update_request(false, 0, 0, 1, 1)[1], 0);
    }

    #[test]
    fn key_event_carries_a_full_u32_keysym() {
        // Unicode keysyms live above 0x01000000, so a u16 field here would
        // silently break every non-Latin-1 character.
        let msg = encode_key_event(true, 0x0100_20AC); // euro sign
        assert_eq!(msg.len(), 8);
        assert_eq!(msg[0], client_message::KEY_EVENT);
        assert_eq!(msg[1], 1, "down");
        assert_eq!(&msg[2..4], &[0, 0], "padding");
        assert_eq!(&msg[4..8], &0x0100_20ACu32.to_be_bytes());

        assert_eq!(encode_key_event(false, 0xFF0D)[1], 0, "up");
    }

    #[test]
    fn pointer_event_is_six_bytes_of_mask_and_position() {
        let msg = encode_pointer_event(0b101, 800, 600);
        assert_eq!(msg.len(), 6);
        assert_eq!(msg[0], client_message::POINTER_EVENT);
        assert_eq!(msg[1], 0b101, "left + right held");
        assert_eq!(&msg[2..4], &800u16.to_be_bytes());
        assert_eq!(&msg[4..6], &600u16.to_be_bytes());
    }

    #[test]
    fn cut_text_encodes_latin1_and_reports_what_it_could_not() {
        let (msg, lossy) = encode_client_cut_text("caf\u{e9}");
        assert!(!lossy, "e-acute is inside Latin-1");
        assert_eq!(msg[0], client_message::CLIENT_CUT_TEXT);
        assert_eq!(&msg[1..4], &[0, 0, 0], "padding");
        assert_eq!(&msg[4..8], &4u32.to_be_bytes(), "four Latin-1 bytes");
        assert_eq!(&msg[8..], &[b'c', b'a', b'f', 0xE9]);

        // Outside Latin-1 there is nowhere to put it in the base protocol, so
        // the caller is told rather than left to wonder.
        let (msg, lossy) = encode_client_cut_text("a\u{20ac}b");
        assert!(lossy, "the euro sign cannot be represented");
        assert_eq!(&msg[8..], b"a?b");
    }

    #[test]
    fn cut_text_strips_carriage_returns() {
        // Left in, a CRLF document pastes into the remote with doubled line
        // breaks; the protocol specifies LF-only.
        let (msg, _) = encode_client_cut_text("a\r\nb");
        assert_eq!(&msg[8..], b"a\nb");
        assert_eq!(&msg[4..8], &3u32.to_be_bytes());
    }

    #[test]
    fn server_cut_text_decodes_latin1_high_bytes() {
        assert_eq!(
            decode_server_cut_text(&[b'c', b'a', b'f', 0xE9]),
            "caf\u{e9}"
        );
    }

    #[test]
    fn client_init_asks_to_share_the_desktop() {
        // Not sharing disconnects whoever else is on that desktop, which is a
        // hostile default for a tool people point at shared machines.
        assert_eq!(encode_client_init(true), vec![1]);
        assert_eq!(encode_client_init(false), vec![0]);
    }

    // -- server messages ----------------------------------------------------

    #[test]
    fn server_init_parses_size_format_and_name() {
        let mut fixed = Vec::new();
        fixed.extend_from_slice(&1280u16.to_be_bytes());
        fixed.extend_from_slice(&800u16.to_be_bytes());
        fixed.extend_from_slice(&PixelFormat::PREFERRED.encode());
        fixed.extend_from_slice(&7u32.to_be_bytes());

        assert_eq!(ServerInit::name_len(&fixed).unwrap(), 7);
        let init = ServerInit::parse(&fixed, b"kiosk-1").unwrap();
        assert_eq!(init.width, 1280);
        assert_eq!(init.height, 800);
        assert_eq!(init.pixel_format, PixelFormat::PREFERRED);
        assert_eq!(init.name, "kiosk-1");
    }

    #[test]
    fn server_init_accepts_a_utf8_name_and_falls_back_to_latin1() {
        let mut fixed = vec![0u8; ServerInit::FIXED_LEN];
        fixed[4..20].copy_from_slice(&PixelFormat::PREFERRED.encode());

        // Most servers send UTF-8 in practice despite the spec saying Latin-1.
        let init = ServerInit::parse(&fixed, "b\u{fc}ro".as_bytes()).unwrap();
        assert_eq!(init.name, "b\u{fc}ro");

        // A genuine Latin-1 name is not valid UTF-8 and must not be mangled.
        let init = ServerInit::parse(&fixed, &[b'b', 0xFC, b'r', b'o']).unwrap();
        assert_eq!(init.name, "b\u{fc}ro");
    }

    #[test]
    fn server_init_rejects_a_short_buffer() {
        assert!(ServerInit::parse(&[0u8; 23], b"").is_err());
        assert!(ServerInit::name_len(&[0u8; 23]).is_err());
    }

    #[test]
    fn rect_header_parses_a_negative_pseudo_encoding() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&10u16.to_be_bytes());
        bytes.extend_from_slice(&20u16.to_be_bytes());
        bytes.extend_from_slice(&64u16.to_be_bytes());
        bytes.extend_from_slice(&48u16.to_be_bytes());
        bytes.extend_from_slice(&encoding::PSEUDO_DESKTOP_SIZE.to_be_bytes());

        let rect = RectHeader::parse(&bytes).unwrap();
        assert_eq!(
            rect,
            RectHeader {
                x: 10,
                y: 20,
                width: 64,
                height: 48,
                encoding: encoding::PSEUDO_DESKTOP_SIZE,
            }
        );
        assert_ne!(rect.encoding, encoding::PSEUDO_LAST_RECT);
    }

    #[test]
    fn last_rect_parses_as_its_own_encoding() {
        // A server that advertises LastRect may send `rect_count = 0xFFFF` and
        // stream rectangles until this one arrives, so the count alone is not
        // trustworthy. The encoding routing turns it into a marker.
        let mut bytes = vec![0u8; 8];
        bytes.extend_from_slice(&encoding::PSEUDO_LAST_RECT.to_be_bytes());
        assert_eq!(
            RectHeader::parse(&bytes).unwrap().encoding,
            encoding::PSEUDO_LAST_RECT
        );
    }

    #[test]
    fn rect_header_rejects_a_short_buffer() {
        assert!(RectHeader::parse(&[0u8; 11]).is_err());
    }
}
