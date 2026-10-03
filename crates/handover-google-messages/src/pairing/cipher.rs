//! Observed Messages payload format using RustCrypto AES-CTR and HMAC-SHA256.
//! Ciphertext || 16-byte counter || 32-byte tag. No transport or key exports.

use aes::Aes256;
use ctr::cipher::{KeyIvInit, StreamCipher};
use hmac::{Hmac, Mac};
use rand::{RngCore, rngs::OsRng};
use sha2::Sha256;
use std::fmt;
use zeroize::Zeroizing;

const PAYLOAD_LIMIT: usize = 512 * 1024;
const OVERHEAD: usize = 48;
type AesCtr = ctr::Ctr128BE<Aes256>;
type Authentication = Hmac<Sha256>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CipherError {
    TooLarge,
    InvalidCiphertext,
    AuthenticationFailed,
    EntropyUnavailable,
    CounterExhausted,
}

pub struct EncryptedPayload(Vec<u8>);
pub struct Plaintext(Zeroizing<Vec<u8>>);

impl fmt::Debug for EncryptedPayload {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("EncryptedPayload { redacted }")
    }
}

impl fmt::Debug for Plaintext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Plaintext { redacted }")
    }
}

impl EncryptedPayload {
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

impl Plaintext {
    /// Borrow only for protocol decoding. Do not put contents in logs or IPC.
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

pub(super) fn encrypt(
    key: &[u8; 32],
    authentication_key: &[u8; 32],
    plaintext: &[u8],
) -> Result<EncryptedPayload, CipherError> {
    if plaintext.len() > PAYLOAD_LIMIT {
        return Err(CipherError::TooLarge);
    }
    let mut counter = [0; 16];
    OsRng
        .try_fill_bytes(&mut counter)
        .map_err(|_| CipherError::EntropyUnavailable)?;
    seal(key, authentication_key, plaintext, counter)
}

fn seal(
    key: &[u8; 32],
    authentication_key: &[u8; 32],
    plaintext: &[u8],
    counter: [u8; 16],
) -> Result<EncryptedPayload, CipherError> {
    if plaintext.len() > PAYLOAD_LIMIT {
        return Err(CipherError::TooLarge);
    }
    let mut buffer = Zeroizing::new(plaintext.to_vec());
    AesCtr::new(key.into(), (&counter).into())
        .try_apply_keystream(&mut buffer)
        .map_err(|_| CipherError::CounterExhausted)?;
    buffer.extend_from_slice(&counter);
    let mut mac =
        Authentication::new_from_slice(authentication_key).expect("fixed 32-byte HMAC key");
    mac.update(&buffer);
    buffer.extend_from_slice(&mac.finalize().into_bytes());
    Ok(EncryptedPayload(std::mem::take(&mut *buffer)))
}

pub(super) fn decrypt(
    key: &[u8; 32],
    authentication_key: &[u8; 32],
    ciphertext: &[u8],
) -> Result<Plaintext, CipherError> {
    if ciphertext.len() < OVERHEAD {
        return Err(CipherError::InvalidCiphertext);
    }
    if ciphertext.len() > PAYLOAD_LIMIT + OVERHEAD {
        return Err(CipherError::TooLarge);
    }
    let (authenticated, tag) = ciphertext.split_at(ciphertext.len() - 32);
    let mut mac =
        Authentication::new_from_slice(authentication_key).expect("fixed 32-byte HMAC key");
    mac.update(authenticated);
    mac.verify_slice(tag)
        .map_err(|_| CipherError::AuthenticationFailed)?;
    // Authentication succeeds before any plaintext is allocated or decrypted.
    let (encrypted, counter) = authenticated.split_at(authenticated.len() - 16);
    let mut plaintext = Zeroizing::new(encrypted.to_vec());
    AesCtr::new(key.into(), counter.into())
        .try_apply_keystream(&mut plaintext)
        .map_err(|_| CipherError::CounterExhausted)?;
    Ok(Plaintext(plaintext))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(input: &str) -> Vec<u8> {
        input
            .as_bytes()
            .chunks_exact(2)
            .map(|part| u8::from_str_radix(std::str::from_utf8(part).unwrap(), 16).unwrap())
            .collect()
    }

    fn keys() -> ([u8; 32], [u8; 32]) {
        (
            std::array::from_fn(|i| i as u8),
            std::array::from_fn(|i| (i + 32) as u8),
        )
    }

    #[test]
    fn matches_independent_webcrypto_at_empty_and_aes_block_boundaries() {
        let (key, mac) = keys();
        let counter = std::array::from_fn(|i| (i + 128) as u8);
        for (length, wire) in [
            (
                0,
                "808182838485868788898a8b8c8d8e8f74e46810acceae2e21ca5ce00a6b422ca762f0ed056b37c6ec82a18a8dd5b50e",
            ),
            (
                1,
                "b0808182838485868788898a8b8c8d8e8ff61b855ecc089857567046dfcd9d05854566babf152d7ebed905c2a0151c9c43",
            ),
            (
                16,
                "b0b1f778cf92b184a95a85fc829766cd808182838485868788898a8b8c8d8e8fd278e927deca82d02e0d4f8d49b97884ff7c4f95b8bbcb244b14c15ca2fa7e4c",
            ),
            (
                33,
                "b0b1f778cf92b184a95a85fc829766cdee94a1717a796ac37db7bcd522f4252f66808182838485868788898a8b8c8d8e8f54515c406efc76af6f485ba11cd6dadf23308f1eacb88c3bf34071b91467e0fd",
            ),
        ] {
            let plaintext = (0..length).map(|i| i as u8).collect::<Vec<_>>();
            let expected = hex(wire);
            assert!(seal(&key, &mac, &plaintext, counter).unwrap().as_bytes() == expected);
            assert!(decrypt(&key, &mac, &expected).unwrap().as_bytes() == plaintext);
        }
    }

    #[test]
    fn every_ciphertext_counter_and_tag_byte_is_authenticated() {
        let (key, mac) = keys();
        let wire = seal(&key, &mac, b"synthetic payload", [42; 16]).unwrap();
        for position in 0..wire.as_bytes().len() {
            let mut changed = wire.as_bytes().to_vec();
            changed[position] ^= 1;
            assert_eq!(
                decrypt(&key, &mac, &changed).unwrap_err(),
                CipherError::AuthenticationFailed
            );
        }
        assert_eq!(
            decrypt(&key, &[0; 32], wire.as_bytes()).unwrap_err(),
            CipherError::AuthenticationFailed
        );
    }

    #[test]
    fn production_encryption_uses_fresh_counters_and_redacted_results() {
        let (key, mac) = keys();
        let a = encrypt(&key, &mac, b"synthetic payload").unwrap();
        let b = encrypt(&key, &mac, b"synthetic payload").unwrap();
        assert!(a.as_bytes() != b.as_bytes());
        let clear = decrypt(&key, &mac, a.as_bytes()).unwrap();
        assert!(clear.as_bytes() == b"synthetic payload");
        assert_eq!(format!("{a:?}"), "EncryptedPayload { redacted }");
        assert_eq!(format!("{clear:?}"), "Plaintext { redacted }");
    }

    #[test]
    fn validates_size_before_crypto_work() {
        let (key, mac) = keys();
        for length in 0..OVERHEAD {
            assert_eq!(
                decrypt(&key, &mac, &vec![0; length]).unwrap_err(),
                CipherError::InvalidCiphertext
            );
        }
        assert_eq!(
            encrypt(&key, &mac, &vec![0; PAYLOAD_LIMIT + 1]).unwrap_err(),
            CipherError::TooLarge
        );
        assert_eq!(
            decrypt(&key, &mac, &vec![0; PAYLOAD_LIMIT + OVERHEAD + 1]).unwrap_err(),
            CipherError::TooLarge
        );
    }
}
