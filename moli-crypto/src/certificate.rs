//! Self-signed X.509 certificates with an opaque, reference-counted private key.
//! Browser algorithm normalization, origins and lifetime policy stay outside
//! this module. Clones lease the key; they never serialize its private bytes.

use std::{
    fmt,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use openssl::{
    asn1::Asn1Time,
    bn::BigNum,
    ec::{EcGroup, EcKey},
    hash::MessageDigest,
    nid::Nid,
    pkey::{PKey, Private},
    rsa::Rsa,
    x509::{X509, X509NameBuilder},
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CertificateKeyAlgorithm {
    EcdsaP256,
    Rsa {
        modulus_bits: u32,
        public_exponent: u32,
    },
}

impl CertificateKeyAlgorithm {
    pub fn is_supported(self) -> bool {
        match self {
            Self::EcdsaP256 => true,
            Self::Rsa {
                modulus_bits,
                public_exponent,
            } => {
                matches!(modulus_bits, 1024 | 2048 | 3072 | 4096)
                    && matches!(public_exponent, 3 | 65537)
            }
        }
    }
}

#[derive(Clone)]
pub struct SelfSignedCertificate {
    certificate: Arc<X509>,
    key: Arc<PKey<Private>>,
    expires_millis: u64,
    sha256_fingerprint: Arc<str>,
}

impl SelfSignedCertificate {
    pub fn generate(
        algorithm: CertificateKeyAlgorithm,
        valid_for: Duration,
    ) -> Result<Self, CertificateError> {
        if !algorithm.is_supported() {
            return Err(CertificateError);
        }
        let key = match algorithm {
            CertificateKeyAlgorithm::EcdsaP256 => {
                let group = EcGroup::from_curve_name(Nid::X9_62_PRIME256V1)?;
                PKey::from_ec_key(EcKey::generate(&group)?)?
            }
            CertificateKeyAlgorithm::Rsa {
                modulus_bits,
                public_exponent,
            } => {
                let exponent = BigNum::from_u32(public_exponent)?;
                PKey::from_rsa(Rsa::generate_with_e(modulus_bits, &exponent)?)?
            }
        };
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| CertificateError)?;
        let expiration = now.checked_add(valid_for).ok_or(CertificateError)?;
        // X.509 validity is encoded to whole seconds. The browser-facing
        // timestamp must describe that same expiry, never a later instant.
        let expires_millis = expiration
            .as_secs()
            .checked_mul(1000)
            .ok_or(CertificateError)?;
        let not_before =
            Asn1Time::from_unix(i64::try_from(now.as_secs()).map_err(|_| CertificateError)?)?;
        let not_after = Asn1Time::from_unix(
            i64::try_from(expiration.as_secs()).map_err(|_| CertificateError)?,
        )?;
        let mut random = [0_u8; 32];
        crate::fill_secure_random(&mut random).map_err(|_| CertificateError)?;
        let common_name: String = random[..16]
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        random[16] |= 1;
        let serial = BigNum::from_slice(&random[16..])?.to_asn1_integer()?;
        let mut name = X509NameBuilder::new()?;
        name.append_entry_by_nid(Nid::COMMONNAME, &common_name)?;
        let name = name.build();
        let mut certificate = X509::builder()?;
        certificate.set_version(2)?;
        certificate.set_serial_number(&serial)?;
        certificate.set_subject_name(&name)?;
        certificate.set_issuer_name(&name)?;
        certificate.set_pubkey(&key)?;
        certificate.set_not_before(&not_before)?;
        certificate.set_not_after(&not_after)?;
        certificate.sign(&key, MessageDigest::sha256())?;
        let certificate = certificate.build();
        let digest = certificate.digest(MessageDigest::sha256())?;
        let sha256_fingerprint = digest
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<Vec<_>>()
            .join(":");
        Ok(Self {
            certificate: Arc::new(certificate),
            key: Arc::new(key),
            expires_millis,
            sha256_fingerprint: sha256_fingerprint.into(),
        })
    }

    pub fn expires_millis(&self) -> u64 {
        self.expires_millis
    }
    pub fn sha256_fingerprint(&self) -> &str {
        &self.sha256_fingerprint
    }
    pub fn public_der(&self) -> Result<Vec<u8>, CertificateError> {
        Ok(self.certificate.to_der()?)
    }
    pub fn same_certificate(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.key, &other.key) && Arc::ptr_eq(&self.certificate, &other.certificate)
    }
}

impl fmt::Debug for SelfSignedCertificate {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SelfSignedCertificate")
            .field("expires_millis", &self.expires_millis)
            .field("sha256_fingerprint", &self.sha256_fingerprint)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CertificateError;
impl From<openssl::error::ErrorStack> for CertificateError {
    fn from(_: openssl::error::ErrorStack) -> Self {
        Self
    }
}
impl fmt::Display for CertificateError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("certificate generation failed")
    }
}
impl std::error::Error for CertificateError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn certificates_have_valid_signatures_and_fingerprints_for_their_actual_der() {
        for algorithm in [
            CertificateKeyAlgorithm::EcdsaP256,
            CertificateKeyAlgorithm::Rsa {
                modulus_bits: 1024,
                public_exponent: 65537,
            },
            CertificateKeyAlgorithm::Rsa {
                modulus_bits: 2048,
                public_exponent: 65537,
            },
        ] {
            let start = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_millis() as u64;
            let value = SelfSignedCertificate::generate(algorithm, Duration::from_secs(2)).unwrap();
            let der = value.public_der().unwrap();
            let decoded = X509::from_der(&der).unwrap();
            assert!(decoded.verify(&decoded.public_key().unwrap()).unwrap());
            assert!(decoded.public_key().unwrap().public_eq(&value.key));
            assert_eq!(
                decoded.signature_algorithm().object().nid(),
                match algorithm {
                    CertificateKeyAlgorithm::EcdsaP256 => Nid::ECDSA_WITH_SHA256,
                    CertificateKeyAlgorithm::Rsa { .. } => Nid::SHA256WITHRSAENCRYPTION,
                }
            );
            let digest = crate::sha256_digest(&der);
            let expected = digest
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<Vec<_>>()
                .join(":");
            assert_eq!(value.sha256_fingerprint(), expected);
            assert!(value.expires_millis() > start + 1000);
            let end = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_millis() as u64;
            assert!(value.expires_millis() <= end + 2000);
        }
    }

    #[test]
    fn clones_keep_the_same_private_handle_and_new_certificates_are_unlinkable() {
        let first =
            SelfSignedCertificate::generate(CertificateKeyAlgorithm::EcdsaP256, Duration::ZERO)
                .unwrap();
        let cloned = first.clone();
        let weak = Arc::downgrade(&first.key);
        let second =
            SelfSignedCertificate::generate(CertificateKeyAlgorithm::EcdsaP256, Duration::ZERO)
                .unwrap();
        assert!(first.same_certificate(&cloned));
        assert!(!first.same_certificate(&second));
        assert_ne!(first.public_der().unwrap(), second.public_der().unwrap());
        assert_ne!(
            first.certificate.serial_number().to_bn().unwrap(),
            second.certificate.serial_number().to_bn().unwrap()
        );
        assert_ne!(
            first.certificate.subject_name().to_der().unwrap(),
            second.certificate.subject_name().to_der().unwrap()
        );
        drop(first);
        assert!(weak.upgrade().is_some());
        drop(cloned);
        assert!(weak.upgrade().is_none());
    }

    #[test]
    fn unsupported_key_parameters_fail_before_key_generation() {
        for algorithm in [
            CertificateKeyAlgorithm::Rsa {
                modulus_bits: u32::MAX,
                public_exponent: 65537,
            },
            CertificateKeyAlgorithm::Rsa {
                modulus_bits: 2048,
                public_exponent: 0,
            },
        ] {
            assert_eq!(
                SelfSignedCertificate::generate(algorithm, Duration::ZERO).unwrap_err(),
                CertificateError
            );
        }
    }
}
