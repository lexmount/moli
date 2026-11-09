use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use openssl::{
    rand::rand_bytes,
    symm::{Cipher, Crypter, Mode},
};

use crate::WebCryptoError;
use crate::jwk::{
    Chacha20Poly1305JsonWebKeyExport, Chacha20Poly1305JsonWebKeyImport, decode_jwk_base64url,
    jwk_key_ops_allow_usages, jwk_use_allows_algorithm,
};
use crate::limits::ensure_raw_key_import_bytes;

pub const CHACHA20_POLY1305_KEY_LENGTH_BITS: usize = 256;
pub const MAX_CHACHA20_POLY1305_OPERATION_BYTES: usize = 16 * 1024 * 1024;

pub fn generate_chacha20_poly1305_key() -> Result<Vec<u8>, WebCryptoError> {
    let mut key_bytes = vec![0_u8; CHACHA20_POLY1305_KEY_LENGTH_BITS / 8];
    rand_bytes(&mut key_bytes).map_err(|_| WebCryptoError::Operation)?;
    Ok(key_bytes)
}

pub fn validate_chacha20_poly1305_key_bytes(bytes: &[u8]) -> Result<(), WebCryptoError> {
    ensure_raw_key_import_bytes(bytes)?;
    if bytes.len() == CHACHA20_POLY1305_KEY_LENGTH_BITS / 8 {
        Ok(())
    } else {
        Err(WebCryptoError::Data)
    }
}

pub fn chacha20_poly1305_encrypt(
    key: &[u8],
    iv: &[u8],
    additional_data: &[u8],
    tag_length_bits: usize,
    plaintext: &[u8],
) -> Result<Vec<u8>, WebCryptoError> {
    ensure_chacha20_poly1305_inputs(key, iv, additional_data, tag_length_bits, plaintext)?;
    let cipher = Cipher::chacha20_poly1305();
    let mut crypter = new_chacha20_poly1305_crypter(Mode::Encrypt, key, iv)?;
    update_aad(&mut crypter, additional_data)?;
    let mut output = update_and_finalize(&mut crypter, cipher, plaintext)?;
    let mut tag = [0_u8; 16];
    crypter
        .get_tag(&mut tag)
        .map_err(|_| WebCryptoError::Operation)?;
    output.extend_from_slice(&tag);
    Ok(output)
}

pub fn chacha20_poly1305_decrypt(
    key: &[u8],
    iv: &[u8],
    additional_data: &[u8],
    tag_length_bits: usize,
    ciphertext_and_tag: &[u8],
) -> Result<Vec<u8>, WebCryptoError> {
    ensure_chacha20_poly1305_inputs(
        key,
        iv,
        additional_data,
        tag_length_bits,
        ciphertext_and_tag,
    )?;
    if ciphertext_and_tag.len() < 16 {
        return Err(WebCryptoError::Operation);
    }
    let split_at = ciphertext_and_tag.len() - 16;
    let (ciphertext, tag) = ciphertext_and_tag.split_at(split_at);
    let cipher = Cipher::chacha20_poly1305();
    let mut crypter = new_chacha20_poly1305_crypter(Mode::Decrypt, key, iv)?;
    update_aad(&mut crypter, additional_data)?;
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

pub fn import_chacha20_poly1305_jwk_key(
    jwk: &Chacha20Poly1305JsonWebKeyImport,
    extractable: bool,
    usages: &[String],
) -> Result<Vec<u8>, WebCryptoError> {
    jwk.validate_resource_limits()?;
    if jwk.kty.as_deref() != Some("oct") {
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
    if jwk.alg.is_some() {
        return Err(WebCryptoError::Data);
    }
    let Some(k) = jwk.k.as_deref() else {
        return Err(WebCryptoError::Data);
    };
    let key_bytes = decode_jwk_base64url(k)?;
    validate_chacha20_poly1305_key_bytes(&key_bytes)?;
    Ok(key_bytes)
}

pub fn export_chacha20_poly1305_jwk(
    key_bytes: &[u8],
    key_ops: Vec<String>,
    ext: bool,
) -> Result<Chacha20Poly1305JsonWebKeyExport, WebCryptoError> {
    validate_chacha20_poly1305_key_bytes(key_bytes)?;
    Ok(Chacha20Poly1305JsonWebKeyExport {
        kty: "oct",
        k: URL_SAFE_NO_PAD.encode(key_bytes),
        key_ops,
        ext,
    })
}

fn ensure_chacha20_poly1305_inputs(
    key: &[u8],
    iv: &[u8],
    additional_data: &[u8],
    tag_length_bits: usize,
    data: &[u8],
) -> Result<(), WebCryptoError> {
    if key.len() != CHACHA20_POLY1305_KEY_LENGTH_BITS / 8
        || iv.len() != 12
        || tag_length_bits != 128
        || additional_data.len() > MAX_CHACHA20_POLY1305_OPERATION_BYTES
        || data.len() > MAX_CHACHA20_POLY1305_OPERATION_BYTES
    {
        Err(WebCryptoError::Operation)
    } else {
        Ok(())
    }
}

fn new_chacha20_poly1305_crypter(
    mode: Mode,
    key: &[u8],
    iv: &[u8],
) -> Result<Crypter, WebCryptoError> {
    let mut crypter = Crypter::new(Cipher::chacha20_poly1305(), mode, key, Some(iv))
        .map_err(|_| WebCryptoError::Operation)?;
    crypter.pad(false);
    Ok(crypter)
}

use crate::symmetric::update_aad;

use crate::symmetric::update_and_finalize;
