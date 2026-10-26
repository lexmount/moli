//! Simple crypto provider utilities for rustls.
//!
//! This module provides helper functions for working with rustls crypto providers,
//! supporting both ring and aws-lc-rs backends.

use std::sync::Arc;

use rustls::crypto::hash::{self, HashAlgorithm};
use rustls::crypto::CryptoProvider;
use rustls::pki_types::CertificateDer;

/// A shared reference to a crypto provider.
pub type Provider = Arc<CryptoProvider>;

/// Returns the default crypto provider.
///
/// This function checks for a process-wide default provider first,
/// then falls back to feature-enabled providers (aws-lc-rs or ring).
///
/// # Panics
///
/// Panics if no provider is available. Either call `CryptoProvider::set_default()`
/// or enable exactly one of the `ring` or `aws-lc-rs` features.
pub fn default_provider() -> Provider {
    // See <https://docs.rs/rustls/latest/rustls/crypto/struct.CryptoProvider.html#using-the-per-process-default-cryptoprovider>
    if let Some(provider) = CryptoProvider::get_default().cloned() {
        return provider;
    }

    #[cfg(all(feature = "aws-lc-rs", not(feature = "ring")))]
    {
        return Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    }
    #[cfg(all(feature = "ring", not(feature = "aws-lc-rs")))]
    {
        return Arc::new(rustls::crypto::ring::default_provider());
    }
    #[allow(unreachable_code)]
    {
        panic!(
        "CryptoProvider::set_default() must be called; or only enable one ring/aws-lc-rs feature."
    );
    }
}

/// Computes the SHA-256 hash of a certificate using the provided crypto provider.
///
/// # Panics
///
/// Panics if the provider doesn't expose a SHA-256 hash algorithm.
pub fn sha256(provider: &Provider, cert: &CertificateDer<'_>) -> hash::Output {
    let hash_provider = provider.cipher_suites.iter().find_map(|suite| {
        let hash_provider = suite.tls13()?.common.hash_provider;
        if hash_provider.algorithm() == HashAlgorithm::SHA256 {
            Some(hash_provider)
        } else {
            None
        }
    });
    if let Some(hash_provider) = hash_provider {
        return hash_provider.hash(cert);
    }

    panic!("No SHA-256 backend available. Ensure your provider exposes SHA-256 or enable the 'ring'/'aws-lc-rs' feature.");
}
