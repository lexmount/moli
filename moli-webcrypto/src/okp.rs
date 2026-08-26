use openssl::{
    pkey::{Id, PKey},
    sign::{Signer, Verifier},
};
use zeroize::Zeroizing;

use crate::jwk::{decode_jwk_base64url, encode_jwk_base64url, jwk_key_ops_allow_usages};
use crate::limits::{ensure_der_key_bytes, ensure_signature_operation_bytes};
use crate::{OkpJsonWebKeyExport, OkpJsonWebKeyImport, WebCryptoError};

mod rust_crypto_448;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WebCryptoOkpCurve {
    Ed25519,
    Ed448,
    X448,
}

impl WebCryptoOkpCurve {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ed25519 => "Ed25519",
            Self::Ed448 => "Ed448",
            Self::X448 => "X448",
        }
    }

    fn raw_len(self) -> usize {
        match self {
            Self::Ed25519 => 32,
            Self::Ed448 => 57,
            Self::X448 => 56,
        }
    }
}

pub struct OkpKeyPair {
    pub private_key: Zeroizing<Vec<u8>>,
    pub public_key: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OkpPublicKey {
    pub key_bytes: Vec<u8>,
    pub curve: WebCryptoOkpCurve,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OkpPrivateKey {
    pub key_bytes: Zeroizing<Vec<u8>>,
    pub curve: WebCryptoOkpCurve,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum OkpImportedKey {
    Public(OkpPublicKey),
    Private(OkpPrivateKey),
}

pub fn generate_okp_key_pair(curve: WebCryptoOkpCurve) -> Result<OkpKeyPair, WebCryptoError> {
    match curve {
        WebCryptoOkpCurve::Ed25519 => {
            let key = PKey::generate_ed25519().map_err(|_| WebCryptoError::Operation)?;
            Ok(OkpKeyPair {
                private_key: Zeroizing::new(
                    key.raw_private_key()
                        .map_err(|_| WebCryptoError::Operation)?,
                ),
                public_key: key
                    .raw_public_key()
                    .map_err(|_| WebCryptoError::Operation)?,
            })
        }
        WebCryptoOkpCurve::Ed448 | WebCryptoOkpCurve::X448 => {
            rust_crypto_448::generate_key_pair(curve)
        }
    }
}

pub fn okp_public_key_from_private(
    curve: WebCryptoOkpCurve,
    private_key: &[u8],
) -> Result<Vec<u8>, WebCryptoError> {
    match curve {
        WebCryptoOkpCurve::Ed25519 => {
            let key = PKey::private_key_from_raw_bytes(private_key, Id::ED25519)
                .map_err(|_| WebCryptoError::Operation)?;
            key.raw_public_key().map_err(|_| WebCryptoError::Operation)
        }
        WebCryptoOkpCurve::Ed448 | WebCryptoOkpCurve::X448 => {
            rust_crypto_448::public_key_from_private(curve, private_key)
        }
    }
}

pub fn import_okp_raw_public_key(
    bytes: &[u8],
    curve: WebCryptoOkpCurve,
) -> Result<OkpPublicKey, WebCryptoError> {
    if bytes.len() != curve.raw_len() {
        return Err(WebCryptoError::Data);
    }
    let key_bytes = match curve {
        WebCryptoOkpCurve::Ed25519 => PKey::public_key_from_raw_bytes(bytes, Id::ED25519)
            .and_then(|key| key.raw_public_key())
            .map_err(|_| WebCryptoError::Data)?,
        WebCryptoOkpCurve::Ed448 | WebCryptoOkpCurve::X448 => bytes.to_vec(),
    };
    Ok(OkpPublicKey { key_bytes, curve })
}

pub fn import_okp_spki_public_key(
    bytes: &[u8],
    curve: WebCryptoOkpCurve,
) -> Result<OkpPublicKey, WebCryptoError> {
    ensure_der_key_bytes(bytes)?;
    match curve {
        WebCryptoOkpCurve::Ed25519 => {
            let key = PKey::public_key_from_der(bytes).map_err(|_| WebCryptoError::Data)?;
            if key.id() != Id::ED25519 {
                return Err(WebCryptoError::Data);
            }
            Ok(OkpPublicKey {
                key_bytes: key.raw_public_key().map_err(|_| WebCryptoError::Data)?,
                curve,
            })
        }
        WebCryptoOkpCurve::Ed448 | WebCryptoOkpCurve::X448 => Ok(OkpPublicKey {
            key_bytes: rust_crypto_448::import_spki_public_key(bytes, curve)?,
            curve,
        }),
    }
}

pub fn import_okp_pkcs8_private_key(
    bytes: &[u8],
    curve: WebCryptoOkpCurve,
) -> Result<OkpPrivateKey, WebCryptoError> {
    ensure_der_key_bytes(bytes)?;
    match curve {
        WebCryptoOkpCurve::Ed25519 => {
            let key = PKey::private_key_from_der(bytes).map_err(|_| WebCryptoError::Data)?;
            if key.id() != Id::ED25519 {
                return Err(WebCryptoError::Data);
            }
            Ok(OkpPrivateKey {
                key_bytes: Zeroizing::new(key.raw_private_key().map_err(|_| WebCryptoError::Data)?),
                curve,
            })
        }
        WebCryptoOkpCurve::Ed448 | WebCryptoOkpCurve::X448 => Ok(OkpPrivateKey {
            key_bytes: rust_crypto_448::import_pkcs8_private_key(bytes, curve)?,
            curve,
        }),
    }
}

pub fn export_okp_spki_public_key(
    curve: WebCryptoOkpCurve,
    public_key: &[u8],
) -> Result<Vec<u8>, WebCryptoError> {
    match curve {
        WebCryptoOkpCurve::Ed25519 => {
            let key = PKey::public_key_from_raw_bytes(public_key, Id::ED25519)
                .map_err(|_| WebCryptoError::Operation)?;
            key.public_key_to_der()
                .map_err(|_| WebCryptoError::Operation)
        }
        WebCryptoOkpCurve::Ed448 | WebCryptoOkpCurve::X448 => {
            rust_crypto_448::export_spki_public_key(curve, public_key)
        }
    }
}

pub fn export_okp_pkcs8_private_key(
    curve: WebCryptoOkpCurve,
    private_key: &[u8],
) -> Result<Vec<u8>, WebCryptoError> {
    match curve {
        WebCryptoOkpCurve::Ed25519 => {
            let key = PKey::private_key_from_raw_bytes(private_key, Id::ED25519)
                .map_err(|_| WebCryptoError::Operation)?;
            key.private_key_to_pkcs8()
                .map_err(|_| WebCryptoError::Operation)
        }
        WebCryptoOkpCurve::Ed448 | WebCryptoOkpCurve::X448 => {
            rust_crypto_448::export_pkcs8_private_key(curve, private_key)
        }
    }
}

pub fn import_okp_jwk_key(
    jwk: &OkpJsonWebKeyImport,
    curve: WebCryptoOkpCurve,
    extractable: bool,
    usages: &[String],
) -> Result<OkpImportedKey, WebCryptoError> {
    jwk.validate_resource_limits()?;
    if jwk.kty.as_deref() != Some("OKP") || jwk.crv.as_deref() != Some(curve.as_str()) {
        return Err(WebCryptoError::Data);
    }
    if extractable && jwk.ext == Some(false) {
        return Err(WebCryptoError::Data);
    }
    if matches!(curve, WebCryptoOkpCurve::Ed25519 | WebCryptoOkpCurve::Ed448)
        && jwk
            .alg
            .as_deref()
            .is_some_and(|alg| alg != curve.as_str() && alg != "EdDSA")
    {
        return Err(WebCryptoError::Data);
    }
    jwk_key_ops_allow_usages(
        jwk.key_ops.as_deref(),
        usages,
        jwk.public_key_use.as_deref(),
    )?;
    let public_key = decode_okp_jwk_public_member(jwk.x.as_deref(), curve)?;
    let Some(d) = jwk.d.as_deref() else {
        return Ok(OkpImportedKey::Public(OkpPublicKey {
            key_bytes: public_key,
            curve,
        }));
    };
    let private_key = Zeroizing::new(decode_okp_jwk_private_value(d, curve)?);
    if okp_public_key_from_private(curve, &private_key)? != public_key {
        return Err(WebCryptoError::Data);
    }
    Ok(OkpImportedKey::Private(OkpPrivateKey {
        key_bytes: private_key,
        curve,
    }))
}

pub fn export_okp_jwk_public_key(
    curve: WebCryptoOkpCurve,
    public_key: &[u8],
    key_ops: Vec<String>,
    ext: bool,
) -> OkpJsonWebKeyExport {
    OkpJsonWebKeyExport {
        kty: "OKP",
        crv: curve.as_str(),
        x: encode_jwk_base64url(public_key),
        d: None,
        alg: okp_jwk_alg(curve),
        key_ops,
        ext,
    }
}

pub fn export_okp_jwk_private_key(
    curve: WebCryptoOkpCurve,
    private_key: &[u8],
    key_ops: Vec<String>,
    ext: bool,
) -> Result<OkpJsonWebKeyExport, WebCryptoError> {
    Ok(OkpJsonWebKeyExport {
        kty: "OKP",
        crv: curve.as_str(),
        x: encode_jwk_base64url(&okp_public_key_from_private(curve, private_key)?),
        d: Some(encode_jwk_base64url(private_key)),
        alg: okp_jwk_alg(curve),
        key_ops,
        ext,
    })
}

fn okp_jwk_alg(curve: WebCryptoOkpCurve) -> Option<&'static str> {
    match curve {
        WebCryptoOkpCurve::Ed25519 | WebCryptoOkpCurve::Ed448 => Some(curve.as_str()),
        WebCryptoOkpCurve::X448 => None,
    }
}

pub fn eddsa_sign(
    curve: WebCryptoOkpCurve,
    private_key: &[u8],
    data: &[u8],
) -> Result<Vec<u8>, WebCryptoError> {
    ensure_signature_operation_bytes(data)?;
    if !matches!(curve, WebCryptoOkpCurve::Ed25519 | WebCryptoOkpCurve::Ed448) {
        return Err(WebCryptoError::Operation);
    }
    match curve {
        WebCryptoOkpCurve::Ed25519 => {
            let key = PKey::private_key_from_raw_bytes(private_key, Id::ED25519)
                .map_err(|_| WebCryptoError::Operation)?;
            let mut signer =
                Signer::new_without_digest(&key).map_err(|_| WebCryptoError::Operation)?;
            signer
                .sign_oneshot_to_vec(data)
                .map_err(|_| WebCryptoError::Operation)
        }
        WebCryptoOkpCurve::Ed448 => rust_crypto_448::sign_ed448(private_key, data),
        WebCryptoOkpCurve::X448 => Err(WebCryptoError::Operation),
    }
}

pub fn eddsa_verify(
    curve: WebCryptoOkpCurve,
    public_key: &[u8],
    data: &[u8],
    signature: &[u8],
) -> Result<bool, WebCryptoError> {
    ensure_signature_operation_bytes(data)?;
    ensure_signature_operation_bytes(signature)?;
    if !matches!(curve, WebCryptoOkpCurve::Ed25519 | WebCryptoOkpCurve::Ed448) {
        return Err(WebCryptoError::Operation);
    }
    if eddsa_has_rejected_verification_point(curve, public_key, signature) {
        return Ok(false);
    }
    match curve {
        WebCryptoOkpCurve::Ed25519 => {
            let key = PKey::public_key_from_raw_bytes(public_key, Id::ED25519)
                .map_err(|_| WebCryptoError::Operation)?;
            let mut verifier =
                Verifier::new_without_digest(&key).map_err(|_| WebCryptoError::Operation)?;
            verifier
                .verify_oneshot(signature, data)
                .map_err(|_| WebCryptoError::Operation)
        }
        WebCryptoOkpCurve::Ed448 => rust_crypto_448::verify_ed448(public_key, data, signature),
        WebCryptoOkpCurve::X448 => Err(WebCryptoError::Operation),
    }
}

fn eddsa_has_rejected_verification_point(
    curve: WebCryptoOkpCurve,
    public_key: &[u8],
    signature: &[u8],
) -> bool {
    match curve {
        WebCryptoOkpCurve::Ed25519 => {
            ed25519_is_small_order_or_noncanonical_small_order(public_key)
                || signature
                    .get(..32)
                    .is_some_and(ed25519_is_small_order_or_noncanonical_small_order)
        }
        WebCryptoOkpCurve::Ed448 | WebCryptoOkpCurve::X448 => false,
    }
}

fn ed25519_is_small_order_or_noncanonical_small_order(point: &[u8]) -> bool {
    const ED25519_SMALL_ORDER_POINTS: [[u8; 32]; 14] = [
        // Canonical encodings from WPT WebCryptoAPI/sign_verify/eddsa_vectors.js
        // and eprint.iacr.org/2020/1244 table 3.
        [
            0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00,
        ],
        [
            0xec, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
            0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
            0xff, 0xff, 0xff, 0x7f,
        ],
        [
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x80,
        ],
        [
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00,
        ],
        [
            0xc7, 0x17, 0x6a, 0x70, 0x3d, 0x4d, 0xd8, 0x4f, 0xba, 0x3c, 0x0b, 0x76, 0x0d, 0x10,
            0x67, 0x0f, 0x2a, 0x20, 0x53, 0xfa, 0x2c, 0x39, 0xcc, 0xc6, 0x4e, 0xc7, 0xfd, 0x77,
            0x92, 0xac, 0x03, 0x7a,
        ],
        [
            0xc7, 0x17, 0x6a, 0x70, 0x3d, 0x4d, 0xd8, 0x4f, 0xba, 0x3c, 0x0b, 0x76, 0x0d, 0x10,
            0x67, 0x0f, 0x2a, 0x20, 0x53, 0xfa, 0x2c, 0x39, 0xcc, 0xc6, 0x4e, 0xc7, 0xfd, 0x77,
            0x92, 0xac, 0x03, 0xfa,
        ],
        [
            0x26, 0xe8, 0x95, 0x8f, 0xc2, 0xb2, 0x27, 0xb0, 0x45, 0xc3, 0xf4, 0x89, 0xf2, 0xef,
            0x98, 0xf0, 0xd5, 0xdf, 0xac, 0x05, 0xd3, 0xc6, 0x33, 0x39, 0xb1, 0x38, 0x02, 0x88,
            0x6d, 0x53, 0xfc, 0x05,
        ],
        [
            0x26, 0xe8, 0x95, 0x8f, 0xc2, 0xb2, 0x27, 0xb0, 0x45, 0xc3, 0xf4, 0x89, 0xf2, 0xef,
            0x98, 0xf0, 0xd5, 0xdf, 0xac, 0x05, 0xd3, 0xc6, 0x33, 0x39, 0xb1, 0x38, 0x02, 0x88,
            0x6d, 0x53, 0xfc, 0x85,
        ],
        // Non-canonical encodings of the same small-order points. RFC 8032
        // verification rejects these as verification inputs; imports still
        // accept the raw bytes so WebCrypto.verify resolves false instead of
        // throwing.
        [
            0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x80,
        ],
        [
            0xec, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
            0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
            0xff, 0xff, 0xff, 0xff,
        ],
        [
            0xee, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
            0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
            0xff, 0xff, 0xff, 0x7f,
        ],
        [
            0xee, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
            0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
            0xff, 0xff, 0xff, 0xff,
        ],
        [
            0xed, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
            0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
            0xff, 0xff, 0xff, 0xff,
        ],
        [
            0xed, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
            0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
            0xff, 0xff, 0xff, 0x7f,
        ],
    ];

    point.len() == 32
        && ED25519_SMALL_ORDER_POINTS
            .iter()
            .any(|known| known.as_slice() == point)
}

pub fn derive_x448_bits(
    private_key: &[u8],
    public_key: &[u8],
    length_bits: usize,
) -> Result<Vec<u8>, WebCryptoError> {
    rust_crypto_448::derive_x448_bits(private_key, public_key, length_bits)
}

fn decode_okp_jwk_public_member(
    value: Option<&str>,
    curve: WebCryptoOkpCurve,
) -> Result<Vec<u8>, WebCryptoError> {
    let Some(value) = value else {
        return Err(WebCryptoError::Data);
    };
    let decoded = decode_okp_jwk_value(value, curve)?;
    Ok(import_okp_raw_public_key(&decoded, curve)?.key_bytes)
}

fn decode_okp_jwk_private_value(
    value: &str,
    curve: WebCryptoOkpCurve,
) -> Result<Vec<u8>, WebCryptoError> {
    let decoded = decode_okp_jwk_value(value, curve)?;
    if curve == WebCryptoOkpCurve::Ed25519 {
        let _ = PKey::private_key_from_raw_bytes(&decoded, Id::ED25519)
            .map_err(|_| WebCryptoError::Data)?;
    }
    Ok(decoded)
}

fn decode_okp_jwk_value(value: &str, curve: WebCryptoOkpCurve) -> Result<Vec<u8>, WebCryptoError> {
    let decoded = decode_jwk_base64url(value)?;
    if decoded.len() != curve.raw_len() {
        return Err(WebCryptoError::Data);
    }
    Ok(decoded)
}
