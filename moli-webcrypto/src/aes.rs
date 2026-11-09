use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use openssl::{
    aes::{AesKey, unwrap_key as openssl_aes_unwrap_key, wrap_key as openssl_aes_wrap_key},
    rand::rand_bytes,
    symm::{Cipher, Crypter, Mode},
};

use crate::jwk::{decode_jwk_base64url, jwk_key_ops_allow_usages, jwk_use_allows_algorithm};
use crate::limits::ensure_raw_key_import_bytes;
use crate::{AesJsonWebKeyExport, AesJsonWebKeyImport, WebCryptoError, WebCryptoKeyAlgorithm};

const AES_CTR_COUNTER_CHUNK_BLOCKS: usize = 1024;
pub const MAX_AES_OPERATION_BYTES: usize = 16 * 1024 * 1024;

pub fn generate_aes_key(length_bits: usize) -> Result<Vec<u8>, WebCryptoError> {
    if !matches!(length_bits, 128 | 192 | 256) {
        return Err(WebCryptoError::Operation);
    }
    let mut key_bytes = vec![0_u8; length_bits / 8];
    rand_bytes(&mut key_bytes).map_err(|_| WebCryptoError::Operation)?;
    Ok(key_bytes)
}
pub fn aes_algorithm_from_jwk_alg(alg: &str) -> Option<(WebCryptoKeyAlgorithm, usize)> {
    let (length, suffix) = match alg.as_bytes() {
        [b'A', b'1', b'2', b'8', rest @ ..] => (128, std::str::from_utf8(rest).ok()?),
        [b'A', b'1', b'9', b'2', rest @ ..] => (192, std::str::from_utf8(rest).ok()?),
        [b'A', b'2', b'5', b'6', rest @ ..] => (256, std::str::from_utf8(rest).ok()?),
        _ => return None,
    };
    let algorithm = match suffix {
        "CBC" => WebCryptoKeyAlgorithm::AesCbc,
        "CTR" => WebCryptoKeyAlgorithm::AesCtr,
        "GCM" => WebCryptoKeyAlgorithm::AesGcm,
        "KW" => WebCryptoKeyAlgorithm::AesKw,
        _ => return None,
    };
    Some((algorithm, length))
}

pub fn validate_aes_key_bytes(bytes: &[u8]) -> Result<usize, WebCryptoError> {
    ensure_raw_key_import_bytes(bytes)?;
    let length_bits = aes_key_length_bits_from_bytes(bytes)?;
    Ok(length_bits)
}

fn aes_key_length_bits_from_bytes(bytes: &[u8]) -> Result<usize, WebCryptoError> {
    let length_bits = bytes.len() * 8;
    if matches!(length_bits, 128 | 192 | 256) {
        Ok(length_bits)
    } else {
        Err(WebCryptoError::Data)
    }
}

pub fn aes_cbc_encrypt(key: &[u8], iv: &[u8], plaintext: &[u8]) -> Result<Vec<u8>, WebCryptoError> {
    ensure_aes_input_len(plaintext)?;
    if iv.len() != 16 {
        return Err(WebCryptoError::Operation);
    }
    run_openssl_cipher(
        aes_cbc_cipher(key)?,
        Mode::Encrypt,
        key,
        Some(iv),
        plaintext,
        true,
    )
}

pub fn aes_cbc_decrypt(
    key: &[u8],
    iv: &[u8],
    ciphertext: &[u8],
) -> Result<Vec<u8>, WebCryptoError> {
    ensure_aes_input_len(ciphertext)?;
    if iv.len() != 16 {
        return Err(WebCryptoError::Operation);
    }
    run_openssl_cipher(
        aes_cbc_cipher(key)?,
        Mode::Decrypt,
        key,
        Some(iv),
        ciphertext,
        true,
    )
}

pub fn aes_ctr_crypt(
    key: &[u8],
    counter: &[u8],
    length_bits: u8,
    data: &[u8],
) -> Result<Vec<u8>, WebCryptoError> {
    ensure_aes_input_len(data)?;
    if counter.len() != 16 || !(1..=128).contains(&length_bits) {
        return Err(WebCryptoError::Operation);
    }
    if data.is_empty() {
        return Ok(Vec::new());
    }
    let block_count = data.len().div_ceil(16);
    let counter_mask = counter_mask(length_bits);
    let counter_bytes: [u8; 16] = counter.try_into().map_err(|_| WebCryptoError::Operation)?;
    let base_counter = u128::from_be_bytes(counter_bytes);
    let counter_value = base_counter & counter_mask;
    if (block_count as u128 - 1) > counter_mask - counter_value {
        return Err(WebCryptoError::Operation);
    }

    let cipher = aes_ecb_cipher(key)?;
    let mut crypter =
        Crypter::new(cipher, Mode::Encrypt, key, None).map_err(|_| WebCryptoError::Operation)?;
    crypter.pad(false);
    let counter_prefix = base_counter & !counter_mask;
    let mut output = data.to_vec();
    let mut processed_blocks = 0;
    let mut output_offset = 0;
    while processed_blocks < block_count {
        let chunk_blocks = (block_count - processed_blocks).min(AES_CTR_COUNTER_CHUNK_BLOCKS);
        let chunk_len = chunk_blocks
            .checked_mul(16)
            .ok_or(WebCryptoError::Operation)?;
        let mut counter_blocks = Vec::with_capacity(chunk_len);
        for block_offset in 0..chunk_blocks {
            let block_index = processed_blocks + block_offset;
            let block_counter =
                counter_prefix | ((counter_value + block_index as u128) & counter_mask);
            counter_blocks.extend_from_slice(&block_counter.to_be_bytes());
        }
        let keystream_len = chunk_len
            .checked_add(cipher.block_size())
            .ok_or(WebCryptoError::Operation)?;
        let mut keystream = vec![0_u8; keystream_len];
        let written = crypter
            .update(&counter_blocks, &mut keystream)
            .map_err(|_| WebCryptoError::Operation)?;
        let chunk_data_len = (output.len() - output_offset).min(chunk_len);
        if written < chunk_data_len {
            return Err(WebCryptoError::Operation);
        }
        for (byte, key_byte) in output[output_offset..output_offset + chunk_data_len]
            .iter_mut()
            .zip(&keystream[..chunk_data_len])
        {
            *byte ^= *key_byte;
        }
        processed_blocks += chunk_blocks;
        output_offset += chunk_data_len;
    }
    let mut final_block = [0_u8; 16];
    let final_written = crypter
        .finalize(&mut final_block)
        .map_err(|_| WebCryptoError::Operation)?;
    if final_written != 0 {
        return Err(WebCryptoError::Operation);
    }
    Ok(output)
}

pub fn aes_gcm_encrypt(
    key: &[u8],
    iv: &[u8],
    additional_data: &[u8],
    tag_length_bits: usize,
    plaintext: &[u8],
) -> Result<Vec<u8>, WebCryptoError> {
    ensure_aes_input_len(iv)?;
    ensure_aes_input_len(additional_data)?;
    ensure_aes_input_len(plaintext)?;
    let tag_len = aes_gcm_tag_len(tag_length_bits)?;
    let cipher = aes_gcm_cipher(key)?;
    let mut crypter = new_gcm_crypter(cipher, Mode::Encrypt, key, iv)?;
    gcm_update_aad(&mut crypter, additional_data)?;
    let mut output = update_and_finalize(&mut crypter, cipher, plaintext)?;
    let mut tag = vec![0_u8; tag_len];
    crypter
        .get_tag(&mut tag)
        .map_err(|_| WebCryptoError::Operation)?;
    output.extend_from_slice(&tag);
    Ok(output)
}

pub fn aes_gcm_decrypt(
    key: &[u8],
    iv: &[u8],
    additional_data: &[u8],
    tag_length_bits: usize,
    ciphertext_and_tag: &[u8],
) -> Result<Vec<u8>, WebCryptoError> {
    ensure_aes_input_len(iv)?;
    ensure_aes_input_len(additional_data)?;
    ensure_aes_input_len(ciphertext_and_tag)?;
    let tag_len = aes_gcm_tag_len(tag_length_bits)?;
    if ciphertext_and_tag.len() < tag_len {
        return Err(WebCryptoError::Operation);
    }
    let split_at = ciphertext_and_tag.len() - tag_len;
    let (ciphertext, tag) = ciphertext_and_tag.split_at(split_at);
    let cipher = aes_gcm_cipher(key)?;
    let mut crypter = new_gcm_crypter(cipher, Mode::Decrypt, key, iv)?;
    gcm_update_aad(&mut crypter, additional_data)?;
    let output_len = ciphertext
        .len()
        .checked_add(cipher.block_size())
        .ok_or(WebCryptoError::Operation)?;
    let mut output = vec![0_u8; output_len];
    let mut written = 0;
    if !ciphertext.is_empty() {
        written = crypter
            .update(ciphertext, &mut output)
            .map_err(|_| WebCryptoError::Operation)?;
    }
    crypter
        .set_tag(tag)
        .map_err(|_| WebCryptoError::Operation)?;
    let final_written = crypter
        .finalize(&mut output[written..])
        .map_err(|_| WebCryptoError::Operation)?;
    output.truncate(written + final_written);
    Ok(output)
}

pub fn aes_kw_wrap(key: &[u8], plaintext: &[u8]) -> Result<Vec<u8>, WebCryptoError> {
    ensure_aes_input_len(plaintext)?;
    if plaintext.len() < 16 || !plaintext.len().is_multiple_of(8) {
        return Err(WebCryptoError::Operation);
    }
    let key = aes_kw_encrypt_key(key)?;
    let output_len = plaintext
        .len()
        .checked_add(8)
        .ok_or(WebCryptoError::Operation)?;
    let mut output = vec![0_u8; output_len];
    let written = openssl_aes_wrap_key(&key, None, &mut output, plaintext)
        .map_err(|_| WebCryptoError::Operation)?;
    output.truncate(written);
    Ok(output)
}

pub fn aes_kw_unwrap(key: &[u8], ciphertext: &[u8]) -> Result<Vec<u8>, WebCryptoError> {
    ensure_aes_input_len(ciphertext)?;
    if ciphertext.len() < 24 || !ciphertext.len().is_multiple_of(8) {
        return Err(WebCryptoError::Operation);
    }
    let key = aes_kw_decrypt_key(key)?;
    let mut output = vec![0_u8; ciphertext.len() - 8];
    let written = openssl_aes_unwrap_key(&key, None, &mut output, ciphertext)
        .map_err(|_| WebCryptoError::Operation)?;
    output.truncate(written);
    Ok(output)
}

fn aes_kw_encrypt_key(key: &[u8]) -> Result<AesKey, WebCryptoError> {
    if !matches!(key.len(), 16 | 24 | 32) {
        return Err(WebCryptoError::Operation);
    }
    AesKey::new_encrypt(key).map_err(|_| WebCryptoError::Operation)
}

fn aes_kw_decrypt_key(key: &[u8]) -> Result<AesKey, WebCryptoError> {
    if !matches!(key.len(), 16 | 24 | 32) {
        return Err(WebCryptoError::Operation);
    }
    AesKey::new_decrypt(key).map_err(|_| WebCryptoError::Operation)
}

fn run_openssl_cipher(
    cipher: Cipher,
    mode: Mode,
    key: &[u8],
    iv: Option<&[u8]>,
    input: &[u8],
    padding: bool,
) -> Result<Vec<u8>, WebCryptoError> {
    let mut crypter = Crypter::new(cipher, mode, key, iv).map_err(|_| WebCryptoError::Operation)?;
    crypter.pad(padding);
    update_and_finalize(&mut crypter, cipher, input)
}

fn new_gcm_crypter(
    cipher: Cipher,
    mode: Mode,
    key: &[u8],
    iv: &[u8],
) -> Result<Crypter, WebCryptoError> {
    if iv.is_empty() {
        return Err(WebCryptoError::Operation);
    }
    let mut crypter =
        Crypter::new(cipher, mode, key, Some(iv)).map_err(|_| WebCryptoError::Operation)?;
    crypter.pad(false);
    Ok(crypter)
}

use crate::symmetric::update_aad as gcm_update_aad;

use crate::symmetric::update_and_finalize;

fn ensure_aes_input_len(input: &[u8]) -> Result<(), WebCryptoError> {
    if input.len() > MAX_AES_OPERATION_BYTES {
        Err(WebCryptoError::Operation)
    } else {
        Ok(())
    }
}

fn aes_cbc_cipher(key: &[u8]) -> Result<Cipher, WebCryptoError> {
    match key.len() {
        16 => Ok(Cipher::aes_128_cbc()),
        24 => Ok(Cipher::aes_192_cbc()),
        32 => Ok(Cipher::aes_256_cbc()),
        _ => Err(WebCryptoError::Operation),
    }
}

fn aes_ecb_cipher(key: &[u8]) -> Result<Cipher, WebCryptoError> {
    match key.len() {
        16 => Ok(Cipher::aes_128_ecb()),
        24 => Ok(Cipher::aes_192_ecb()),
        32 => Ok(Cipher::aes_256_ecb()),
        _ => Err(WebCryptoError::Operation),
    }
}

fn aes_gcm_cipher(key: &[u8]) -> Result<Cipher, WebCryptoError> {
    match key.len() {
        16 => Ok(Cipher::aes_128_gcm()),
        24 => Ok(Cipher::aes_192_gcm()),
        32 => Ok(Cipher::aes_256_gcm()),
        _ => Err(WebCryptoError::Operation),
    }
}

fn aes_gcm_tag_len(tag_length_bits: usize) -> Result<usize, WebCryptoError> {
    if matches!(tag_length_bits, 32 | 64 | 96 | 104 | 112 | 120 | 128) {
        Ok(tag_length_bits / 8)
    } else {
        Err(WebCryptoError::Operation)
    }
}

fn counter_mask(length_bits: u8) -> u128 {
    if length_bits == 128 {
        u128::MAX
    } else {
        (1_u128 << length_bits) - 1
    }
}
pub fn import_aes_jwk_key(
    jwk: &AesJsonWebKeyImport,
    expected_algorithm: WebCryptoKeyAlgorithm,
    extractable: bool,
    usages: &[String],
) -> Result<Vec<u8>, WebCryptoError> {
    jwk.validate_resource_limits()?;
    if !expected_algorithm.is_aes() || jwk.kty.as_deref() != Some("oct") {
        return Err(WebCryptoError::Data);
    }
    if extractable && jwk.ext == Some(false) {
        return Err(WebCryptoError::Data);
    }
    jwk_use_allows_algorithm(jwk.public_key_use.as_deref(), "enc")?;
    jwk_key_ops_allow_usages(
        jwk.key_ops.as_deref(),
        usages,
        jwk.public_key_use.as_deref(),
    )?;
    let Some(k) = jwk.k.as_deref() else {
        return Err(WebCryptoError::Data);
    };
    let key_bytes = decode_jwk_base64url(k)?;
    let length_bits = aes_key_length_bits_from_bytes(&key_bytes)?;
    if let Some(alg) = jwk.alg.as_deref() {
        match aes_algorithm_from_jwk_alg(alg) {
            Some((jwk_algorithm, jwk_length_bits))
                if jwk_algorithm == expected_algorithm && jwk_length_bits == length_bits => {}
            _ => return Err(WebCryptoError::Data),
        }
    }
    Ok(key_bytes)
}

pub fn export_aes_jwk(
    algorithm: WebCryptoKeyAlgorithm,
    key_bytes: &[u8],
    key_ops: Vec<String>,
    ext: bool,
) -> Result<AesJsonWebKeyExport, WebCryptoError> {
    let length_bits = validate_aes_key_bytes(key_bytes)?;
    let Some(alg) = algorithm.jwk_aes_alg(length_bits) else {
        return Err(WebCryptoError::Data);
    };
    Ok(AesJsonWebKeyExport {
        kty: "oct",
        k: URL_SAFE_NO_PAD.encode(key_bytes),
        alg,
        key_ops,
        ext,
    })
}
