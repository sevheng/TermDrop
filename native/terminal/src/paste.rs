//! Turning clipboard text into what is written to the terminal.

/// Pastes larger than this go out in chunks; smaller ones in one write.
/// Both numbers match `TerminalTab.vue`, which settled on them because one
/// huge write overran slow remote line disciplines.
pub const PASTE_CHUNK_THRESHOLD: usize = 50;
pub const PASTE_CHUNK_SIZE: usize = 512;

const BRACKET_START: &str = "\x1b[200~";
const BRACKET_END: &str = "\x1b[201~";

/// The bytes a paste sends. Line endings become CR, as a typed Enter would.
/// With bracketed paste on, the text is wrapped in the 200~/201~ markers and
/// any marker inside it is removed, so pasted text cannot end the bracket
/// early and have the rest run as typed commands.
pub fn paste_payload(text: &str, bracketed: bool) -> Vec<u8> {
    let normalized = text.replace("\r\n", "\r").replace('\n', "\r");
    if bracketed {
        // Repeat until stable: removing one marker can join the pieces
        // around it into another.
        let mut inner = normalized;
        while inner.contains(BRACKET_END) || inner.contains(BRACKET_START) {
            inner = inner.replace(BRACKET_END, "").replace(BRACKET_START, "");
        }
        format!("{BRACKET_START}{inner}{BRACKET_END}").into_bytes()
    } else {
        normalized.into_bytes()
    }
}

/// Splits a payload into writes of at most [`PASTE_CHUNK_SIZE`] bytes,
/// never inside a UTF-8 character. Payloads at or under
/// [`PASTE_CHUNK_THRESHOLD`] stay whole.
pub fn paste_chunks(payload: &[u8]) -> Vec<Vec<u8>> {
    if payload.len() <= PASTE_CHUNK_THRESHOLD {
        return vec![payload.to_vec()];
    }
    let mut chunks = Vec::with_capacity(payload.len() / PASTE_CHUNK_SIZE + 1);
    let mut start = 0;
    while start < payload.len() {
        let mut end = (start + PASTE_CHUNK_SIZE).min(payload.len());
        // Back up over UTF-8 continuation bytes (10xxxxxx).
        while end < payload.len() && end > start && (payload[end] & 0xC0) == 0x80 {
            end -= 1;
        }
        if end == start {
            end = (start + PASTE_CHUNK_SIZE).min(payload.len());
        }
        chunks.push(payload[start..end].to_vec());
        start = end;
    }
    chunks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn newlines_become_carriage_returns() {
        assert_eq!(paste_payload("a\nb\r\nc", false), b"a\rb\rc");
    }

    #[test]
    fn bracketed_paste_wraps_and_strips_markers() {
        assert_eq!(paste_payload("ls\n", true), b"\x1b[200~ls\r\x1b[201~");
        let hostile = "echo hi\x1b[201~rm -rf ~\n";
        assert_eq!(
            paste_payload(hostile, true),
            b"\x1b[200~echo hirm -rf ~\r\x1b[201~"
        );
    }

    #[test]
    fn small_pastes_are_one_write() {
        assert_eq!(paste_chunks(b"hello"), vec![b"hello".to_vec()]);
    }

    #[test]
    fn large_pastes_are_chunked_at_512() {
        let data = vec![b'x'; 1300];
        let chunks = paste_chunks(&data);
        assert_eq!(
            chunks.iter().map(Vec::len).collect::<Vec<_>>(),
            [512, 512, 276]
        );
        assert_eq!(chunks.concat(), data);
    }

    #[test]
    fn chunks_never_split_a_character() {
        // 511 ASCII bytes then a 3-byte character straddling the boundary.
        let mut text = "a".repeat(511);
        text.push('€');
        text.push_str(&"b".repeat(600));
        let chunks = paste_chunks(text.as_bytes());
        assert_eq!(chunks[0].len(), 511);
        for chunk in &chunks {
            assert!(std::str::from_utf8(chunk).is_ok());
            assert!(chunk.len() <= PASTE_CHUNK_SIZE);
        }
        assert_eq!(chunks.concat(), text.as_bytes());
    }
}
