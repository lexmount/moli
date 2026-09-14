use std::num::NonZeroU32;

use aws_lc_rs::{
    aead::{AES_256_GCM, Aad, LessSafeKey, Nonce, UnboundKey},
    cipher::{AES_128, DecryptionContext, PaddedBlockDecryptingKey, UnboundCipherKey},
    iv::{FixedLength, IV_LEN_128_BIT},
    pbkdf2::{self, PBKDF2_HMAC_SHA1},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LegacyCryptoError;

impl std::fmt::Display for LegacyCryptoError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("legacy cryptographic operation failed")
    }
}

impl std::error::Error for LegacyCryptoError {}

/// Decrypts an AES-128-CBC value and removes PKCS#7 padding.
pub fn aes_128_cbc_pkcs7_decrypt(
    key: &[u8],
    iv: &[u8; 16],
    ciphertext: &[u8],
) -> Result<Vec<u8>, LegacyCryptoError> {
    let key = UnboundCipherKey::new(&AES_128, key).map_err(|_| LegacyCryptoError)?;
    let key = PaddedBlockDecryptingKey::cbc_pkcs7(key).map_err(|_| LegacyCryptoError)?;
    let context = DecryptionContext::Iv128(FixedLength::<IV_LEN_128_BIT>::from(iv));
    let mut output = ciphertext.to_vec();
    let plaintext = key
        .decrypt(&mut output, context)
        .map_err(|_| LegacyCryptoError)?;
    Ok(plaintext.to_vec())
}

/// Authenticates and decrypts an AES-256-GCM value with a 128-bit tag.
pub fn aes_256_gcm_decrypt(
    key: &[u8],
    nonce: &[u8],
    ciphertext_and_tag: &[u8],
) -> Result<Vec<u8>, LegacyCryptoError> {
    let key = UnboundKey::new(&AES_256_GCM, key).map_err(|_| LegacyCryptoError)?;
    let key = LessSafeKey::new(key);
    let nonce = Nonce::try_assume_unique_for_key(nonce).map_err(|_| LegacyCryptoError)?;
    let mut output = ciphertext_and_tag.to_vec();
    let plaintext = key
        .open_in_place(nonce, Aad::empty(), &mut output)
        .map_err(|_| LegacyCryptoError)?;
    Ok(plaintext.to_vec())
}

/// Derives bytes using the legacy PBKDF2-HMAC-SHA1 construction.
pub fn derive_pbkdf2_hmac_sha1(
    password: &[u8],
    salt: &[u8],
    iterations: NonZeroU32,
    output: &mut [u8],
) {
    pbkdf2::derive(PBKDF2_HMAC_SHA1, iterations, salt, password, output);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(input: &str) -> Vec<u8> {
        input
            .as_bytes()
            .chunks_exact(2)
            .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
            .collect()
    }

    #[test]
    fn aes_cbc_pkcs7_matches_known_answer() {
        let key = hex("000102030405060708090a0b0c0d0e0f");
        let ciphertext = hex("29aee51004eb86ec78f2b81e2dd7e543");
        assert_eq!(
            aes_128_cbc_pkcs7_decrypt(&key, &[b' '; 16], &ciphertext).unwrap(),
            b"hello"
        );
    }

    #[test]
    fn aes_256_gcm_matches_nist_known_answer() {
        let key = [0_u8; 32];
        let nonce = [0_u8; 12];
        let ciphertext_and_tag = hex(concat!(
            "cea7403d4d606b6e074ec5d3baf39d18",
            "d0d1c8a799996bf0265b98b5d48ab919"
        ));
        assert_eq!(
            aes_256_gcm_decrypt(&key, &nonce, &ciphertext_and_tag).unwrap(),
            [0_u8; 16]
        );
    }

    #[test]
    fn pbkdf2_sha1_matches_rfc_6070() {
        let mut output = [0_u8; 20];
        derive_pbkdf2_hmac_sha1(
            b"password",
            b"salt",
            NonZeroU32::new(1).unwrap(),
            &mut output,
        );
        assert_eq!(
            output,
            [
                0x0c, 0x60, 0xc8, 0x0f, 0x96, 0x1f, 0x0e, 0x71, 0xf3, 0xa9, 0xb5, 0x24, 0xaf, 0x60,
                0x12, 0x06, 0x2f, 0xe0, 0x37, 0xa6,
            ]
        );
    }
}
