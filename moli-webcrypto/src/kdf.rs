use moli_crypto::derive_hkdf_bytes;
use openssl::pkcs5;

use crate::limits::ensure_kdf_parameter_bytes;
use crate::{WebCryptoError, WebCryptoHashAlgorithm};

pub const MAX_KDF_DERIVED_BITS: usize = 8 * 1024 * 1024;
pub const MAX_PBKDF2_ITERATIONS: u32 = 1_000_000;

pub fn derive_hkdf_bits(
    hash: WebCryptoHashAlgorithm,
    base_key: &[u8],
    salt: &[u8],
    info: &[u8],
    length_bits: usize,
) -> Result<Vec<u8>, WebCryptoError> {
    ensure_kdf_parameter_bytes(base_key)?;
    ensure_kdf_parameter_bytes(salt)?;
    ensure_kdf_parameter_bytes(info)?;
    if length_bits > MAX_KDF_DERIVED_BITS || !length_bits.is_multiple_of(8) {
        return Err(WebCryptoError::Operation);
    }
    let length_bytes = length_bits / 8;
    if length_bytes == 0 {
        return Ok(Vec::new());
    }
    let hash_len = hash.output_len_bytes();
    if length_bytes > 255 * hash_len {
        return Err(WebCryptoError::Operation);
    }

    derive_hkdf_bytes(hash.digest_algorithm(), base_key, salt, info, length_bytes)
        .map_err(|_| WebCryptoError::Operation)
}

pub fn derive_pbkdf2_bits(
    hash: WebCryptoHashAlgorithm,
    password: &[u8],
    salt: &[u8],
    iterations: u32,
    length_bits: usize,
) -> Result<Vec<u8>, WebCryptoError> {
    ensure_kdf_parameter_bytes(password)?;
    ensure_kdf_parameter_bytes(salt)?;
    if iterations == 0
        || iterations > MAX_PBKDF2_ITERATIONS
        || length_bits > MAX_KDF_DERIVED_BITS
        || !length_bits.is_multiple_of(8)
    {
        return Err(WebCryptoError::Operation);
    }
    let length_bytes = length_bits / 8;
    if length_bytes == 0 {
        return Ok(Vec::new());
    }
    let hash_len = hash.output_len_bytes();
    let block_count = length_bytes.div_ceil(hash_len);
    if block_count > u32::MAX as usize {
        return Err(WebCryptoError::Operation);
    }

    let mut derived = vec![0_u8; length_bytes];
    pkcs5::pbkdf2_hmac(
        password,
        salt,
        iterations as usize,
        hash.message_digest(),
        &mut derived,
    )
    .map_err(|_| WebCryptoError::Operation)?;
    Ok(derived)
}
