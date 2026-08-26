use moli_crypto::DigestAlgorithm;
use openssl::hash::MessageDigest;

use crate::{WebCryptoError, limits::ensure_digest_operation_bytes};

#[derive(Clone, Copy, Debug, Eq, PartialEq, strum::AsRefStr, strum::EnumString)]
pub enum WebCryptoHashAlgorithm {
    #[strum(serialize = "sha-1")]
    Sha1,
    #[strum(serialize = "sha-256")]
    Sha256,
    #[strum(serialize = "sha-384")]
    Sha384,
    #[strum(serialize = "sha-512")]
    Sha512,
}
impl WebCryptoHashAlgorithm {
    pub fn digest_bytes(self, data: impl AsRef<[u8]>) -> Vec<u8> {
        self.digest_algorithm().digest_bytes(data)
    }

    pub fn digest(self, data: impl AsRef<[u8]>) -> Result<Vec<u8>, WebCryptoError> {
        Ok(self.digest_bytes(data))
    }

    pub fn digest_with_limit(self, data: impl AsRef<[u8]>) -> Result<Vec<u8>, WebCryptoError> {
        ensure_digest_operation_bytes(data.as_ref())?;
        self.digest(data)
    }

    pub fn default_hmac_key_len_bytes(self) -> usize {
        match self {
            Self::Sha384 | Self::Sha512 => 128,
            Self::Sha256 | Self::Sha1 => 64,
        }
    }

    pub fn jwk_hmac_alg(self) -> &'static str {
        match self {
            Self::Sha1 => "HS1",
            Self::Sha256 => "HS256",
            Self::Sha384 => "HS384",
            Self::Sha512 => "HS512",
        }
    }

    pub fn output_len_bytes(self) -> usize {
        self.digest_algorithm().output_len_bytes()
    }

    pub(crate) fn digest_algorithm(self) -> DigestAlgorithm {
        match self {
            Self::Sha1 => DigestAlgorithm::Sha1,
            Self::Sha256 => DigestAlgorithm::Sha256,
            Self::Sha384 => DigestAlgorithm::Sha384,
            Self::Sha512 => DigestAlgorithm::Sha512,
        }
    }

    // Keep the WebCrypto-to-OpenSSL mapping for algorithms that still consume
    // an OpenSSL digest descriptor (HMAC, RSA, and PBKDF2) in one place.
    pub(crate) fn message_digest(self) -> MessageDigest {
        match self {
            Self::Sha1 => MessageDigest::sha1(),
            Self::Sha256 => MessageDigest::sha256(),
            Self::Sha384 => MessageDigest::sha384(),
            Self::Sha512 => MessageDigest::sha512(),
        }
    }
}
