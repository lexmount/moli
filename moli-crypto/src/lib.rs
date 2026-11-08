//! Backend-neutral cryptographic building blocks shared by Moli crates.
//!
//! This crate deliberately contains no WebIDL, browser-policy, algorithm-name
//! normalization, WebCrypto key, or JWK semantics. Those belong to the browser-facing
//! owner (`moli-webcrypto` or the renderer subsystem using the primitive).

mod certificate;
mod digest;
mod ed25519;
mod hkdf;
mod legacy;
mod random;

pub use certificate::{CertificateError, CertificateKeyAlgorithm, SelfSignedCertificate};
pub use digest::{DigestAlgorithm, Sha256Context, sha1_digest, sha256_digest, sha256_hex};
pub use ed25519::{Ed25519Error, Ed25519SigningKey};
pub use hkdf::{HkdfError, derive_hkdf_bytes};
pub use legacy::{
    LegacyCryptoError, aes_128_cbc_pkcs7_decrypt, aes_256_gcm_decrypt, derive_pbkdf2_hmac_sha1,
};
pub use random::fill_secure_random;

fn hex_lower(input: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(input.len() * 2);
    for &byte in input {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}

#[cfg(test)]
mod tests {
    use super::hex_lower;

    #[test]
    fn hex_lower_matches_standard_formatting_for_all_bytes() {
        assert_eq!(hex_lower(&[]), "");
        let input: Vec<_> = (u8::MIN..=u8::MAX).collect();
        let expected: String = input.iter().map(|byte| format!("{byte:02x}")).collect();
        assert_eq!(hex_lower(&input), expected);
    }
}
