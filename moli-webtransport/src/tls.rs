use std::{fs, io::Cursor, sync::Arc};

use anyhow::{Context, Result, ensure};
use moli_curl::CurlTlsConfig;
use rustls::pki_types::CertificateDer;

mod fingerprints;

#[cfg(test)]
pub(crate) mod tests;

pub(crate) fn configure(config: &CurlTlsConfig) -> Result<rustls::ClientConfig> {
    configure_inner(config, Vec::new())
}

pub(crate) fn configure_with_hashes(
    config: &CurlTlsConfig,
    hashes: Vec<crate::CertificateHash>,
) -> Result<rustls::ClientConfig> {
    ensure!(
        !hashes.is_empty(),
        "certificate hash policy must be nonempty"
    );
    configure_inner(config, hashes)
}

fn configure_inner(
    config: &CurlTlsConfig,
    hashes: Vec<crate::CertificateHash>,
) -> Result<rustls::ClientConfig> {
    ensure!(
        config.verify,
        "WebTransport requires certificate verification"
    );
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let builder = rustls::ClientConfig::builder_with_provider(provider.clone())
        .with_protocol_versions(&[&rustls::version::TLS13])?;
    let builder = if hashes.is_empty() {
        builder.with_root_certificates(load_roots(config)?)
    } else {
        builder
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(fingerprints::Verifier::new(
                hashes, provider,
            )?))
    };
    // WebTransport's request credentials mode is omit, including TLS client
    // identities configured for ordinary browser HTTP requests.
    let mut tls = builder.with_no_client_auth();
    tls.alpn_protocols = vec![web_transport_quinn::ALPN.as_bytes().to_vec()];
    Ok(tls)
}

fn load_roots(config: &CurlTlsConfig) -> Result<rustls::RootCertStore> {
    let mut roots = rustls::RootCertStore::empty();
    if let Some(path) = &config.ca_cert {
        let bytes = fs::read(path).context("failed to read WebTransport CA file")?;
        let mut certs =
            rustls_pemfile::certs(&mut Cursor::new(&bytes)).collect::<std::io::Result<Vec<_>>>()?;
        if certs.is_empty() {
            certs.push(CertificateDer::from(bytes));
        }
        for certificate in certs {
            roots
                .add(certificate)
                .context("invalid WebTransport CA certificate")?;
        }
    } else {
        let native = rustls_native_certs::load_native_certs();
        roots.add_parsable_certificates(native.certs);
    }
    ensure!(
        !roots.is_empty(),
        "WebTransport has no trusted root certificates"
    );
    Ok(roots)
}
