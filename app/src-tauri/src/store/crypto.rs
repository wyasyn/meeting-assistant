//! Audio chunk encryption (NFR-13): AES-256-GCM with the database key (docs/04, ADR-014).
//! File layout: `MAC1` | 12-byte random nonce | ciphertext with 16-byte tag.

use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Nonce};

use super::key::KEY_LEN;
use super::StoreError;

const MAGIC: &[u8; 4] = b"MAC1";
const NONCE_LEN: usize = 12;
const HEADER_LEN: usize = MAGIC.len() + NONCE_LEN;

/// The 256-bit key for audio chunks. Debug never prints it.
#[derive(Clone)]
pub struct AudioKey([u8; KEY_LEN]);

impl AudioKey {
    pub fn new(key: &[u8]) -> Result<Self, StoreError> {
        <[u8; KEY_LEN]>::try_from(key)
            .map(Self)
            .map_err(|_| StoreError::BadKey)
    }

    fn cipher(&self) -> Aes256Gcm {
        Aes256Gcm::new(&self.0.into())
    }
}

impl std::fmt::Debug for AudioKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("AudioKey(..)")
    }
}

/// Binds a chunk to its place so files cannot be swapped or renamed undetected.
pub fn chunk_aad(meeting_id: &str, track: &str, index: u32) -> Vec<u8> {
    format!("{meeting_id}/{track}/{index}").into_bytes()
}

pub fn encrypt_chunk(key: &AudioKey, aad: &[u8], plain: &[u8]) -> Result<Vec<u8>, StoreError> {
    let mut nonce = [0u8; NONCE_LEN];
    getrandom::fill(&mut nonce).map_err(|e| StoreError::Random(e.to_string()))?;
    let sealed = key
        .cipher()
        .encrypt(&Nonce::from(nonce), Payload { msg: plain, aad })
        .map_err(|_| StoreError::Crypto)?;
    let mut out = Vec::with_capacity(HEADER_LEN + sealed.len());
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&sealed);
    Ok(out)
}

/// Fails on a wrong key, wrong AAD, truncation or any changed byte.
pub fn decrypt_chunk(key: &AudioKey, aad: &[u8], file: &[u8]) -> Result<Vec<u8>, StoreError> {
    if file.len() < HEADER_LEN || &file[..MAGIC.len()] != MAGIC {
        return Err(StoreError::Crypto);
    }
    let nonce: [u8; NONCE_LEN] = file[MAGIC.len()..HEADER_LEN]
        .try_into()
        .map_err(|_| StoreError::Crypto)?;
    key.cipher()
        .decrypt(
            &Nonce::from(nonce),
            Payload {
                msg: &file[HEADER_LEN..],
                aad,
            },
        )
        .map_err(|_| StoreError::Crypto)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(byte: u8) -> AudioKey {
        AudioKey::new(&[byte; KEY_LEN]).unwrap()
    }

    #[test]
    fn nfr_13_chunk_round_trips_and_is_not_plain() {
        let aad = chunk_aad("m1", "mic", 1);
        let sealed = encrypt_chunk(&key(1), &aad, b"OggS audio").unwrap();
        assert!(sealed.starts_with(MAGIC));
        assert!(!sealed.windows(4).any(|w| w == b"OggS"));
        assert_eq!(
            decrypt_chunk(&key(1), &aad, &sealed).unwrap(),
            b"OggS audio"
        );
    }

    #[test]
    fn nfr_13_wrong_key_aad_or_tampering_fails() {
        let aad = chunk_aad("m1", "mic", 1);
        let sealed = encrypt_chunk(&key(1), &aad, b"audio").unwrap();
        assert!(decrypt_chunk(&key(2), &aad, &sealed).is_err());
        assert!(decrypt_chunk(&key(1), &chunk_aad("m1", "mic", 2), &sealed).is_err());
        assert!(decrypt_chunk(&key(1), &chunk_aad("m1", "sys", 1), &sealed).is_err());
        let mut tampered = sealed.clone();
        *tampered.last_mut().unwrap() ^= 1;
        assert!(decrypt_chunk(&key(1), &aad, &tampered).is_err());
        assert!(decrypt_chunk(&key(1), &aad, &sealed[..10]).is_err());
    }

    #[test]
    fn nonces_differ_between_chunks() {
        let aad = chunk_aad("m1", "mic", 1);
        let a = encrypt_chunk(&key(1), &aad, b"same").unwrap();
        let b = encrypt_chunk(&key(1), &aad, b"same").unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn debug_hides_the_key() {
        assert_eq!(format!("{:?}", key(7)), "AudioKey(..)");
    }
}
