//! WebTransport authenticates a pinned leaf, rather than a DNS name or CA.
//! TLS CertificateVerify still proves possession of that leaf's private key.

use std::{cmp::Ordering, sync::Arc};

use anyhow::{Result, ensure};
use der::{Decode, Reader, TagMode, TagNumber, asn1::AnyRef};
use openssl::{asn1::Asn1Time, hash::MessageDigest, pkey::Id, x509::X509};
use rustls::{
    DigitallySignedStruct, Error, SignatureScheme,
    client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier},
    crypto::{CryptoProvider, verify_tls12_signature, verify_tls13_signature},
    pki_types::{CertificateDer, ServerName, UnixTime},
};

#[derive(Debug)]
pub(super) struct Verifier {
    hashes: Vec<Vec<u8>>,
    provider: Arc<CryptoProvider>,
}

impl Verifier {
    pub(super) fn new(
        hashes: Vec<crate::CertificateHash>,
        provider: Arc<CryptoProvider>,
    ) -> Result<Self> {
        let hashes: Vec<_> = hashes
            .into_iter()
            .filter(|hash| hash.algorithm.eq_ignore_ascii_case("sha-256") && hash.value.len() == 32)
            .map(|hash| hash.value)
            .collect();
        ensure!(
            !hashes.is_empty(),
            "no supported WebTransport certificate hash"
        );
        Ok(Self { hashes, provider })
    }

    fn verify(&self, certificate: &CertificateDer<'_>, now: UnixTime) -> Result<()> {
        let digest = openssl::hash::hash(MessageDigest::sha256(), certificate.as_ref())?;
        ensure!(
            self.hashes
                .iter()
                .any(|hash| hash.as_slice() == digest.as_ref()),
            "WebTransport certificate hash mismatch"
        );
        let version = x509_version(certificate.as_ref())?;
        let certificate = X509::from_der(certificate.as_ref())?;
        ensure!(
            version == Some(2),
            "WebTransport requires an X.509v3 certificate"
        );
        let key = certificate.public_key()?;
        ensure!(
            !matches!(key.id(), Id::RSA | Id::RSA_PSS),
            "WebTransport certificate hashes do not permit RSA keys"
        );
        let lifetime = certificate.not_before().diff(certificate.not_after())?;
        let seconds = i64::from(lifetime.days) * 86400 + i64::from(lifetime.secs);
        ensure!(
            (0..=14 * 86400).contains(&seconds),
            "WebTransport pinned certificate validity exceeds two weeks"
        );
        let now = Asn1Time::from_unix(i64::try_from(now.as_secs())?)?;
        ensure!(
            certificate.not_before().compare(&now)? != Ordering::Greater,
            "WebTransport pinned certificate is not yet valid"
        );
        ensure!(
            certificate.not_after().compare(&now)? != Ordering::Less,
            "WebTransport pinned certificate has expired"
        );
        Ok(())
    }
}

impl ServerCertVerifier for Verifier {
    fn verify_server_cert(
        &self,
        certificate: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, Error> {
        self.verify(certificate, now)
            .map_err(|error| Error::General(error.to_string()))?;
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        certificate: &CertificateDer<'_>,
        signature: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        verify_tls12_signature(
            message,
            certificate,
            signature,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        certificate: &CertificateDer<'_>,
        signature: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        verify_tls13_signature(
            message,
            certificate,
            signature,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}

// OpenSSL's safe version accessor is unavailable with its AWS-LC backend.
// Decode the explicit ASN.1 version field with the shared DER reader; OpenSSL
// and TLS still parse and validate the complete certificate independently.
fn x509_version(certificate: &[u8]) -> Result<Option<u8>, der::Error> {
    AnyRef::from_der(certificate)?.sequence(|reader| {
        let tbs = reader.decode::<AnyRef<'_>>()?;
        reader.decode::<AnyRef<'_>>()?;
        reader.decode::<AnyRef<'_>>()?;
        tbs.sequence(|reader| {
            let version = reader.context_specific::<u8>(TagNumber(0), TagMode::Explicit)?;
            reader.read_slice(reader.remaining_len())?;
            Ok(version)
        })
    })
}
