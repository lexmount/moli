use crate::WebCryptoError;
use openssl::symm::{Cipher, Crypter};

pub(crate) fn update_and_finalize(
    crypter: &mut Crypter,
    cipher: Cipher,
    input: &[u8],
) -> Result<Vec<u8>, WebCryptoError> {
    let output_len = input
        .len()
        .checked_add(cipher.block_size())
        .ok_or(WebCryptoError::Operation)?;
    let mut output = vec![0_u8; output_len];
    let mut written = 0;
    if !input.is_empty() {
        written = crypter
            .update(input, &mut output)
            .map_err(|_| WebCryptoError::Operation)?;
    }
    let final_written = crypter
        .finalize(&mut output[written..])
        .map_err(|_| WebCryptoError::Operation)?;
    output.truncate(written + final_written);
    Ok(output)
}

pub(crate) fn update_aad(
    crypter: &mut Crypter,
    additional_data: &[u8],
) -> Result<(), WebCryptoError> {
    if additional_data.is_empty() {
        return Ok(());
    }
    crypter
        .aad_update(additional_data)
        .map_err(|_| WebCryptoError::Operation)
}
