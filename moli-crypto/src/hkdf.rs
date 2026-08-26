use std::fmt;

use aws_lc_rs::hkdf::{
    self, HKDF_SHA1_FOR_LEGACY_USE_ONLY, HKDF_SHA256, HKDF_SHA384, HKDF_SHA512, KeyType,
};

use crate::DigestAlgorithm;

#[derive(Clone, Copy)]
struct OutputLength(usize);

impl KeyType for OutputLength {
    fn len(&self) -> usize {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HkdfError;

impl fmt::Display for HkdfError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("HKDF derivation failed")
    }
}

impl std::error::Error for HkdfError {}

pub fn derive_hkdf_bytes(
    algorithm: DigestAlgorithm,
    secret: &[u8],
    salt: &[u8],
    info: &[u8],
    output_len: usize,
) -> Result<Vec<u8>, HkdfError> {
    if output_len == 0 {
        return Ok(Vec::new());
    }

    let algorithm = match algorithm {
        DigestAlgorithm::Sha1 => HKDF_SHA1_FOR_LEGACY_USE_ONLY,
        DigestAlgorithm::Sha256 => HKDF_SHA256,
        DigestAlgorithm::Sha384 => HKDF_SHA384,
        DigestAlgorithm::Sha512 => HKDF_SHA512,
    };
    let salt = hkdf::Salt::new(algorithm, salt);
    let pseudo_random_key = salt.extract(secret);
    let info = [info];
    let output_key_material = pseudo_random_key
        .expand(&info, OutputLength(output_len))
        .map_err(|_| HkdfError)?;
    let mut output = vec![0_u8; output_len];
    output_key_material
        .fill(&mut output)
        .map_err(|_| HkdfError)?;
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derives_rfc5869_sha256_vector() {
        let output = derive_hkdf_bytes(
            DigestAlgorithm::Sha256,
            &[0x0b; 22],
            &[
                0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c,
            ],
            &[0xf0, 0xf1, 0xf2, 0xf3, 0xf4, 0xf5, 0xf6, 0xf7, 0xf8, 0xf9],
            42,
        )
        .expect("RFC 5869 vector should derive");

        assert_eq!(
            output,
            [
                0x3c, 0xb2, 0x5f, 0x25, 0xfa, 0xac, 0xd5, 0x7a, 0x90, 0x43, 0x4f, 0x64, 0xd0, 0x36,
                0x2f, 0x2a, 0x2d, 0x2d, 0x0a, 0x90, 0xcf, 0x1a, 0x5a, 0x4c, 0x5d, 0xb0, 0x2d, 0x56,
                0xec, 0xc4, 0xc5, 0xbf, 0x34, 0x00, 0x72, 0x08, 0xd5, 0xb8, 0x87, 0x18, 0x58, 0x65,
            ]
        );
    }

    #[test]
    fn accepts_empty_webcrypto_inputs_and_output() {
        assert_eq!(
            derive_hkdf_bytes(DigestAlgorithm::Sha256, &[], &[], &[], 32)
                .expect("empty WebCrypto HKDF inputs should derive"),
            [
                0xeb, 0x70, 0xf0, 0x1d, 0xed, 0xe9, 0xaf, 0xaf, 0xa4, 0x49, 0xee, 0xe1, 0xb1, 0x28,
                0x65, 0x04, 0xe1, 0xf6, 0x23, 0x88, 0xb3, 0xf7, 0xdd, 0x4f, 0x95, 0x66, 0x97, 0xb0,
                0xe8, 0x28, 0xfe, 0x18,
            ]
        );
        assert_eq!(
            derive_hkdf_bytes(DigestAlgorithm::Sha256, &[], &[], &[], 0),
            Ok(Vec::new())
        );
    }
}
