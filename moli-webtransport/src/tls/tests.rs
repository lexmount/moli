use std::{sync::Arc, time::Duration};

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
use rustls::{
    client::danger::ServerCertVerifier,
    pki_types::{CertificateDer, ServerName, UnixTime},
};

use super::fingerprints::Verifier;

const NOW: u64 = 1_790_000_000;
const DAY: i64 = 86400;

pub(crate) fn identity(start: i64, end: i64, rsa: bool, version: i32) -> (X509, PKey<Private>) {
    let key = if rsa {
        PKey::from_rsa(Rsa::generate(2048).unwrap()).unwrap()
    } else {
        let group = EcGroup::from_curve_name(Nid::X9_62_PRIME256V1).unwrap();
        PKey::from_ec_key(EcKey::generate(&group).unwrap()).unwrap()
    };
    let mut name = X509NameBuilder::new().unwrap();
    name.append_entry_by_text("CN", "pinned-only.invalid")
        .unwrap();
    let name = name.build();
    let serial = BigNum::from_u32(1).unwrap().to_asn1_integer().unwrap();
    let mut certificate = X509::builder().unwrap();
    certificate.set_version(version).unwrap();
    certificate.set_serial_number(&serial).unwrap();
    certificate.set_subject_name(&name).unwrap();
    certificate.set_issuer_name(&name).unwrap();
    certificate.set_pubkey(&key).unwrap();
    certificate
        .set_not_before(&Asn1Time::from_unix(start).unwrap())
        .unwrap();
    certificate
        .set_not_after(&Asn1Time::from_unix(end).unwrap())
        .unwrap();
    certificate.sign(&key, MessageDigest::sha256()).unwrap();
    (certificate.build(), key)
}

fn hash(der: &[u8], algorithm: &str) -> crate::CertificateHash {
    crate::CertificateHash {
        algorithm: algorithm.to_owned(),
        value: openssl::hash::hash(MessageDigest::sha256(), der)
            .unwrap()
            .to_vec(),
    }
}

fn verify(certificate: &X509, hashes: Vec<crate::CertificateHash>) -> anyhow::Result<()> {
    let verifier = Verifier::new(
        hashes,
        Arc::new(rustls::crypto::aws_lc_rs::default_provider()),
    )?;
    let certificate = CertificateDer::from(certificate.to_der()?);
    // A matching fingerprint authenticates the certificate itself, not its CN
    // or a CA path. TLS signatures are verified separately by the provider.
    verifier.verify_server_cert(
        &certificate,
        &[],
        &ServerName::try_from("different.invalid").unwrap(),
        &[],
        UnixTime::since_unix_epoch(Duration::from_secs(NOW)),
    )?;
    Ok(())
}

#[test]
fn matching_sha256_pin_accepts_a_current_short_lived_self_signed_leaf() {
    let (certificate, _) = identity(NOW as i64 - DAY, NOW as i64 + 13 * DAY, false, 2);
    let der = certificate.to_der().unwrap();
    verify(&certificate, vec![hash(&der, "ShA-256")]).unwrap();
}

#[test]
fn any_supported_matching_pin_can_authenticate_the_leaf() {
    let (certificate, _) = identity(NOW as i64 - DAY, NOW as i64 + DAY, false, 2);
    let der = certificate.to_der().unwrap();
    verify(
        &certificate,
        vec![
            hash(&der, "sha-512"),
            crate::CertificateHash {
                algorithm: "sha-256".into(),
                value: vec![0; 32],
            },
            hash(&der, "SHA-256"),
        ],
    )
    .unwrap();
}

#[test]
fn mismatched_or_unsupported_pins_never_fall_back_to_certificate_trust() {
    let (certificate, _) = identity(NOW as i64 - DAY, NOW as i64 + DAY, false, 2);
    assert!(
        verify(
            &certificate,
            vec![crate::CertificateHash {
                algorithm: "sha-256".into(),
                value: vec![0; 32],
            }]
        )
        .unwrap_err()
        .to_string()
        .contains("hash mismatch")
    );
    assert!(
        verify(
            &certificate,
            vec![hash(&certificate.to_der().unwrap(), "sha-384")]
        )
        .is_err()
    );
    assert!(
        verify(
            &certificate,
            vec![crate::CertificateHash {
                algorithm: "sha-256".into(),
                value: vec![0; 31],
            }]
        )
        .is_err()
    );
}

#[test]
fn matching_pin_does_not_override_certificate_time_or_version_policy() {
    for (start, end, version, message) in [
        (NOW as i64 - DAY, NOW as i64 + 13 * DAY + 1, 2, "two weeks"),
        (NOW as i64 - 2 * DAY, NOW as i64 - DAY, 2, "expired"),
        (NOW as i64 + 1, NOW as i64 + DAY, 2, "not yet valid"),
        (NOW as i64 - DAY, NOW as i64 + DAY, 0, "X.509v3"),
    ] {
        let (certificate, _) = identity(start, end, false, version);
        let error = verify(
            &certificate,
            vec![hash(&certificate.to_der().unwrap(), "sha-256")],
        )
        .unwrap_err();
        assert!(error.to_string().contains(message), "{message}: {error}");
    }
}

#[test]
fn matching_pin_does_not_enable_an_rsa_certificate() {
    let (certificate, _) = identity(NOW as i64 - DAY, NOW as i64 + DAY, true, 2);
    let error = verify(
        &certificate,
        vec![hash(&certificate.to_der().unwrap(), "sha-256")],
    )
    .unwrap_err();
    assert!(error.to_string().contains("RSA"), "{error}");
}

#[test]
fn matching_hash_of_malformed_der_is_not_a_certificate() {
    let der = CertificateDer::from(b"not an X.509 certificate".to_vec());
    let verifier = Verifier::new(
        vec![hash(&der, "sha-256")],
        Arc::new(rustls::crypto::aws_lc_rs::default_provider()),
    )
    .unwrap();
    assert!(
        verifier
            .verify_server_cert(
                &der,
                &[],
                &ServerName::try_from("wt.test").unwrap(),
                &[],
                UnixTime::since_unix_epoch(Duration::from_secs(NOW))
            )
            .is_err()
    );
}

#[test]
fn webtransport_omits_http_client_identity_even_with_pinned_server_trust() {
    let (certificate, _) = identity(NOW as i64 - DAY, NOW as i64 + DAY, false, 2);
    let config = moli_curl::CurlTlsConfig {
        ca_cert: Some("not-a-server-ca.pem".into()),
        client_cert: Some("not-a-client-certificate.pem".into()),
        client_key: Some("not-a-client-key.pem".into()),
        client_cert_password: Some("unused-password".into()),
        ..Default::default()
    };
    let tls = super::configure_with_hashes(
        &config,
        vec![hash(&certificate.to_der().unwrap(), "sha-256")],
    )
    .unwrap();
    assert!(!tls.client_auth_cert_resolver.has_certs());
}
