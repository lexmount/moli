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
        #[cfg(target_os = "macos")]
        if self.ca_cert.is_some() {
            // An explicit CA file overrides SSL_CERT_DIR as well as
            // SSL_CERT_FILE, which curl-rust applies to a fresh handle.
            clear_ca_directory(easy, curl_sys::CURLOPT_CAPATH)?;
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

    /// Install certificate-chain and hostname verification for an HTTPS proxy.
    ///
    /// libcurl keeps proxy TLS settings separate from origin TLS settings. The
    /// configured CA is shared, but client identities remain origin-only unless
    /// a dedicated proxy identity is added in the future.
    pub fn configure_https_proxy<H: Handler>(&self, easy: &mut Easy2<H>) -> Result<()> {
        easy.proxy_ssl_verify_peer(self.verify)
            .context("failed to configure curl HTTPS proxy peer verification")?;
        easy.proxy_ssl_verify_host(self.verify)
            .context("failed to configure curl HTTPS proxy host verification")?;
        if let Some(ca_cert) = &self.ca_cert {
            let ca_cert_text = ca_cert.to_str().with_context(|| {
                format!(
                    "curl HTTPS proxy CA certificate path is not valid UTF-8: `{}`",
                    ca_cert.display()
                )
            })?;
            easy.proxy_cainfo(ca_cert_text).with_context(|| {
                format!(
                    "failed to configure curl HTTPS proxy CA certificate `{}`",
                    ca_cert.display()
                )
            })?;
        }
        #[cfg(target_os = "macos")]
        if self.ca_cert.is_some() {
            clear_ca_directory(easy, curl_sys::CURLOPT_PROXY_CAPATH)?;
        } else if self.verify {
            // curl-rust applies explicit certificate environment variables to
            // the origin. Moli shares these sources with HTTPS proxies too.
            // Leave every CA option untouched when no override was supplied:
            // libcurl then selects its built-in Apple SecTrust verifier.
            if let Some(file) = std::env::var_os("SSL_CERT_FILE") {
                easy.proxy_cainfo(file.to_str().context("SSL_CERT_FILE is not valid UTF-8")?)
                    .context("failed to configure HTTPS proxy SSL_CERT_FILE")?;
            }
            if let Some(directory) = std::env::var_os("SSL_CERT_DIR") {
                easy.proxy_capath(std::path::Path::new(&directory))
                    .context("failed to configure HTTPS proxy SSL_CERT_DIR")?;
            }
        }
        Ok(())
    }
}

#[cfg(target_os = "macos")]
fn clear_ca_directory<H: Handler>(easy: &mut Easy2<H>, option: curl_sys::CURLoption) -> Result<()> {
    // SAFETY: the caller supplies a CA directory string option, `easy` is
    // exclusively borrowed, and libcurl accepts NULL to clear this setting.
    let result = unsafe {
        curl_sys::curl_easy_setopt(easy.raw(), option, std::ptr::null::<std::ffi::c_char>())
    };
    if result != curl_sys::CURLE_OK {
        return Err(curl::Error::new(result)).context("failed to clear curl CA directory override");
    }
    Ok(())
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
