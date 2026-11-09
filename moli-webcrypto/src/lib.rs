mod aes;
mod algorithm;
mod bits;
mod chacha;
mod ec;
mod error;
mod hash;
mod hmac;
mod jwk;
mod kdf;
mod limits;
mod okp;
mod rsa;
mod symmetric;
mod x25519;

pub use aes::{
    MAX_AES_OPERATION_BYTES, aes_cbc_decrypt, aes_cbc_encrypt, aes_ctr_crypt, aes_gcm_decrypt,
    aes_gcm_encrypt, aes_kw_unwrap, aes_kw_wrap, export_aes_jwk, generate_aes_key,
    import_aes_jwk_key, validate_aes_key_bytes,
};
pub use algorithm::WebCryptoKeyAlgorithm;
pub use chacha::{
    CHACHA20_POLY1305_KEY_LENGTH_BITS, MAX_CHACHA20_POLY1305_OPERATION_BYTES,
    chacha20_poly1305_decrypt, chacha20_poly1305_encrypt, export_chacha20_poly1305_jwk,
    generate_chacha20_poly1305_key, import_chacha20_poly1305_jwk_key,
    validate_chacha20_poly1305_key_bytes,
};
pub use ec::{
    EcImportedKey, EcKeyPair, EcPrivateKey, EcPublicKey, WebCryptoEcNamedCurve, derive_ecdh_bits,
    ec_public_key_from_private, ecdsa_sign, ecdsa_verify, export_ec_jwk_private_key,
    export_ec_jwk_public_key, export_ec_raw_public_key, generate_ec_key_pair, import_ec_jwk_key,
    import_ec_pkcs8_private_key, import_ec_raw_public_key, import_ec_spki_public_key,
};
pub use error::WebCryptoError;
pub use hash::WebCryptoHashAlgorithm;
pub use hmac::{
    MAX_HMAC_KEY_LENGTH_BITS, export_hmac_jwk, generate_hmac_key, hmac_signature,
    import_hmac_jwk_key, validate_hmac_import_key_bytes, verify_hmac,
};
pub use jwk::{
    AesJsonWebKeyExport, AesJsonWebKeyImport, Chacha20Poly1305JsonWebKeyExport,
    Chacha20Poly1305JsonWebKeyImport, EcJsonWebKeyExport, EcJsonWebKeyImport, HmacJsonWebKeyExport,
    HmacJsonWebKeyImport, MAX_JWK_KEY_OPS, MAX_JWK_MEMBER_BYTES, MAX_JWK_SERIALIZED_BYTES,
    OkpJsonWebKeyExport, OkpJsonWebKeyImport, RsaJsonWebKeyExport, RsaJsonWebKeyImport,
};
pub use kdf::{MAX_KDF_DERIVED_BITS, MAX_PBKDF2_ITERATIONS, derive_hkdf_bits, derive_pbkdf2_bits};
pub use limits::{
    MAX_DER_KEY_BYTES, MAX_DIGEST_OPERATION_BYTES, MAX_KDF_PARAMETER_BYTES,
    MAX_RAW_KEY_IMPORT_BYTES, MAX_RSA_OAEP_LABEL_BYTES, MAX_SIGNATURE_OPERATION_BYTES,
};
pub use okp::{
    OkpImportedKey, OkpKeyPair, OkpPrivateKey, OkpPublicKey, WebCryptoOkpCurve, derive_x448_bits,
    eddsa_sign, eddsa_verify, export_okp_jwk_private_key, export_okp_jwk_public_key,
    export_okp_pkcs8_private_key, export_okp_spki_public_key, generate_okp_key_pair,
    import_okp_jwk_key, import_okp_pkcs8_private_key, import_okp_raw_public_key,
    import_okp_spki_public_key, okp_public_key_from_private,
};
pub use rsa::{
    MAX_RSA_MODULUS_LENGTH_BITS, MAX_RSA_PUBLIC_EXPONENT_BYTES, MIN_RSA_MODULUS_LENGTH_BITS,
    RsaImportedKey, RsaKeyPair, RsaPrivateKey, RsaPublicKey, export_rsa_jwk_private_key,
    export_rsa_jwk_public_key, generate_rsa_key_pair, import_rsa_jwk_key,
    import_rsa_pkcs8_private_key, import_rsa_spki_public_key, rsa_oaep_decrypt, rsa_oaep_encrypt,
    rsa_pkcs1_sign, rsa_pkcs1_verify, rsa_pss_sign, rsa_pss_verify, rsa_public_key_from_private,
};
pub use x25519::{
    X25519ImportedKey, X25519KeyPair, derive_x25519_bits, export_x25519_jwk_private_key,
    export_x25519_jwk_public_key, export_x25519_pkcs8_private_key, export_x25519_spki_public_key,
    generate_x25519_key_pair, import_x25519_jwk_key, import_x25519_pkcs8_private_key,
    import_x25519_raw_public_key, import_x25519_spki_public_key, x25519_public_key_from_private,
};

#[cfg(test)]
mod tests;
