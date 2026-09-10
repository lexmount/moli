use moli_webcrypto::{
    AesJsonWebKeyImport, Chacha20Poly1305JsonWebKeyImport, EcImportedKey, EcJsonWebKeyImport,
    EcKeyPair, HmacJsonWebKeyImport, MAX_AES_OPERATION_BYTES, MAX_DER_KEY_BYTES,
    MAX_DIGEST_OPERATION_BYTES, MAX_HMAC_KEY_LENGTH_BITS, MAX_JWK_KEY_OPS, MAX_JWK_MEMBER_BYTES,
    MAX_JWK_SERIALIZED_BYTES, MAX_KDF_DERIVED_BITS, MAX_KDF_PARAMETER_BYTES, MAX_PBKDF2_ITERATIONS,
    MAX_RAW_KEY_IMPORT_BYTES, MAX_RSA_MODULUS_LENGTH_BITS, MAX_RSA_OAEP_LABEL_BYTES,
    MAX_RSA_PUBLIC_EXPONENT_BYTES, MAX_SIGNATURE_OPERATION_BYTES, MIN_RSA_MODULUS_LENGTH_BITS,
    OkpImportedKey, OkpJsonWebKeyImport, OkpKeyPair, RsaImportedKey, RsaJsonWebKeyImport,
    RsaKeyPair, WebCryptoEcNamedCurve, WebCryptoError, WebCryptoHashAlgorithm,
    WebCryptoKeyAlgorithm, WebCryptoOkpCurve, X25519ImportedKey, X25519KeyPair, aes_cbc_decrypt,
    aes_cbc_encrypt, aes_ctr_crypt, aes_gcm_decrypt, aes_gcm_encrypt, aes_kw_unwrap, aes_kw_wrap,
    chacha20_poly1305_decrypt, chacha20_poly1305_encrypt, derive_ecdh_bits, derive_hkdf_bits,
    derive_pbkdf2_bits, derive_x448_bits, derive_x25519_bits, ec_public_key_from_private,
    ecdsa_sign, ecdsa_verify, eddsa_sign, eddsa_verify, export_aes_jwk,
    export_chacha20_poly1305_jwk, export_ec_jwk_private_key, export_ec_jwk_public_key,
    export_ec_raw_public_key, export_hmac_jwk, export_okp_jwk_private_key,
    export_okp_jwk_public_key, export_okp_pkcs8_private_key, export_okp_spki_public_key,
    export_rsa_jwk_private_key, export_rsa_jwk_public_key, export_x25519_jwk_private_key,
    export_x25519_jwk_public_key, export_x25519_pkcs8_private_key, export_x25519_spki_public_key,
    generate_aes_key, generate_chacha20_poly1305_key, generate_ec_key_pair, generate_hmac_key,
    generate_okp_key_pair, generate_rsa_key_pair, generate_x25519_key_pair, hmac_signature,
    import_aes_jwk_key, import_chacha20_poly1305_jwk_key, import_ec_jwk_key,
    import_ec_pkcs8_private_key, import_ec_raw_public_key, import_ec_spki_public_key,
    import_hmac_jwk_key, import_okp_jwk_key, import_okp_pkcs8_private_key,
    import_okp_raw_public_key, import_okp_spki_public_key, import_rsa_jwk_key,
    import_rsa_pkcs8_private_key, import_rsa_spki_public_key, import_x25519_jwk_key,
    import_x25519_pkcs8_private_key, import_x25519_raw_public_key, import_x25519_spki_public_key,
    okp_public_key_from_private, rsa_oaep_decrypt, rsa_oaep_encrypt, rsa_pkcs1_sign,
    rsa_pkcs1_verify, rsa_pss_sign, rsa_pss_verify, rsa_public_key_from_private,
    validate_aes_key_bytes, validate_chacha20_poly1305_key_bytes, validate_hmac_import_key_bytes,
    verify_hmac, x25519_public_key_from_private,
};

use crate::{
    context_bootstrap::WebCryptoTaskResult, util::context_host_ptr_from_global_bridge, webidl,
};

use super::helpers::{
    PendingCryptoPromise, WebCryptoRejection, crypto_algorithm_name, set_rejected_webcrypto_promise,
};
use super::keys::{
    CryptoKeyAlgorithmClonePayload, CryptoKeyClonePayload, build_hmac_algorithm_object,
    build_named_algorithm_object, build_symmetric_algorithm_object, crypto_key_algorithm_object,
    crypto_key_bytes, crypto_key_clone_payload_from_object, crypto_key_extractable,
    crypto_key_has_usage, crypto_key_kind, crypto_key_object_from_clone_payload, crypto_key_usages,
    is_crypto_key_object, new_crypto_key_object,
};
use super::*;

// The shared cipher entrypoint covers AES and RSA-OAEP. Valid RSA-OAEP payloads
// are far below the AES product cap, so use the same bound to reject hostile
// oversized BufferSource inputs before renderer-side copying.
const MAX_CIPHER_OPERATION_BYTES: usize = MAX_AES_OPERATION_BYTES;

mod cipher;
mod derive;
mod export_import;
mod key_gen;
mod params;
mod supports;
mod tasks;

pub(crate) use cipher::*;
pub(crate) use derive::*;
pub(crate) use export_import::*;
pub(crate) use key_gen::*;
pub(crate) use params::*;
pub(crate) use supports::*;
pub(crate) use tasks::*;
