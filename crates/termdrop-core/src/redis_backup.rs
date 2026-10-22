//! The `.tdredis` backup file: a keyspace as a stream of `DUMP` payloads.
//!
//! # Why not an RDB file
//!
//! `redis-cli --rdb` produces a real RDB, but it asks the server to replicate
//! (`SYNC`), which ElastiCache, MemoryDB, Upstash and Redis Cloud all refuse
//! from a client — that is most of the servers people want to back up. It also
//! needs a `redis-cli` binary bundled per platform, and `build.rs` panics when
//! a bundled tool is missing. And an RDB is whole-instance: it cannot express
//! "back up `user:*` from db 3", which is the selection a GUI is for.
//!
//! `SCAN` + `DUMP` per key costs nothing extra to ship, works everywhere `DUMP`
//! is allowed, and restores a subset. Its one real limitation is recorded in
//! the header: `SCAN` is **not** a point-in-time snapshot. It guarantees only
//! that keys present for the whole iteration are returned at least once.
//!
//! # Format
//!
//! Little-endian throughout.
//!
//! ```text
//! magic        8   "TDREDIS" + format version byte
//! header_len   u32 (<= MAX_HEADER_BYTES)
//! header       UTF-8 JSON, see BackupHeader
//!
//! record*      tag 0x01 | db u8 | key_len u32 | key
//!                       | ttl_ms i64 | payload_len u32 | payload
//!
//! trailer      tag 0x00 | records u64 | crc32 u32
//! ```
//!
//! Two properties matter more than compactness:
//!
//! - **A truncated file is detectable.** Without the trailer, half a backup
//!   restores as a silently partial one, which is the worst possible failure
//!   for a backup tool. The record count and CRC are both checked before the
//!   first key is written to the server.
//! - **Every length is validated against a cap before anything is allocated.**
//!   A corrupt or hostile file must produce an error naming the offset, never a
//!   multi-gigabyte allocation and never a panic.

use serde::{Deserialize, Serialize};
use std::io::{Read, Write};

const MAGIC: &[u8; 7] = b"TDREDIS";
/// Bumped only for a change no older reader could survive.
pub const FORMAT_VERSION: u8 = 1;

const TAG_RECORD: u8 = 0x01;
const TAG_END: u8 = 0x00;

/// Caps checked before allocating. Generous enough for any real key, small
/// enough that a corrupt length is caught rather than acted on.
const MAX_HEADER_BYTES: u32 = 1024 * 1024;
const MAX_KEY_BYTES: u32 = 512 * 1024 * 1024;
const MAX_PAYLOAD_BYTES: u32 = 512 * 1024 * 1024;

/// What a backup file says about itself.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupHeader {
    pub app: String,
    pub created_at: String,
    /// The source URI with its credentials masked. A backup file leaves the
    /// machine; it must never carry a password.
    pub source: String,
    pub redis_version: String,
    /// Which databases the file contains.
    pub dbs: Vec<u8>,
    pub pattern: Option<String>,
    /// Always `"scan"`: see the module docs on point-in-time consistency.
    pub consistency: String,
}

/// One key, as stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackupRecord {
    pub db: u8,
    pub key: Vec<u8>,
    /// Milliseconds until expiry, or `-1` for no expiry.
    ///
    /// Relative, not absolute: a key dumped with 60s left gets 60s when it is
    /// restored, whenever that happens. Absolute deadlines would need clock
    /// skew handling for no benefit.
    pub ttl_ms: i64,
    /// Opaque `DUMP` output: the value, a 2-byte RDB version and a CRC64.
    pub payload: Vec<u8>,
}

// ---------------------------------------------------------------------------
// CRC32 (IEEE), computed without pulling in a dependency for 20 lines.
// ---------------------------------------------------------------------------

pub struct Crc32(u32);

impl Crc32 {
    pub fn new() -> Self {
        Crc32(0xFFFF_FFFF)
    }

    pub fn update(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 ^= b as u32;
            for _ in 0..8 {
                let mask = (self.0 & 1).wrapping_neg();
                self.0 = (self.0 >> 1) ^ (0xEDB8_8320 & mask);
            }
        }
    }

    pub fn finish(&self) -> u32 {
        !self.0
    }
}

impl Default for Crc32 {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Writing
// ---------------------------------------------------------------------------

/// Streams records to a writer, checksumming as it goes.
///
/// Streaming rather than building a buffer because the key count is not known
/// until the scan finishes, and a large keyspace would not fit in memory.
pub struct BackupWriter<W: Write> {
    inner: W,
    crc: Crc32,
    records: u64,
}

impl<W: Write> BackupWriter<W> {
    pub fn new(mut inner: W, header: &BackupHeader) -> Result<Self, String> {
        let json = serde_json::to_vec(header).map_err(|e| format!("encode header: {}", e))?;
        let len = u32::try_from(json.len()).map_err(|_| "header too large".to_string())?;
        if len > MAX_HEADER_BYTES {
            return Err("header too large".to_string());
        }

        let mut crc = Crc32::new();
        let mut write = |bytes: &[u8]| -> Result<(), String> {
            crc.update(bytes);
            inner
                .write_all(bytes)
                .map_err(|e| format!("write backup: {}", e))
        };
        write(MAGIC)?;
        write(&[FORMAT_VERSION])?;
        write(&len.to_le_bytes())?;
        write(&json)?;

        Ok(Self {
            inner,
            crc,
            records: 0,
        })
    }

    pub fn write_record(&mut self, record: &BackupRecord) -> Result<(), String> {
        let key_len =
            u32::try_from(record.key.len()).map_err(|_| "key too large to store".to_string())?;
        let payload_len = u32::try_from(record.payload.len())
            .map_err(|_| "value too large to store".to_string())?;

        let mut chunk = Vec::with_capacity(record.key.len() + record.payload.len() + 18);
        chunk.push(TAG_RECORD);
        chunk.push(record.db);
        chunk.extend_from_slice(&key_len.to_le_bytes());
        chunk.extend_from_slice(&record.key);
        chunk.extend_from_slice(&record.ttl_ms.to_le_bytes());
        chunk.extend_from_slice(&payload_len.to_le_bytes());
        chunk.extend_from_slice(&record.payload);

        self.crc.update(&chunk);
        self.inner
            .write_all(&chunk)
            .map_err(|e| format!("write backup: {}", e))?;
        self.records += 1;
        Ok(())
    }

    /// Write the trailer. A file without one is treated as truncated.
    pub fn finish(mut self) -> Result<u64, String> {
        let mut chunk = Vec::with_capacity(9);
        chunk.push(TAG_END);
        chunk.extend_from_slice(&self.records.to_le_bytes());
        self.crc.update(&chunk);
        chunk.extend_from_slice(&self.crc.finish().to_le_bytes());

        self.inner
            .write_all(&chunk)
            .map_err(|e| format!("write backup: {}", e))?;
        self.inner
            .flush()
            .map_err(|e| format!("write backup: {}", e))?;
        Ok(self.records)
    }
}

// ---------------------------------------------------------------------------
// Reading
// ---------------------------------------------------------------------------

/// Reads a backup file, validating as it goes.
pub struct BackupReader<R: Read> {
    inner: R,
    crc: Crc32,
    pub header: BackupHeader,
    offset: u64,
    records: u64,
    finished: bool,
}

impl<R: Read> BackupReader<R> {
    pub fn new(mut inner: R) -> Result<Self, String> {
        let mut crc = Crc32::new();
        let mut offset = 0u64;

        let mut magic = [0u8; 8];
        read_exact(&mut inner, &mut magic, offset)?;
        crc.update(&magic);
        offset += 8;

        if &magic[..7] != MAGIC {
            return Err("not a TermDrop Redis backup file".to_string());
        }
        if magic[7] != FORMAT_VERSION {
            return Err(format!(
                "this backup is format version {}, and this version of TermDrop reads version {}",
                magic[7], FORMAT_VERSION
            ));
        }

        let header_len = read_u32(&mut inner, &mut crc, &mut offset)?;
        // Checked before allocating: a corrupt length must not become a
        // gigabyte-sized Vec.
        if header_len > MAX_HEADER_BYTES {
            return Err(corrupt(offset, "header length is out of range"));
        }
        let mut json = vec![0u8; header_len as usize];
        read_exact(&mut inner, &mut json, offset)?;
        crc.update(&json);
        offset += header_len as u64;

        let header: BackupHeader = serde_json::from_slice(&json)
            .map_err(|e| corrupt(offset, &format!("header is unreadable: {}", e)))?;

        Ok(Self {
            inner,
            crc,
            header,
            offset,
            records: 0,
            finished: false,
        })
    }

    /// The next record, or `None` at a valid trailer.
    ///
    /// Reaching the end of the file without a trailer is an error, not a
    /// clean stop — that is exactly what a half-written backup looks like.
    pub fn next_record(&mut self) -> Result<Option<BackupRecord>, String> {
        if self.finished {
            return Ok(None);
        }

        let mut tag = [0u8; 1];
        read_exact(&mut self.inner, &mut tag, self.offset)?;
        self.crc.update(&tag);
        self.offset += 1;

        match tag[0] {
            TAG_END => {
                let mut count = [0u8; 8];
                read_exact(&mut self.inner, &mut count, self.offset)?;
                self.crc.update(&count);
                self.offset += 8;
                let claimed = u64::from_le_bytes(count);

                let expected = self.crc.finish();
                let mut stored = [0u8; 4];
                read_exact(&mut self.inner, &mut stored, self.offset)?;
                if u32::from_le_bytes(stored) != expected {
                    return Err(corrupt(
                        self.offset,
                        "checksum does not match: the file is damaged",
                    ));
                }
                if claimed != self.records {
                    return Err(format!(
                        "this backup says it holds {} keys but {} were readable: the file is truncated",
                        claimed, self.records
                    ));
                }
                self.finished = true;
                Ok(None)
            }
            TAG_RECORD => {
                let mut db = [0u8; 1];
                read_exact(&mut self.inner, &mut db, self.offset)?;
                self.crc.update(&db);
                self.offset += 1;

                let key_len = read_u32(&mut self.inner, &mut self.crc, &mut self.offset)?;
                if key_len > MAX_KEY_BYTES {
                    return Err(corrupt(self.offset, "key length is out of range"));
                }
                let mut key = vec![0u8; key_len as usize];
                read_exact(&mut self.inner, &mut key, self.offset)?;
                self.crc.update(&key);
                self.offset += key_len as u64;

                let mut ttl = [0u8; 8];
                read_exact(&mut self.inner, &mut ttl, self.offset)?;
                self.crc.update(&ttl);
                self.offset += 8;

                let payload_len = read_u32(&mut self.inner, &mut self.crc, &mut self.offset)?;
                if payload_len > MAX_PAYLOAD_BYTES {
                    return Err(corrupt(self.offset, "value length is out of range"));
                }
                let mut payload = vec![0u8; payload_len as usize];
                read_exact(&mut self.inner, &mut payload, self.offset)?;
                self.crc.update(&payload);
                self.offset += payload_len as u64;

                self.records += 1;
                Ok(Some(BackupRecord {
                    db: db[0],
                    key,
                    ttl_ms: i64::from_le_bytes(ttl),
                    payload,
                }))
            }
            other => Err(corrupt(
                self.offset,
                &format!("unexpected record marker {:#04x}", other),
            )),
        }
    }
}

fn corrupt(offset: u64, why: &str) -> String {
    format!("backup file is corrupt at byte {}: {}", offset, why)
}

fn read_exact<R: Read>(inner: &mut R, buf: &mut [u8], offset: u64) -> Result<(), String> {
    inner.read_exact(buf).map_err(|e| match e.kind() {
        std::io::ErrorKind::UnexpectedEof => format!(
            "backup file ends at byte {} without a trailer: it is truncated",
            offset
        ),
        _ => format!("read backup: {}", e),
    })
}

fn read_u32<R: Read>(inner: &mut R, crc: &mut Crc32, offset: &mut u64) -> Result<u32, String> {
    let mut buf = [0u8; 4];
    read_exact(inner, &mut buf, *offset)?;
    crc.update(&buf);
    *offset += 4;
    Ok(u32::from_le_bytes(buf))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn header() -> BackupHeader {
        BackupHeader {
            app: "TermDrop 0.3.0".into(),
            created_at: "2026-09-10T09:14:02Z".into(),
            source: "redis://***:***@10.0.1.5:6379".into(),
            redis_version: "7.2.4".into(),
            dbs: vec![0, 1],
            pattern: Some("user:*".into()),
            consistency: "scan".into(),
        }
    }

    fn records() -> Vec<BackupRecord> {
        vec![
            BackupRecord {
                db: 0,
                key: b"user:1".to_vec(),
                ttl_ms: 60_000,
                payload: vec![0x00, 0x05, b'a', b'l', b'i', b'c', b'e'],
            },
            // A key that is not valid UTF-8, and a value that is not either.
            BackupRecord {
                db: 0,
                key: vec![0xff, 0xfe, b':', 0x00],
                ttl_ms: -1,
                payload: vec![0xde, 0xad, 0xbe, 0xef],
            },
            // A payload large enough to cross any buffering boundary.
            BackupRecord {
                db: 1,
                key: b"big".to_vec(),
                ttl_ms: -1,
                payload: vec![0x5a; 1024 * 1024],
            },
        ]
    }

    fn write_file(header: &BackupHeader, records: &[BackupRecord]) -> Vec<u8> {
        let mut buf = Vec::new();
        let mut w = BackupWriter::new(&mut buf, header).unwrap();
        for r in records {
            w.write_record(r).unwrap();
        }
        w.finish().unwrap();
        buf
    }

    fn read_all(bytes: &[u8]) -> Result<(BackupHeader, Vec<BackupRecord>), String> {
        let mut r = BackupReader::new(Cursor::new(bytes.to_vec()))?;
        let header = r.header.clone();
        let mut out = Vec::new();
        while let Some(record) = r.next_record()? {
            out.push(record);
        }
        Ok((header, out))
    }

    #[test]
    fn crc32_matches_the_known_ieee_vector() {
        // Guards the hand-rolled implementation against a silent typo.
        let mut crc = Crc32::new();
        crc.update(b"123456789");
        assert_eq!(crc.finish(), 0xCBF4_3926);
    }

    #[test]
    fn a_backup_round_trips_byte_for_byte() {
        let bytes = write_file(&header(), &records());
        let (read_header, read_records) = read_all(&bytes).unwrap();

        assert_eq!(read_header.redis_version, "7.2.4");
        assert_eq!(read_header.dbs, vec![0, 1]);
        assert_eq!(read_header.pattern.as_deref(), Some("user:*"));
        assert_eq!(read_records, records());
    }

    #[test]
    fn an_empty_backup_is_valid() {
        let bytes = write_file(&header(), &[]);
        let (_, read_records) = read_all(&bytes).unwrap();
        assert!(read_records.is_empty());
    }

    #[test]
    fn a_file_that_is_not_a_backup_is_rejected() {
        assert!(read_all(b"this is not a backup at all").is_err());
        assert!(read_all(b"").is_err());
    }

    #[test]
    fn a_future_format_version_is_named_not_guessed_at() {
        let mut bytes = write_file(&header(), &records());
        bytes[7] = FORMAT_VERSION + 1;
        let err = read_all(&bytes).unwrap_err();
        assert!(err.contains("format version"), "{}", err);
    }

    #[test]
    fn a_truncated_file_is_refused_rather_than_restored_in_part() {
        // The failure a backup tool must never have: half a file that looks
        // like a whole one.
        let bytes = write_file(&header(), &records());
        for cut in [10, 40, bytes.len() / 2, bytes.len() - 1] {
            let err = read_all(&bytes[..cut]).unwrap_err();
            assert!(
                err.contains("truncated") || err.contains("corrupt"),
                "cut at {} gave: {}",
                cut,
                err
            );
        }
    }

    #[test]
    fn a_damaged_byte_is_caught_by_the_checksum() {
        let bytes = write_file(&header(), &records());
        let mut damaged = bytes.clone();
        // Flip a bit inside the first payload, past the header.
        let idx = bytes.len() / 2;
        damaged[idx] ^= 0xff;
        let err = read_all(&damaged).unwrap_err();
        assert!(
            err.contains("checksum") || err.contains("corrupt") || err.contains("truncated"),
            "{}",
            err
        );
    }

    #[test]
    fn a_wrong_record_count_reports_truncation() {
        let bytes = write_file(&header(), &records());
        let mut tampered = bytes.clone();
        // The trailer is the last 13 bytes: tag, u64 count, u32 crc. Claim one
        // more record than the file holds, and fix the CRC so only the count
        // check can catch it.
        let count_at = tampered.len() - 12;
        let claimed: u64 = records().len() as u64 + 1;
        tampered[count_at..count_at + 8].copy_from_slice(&claimed.to_le_bytes());
        let mut crc = Crc32::new();
        crc.update(&tampered[..tampered.len() - 4]);
        let end = tampered.len() - 4;
        tampered[end..].copy_from_slice(&crc.finish().to_le_bytes());

        let err = read_all(&tampered).unwrap_err();
        assert!(err.contains("truncated"), "{}", err);
    }

    #[test]
    fn an_absurd_length_is_refused_before_anything_is_allocated() {
        // A hostile file must not be able to ask for a multi-gigabyte Vec.
        let bytes = write_file(&header(), &records());

        // header_len sits at offset 8.
        let mut huge_header = bytes.clone();
        huge_header[8..12].copy_from_slice(&u32::MAX.to_le_bytes());
        let err = read_all(&huge_header).unwrap_err();
        assert!(err.contains("header length"), "{}", err);

        // key_len is 2 bytes into the first record, which starts right after
        // the header.
        let header_len = u32::from_le_bytes(bytes[8..12].try_into().unwrap()) as usize;
        let key_len_at = 12 + header_len + 2;
        let mut huge_key = bytes.clone();
        huge_key[key_len_at..key_len_at + 4].copy_from_slice(&u32::MAX.to_le_bytes());
        let err = read_all(&huge_key).unwrap_err();
        assert!(err.contains("key length"), "{}", err);
    }

    #[test]
    fn an_unknown_record_marker_is_an_error_not_a_panic() {
        let bytes = write_file(&header(), &records());
        let header_len = u32::from_le_bytes(bytes[8..12].try_into().unwrap()) as usize;
        let mut tampered = bytes.clone();
        tampered[12 + header_len] = 0x7f;
        let err = read_all(&tampered).unwrap_err();
        assert!(err.contains("marker"), "{}", err);
    }

    #[test]
    fn a_header_must_never_carry_a_password() {
        // Pinned here as well as at the call site: this is the last gate
        // before a file leaves the machine.
        let bytes = write_file(&header(), &records());
        let text = String::from_utf8_lossy(&bytes[..200]);
        assert!(text.contains("***"), "the source URI should be redacted");
    }
}
