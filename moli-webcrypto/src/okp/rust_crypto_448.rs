use ed448_goldilocks::{Signature, SigningKey, VerifyingKey};
use moli_crypto::fill_secure_random;
use pkcs8::{
    AlgorithmIdentifierRef, ObjectIdentifier, PrivateKeyInfoRef, SubjectPublicKeyInfoRef,
    der::{
        Decode, Encode,
        asn1::{BitStringRef, OctetStringRef},
    },
};
use x448::{PublicKey as X448PublicKey, StaticSecret as X448PrivateKey};
use zeroize::Zeroizing;

use super::{OkpKeyPair, WebCryptoOkpCurve};
use crate::WebCryptoError;
use crate::bits::truncate_derived_bits;

const ED448_OID: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.3.101.113");
const X448_OID: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.3.101.111");

pub(super) fn generate_key_pair(curve: WebCryptoOkpCurve) -> Result<OkpKeyPair, WebCryptoError> {
    match curve {
        WebCryptoOkpCurve::Ed448 => generate_ed448_key_pair(),
        WebCryptoOkpCurve::X448 => generate_x448_key_pair(),
        WebCryptoOkpCurve::Ed25519 => Err(WebCryptoError::Operation),
    }
}

fn generate_ed448_key_pair() -> Result<OkpKeyPair, WebCryptoError> {
    let mut private_key = Zeroizing::new(vec![0_u8; WebCryptoOkpCurve::Ed448.raw_len()]);
    fill_secure_random(&mut private_key).map_err(|_| WebCryptoError::Operation)?;
    let signing_key = ed448_signing_key(&private_key)?;
    let public_key = signing_key.verifying_key().to_bytes().to_vec();
    Ok(OkpKeyPair {
        private_key,
        public_key,
    })
}

fn generate_x448_key_pair() -> Result<OkpKeyPair, WebCryptoError> {
    let mut seed = Zeroizing::new([0_u8; 56]);
    fill_secure_random(seed.as_mut()).map_err(|_| WebCryptoError::Operation)?;
    let private_key = X448PrivateKey::from(*seed);
    let public_key = X448PublicKey::from(&private_key).as_bytes().to_vec();
    Ok(OkpKeyPair {
        private_key: Zeroizing::new(private_key.as_bytes().to_vec()),
        public_key,
    })
}

pub(super) fn public_key_from_private(
    curve: WebCryptoOkpCurve,
    private_key: &[u8],
) -> Result<Vec<u8>, WebCryptoError> {
    match curve {
        WebCryptoOkpCurve::Ed448 => Ok(ed448_signing_key(private_key)?
            .verifying_key()
            .to_bytes()
            .to_vec()),
        WebCryptoOkpCurve::X448 => {
            let private_key = x448_private_key(private_key)?;
            Ok(X448PublicKey::from(&private_key).as_bytes().to_vec())
        }
        WebCryptoOkpCurve::Ed25519 => Err(WebCryptoError::Operation),
    }
}

pub(super) fn import_spki_public_key(
    bytes: &[u8],
    curve: WebCryptoOkpCurve,
) -> Result<Vec<u8>, WebCryptoError> {
    let spki = SubjectPublicKeyInfoRef::try_from(bytes).map_err(|_| WebCryptoError::Data)?;
    validate_algorithm_identifier(spki.algorithm, curve)?;
    let public_key = spki
        .subject_public_key
        .as_bytes()
        .ok_or(WebCryptoError::Data)?;
    validate_raw_key_len(public_key, curve, WebCryptoError::Data)?;
    Ok(public_key.to_vec())
}

pub(super) fn import_pkcs8_private_key(
    bytes: &[u8],
    curve: WebCryptoOkpCurve,
) -> Result<Zeroizing<Vec<u8>>, WebCryptoError> {
    let private_key_info = PrivateKeyInfoRef::try_from(bytes).map_err(|_| WebCryptoError::Data)?;
    validate_algorithm_identifier(private_key_info.algorithm, curve)?;
    let nested_private_key = <&OctetStringRef>::from_der(private_key_info.private_key.as_bytes())
        .map_err(|_| WebCryptoError::Data)?;
    let private_key = nested_private_key.as_bytes();
    validate_raw_key_len(private_key, curve, WebCryptoError::Data)?;

    if let Some(encoded_public_key) = private_key_info.public_key {
        let encoded_public_key = encoded_public_key.as_bytes().ok_or(WebCryptoError::Data)?;
        validate_raw_key_len(encoded_public_key, curve, WebCryptoError::Data)?;
        if public_key_from_private(curve, private_key)? != encoded_public_key {
            return Err(WebCryptoError::Data);
        }
    }

    Ok(Zeroizing::new(private_key.to_vec()))
}

pub(super) fn export_spki_public_key(
    curve: WebCryptoOkpCurve,
    public_key: &[u8],
) -> Result<Vec<u8>, WebCryptoError> {
    validate_raw_key_len(public_key, curve, WebCryptoError::Operation)?;
    let subject_public_key =
        BitStringRef::new(0, public_key).map_err(|_| WebCryptoError::Operation)?;
    SubjectPublicKeyInfoRef {
        algorithm: algorithm_identifier(curve)?,
        subject_public_key,
    }
    .to_der()
    .map_err(|_| WebCryptoError::Operation)
}

pub(super) fn export_pkcs8_private_key(
    curve: WebCryptoOkpCurve,
    private_key: &[u8],
) -> Result<Vec<u8>, WebCryptoError> {
    validate_raw_key_len(private_key, curve, WebCryptoError::Operation)?;
    let mut nested_private_key = Zeroizing::new(Vec::with_capacity(private_key.len() + 2));
    nested_private_key.push(0x04);
    nested_private_key.push(private_key.len() as u8);
    nested_private_key.extend_from_slice(private_key);
    let private_key =
        OctetStringRef::new(&nested_private_key).map_err(|_| WebCryptoError::Operation)?;
    PrivateKeyInfoRef::new(algorithm_identifier(curve)?, private_key)
        .to_der()
        .map_err(|_| WebCryptoError::Operation)
}

pub(super) fn sign_ed448(private_key: &[u8], data: &[u8]) -> Result<Vec<u8>, WebCryptoError> {
    Ok(ed448_signing_key(private_key)?
        .sign_raw(data)
        .to_bytes()
        .to_vec())
}

pub(super) fn verify_ed448(
    public_key: &[u8],
    data: &[u8],
    signature: &[u8],
) -> Result<bool, WebCryptoError> {
    let public_key: &[u8; 57] = public_key
        .try_into()
        .map_err(|_| WebCryptoError::Operation)?;
    let Ok(public_key) = VerifyingKey::from_bytes(public_key) else {
        return Ok(false);
    };
    let Ok(signature) = Signature::try_from(signature) else {
        return Ok(false);
    };
    Ok(public_key.verify_raw(&signature, data).is_ok())
}

pub(super) fn derive_x448_bits(
    private_key: &[u8],
    public_key: &[u8],
    length_bits: usize,
) -> Result<Vec<u8>, WebCryptoError> {
    let private_key = private_key
        .try_into()
        .map_err(|_| WebCryptoError::Operation)?;
    let public_key = public_key
        .try_into()
        .map_err(|_| WebCryptoError::Operation)?;
    let secret = x448::x448(private_key, public_key).ok_or(WebCryptoError::Operation)?;
    if secret.iter().all(|byte| *byte == 0) {
        return Err(WebCryptoError::Operation);
    }
    truncate_derived_bits(&secret, length_bits)
}

fn ed448_signing_key(private_key: &[u8]) -> Result<SigningKey, WebCryptoError> {
    SigningKey::try_from(private_key).map_err(|_| WebCryptoError::Operation)
}

fn x448_private_key(private_key: &[u8]) -> Result<X448PrivateKey, WebCryptoError> {
    let private_key: [u8; 56] = private_key
        .try_into()
        .map_err(|_| WebCryptoError::Operation)?;
    Ok(X448PrivateKey::from(private_key))
}

fn algorithm_identifier(
    curve: WebCryptoOkpCurve,
) -> Result<AlgorithmIdentifierRef<'static>, WebCryptoError> {
    Ok(AlgorithmIdentifierRef {
        oid: algorithm_oid(curve)?,
        parameters: None,
    })
}

fn validate_algorithm_identifier(
    algorithm: AlgorithmIdentifierRef<'_>,
    curve: WebCryptoOkpCurve,
) -> Result<(), WebCryptoError> {
    if algorithm.oid != algorithm_oid(curve)? || algorithm.parameters.is_some() {
        return Err(WebCryptoError::Data);
    }
    Ok(())
}

fn algorithm_oid(curve: WebCryptoOkpCurve) -> Result<ObjectIdentifier, WebCryptoError> {
    match curve {
        WebCryptoOkpCurve::Ed448 => Ok(ED448_OID),
        WebCryptoOkpCurve::X448 => Ok(X448_OID),
        WebCryptoOkpCurve::Ed25519 => Err(WebCryptoError::Operation),
    }
}

fn validate_raw_key_len(
    key: &[u8],
    curve: WebCryptoOkpCurve,
    error: WebCryptoError,
) -> Result<(), WebCryptoError> {
    if key.len() != curve.raw_len() {
        return Err(error);
    }
    Ok(())
}
