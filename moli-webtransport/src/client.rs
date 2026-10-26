use std::{
    net::{IpAddr, SocketAddr},
    sync::Arc,
};

use anyhow::{Context, Result, anyhow, bail, ensure};
use futures_util::{StreamExt, stream};
use moli_curl::{HostResolveOverrides, select_proxy_route};
use moli_dns_resolver::{DnsResolverService, DnsTarget};
use tokio::sync::{OnceCell, oneshot};
use tokio_util::sync::CancellationToken;
use url::Host;

use crate::session::EndpointOwner;
use crate::{Config, CongestionControl, ConnectOptions, Session};

const MAX_PENDING_HANDSHAKES: usize = 4;

/// Unique browser-network owner. Creation starts no threads or sockets.
pub struct Owner {
    connector: Connector,
}

impl Owner {
    pub fn new(config: Config) -> Self {
        Self {
            connector: Connector {
                inner: Arc::new(Inner {
                    config,
                    tls: OnceCell::new(),
                    cancelled: CancellationToken::new(),
                }),
            },
        }
    }

    pub fn connector(&self) -> Connector {
        self.connector.clone()
    }
}

impl Drop for Owner {
    fn drop(&mut self) {
        self.connector.request_shutdown();
    }
}

struct Inner {
    config: Config,
    tls: OnceCell<rustls::ClientConfig>,
    cancelled: CancellationToken,
}

/// Cloneable request capability; retaining it does not keep the owner live.
#[derive(Clone)]
pub struct Connector {
    inner: Arc<Inner>,
}

impl std::fmt::Debug for Connector {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WebTransportConnector")
            .field("closed", &self.is_closed())
            .finish_non_exhaustive()
    }
}

impl Connector {
    /// Runs on the execution context's captured resource executor. The binding
    /// must not poll this future synchronously inside a V8 callback.
    pub async fn connect(&self, options: ConnectOptions) -> Result<Session> {
        tokio::select! {
            biased;
            () = self.inner.cancelled.cancelled() => bail!("WebTransport network owner retired"),
            result = async {
                if let Some(timeout) = self.inner.config.connect_timeout {
                    tokio::time::timeout(timeout, self.connect_inner(options)).await
                        .context("WebTransport connection timed out")?
                } else {
                    self.connect_inner(options).await
                }
            } => result,
        }
    }

    pub fn request_shutdown(&self) {
        self.inner.cancelled.cancel();
    }

    pub fn is_closed(&self) -> bool {
        self.inner.cancelled.is_cancelled()
    }

    async fn connect_inner(&self, options: ConnectOptions) -> Result<Session> {
        let url = &options.url;
        ensure!(
            url.scheme() == "https"
                && url.fragment().is_none()
                && url.username().is_empty()
                && url.password().is_none(),
            "invalid WebTransport URL"
        );
        let host = url.host().context("WebTransport URL has no host")?;
        let port = url
            .port_or_known_default()
            .context("WebTransport URL has no port")?;
        ensure!(port != 0, "WebTransport cannot connect to port 0");
        let config = &self.inner.config;
        ensure!(
            !select_proxy_route(url, config.proxy.as_deref(), config.no_proxy.as_deref())?
                .is_proxy(),
            "WebTransport HTTP/3 cannot use the selected HTTP/SOCKS proxy"
        );
        let server_name = match &host {
            Host::Domain(name) => (*name).to_owned(),
            Host::Ipv4(ip) => ip.to_string(),
            Host::Ipv6(ip) => ip.to_string(),
        };
        let overrides = HostResolveOverrides::parse(&config.host_resolve)?;
        let addresses: Arc<[IpAddr]> =
            if let Some(addresses) = overrides.addresses_for(&server_name, port) {
                addresses.to_vec().into()
            } else {
                match host {
                    Host::Ipv4(ip) => Arc::from([IpAddr::V4(ip)]),
                    Host::Ipv6(ip) => Arc::from([IpAddr::V6(ip)]),
                    Host::Domain(_) => {
                        let (tx, rx) = oneshot::channel();
                        DnsResolverService::shared()
                            .map_err(|e| anyhow!(e))?
                            .resolve(
                                config.dns_partition,
                                DnsTarget::new(server_name.clone(), port),
                                move |result| {
                                    let _ = tx.send(result);
                                },
                            );
                        rx.await
                            .context("WebTransport DNS completion dropped")?
                            .map_err(|e| anyhow!(e))?
                    }
                }
            };
        config
            .address_policy
            .check_addresses(&addresses, url.as_str())?;
        ensure!(
            !addresses.is_empty(),
            "WebTransport DNS returned no addresses"
        );
        let tls = if options.server_certificate_hashes.is_empty() {
            self.inner
                .tls
                .get_or_try_init(|| async {
                    let config = config.tls.clone();
                    tokio::task::spawn_blocking(move || crate::tls::configure(&config))
                        .await
                        .context("WebTransport TLS configuration task failed")?
                })
                .await?
                .clone()
        } else {
            // A session's explicit pins must never populate the browser's
            // cached Web PKI verifier or fall back to its configured CA file.
            let config = config.tls.clone();
            let hashes = options.server_certificate_hashes;
            tokio::task::spawn_blocking(move || crate::tls::configure_with_hashes(&config, hashes))
                .await
                .context("WebTransport certificate hash configuration task failed")??
        };
        let crypto = quinn::crypto::rustls::QuicClientConfig::try_from(tls)?;
        let mut client_config = quinn::ClientConfig::new(Arc::new(crypto));
        let mut transport = quinn::TransportConfig::default();
        match options.congestion_control {
            CongestionControl::Default => {}
            CongestionControl::Throughput => {
                transport.congestion_controller_factory(Arc::new(
                    quinn::congestion::CubicConfig::default(),
                ));
            }
            CongestionControl::LowLatency => {
                transport.congestion_controller_factory(Arc::new(
                    quinn::congestion::BbrConfig::default(),
                ));
            }
        }
        client_config.transport_config(Arc::new(transport));
        let mut request = web_transport_quinn::proto::ConnectRequest::new(options.url)
            .with_protocols(options.protocols)
            .with_headers(options.headers);
        request
            .headers
            .insert(http::header::ORIGIN, options.origin.parse()?);
        // Race a bounded set of admitted transport addresses. An unresponsive
        // IPv6/IPv4 candidate must not consume the entire connection deadline.
        // Only the selected TLS connection sends an HTTP application request.
        let mut attempts = stream::iter(addresses.iter().copied())
            .map(|ip| {
                let config = client_config.clone();
                let server_name = server_name.clone();
                async move {
                    let bind = if ip.is_ipv4() { "0.0.0.0:0" } else { "[::]:0" };
                    let endpoint = EndpointOwner(quinn::Endpoint::client(bind.parse()?)?);
                    let remote = SocketAddr::new(ip, port);
                    let connection = endpoint
                        .0
                        .connect_with(config, remote, &server_name)?
                        .await?;
                    Ok::<_, anyhow::Error>((endpoint, connection))
                }
            })
            .buffer_unordered(MAX_PENDING_HANDSHAKES);
        let mut last_error = None;
        while let Some(result) = attempts.next().await {
            let (endpoint, connection) = match result {
                Ok(connection) => connection,
                Err(error) => {
                    // A peer answered with a terminal TLS failure. Do not
                    // hide a rejected certificate behind another address's
                    // idle timeout or retry it as a connectivity failure.
                    if let Some(quinn::ConnectionError::TransportError(transport)) =
                        error.downcast_ref::<quinn::ConnectionError>()
                        && (0x100..0x200).contains(&u64::from(transport.code))
                    {
                        return Err(error);
                    }
                    last_error = Some(error);
                    continue;
                }
            };
            drop(attempts);
            // HTTP-level rejection is terminal; it must not retry another
            // address and create a second application session.
            let native = web_transport_quinn::Session::connect(connection, request).await?;
            return Ok(Session::new(endpoint, native, self.inner.cancelled.clone()));
        }
        Err(last_error.unwrap_or_else(|| anyhow!("WebTransport connection had no usable address")))
    }
}
