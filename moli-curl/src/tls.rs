use std::{fmt, path::PathBuf};

use anyhow::{Context, Result};
use curl::easy::{Easy2, Handler};

/// Owned TLS settings shared by HTTP and WebSocket connections.
#[derive(Clone, PartialEq, Eq)]
pub struct CurlTlsConfig {
    /// Verify both the server certificate chain and hostname.
    pub verify: bool,
    pub ca_cert: Option<PathBuf>,
    pub client_cert: Option<PathBuf>,
    pub client_key: Option<PathBuf>,
    pub client_cert_password: Option<String>,
}

impl Default for CurlTlsConfig {
    fn default() -> Self {
        Self {
            verify: true,
            ca_cert: None,
            client_cert: None,
            client_key: None,
            client_cert_password: None,
        }
    }
}

impl fmt::Debug for CurlTlsConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CurlTlsConfig")
            .field("verify", &self.verify)
            .field("ca_cert", &self.ca_cert)
            .field("client_cert", &self.client_cert)
            .field("client_key", &self.client_key)
            .field(
                "client_cert_password",
                &self.client_cert_password.as_ref().map(|_| "[REDACTED]"),
            )
            .finish()
    }
}

impl CurlTlsConfig {
    /// Install TLS settings on a fresh handle, before its first transfer.
    ///
    /// The caller decides whether credentials are allowed for the current URL.
    /// CA trust applies even when the client identity is omitted. Using a fresh
    /// handle lets curl isolate reusable connections by their TLS settings.
    pub fn configure<H: Handler>(
        &self,
        easy: &mut Easy2<H>,
        include_client_identity: bool,
    ) -> Result<()> {
        easy.ssl_verify_peer(self.verify)
            .context("failed to configure curl TLS peer verification")?;
        easy.ssl_verify_host(self.verify)
            .context("failed to configure curl TLS host verification")?;
        if let Some(ca_cert) = &self.ca_cert {
            easy.cainfo(ca_cert).with_context(|| {
                format!(
                    "failed to configure curl CA certificate `{}`",
                    ca_cert.display()
                )
            })?;
        }
        if include_client_identity {
            if let Some(client_cert) = &self.client_cert {
                easy.ssl_cert(client_cert).with_context(|| {
                    format!(
                        "failed to configure curl client certificate `{}`",
                        client_cert.display()
                    )
                })?;
                let cert_type = match client_cert
                    .extension()
                    .and_then(|extension| extension.to_str())
                    .map(str::to_ascii_lowercase)
                    .as_deref()
                {
                    Some("p12" | "pfx") => "P12",
                    Some("cer" | "der") => "DER",
                    _ => "PEM",
                };
                easy.ssl_cert_type(cert_type)
                    .context("failed to configure curl client certificate type")?;
            }
            if let Some(client_key) = &self.client_key {
                easy.ssl_key(client_key).with_context(|| {
                    format!(
                        "failed to configure curl client private key `{}`",
                        client_key.display()
                    )
                })?;
                easy.ssl_key_type("PEM")
                    .context("failed to configure curl client private key type")?;
            }
            if let Some(password) = &self.client_cert_password {
                easy.key_password(password)
                    .context("failed to configure curl client certificate password")?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_omits_client_certificate_password() {
        let config = CurlTlsConfig {
            client_cert_password: Some("private-certificate-password".to_owned()),
            ..CurlTlsConfig::default()
        };
        let debug = format!("{config:?}");
        assert!(!debug.contains("private-certificate-password"));
        assert!(debug.contains("[REDACTED]"));
    }
}
