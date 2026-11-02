#[cfg(any(feature = "aws-lc-rs", feature = "ring"))]
use std::sync::Arc;
use std::task::Poll;

use futures::{
    future::{BoxFuture, Either},
    stream::FuturesUnordered,
    StreamExt,
};
#[cfg(any(feature = "aws-lc-rs", feature = "ring"))]
use rustls::pki_types::{CertificateDer, PrivateKeyDer};

#[cfg(any(feature = "aws-lc-rs", feature = "ring"))]
use crate::client::{controller_factory, transport_config, ControllerFactory};
#[cfg(any(feature = "aws-lc-rs", feature = "ring"))]
use crate::{crypto, CongestionControl};
use crate::{
    proto::{ConnectRequest, ConnectResponse},
    Connecting, ServerError, Session, Settings,
};

#[cfg(any(feature = "aws-lc-rs", feature = "ring"))]
/// Construct a WebTransport [Server] using sane defaults.
///
/// This is optional; advanced users may use [Server::new] directly.
pub struct ServerBuilder {
    provider: crypto::Provider,
    addr: std::net::SocketAddr,
    congestion_controller: Option<ControllerFactory>,
}

#[cfg(any(feature = "aws-lc-rs", feature = "ring"))]
impl Default for ServerBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(any(feature = "aws-lc-rs", feature = "ring"))]
impl ServerBuilder {
    /// Create a server builder with sane defaults.
    pub fn new() -> Self {
        Self {
            provider: crypto::default_provider(),
            addr: "[::]:443".parse().unwrap(),
            congestion_controller: None,
        }
    }

    /// Listen on the specified address.
    pub fn with_addr(self, addr: std::net::SocketAddr) -> Self {
        Self { addr, ..self }
    }

    /// Enable the specified congestion controller.
    pub fn with_congestion_control(mut self, algorithm: CongestionControl) -> Self {
        self.congestion_controller = controller_factory(algorithm);
        self
    }

    /// Supply a certificate used for TLS.
    // TODO support multiple certs based on...?
    pub fn with_certificate(
        self,
        chain: Vec<CertificateDer<'static>>,
        key: PrivateKeyDer<'static>,
    ) -> Result<Server, ServerError> {
        let transport = transport_config(self.congestion_controller.as_ref());
        let config = self.config(chain, key, transport)?;

        let server = quinn::Endpoint::server(config, self.addr)
            .map_err(|e| ServerError::IoError(e.into()))?;

        Ok(Server::new(server))
    }

    /// Build the quinn config, taking the transport separately so the caller (and the
    /// tests) can tell which one ends up attached.
    fn config(
        &self,
        chain: Vec<CertificateDer<'static>>,
        key: PrivateKeyDer<'static>,
        transport: Arc<quinn::TransportConfig>,
    ) -> Result<quinn::ServerConfig, ServerError> {
        // Standard Quinn setup
        let mut config = rustls::ServerConfig::builder_with_provider(self.provider.clone())
            .with_protocol_versions(&[&rustls::version::TLS13])?
            .with_no_client_auth()
            .with_single_cert(chain, key)?;

        config.alpn_protocols = vec![crate::ALPN.as_bytes().to_vec()]; // this one is important

        let config: quinn::crypto::rustls::QuicServerConfig = config.try_into().unwrap();
        let mut config = quinn::ServerConfig::with_crypto(Arc::new(config));
        config.transport_config(transport);

        Ok(config)
    }
}

/// A WebTransport server that accepts new sessions.
pub struct Server {
    endpoint: quinn::Endpoint,
    accept: FuturesUnordered<BoxFuture<'static, Result<Request, ServerError>>>,
}

impl core::ops::Deref for Server {
    type Target = quinn::Endpoint;

    fn deref(&self) -> &Self::Target {
        &self.endpoint
    }
}

impl Server {
    /// Manually create a new server with a manually constructed Endpoint.
    ///
    /// NOTE: The ALPN must be set to `crate::ALPN` for WebTransport to work.
    pub fn new(endpoint: quinn::Endpoint) -> Self {
        Self {
            endpoint,
            accept: Default::default(),
        }
    }

    /// Accept a new WebTransport session Request from a client.
    pub async fn accept(&mut self) -> Option<Request> {
        loop {
            // Borrow both halves inside the block so the winner can be handled afterwards.
            let winner = {
                let incoming = self.endpoint.accept();
                let pending = &mut self.accept;

                // An empty set of pending handshakes must not end the accept loop.
                let handshake = futures::future::poll_fn(|cx| match pending.poll_next_unpin(cx) {
                    Poll::Ready(Some(res)) => Poll::Ready(res),
                    _ => Poll::Pending,
                });

                futures::pin_mut!(incoming, handshake);

                // Handshakes go first: there are only as many as the arrivals already taken,
                // whereas a steady stream of arrivals would never let them be polled.
                match futures::future::select(handshake, incoming).await {
                    Either::Left((res, _)) => Either::Right(res),
                    Either::Right((incoming, _)) => Either::Left(incoming),
                }
            };

            match winner {
                Either::Left(incoming) => {
                    let conn = incoming?;
                    self.accept.push(Box::pin(async move {
                        let conn = conn.await?;
                        Request::accept(conn).await
                    }));
                }
                Either::Right(res) => {
                    if let Ok(session) = res {
                        return Some(session);
                    }
                }
            }
        }
    }
}

/// A mostly complete WebTransport handshake, just awaiting the server's decision on whether to accept or reject the session based on the URL.
pub struct Request {
    conn: quinn::Connection,
    settings: Settings,
    connect: Connecting,
}

impl Request {
    /// Accept a new WebTransport session from a client.
    pub async fn accept(conn: quinn::Connection) -> Result<Self, ServerError> {
        // Perform the H3 handshake by sending/reciving SETTINGS frames.
        let settings = Settings::connect(&conn).await?;

        // Accept the CONNECT request but don't send a response yet.
        let connect = Connecting::accept(&conn).await?;

        // Return the resulting request with a reference to the settings/connect streams.
        Ok(Self {
            conn,
            settings,
            connect,
        })
    }

    pub async fn ok(self) -> Result<Session, ServerError> {
        self.respond(ConnectResponse::OK).await
    }

    /// Reply to the session with the given response, usually 200 OK.
    ///
    /// [ConnectResponse::with_protocol] can be used to select a subprotocol.
    pub async fn respond(
        self,
        response: impl Into<ConnectResponse>,
    ) -> Result<Session, ServerError> {
        let response = response.into();
        let connect = self.connect.respond(response).await?;
        Ok(Session::new(self.conn, self.settings, connect))
    }

    /// Reject the session with the given status code.
    pub async fn reject(self, status: http::StatusCode) -> Result<(), ServerError> {
        self.connect.reject(status).await?;
        Ok(())
    }

    /// Returns the underlying QUIC connection.
    pub fn conn(&self) -> &quinn::Connection {
        &self.conn
    }

    /// The remote peer's address.
    #[deprecated(note = "use conn().remote_address() instead")]
    pub fn remote_address(&self) -> std::net::SocketAddr {
        self.conn.remote_address()
    }

    /// Returns the CONNECT request that was sent by the client.
    ///
    /// DEPRECATED: You can access this via the Deref impl.
    pub fn connect(&self) -> &ConnectRequest {
        &self.connect
    }
}

impl core::ops::Deref for Request {
    type Target = ConnectRequest;

    fn deref(&self) -> &Self::Target {
        &self.connect
    }
}

#[cfg(all(test, any(feature = "aws-lc-rs", feature = "ring")))]
mod tests {
    use super::*;
    use rustls::pki_types::PrivatePkcs8KeyDer;

    fn self_signed() -> (Vec<CertificateDer<'static>>, PrivateKeyDer<'static>) {
        let key = rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
        let chain = vec![CertificateDer::from(key.cert.der().to_vec())];
        let der = rcgen::KeyPair::serialize_der(&key.signing_key);

        (chain, PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(der)))
    }

    /// `ServerBuilder::new` goes through [crypto::default_provider], which panics when
    /// both backends are compiled in and no process-wide default is installed. Pick one
    /// here rather than mutating global state from a test.
    fn builder() -> ServerBuilder {
        #[cfg(feature = "aws-lc-rs")]
        let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
        #[cfg(all(feature = "ring", not(feature = "aws-lc-rs")))]
        let provider = Arc::new(rustls::crypto::ring::default_provider());

        ServerBuilder {
            provider,
            addr: "[::]:0".parse().unwrap(),
            congestion_controller: None,
        }
    }

    /// quinn installs its own default transport config, so the builder's has to be
    /// attached explicitly or every server knob (congestion control) silently no-ops.
    #[test]
    fn builder_transport_reaches_the_config() {
        let (chain, key) = self_signed();

        let builder = builder().with_congestion_control(CongestionControl::LowLatency);
        assert!(builder.congestion_controller.is_some());

        let transport = transport_config(builder.congestion_controller.as_ref());
        let config = builder.config(chain, key, transport.clone()).unwrap();

        assert!(Arc::ptr_eq(&config.transport, &transport));
    }
}
