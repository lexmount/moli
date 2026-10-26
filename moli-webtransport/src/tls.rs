use std::{fs, io::Cursor, sync::Arc};

use anyhow::{Context, Result, ensure};
use moli_curl::CurlTlsConfig;
use rustls::pki_types::CertificateDer;

pub(crate) fn configure(config: &CurlTlsConfig) -> Result<rustls::ClientConfig> {
    ensure!(
        config.verify,
        "WebTransport requires certificate verification"
    );
    ensure!(
        config.client_cert_password.is_none(),
        "WebTransport requires an unencrypted PEM client identity"
    );
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
    let builder = rustls::ClientConfig::builder_with_provider(Arc::new(
        rustls::crypto::aws_lc_rs::default_provider(),
    ))
    .with_protocol_versions(&[&rustls::version::TLS13])?
    .with_root_certificates(roots);
    let mut tls = if let Some(path) = &config.client_cert {
        let bytes = fs::read(path).context("failed to read WebTransport client certificate")?;
        let chain =
            rustls_pemfile::certs(&mut Cursor::new(bytes)).collect::<std::io::Result<Vec<_>>>()?;
        let key_path = config.client_key.as_ref().unwrap_or(path);
        let key = fs::read(key_path).context("failed to read WebTransport client key")?;
        let key = rustls_pemfile::private_key(&mut Cursor::new(key))?
            .context("WebTransport client identity has no PEM private key")?;
        builder.with_client_auth_cert(chain, key)?
    } else {
        ensure!(
            config.client_key.is_none(),
            "WebTransport client key requires a certificate"
        );
        builder.with_no_client_auth()
    };
    tls.alpn_protocols = vec![web_transport_quinn::ALPN.as_bytes().to_vec()];
    Ok(tls)
}
