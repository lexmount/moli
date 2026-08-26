//! Backend-neutral cryptographic building blocks shared by Moli crates.
//!
//! This crate deliberately contains no WebIDL, browser-policy, algorithm-name
//! normalization, key, or JWK semantics. Those belong to the browser-facing
//! owner (`moli-webcrypto` or the renderer subsystem using the primitive).

mod digest;
mod ed25519;
mod hkdf;
mod random;

pub use digest::{DigestAlgorithm, Sha256Context, sha1_digest, sha256_digest, sha256_hex};
pub use ed25519::{Ed25519Error, Ed25519SigningKey};
pub use hkdf::{HkdfError, derive_hkdf_bytes};
pub use random::fill_secure_random;
