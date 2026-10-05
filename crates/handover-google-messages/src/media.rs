//! Bounded first-party media framing. Network and staging are separate.
//!
//! Independently observed in the public web client's SVb/TVb and PVb/QVb:
//! header [0, 15], 32 KiB encrypted chunks, nonce prefix, 128-bit GCM tag,
//! and five-byte authenticated data containing the final flag and index.

use aes_gcm::{
    Aes256Gcm, KeyInit, Nonce,
    aead::{Aead, Payload},
};
use rand::{RngCore, rngs::OsRng};
use zeroize::Zeroizing;

const HEADER: [u8; 2] = [0, 15];
const CHUNK_BYTES: usize = 32 * 1024;
const NONCE_BYTES: usize = 12;
const OVERHEAD: usize = NONCE_BYTES + 16;
const PLAIN_CHUNK_BYTES: usize = CHUNK_BYTES - OVERHEAD;
pub const MAX_MEDIA_BYTES: usize = handover_gmessages::staging::MAX_STAGED_BYTES as usize;
const MAX_ENCODED_BYTES: usize =
    2 + MAX_MEDIA_BYTES + MAX_MEDIA_BYTES.div_ceil(PLAIN_CHUNK_BYTES) * OVERHEAD;

/// Fixed categories contain no file contents, keys, or server data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaError {
    Empty,
    TooLarge,
    UnsupportedFraming,
    InvalidFraming,
    Authentication,
    Randomness,
}

impl std::fmt::Display for MediaError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "media {:?}", self)
    }
}

impl std::error::Error for MediaError {}

/// Encrypt one nonempty file. Each chunk gets a fresh OS-generated nonce.
/// The caller owns the key and must keep it below the normalized IPC model.
pub fn encrypt(bytes: &[u8], key: &[u8; 32]) -> Result<Zeroizing<Vec<u8>>, MediaError> {
    encrypt_with_nonce(bytes, key, |nonce| {
        OsRng
            .try_fill_bytes(nonce)
            .map_err(|_| MediaError::Randomness)
    })
}

fn encrypt_with_nonce(
    bytes: &[u8],
    key: &[u8; 32],
    mut nonce_source: impl FnMut(&mut [u8; NONCE_BYTES]) -> Result<(), MediaError>,
) -> Result<Zeroizing<Vec<u8>>, MediaError> {
    if bytes.is_empty() {
        return Err(MediaError::Empty);
    }
    if bytes.len() > MAX_MEDIA_BYTES {
        return Err(MediaError::TooLarge);
    }
    let cipher = Aes256Gcm::new(key.into());
    let count = bytes.len().div_ceil(PLAIN_CHUNK_BYTES);
    let mut output = Zeroizing::new(Vec::with_capacity(2 + bytes.len() + count * OVERHEAD));
    output.extend_from_slice(&HEADER);
    for (index, chunk) in bytes.chunks(PLAIN_CHUNK_BYTES).enumerate() {
        let mut nonce = [0; NONCE_BYTES];
        nonce_source(&mut nonce)?;
        let aad = chunk_aad(index, index + 1 == count);
        let encoded = Zeroizing::new(
            cipher
                .encrypt(
                    Nonce::from_slice(&nonce),
                    Payload {
                        msg: chunk,
                        aad: &aad,
                    },
                )
                .map_err(|_| MediaError::Authentication)?,
        );
        output.extend_from_slice(&nonce);
        output.extend_from_slice(&encoded);
    }
    Ok(output)
}

/// Authenticate the entire file before returning any plaintext. Reordered,
/// appended, or truncated chunks fail because index and finality are bound.
pub fn decrypt(bytes: &[u8], key: &[u8; 32]) -> Result<Zeroizing<Vec<u8>>, MediaError> {
    if bytes.len() > MAX_ENCODED_BYTES {
        return Err(MediaError::TooLarge);
    }
    if bytes.len() < 2 || bytes[..2] != HEADER {
        return Err(MediaError::UnsupportedFraming);
    }
    let body = &bytes[2..];
    if body.is_empty() || body.len() % CHUNK_BYTES <= OVERHEAD && body.len() % CHUNK_BYTES != 0 {
        return Err(MediaError::InvalidFraming);
    }
    let cipher = Aes256Gcm::new(key.into());
    let count = body.len().div_ceil(CHUNK_BYTES);
    let mut output = Zeroizing::new(Vec::with_capacity(body.len() - count * OVERHEAD));
    for (index, chunk) in body.chunks(CHUNK_BYTES).enumerate() {
        let aad = chunk_aad(index, index + 1 == count);
        let plain = Zeroizing::new(
            cipher
                .decrypt(
                    Nonce::from_slice(&chunk[..NONCE_BYTES]),
                    Payload {
                        msg: &chunk[NONCE_BYTES..],
                        aad: &aad,
                    },
                )
                .map_err(|_| MediaError::Authentication)?,
        );
        if output.len() + plain.len() > MAX_MEDIA_BYTES {
            return Err(MediaError::TooLarge);
        }
        output.extend_from_slice(&plain);
    }
    Ok(output)
}

fn chunk_aad(index: usize, final_chunk: bool) -> [u8; 5] {
    // The 50 MiB bound keeps the chunk index far below u32::MAX.
    let mut aad = [0; 5];
    aad[0] = u8::from(final_chunk);
    aad[1..].copy_from_slice(&(index as u32).to_be_bytes());
    aad
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_independent_webcrypto_fixture() {
        // Node WebCrypto AES-GCM, key [7;32], nonce [3;12], AAD [1,0,0,0,0].
        let hex = "000f0303030303030303030303036d9fcd67355e3b305a2d26388236df6a8444949ac6f0bd66856acbb3b477974d54cd21d0e4c8";
        let fixture: Vec<u8> = (0..hex.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
            .collect();
        let plain = b"Handover media fixture";
        assert_eq!(&*decrypt(&fixture, &[7; 32]).unwrap(), plain);
        let encoded = encrypt_with_nonce(plain, &[7; 32], |nonce| {
            nonce.fill(3);
            Ok(())
        })
        .unwrap();
        assert_eq!(*encoded, fixture);
    }

    #[test]
    fn handles_full_and_partial_chunk_boundaries() {
        for size in [
            1,
            PLAIN_CHUNK_BYTES - 1,
            PLAIN_CHUNK_BYTES,
            PLAIN_CHUNK_BYTES + 1,
            PLAIN_CHUNK_BYTES * 2,
        ] {
            let plain = vec![91; size];
            let encoded = encrypt(&plain, &[5; 32]).unwrap();
            assert_eq!(*decrypt(&encoded, &[5; 32]).unwrap(), plain);
        }
    }

    #[test]
    fn multi_chunk_encoding_matches_webcrypto_digest() {
        use sha2::{Digest, Sha256};
        let encoded = encrypt_with_nonce(&vec![91; PLAIN_CHUNK_BYTES * 2 + 1], &[7; 32], |nonce| {
            nonce.fill(3);
            Ok(())
        })
        .unwrap();
        assert_eq!(
            format!("{:x}", Sha256::digest(encoded.as_slice())),
            "15cd0dd4f52f82b1c820bb5c30aad1cfeba83db90baba72412a5d54bd470bc50"
        );
    }

    #[test]
    fn rejects_wrong_key_tampering_reordering_and_truncation() {
        let encoded = encrypt(&vec![17; PLAIN_CHUNK_BYTES * 3], &[5; 32]).unwrap();
        assert_eq!(decrypt(&encoded, &[6; 32]), Err(MediaError::Authentication));
        let mut corrupted = encoded.to_vec();
        corrupted[20] ^= 1;
        assert_eq!(
            decrypt(&corrupted, &[5; 32]),
            Err(MediaError::Authentication)
        );
        let mut reordered = encoded.to_vec();
        reordered[2..2 + CHUNK_BYTES]
            .copy_from_slice(&encoded[2 + CHUNK_BYTES..2 + 2 * CHUNK_BYTES]);
        reordered[2 + CHUNK_BYTES..2 + 2 * CHUNK_BYTES]
            .copy_from_slice(&encoded[2..2 + CHUNK_BYTES]);
        assert_eq!(
            decrypt(&reordered, &[5; 32]),
            Err(MediaError::Authentication)
        );
        assert_eq!(
            decrypt(&encoded[..2 + 2 * CHUNK_BYTES], &[5; 32]),
            Err(MediaError::Authentication)
        );
        assert!(decrypt(&encoded[..encoded.len() - 1], &[5; 32]).is_err());
        let mut appended = encoded.to_vec();
        appended.extend_from_slice(&encoded[2..2 + CHUNK_BYTES]);
        assert_eq!(
            decrypt(&appended, &[5; 32]),
            Err(MediaError::Authentication)
        );
    }

    #[test]
    fn rejects_empty_unsupported_and_oversized_files() {
        assert_eq!(encrypt(&[], &[0; 32]), Err(MediaError::Empty));
        assert_eq!(decrypt(&HEADER, &[0; 32]), Err(MediaError::InvalidFraming));
        assert_eq!(
            decrypt(&[0, 14, 1], &[0; 32]),
            Err(MediaError::UnsupportedFraming)
        );
        assert_eq!(
            encrypt(&vec![0; MAX_MEDIA_BYTES + 1], &[0; 32]),
            Err(MediaError::TooLarge)
        );
        assert_eq!(
            decrypt(&vec![0; MAX_ENCODED_BYTES + 1], &[0; 32]),
            Err(MediaError::TooLarge)
        );
    }

    #[test]
    fn randomness_failure_returns_no_partial_file() {
        assert_eq!(
            encrypt_with_nonce(&[1], &[0; 32], |_| Err(MediaError::Randomness)),
            Err(MediaError::Randomness)
        );
    }
}
