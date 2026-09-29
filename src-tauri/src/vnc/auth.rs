//! RFB authentication: choosing a security type and answering its challenge.
//!
//! Three types are implemented — `None`, the classic VNC password, and Apple's
//! Diffie-Hellman scheme used by macOS Screen Sharing. Everything here is pure:
//! the challenge bytes come in as arguments and the response comes back as a
//! value, so the whole of it is testable without a server.
//!
//! Both real schemes have a detail that is easy to get subtly wrong and
//! impossible to debug from the far end, because a wrong answer and a wrong
//! password produce the identical "authentication failed" from the server. Each
//! is called out at its function.

use aes::Aes128;
use cipher::generic_array::GenericArray;
use cipher::{BlockEncrypt, KeyInit};
use des::Des;
use md5::{Digest, Md5};
use num_bigint::BigUint;
use rand::rngs::OsRng;
use rand::RngCore;

use super::proto::{security_type_name, SecurityType};

/// Pick the security type to use from the list the server offered.
///
/// Preference runs strongest-first: Apple's scheme encrypts the credentials
/// with a key neither end transmits, the VNC password at least does not send
/// the password in clear, and `None` is whatever is left.
///
/// ARD is only preferred when a username is configured, because the scheme has
/// nowhere to put an absent one — a Mac will simply reject the block. If the
/// server offers nothing else, that is worth saying plainly rather than letting
/// the user retype a password that was never the problem.
pub fn select_security_type(offered: &[u8], have_username: bool) -> Result<SecurityType, String> {
    if offered.is_empty() {
        return Err("the server offered no security types".to_string());
    }
    let has = |t: u8| offered.contains(&t);

    if has(SecurityType::ARD) && have_username {
        return Ok(SecurityType::Ard);
    }
    if has(SecurityType::VNC_AUTH) {
        return Ok(SecurityType::VncAuth);
    }
    if has(SecurityType::NONE) {
        return Ok(SecurityType::None);
    }
    if has(SecurityType::ARD) {
        return Err(
            "this server uses Apple Screen Sharing authentication, which needs a user name. \
             Add one to this host's VNC settings."
                .to_string(),
        );
    }

    // Naming what was actually offered is the difference between a user fixing
    // their config and a user retyping a correct password all afternoon.
    let names: Vec<String> = offered
        .iter()
        .map(|&t| format!("{} ({})", security_type_name(t), t))
        .collect();
    Err(format!(
        "no supported authentication method: this server offers {}. \
         TermDrop supports None, VNC password and Apple Screen Sharing.",
        names.join(", ")
    ))
}

// ---------------------------------------------------------------------------
// VNC password (security type 2)
// ---------------------------------------------------------------------------

/// Turn a password into the DES key the VNC challenge expects.
///
/// **Every byte's bits are reversed.** The original AT&T implementation fed the
/// password to a DES routine that consumed bits in the opposite order from the
/// one the standard specifies, and because the server had the same bug the
/// mistake was invisible. Thirty years of servers are now bug-compatible with
/// it, so reproducing it exactly is the only way to interoperate.
///
/// The key is the first 8 bytes of the password, NUL-padded if shorter and
/// silently truncated if longer — also part of the wire behaviour, not a
/// shortcut: a server given a 20-character password only ever checks 8 of them.
fn vnc_auth_key(password: &str) -> [u8; 8] {
    let mut key = [0u8; 8];
    for (slot, byte) in key.iter_mut().zip(password.bytes()) {
        *slot = byte.reverse_bits();
    }
    key
}

/// Answer a VNC-password challenge: DES-ECB each half of the 16-byte challenge.
pub fn vnc_auth_response(password: &str, challenge: &[u8; 16]) -> [u8; 16] {
    let cipher = Des::new(GenericArray::from_slice(&vnc_auth_key(password)));
    let mut response = *challenge;
    // Two independent ECB blocks — there is no chaining and no IV.
    for chunk in response.as_chunks_mut::<8>().0 {
        cipher.encrypt_block(GenericArray::from_mut_slice(chunk));
    }
    response
}

// ---------------------------------------------------------------------------
// Apple ARD (security type 30)
// ---------------------------------------------------------------------------

/// The credential block is a fixed 128 bytes regardless of how short the
/// username and password are — the remaining space is random padding, which is
/// what stops the ciphertext length from leaking their lengths.
pub const ARD_CREDENTIAL_LEN: usize = 128;
/// Where the password starts inside that block. The username occupies 0..64.
const ARD_PASSWORD_OFFSET: usize = 64;
/// Each field must leave room for its NUL terminator.
const ARD_FIELD_MAX: usize = ARD_PASSWORD_OFFSET - 1;

/// The Diffie-Hellman parameters the server sends before an ARD handshake.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArdParams {
    pub generator: u16,
    pub prime: Vec<u8>,
    pub peer_key: Vec<u8>,
}

impl ArdParams {
    /// The fixed-size preamble: generator and key length.
    pub const HEADER_LEN: usize = 4;

    /// Key length in bytes, read from the 4-byte preamble. The caller then
    /// reads `2 * key_len` more bytes for [`ArdParams::parse`].
    pub fn key_len(header: &[u8]) -> Result<usize, String> {
        if header.len() < Self::HEADER_LEN {
            return Err(format!("truncated ARD parameters: {} bytes", header.len()));
        }
        let key_len = u16::from_be_bytes([header[2], header[3]]) as usize;
        if key_len == 0 {
            return Err("the server sent a zero-length ARD key".to_string());
        }
        // A sane Diffie-Hellman modulus is 128-512 bytes. The cap exists
        // because this length drives an allocation and a modpow, both from
        // bytes we have not authenticated yet.
        if key_len > 1024 {
            return Err(format!("implausible ARD key length: {} bytes", key_len));
        }
        Ok(key_len)
    }

    pub fn parse(header: &[u8], body: &[u8]) -> Result<Self, String> {
        let key_len = Self::key_len(header)?;
        if body.len() < key_len * 2 {
            return Err(format!(
                "truncated ARD key material: expected {} bytes, got {}",
                key_len * 2,
                body.len()
            ));
        }
        Ok(Self {
            generator: u16::from_be_bytes([header[0], header[1]]),
            prime: body[..key_len].to_vec(),
            peer_key: body[key_len..key_len * 2].to_vec(),
        })
    }
}

/// Lay out the 128-byte credential block.
///
/// `padding` is the random fill the block starts as; the two fields are written
/// over it. It is a parameter rather than generated here so the layout can be
/// tested deterministically — the randomness is the caller's to supply.
fn ard_credential_block(
    username: &str,
    password: &str,
    padding: &[u8; ARD_CREDENTIAL_LEN],
) -> [u8; ARD_CREDENTIAL_LEN] {
    let mut block = *padding;
    for (offset, field) in [(0usize, username), (ARD_PASSWORD_OFFSET, password)] {
        // Truncate on a character boundary: slicing raw bytes at a fixed
        // length could split a multi-byte character and hand the server half
        // of one.
        let end = field.floor_char_boundary_compat(ARD_FIELD_MAX);
        let bytes = &field.as_bytes()[..end];
        block[offset..offset + bytes.len()].copy_from_slice(bytes);
        block[offset + bytes.len()] = 0;
    }
    block
}

/// `str::floor_char_boundary` is still unstable, so this is the same idea.
trait FloorCharBoundary {
    fn floor_char_boundary_compat(&self, index: usize) -> usize;
}

impl FloorCharBoundary for str {
    fn floor_char_boundary_compat(&self, index: usize) -> usize {
        if index >= self.len() {
            return self.len();
        }
        let mut i = index;
        while i > 0 && !self.is_char_boundary(i) {
            i -= 1;
        }
        i
    }
}

/// Left-pad a big-endian integer to exactly `len` bytes.
///
/// `BigUint::to_bytes_be` returns the minimal representation, so a shared
/// secret that happens to start with a zero byte comes back one byte short.
/// Feeding that straight to MD5 produces a different key from the server's
/// roughly one time in 256 — an intermittent auth failure that looks like a
/// flaky network.
fn pad_be(value: &BigUint, len: usize) -> Result<Vec<u8>, String> {
    let bytes = value.to_bytes_be();
    if bytes.len() > len {
        return Err("ARD key material overflowed the negotiated length".to_string());
    }
    let mut out = vec![0u8; len - bytes.len()];
    out.extend_from_slice(&bytes);
    Ok(out)
}

/// Build the ARD response, with the random material supplied by the caller.
///
/// Returns `ciphertext(128) || our_public_key(key_len)`, which is exactly what
/// goes on the wire.
pub fn ard_response_with(
    params: &ArdParams,
    username: &str,
    password: &str,
    private_key: &[u8],
    padding: &[u8; ARD_CREDENTIAL_LEN],
) -> Result<Vec<u8>, String> {
    let key_len = params.prime.len();
    let prime = BigUint::from_bytes_be(&params.prime);
    if prime == BigUint::from(0u8) {
        return Err("the server sent a zero ARD prime".to_string());
    }
    let generator = BigUint::from(params.generator);
    let private = BigUint::from_bytes_be(private_key);
    let peer = BigUint::from_bytes_be(&params.peer_key);

    // `modpow` here is not constant-time. That is acceptable for a one-shot
    // ephemeral exponent used once against a server we are about to hand a
    // password to anyway; it would not be for a long-lived key.
    let our_public = pad_be(&generator.modpow(&private, &prime), key_len)?;
    let shared = pad_be(&peer.modpow(&private, &prime), key_len)?;

    let aes_key = Md5::digest(&shared);
    let cipher = Aes128::new(GenericArray::from_slice(&aes_key));

    // AES-128-ECB over the whole block: eight independent blocks, no IV and no
    // padding, because the plaintext is already an exact multiple of 16.
    let mut ciphertext = ard_credential_block(username, password, padding);
    for chunk in ciphertext.as_chunks_mut::<16>().0 {
        cipher.encrypt_block(GenericArray::from_mut_slice(chunk));
    }

    let mut out = Vec::with_capacity(ARD_CREDENTIAL_LEN + key_len);
    out.extend_from_slice(&ciphertext);
    out.extend_from_slice(&our_public);
    Ok(out)
}

/// Build the ARD response, drawing the private key and padding from the OS.
pub fn ard_response(params: &ArdParams, username: &str, password: &str) -> Result<Vec<u8>, String> {
    let mut private_key = vec![0u8; params.prime.len()];
    OsRng.fill_bytes(&mut private_key);
    let mut padding = [0u8; ARD_CREDENTIAL_LEN];
    OsRng.fill_bytes(&mut padding);
    ard_response_with(params, username, password, &private_key, &padding)
}

// ---------------------------------------------------------------------------
// SecurityResult
// ---------------------------------------------------------------------------

/// Whether the four-byte `SecurityResult` says the handshake succeeded.
pub fn security_result_ok(bytes: &[u8]) -> Result<bool, String> {
    if bytes.len() < 4 {
        return Err(format!("truncated SecurityResult: {} bytes", bytes.len()));
    }
    Ok(u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) == 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use cipher::BlockDecrypt;

    // -- security type selection --------------------------------------------

    #[test]
    fn prefers_the_strongest_method_the_server_offers() {
        assert_eq!(
            select_security_type(&[1, 2], false).unwrap(),
            SecurityType::VncAuth,
            "a password beats no authentication at all"
        );
        assert_eq!(
            select_security_type(&[1], false).unwrap(),
            SecurityType::None
        );
        assert_eq!(
            select_security_type(&[30, 2], true).unwrap(),
            SecurityType::Ard,
            "Apple's scheme beats the DES challenge when a username exists"
        );
    }

    #[test]
    fn ard_is_skipped_when_no_username_is_configured() {
        // The scheme has nowhere to put an absent username, so falling back is
        // strictly better than sending a block the Mac will reject.
        assert_eq!(
            select_security_type(&[30, 2], false).unwrap(),
            SecurityType::VncAuth
        );

        // With nothing to fall back to, say what to fix.
        let err = select_security_type(&[30], false).unwrap_err();
        assert!(err.contains("user name"), "unhelpful error: {}", err);
    }

    #[test]
    fn an_unsupported_server_names_what_it_offered() {
        // Against a RealVNC server this is the difference between fixing the
        // config and retyping a correct password all afternoon.
        let err = select_security_type(&[5, 6], false).unwrap_err();
        assert!(err.contains("RA2"), "should name RA2: {}", err);
        assert!(err.contains('5'), "should give the numeric type: {}", err);

        assert!(select_security_type(&[], false).is_err());
    }

    // -- VNC password -------------------------------------------------------

    #[test]
    fn the_des_key_has_every_bit_reversed() {
        // The whole quirk, isolated. 'a' is 0x61 = 0b0110_0001, which reversed
        // is 0b1000_0110 = 0x86.
        assert_eq!(vnc_auth_key("a")[0], 0x86);
        // 0x01 -> 0x80 is the clearest possible demonstration.
        assert_eq!(vnc_auth_key("\u{1}")[0], 0x80);
    }

    #[test]
    fn the_des_key_is_nul_padded_and_truncated_to_eight_bytes() {
        let short = vnc_auth_key("ab");
        assert_eq!(&short[2..], &[0; 6], "padded with NUL, not repeated");

        // A server only ever checks eight characters, so this is wire
        // behaviour rather than a shortcut.
        assert_eq!(
            vnc_auth_key("abcdefgh"),
            vnc_auth_key("abcdefghIGNORED"),
            "characters past the eighth do not participate"
        );
    }

    #[test]
    fn the_response_is_the_challenge_encrypted_in_two_independent_blocks() {
        let challenge: [u8; 16] = std::array::from_fn(|i| i as u8);
        let response = vnc_auth_response("secret", &challenge);

        // Decrypting with the same mangled key must give the challenge back.
        // This pins the key derivation and the ECB block split together, which
        // is what a wrong answer would break.
        let cipher = Des::new(GenericArray::from_slice(&vnc_auth_key("secret")));
        let mut recovered = response;
        for chunk in recovered.as_chunks_mut::<8>().0 {
            cipher.decrypt_block(GenericArray::from_mut_slice(chunk));
        }
        assert_eq!(recovered, challenge);

        // ECB means identical halves encrypt identically — the property that
        // tells us there is no chaining and no IV.
        let repeated = [0xABu8; 16];
        let out = vnc_auth_response("secret", &repeated);
        assert_eq!(&out[..8], &out[8..], "the two blocks are independent");
    }

    #[test]
    fn a_different_password_gives_a_different_response() {
        let challenge = [0x5Au8; 16];
        assert_ne!(
            vnc_auth_response("hunter2", &challenge),
            vnc_auth_response("hunter3", &challenge)
        );
        // Deterministic: the same inputs must always answer the same way.
        assert_eq!(
            vnc_auth_response("hunter2", &challenge),
            vnc_auth_response("hunter2", &challenge)
        );
    }

    // -- ARD ----------------------------------------------------------------

    /// A small but real Diffie-Hellman group, so the maths is exercised without
    /// a 512-byte modpow in every test run.
    fn test_params(peer_key: Vec<u8>) -> ArdParams {
        ArdParams {
            generator: 2,
            // 2^127 - 1, a Mersenne prime.
            prime: {
                let mut p = vec![0x7F];
                p.extend_from_slice(&[0xFF; 15]);
                p
            },
            peer_key,
        }
    }

    #[test]
    fn ard_parameters_parse_and_reject_nonsense() {
        let header = [0, 2, 0, 4]; // generator 2, key length 4
        let body = [1, 2, 3, 4, 5, 6, 7, 8];
        let params = ArdParams::parse(&header, &body).unwrap();
        assert_eq!(params.generator, 2);
        assert_eq!(params.prime, vec![1, 2, 3, 4]);
        assert_eq!(params.peer_key, vec![5, 6, 7, 8]);

        // These drive an allocation and a modpow from unauthenticated bytes.
        assert!(ArdParams::key_len(&[0, 2, 0, 0]).is_err(), "zero length");
        assert!(
            ArdParams::key_len(&[0, 2, 0xFF, 0xFF]).is_err(),
            "implausible"
        );
        assert!(ArdParams::parse(&header, &[1, 2, 3]).is_err(), "truncated");
    }

    #[test]
    fn the_credential_block_puts_each_field_at_its_fixed_offset() {
        let padding = [0xAAu8; ARD_CREDENTIAL_LEN];
        let block = ard_credential_block("alice", "s3cret", &padding);

        assert_eq!(&block[..5], b"alice");
        assert_eq!(block[5], 0, "username is NUL-terminated");
        assert_eq!(block[6], 0xAA, "padding resumes after the terminator");

        assert_eq!(&block[64..70], b"s3cret");
        assert_eq!(block[70], 0, "password is NUL-terminated");
        assert_eq!(block.len(), ARD_CREDENTIAL_LEN);
    }

    #[test]
    fn the_credential_block_keeps_its_random_padding() {
        // The padding is what stops the ciphertext from leaking how long the
        // username and password are, so it must not be zeroed.
        let padding = [0xAAu8; ARD_CREDENTIAL_LEN];
        let block = ard_credential_block("a", "b", &padding);
        assert!(
            block[10..60].iter().all(|&b| b == 0xAA),
            "padding was overwritten"
        );
        assert!(block[100..].iter().all(|&b| b == 0xAA));
    }

    #[test]
    fn overlong_fields_are_truncated_without_splitting_a_character() {
        let padding = [0u8; ARD_CREDENTIAL_LEN];
        let long = "e".repeat(200);
        let block = ard_credential_block(&long, &long, &padding);
        assert_eq!(
            block[ARD_FIELD_MAX], 0,
            "username terminator stays in range"
        );
        assert_eq!(block[ARD_CREDENTIAL_LEN - 1], 0, "password terminator too");

        // A multi-byte character straddling the cut must not be halved.
        let multi = "\u{e9}".repeat(100); // two bytes each
        let block = ard_credential_block(&multi, "x", &padding);
        let name_end = block[..ARD_PASSWORD_OFFSET]
            .iter()
            .position(|&b| b == 0)
            .unwrap();
        assert!(
            std::str::from_utf8(&block[..name_end]).is_ok(),
            "truncation split a character"
        );
    }

    #[test]
    fn ard_agrees_on_the_same_shared_secret_as_the_server() {
        // Play the server's half: pick b, publish g^b, and check that the key
        // we derived from peer^a is the one it derives from our_pub^b.
        let server_private = BigUint::from(0x1234_5678_9ABC_DEF0u64);
        let prime = {
            let mut p = vec![0x7F];
            p.extend_from_slice(&[0xFF; 15]);
            BigUint::from_bytes_be(&p)
        };
        let generator = BigUint::from(2u8);
        let server_public = generator.modpow(&server_private, &prime);

        let params = test_params(pad_be(&server_public, 16).unwrap());
        let client_private = vec![
            0x0F, 0xED, 0xCB, 0xA9, 0x87, 0x65, 0x43, 0x21, 0x0F, 0xED, 0xCB, 0xA9, 0x87, 0x65,
            0x43, 0x21,
        ];
        let padding = [0x11u8; ARD_CREDENTIAL_LEN];

        let response =
            ard_response_with(&params, "alice", "s3cret", &client_private, &padding).unwrap();
        assert_eq!(
            response.len(),
            ARD_CREDENTIAL_LEN + params.prime.len(),
            "ciphertext then our public key"
        );

        // The server derives the key from our public half.
        let our_public = BigUint::from_bytes_be(&response[ARD_CREDENTIAL_LEN..]);
        let server_shared = our_public.modpow(&server_private, &prime);
        let server_key = Md5::digest(pad_be(&server_shared, 16).unwrap());

        // If the agreement holds, the server can read the credentials back.
        let cipher = Aes128::new(GenericArray::from_slice(&server_key));
        let mut plain = response[..ARD_CREDENTIAL_LEN].to_vec();
        for chunk in plain.as_chunks_mut::<16>().0 {
            cipher.decrypt_block(GenericArray::from_mut_slice(chunk));
        }
        assert_eq!(&plain[..5], b"alice", "server could not read the username");
        assert_eq!(
            &plain[64..70],
            b"s3cret",
            "server could not read the password"
        );
    }

    #[test]
    fn ard_left_pads_key_material_to_the_negotiated_length() {
        // A shared secret with a leading zero byte comes back from
        // `to_bytes_be` one byte short. Unpadded, that hashes to a different
        // key from the server's about one time in 256 — an intermittent
        // failure that reads as a flaky network.
        assert_eq!(pad_be(&BigUint::from(1u8), 4).unwrap(), vec![0, 0, 0, 1]);
        assert_eq!(pad_be(&BigUint::from(0u8), 2).unwrap(), vec![0, 0]);
        assert!(pad_be(&BigUint::from(0x1_0000u32), 2).is_err(), "overflow");
    }

    #[test]
    fn ard_response_is_random_per_call() {
        // Two calls must not produce the same bytes: the private key and the
        // padding are both freshly drawn, and a repeat would mean OsRng was
        // not actually consulted.
        let params = test_params(vec![0x03; 16]);
        let a = ard_response(&params, "alice", "s3cret").unwrap();
        let b = ard_response(&params, "alice", "s3cret").unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn ard_rejects_a_zero_prime_rather_than_panicking_in_modpow() {
        let params = ArdParams {
            generator: 2,
            prime: vec![0; 16],
            peer_key: vec![1; 16],
        };
        assert!(ard_response(&params, "a", "b").is_err());
    }

    // -- SecurityResult -----------------------------------------------------

    #[test]
    fn security_result_reads_zero_as_success() {
        assert!(security_result_ok(&[0, 0, 0, 0]).unwrap());
        assert!(!security_result_ok(&[0, 0, 0, 1]).unwrap());
        assert!(security_result_ok(&[0, 0, 0]).is_err());
    }
}
