use std::{fs, path::PathBuf, sync::Arc};

use anyhow::{Result, bail};
use moli_cookie_jar::new_shared_browser_cookie_store;
use parking_lot::Mutex;
use rcgen::{
    BasicConstraints, CertificateParams, DnType, ExtendedKeyUsagePurpose, IsCa, KeyPair,
    KeyUsagePurpose,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::oneshot,
    task::{JoinHandle, JoinSet},
    time::{Duration, timeout},
};
use tokio_rustls::{
    TlsAcceptor,
    rustls::{
        RootCertStore, ServerConfig,
        pki_types::{CertificateDer, PrivatePkcs8KeyDer},
        server::WebPkiClientVerifier,
    },
};
use url::Url;

use crate::{
    FetchCancelHandle, FetchClient, FetchClientHandle, FetchConfig, RedirectInfo, RedirectSource,
    Request, RequestCredentialsMode, RequestMode,
};

use super::support::unique_test_cache_dir;

struct TlsCredentials {
    dir: PathBuf,
    acceptor: TlsAcceptor,
    client_certificate: CertificateDer<'static>,
}

impl TlsCredentials {
    fn new() -> Result<Self> {
        let ca_key = KeyPair::generate()?;
        let mut ca_params = CertificateParams::new(Vec::new())?;
        ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        ca_params.key_usages = vec![KeyUsagePurpose::KeyCertSign];
        ca_params
            .distinguished_name
            .push(DnType::CommonName, "Moli fetch test CA");
        let ca = ca_params.self_signed(&ca_key)?;

        let server_key = KeyPair::generate()?;
        let mut server_params =
            CertificateParams::new(vec!["localhost".to_owned(), "127.0.0.1".to_owned()])?;
        server_params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
        let server_cert = server_params.signed_by(&server_key, &ca, &ca_key)?;

        let client_key = KeyPair::generate()?;
        let mut client_params = CertificateParams::new(Vec::new())?;
        client_params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ClientAuth];
        let client_cert = client_params.signed_by(&client_key, &ca, &ca_key)?;

        let mut roots = RootCertStore::empty();
        roots.add(ca.der().clone())?;
        // Request a client certificate but allow anonymous handshakes so the
        // tests can inspect what the server actually received in either mode.
        let verifier = WebPkiClientVerifier::builder(Arc::new(roots))
            .allow_unauthenticated()
            .build()?;
        let mut config = ServerConfig::builder()
            .with_client_cert_verifier(verifier)
            .with_single_cert(
                vec![server_cert.der().clone()],
                PrivatePkcs8KeyDer::from(server_key.serialize_der()).into(),
            )?;
        config.alpn_protocols = vec![b"http/1.1".to_vec()];

        let credentials = Self {
            dir: unique_test_cache_dir(),
            acceptor: TlsAcceptor::from(Arc::new(config)),
            client_certificate: client_cert.der().clone(),
        };
        fs::create_dir(&credentials.dir)?;
        fs::write(credentials.dir.join("ca.pem"), ca.pem())?;
        fs::write(credentials.dir.join("client.pem"), client_cert.pem())?;
        fs::write(
            credentials.dir.join("client-key.pem"),
            client_key.serialize_pem(),
        )?;
        Ok(credentials)
    }

    fn fetch_config(&self) -> FetchConfig {
        let mut config = FetchConfig::default();
        config.set_http_proxy(Some(String::new()));
        config.set_request_timeout_ms(5_000);
        // Keep verification enabled: even anonymous requests need this CA.
        config.set_tls_credentials(
            Some(self.dir.join("ca.pem")),
            Some(self.dir.join("client.pem")),
            Some(self.dir.join("client-key.pem")),
            Some("test-password".to_owned()),
        );
        config
    }
}

impl Drop for TlsCredentials {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

#[derive(Clone, Debug)]
struct ObservedRequest {
    connection_id: usize,
    path: String,
    client_certificates: Vec<CertificateDer<'static>>,
}

struct TlsServer {
    url: Url,
    requests: Arc<Mutex<Vec<ObservedRequest>>>,
    task: JoinHandle<()>,
}

#[derive(Debug)]
struct ObservedHttpsProxyRequest {
    request_head: String,
    server_name: Option<String>,
    client_certificates: Vec<CertificateDer<'static>>,
}

struct HttpsProxy {
    url: String,
    task: Option<JoinHandle<Result<ObservedHttpsProxyRequest>>>,
}

impl HttpsProxy {
    async fn spawn(credentials: &TlsCredentials) -> Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let port = listener.local_addr()?.port();
        let acceptor = credentials.acceptor.clone();
        let task = tokio::spawn(async move {
            let (stream, _) = listener.accept().await?;
            let mut stream = acceptor.accept(stream).await?;
            let server_name = stream.get_ref().1.server_name().map(str::to_owned);
            let client_certificates = stream
                .get_ref()
                .1
                .peer_certificates()
                .unwrap_or_default()
                .to_vec();
            let mut request_head = Vec::new();
            while !request_head.ends_with(b"\r\n\r\n") {
                let mut byte = [0];
                if stream.read(&mut byte).await? == 0 {
                    bail!("HTTPS proxy client closed before completing its request");
                }
                request_head.push(byte[0]);
                if request_head.len() > 64 * 1024 {
                    bail!("HTTPS proxy request headers are too large");
                }
            }
            let body = b"through-https-proxy";
            stream
                .write_all(
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    )
                    .as_bytes(),
                )
                .await?;
            stream.write_all(body).await?;
            stream.flush().await?;
            Ok(ObservedHttpsProxyRequest {
                request_head: String::from_utf8(request_head)?,
                server_name,
                client_certificates,
            })
        });
        Ok(Self {
            url: format!("https://localhost:{port}"),
            task: Some(task),
        })
    }

    async fn finish(mut self) -> Result<ObservedHttpsProxyRequest> {
        let task = self
            .task
            .take()
            .expect("HTTPS proxy task should be present");
        timeout(Duration::from_secs(5), task).await??
    }
}

impl Drop for HttpsProxy {
    fn drop(&mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

impl TlsServer {
    async fn spawn(credentials: &TlsCredentials) -> Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let port = listener.local_addr()?.port();
        let url = Url::parse(&format!("https://127.0.0.1:{port}/whoami"))?;
        let requests = Arc::new(Mutex::new(Vec::new()));
        let observed = Arc::clone(&requests);
        let acceptor = credentials.acceptor.clone();
        let task = tokio::spawn(async move {
            let mut connections = JoinSet::new();
            let mut connection_id = 0;
            while let Ok((stream, _)) = listener.accept().await {
                let acceptor = acceptor.clone();
                let observed = Arc::clone(&observed);
                connections.spawn(async move {
                    let mut stream = acceptor.accept(stream).await?;
                    let client_certificates = stream
                        .get_ref()
                        .1
                        .peer_certificates()
                        .unwrap_or_default()
                        .to_vec();
                    loop {
                        let mut head = Vec::new();
                        while !head.ends_with(b"\r\n\r\n") {
                            let mut byte = [0];
                            if stream.read(&mut byte).await? == 0 {
                                return Ok::<(), anyhow::Error>(());
                            }
                            head.push(byte[0]);
                        }
                        let head = String::from_utf8(head)?;
                        let path = head.split_whitespace().nth(1).unwrap().to_owned();
                        let cors_headers = head.lines().filter_map(|line| line.split_once(':'))
                            .find(|(name, _)| name.eq_ignore_ascii_case("Origin"))
                            .map(|(_, origin)| format!("Access-Control-Allow-Origin: {}\r\nAccess-Control-Allow-Credentials: true\r\n", origin.trim()))
                            .unwrap_or_default();
                        observed.lock().push(ObservedRequest {
                            connection_id,
                            path: path.clone(),
                            client_certificates: client_certificates.clone(),
                        });
                        let response = match path.as_str() {
                            "/redirect-cross" => format!(
                                "HTTP/1.1 302 Found\r\n{cors_headers}Location: https://localhost:{port}/redirect-back\r\nContent-Length: 0\r\n\r\n"
                            ),
                            "/redirect-back" => format!(
                                "HTTP/1.1 302 Found\r\n{cors_headers}Location: https://127.0.0.1:{port}/whoami\r\nContent-Length: 0\r\n\r\n"
                            ),
                            _ => format!("HTTP/1.1 200 OK\r\n{cors_headers}Content-Type: text/plain\r\nContent-Length: 2\r\n\r\nok"),
                        };
                        // HTTP/1.1 stays open for further requests, including
                        // any incorrect reuse of an authenticated connection.
                        stream.write_all(response.as_bytes()).await?;
                        stream.flush().await?;
                    }
                });
                connection_id += 1;
            }
        });
        Ok(Self {
            url,
            requests,
            task,
        })
    }
}

impl Drop for TlsServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

#[derive(Clone, Copy, Debug)]
enum Transport {
    Buffered,
    Html,
    Raw,
}

impl Transport {
    const ALL: [Self; 3] = [Self::Buffered, Self::Html, Self::Raw];

    async fn fetch(self, client: &FetchClientHandle, request: Request) -> Result<()> {
        let body = match self {
            Self::Buffered => {
                let (tx, rx) = oneshot::channel();
                client.fetch_with_cancel_callback(
                    request,
                    FetchCancelHandle::new(),
                    move |response| {
                        let _ = tx.send(response);
                    },
                )?;
                rx.await??.body_text().to_owned()
            }
            Self::Html => {
                let mut response = client.fetch_html_stream(request).await?;
                let mut body = String::new();
                while let Some(chunk) = response.next_chunk().await {
                    body.push_str(&chunk);
                }
                response.finish().await?;
                body
            }
            Self::Raw => client.fetch(request).await?.body_text().to_owned(),
        };
        assert_eq!(body, "ok");
        Ok(())
    }
}

#[tokio::test]
async fn https_proxy_uses_shared_dns_and_proxy_hostname_tls() -> Result<()> {
    let credentials = TlsCredentials::new()?;
    let proxy = HttpsProxy::spawn(&credentials).await?;
    let mut config = credentials.fetch_config();
    config.set_http_proxy(Some(proxy.url.clone()));
    config.set_http_no_proxy(Some(String::new()));
    let client = FetchClient::new(&config, new_shared_browser_cookie_store());

    let response = client
        .fetch(Request::get(
            "http://fetch-target.invalid/through-https-proxy",
        )?)
        .await?;
    assert_eq!(response.body_text(), "through-https-proxy");
    assert!(client.shutdown().is_clean());

    let observed = proxy.finish().await?;
    assert_eq!(observed.server_name.as_deref(), Some("localhost"));
    assert!(
        observed
            .request_head
            .starts_with("GET http://fetch-target.invalid/through-https-proxy HTTP/1.1\r\n"),
        "unexpected HTTPS proxy request: {:?}",
        observed.request_head
    );
    assert!(
        observed.client_certificates.is_empty(),
        "origin client identity must not be sent to the HTTPS proxy"
    );
    Ok(())
}

#[tokio::test]
async fn tls_client_certificate_respects_credentials_in_every_transport() -> Result<()> {
    let credentials = TlsCredentials::new()?;
    let server = TlsServer::spawn(&credentials).await?;
    let cross_origin = Url::parse("https://other.example.test/")?;
    for transport in Transport::ALL {
        for (mode, initiator, authenticated) in [
            (RequestCredentialsMode::Include, &cross_origin, true),
            (RequestCredentialsMode::Omit, &server.url, false),
            (RequestCredentialsMode::SameOrigin, &server.url, true),
            (RequestCredentialsMode::SameOrigin, &cross_origin, false),
        ] {
            // Each case uses a fresh client to exercise a new TLS handshake.
            let client = FetchClient::new(
                &credentials.fetch_config(),
                new_shared_browser_cookie_store(),
            );
            let request = Request::get(server.url.as_str())?
                .with_initiator_url(initiator)
                .with_request_origin(moli_url::WebOrigin::from_url(initiator))
                .with_credentials_mode(mode);
            transport.fetch(&client, request).await?;
            let observed = server.requests.lock().pop().unwrap();
            let expected: Vec<_> = authenticated
                .then(|| credentials.client_certificate.clone())
                .into_iter()
                .collect();
            assert_eq!(
                observed.client_certificates, expected,
                "{transport:?} {mode:?}"
            );
            assert!(client.shutdown().is_clean());
        }
    }
    Ok(())
}

#[tokio::test]
async fn tls_client_certificate_isolated_on_reused_connections() -> Result<()> {
    let credentials = TlsCredentials::new()?;
    let server = TlsServer::spawn(&credentials).await?;
    let cross_origin = Url::parse("https://other.example.test/")?;
    for transport in Transport::ALL {
        let client = FetchClient::new(
            &credentials.fetch_config(),
            new_shared_browser_cookie_store(),
        );
        for (mode, initiator) in [
            (RequestCredentialsMode::Include, &server.url),
            (RequestCredentialsMode::Include, &server.url),
            (RequestCredentialsMode::Omit, &server.url),
            (RequestCredentialsMode::Omit, &server.url),
            (RequestCredentialsMode::SameOrigin, &cross_origin),
            (RequestCredentialsMode::SameOrigin, &server.url),
        ] {
            let request = Request::get(server.url.as_str())?
                .with_initiator_url(initiator)
                .with_request_origin(moli_url::WebOrigin::from_url(initiator))
                .with_credentials_mode(mode);
            transport.fetch(&client, request).await?;
        }
        let observed = std::mem::take(&mut *server.requests.lock());
        assert_eq!(observed.len(), 6);
        for (request, authenticated) in observed.iter().zip([true, true, false, false, false, true])
        {
            let expected: Vec<_> = authenticated
                .then(|| credentials.client_certificate.clone())
                .into_iter()
                .collect();
            assert_eq!(request.client_certificates, expected, "{transport:?}");
        }
        assert_eq!(observed[0].connection_id, observed[1].connection_id);
        assert_ne!(observed[1].connection_id, observed[2].connection_id);
        assert_eq!(observed[2].connection_id, observed[3].connection_id);
        assert_eq!(observed[3].connection_id, observed[4].connection_id);
        assert_eq!(observed[0].connection_id, observed[5].connection_id);
        assert!(client.shutdown().is_clean());
    }
    Ok(())
}

#[tokio::test]
async fn tls_client_certificate_preserves_cross_origin_redirect_taint() -> Result<()> {
    let credentials = TlsCredentials::new()?;
    let server = TlsServer::spawn(&credentials).await?;
    for transport in Transport::ALL {
        for (mode, expected_authentication) in [
            (RequestCredentialsMode::Include, [true, true, true]),
            (RequestCredentialsMode::Omit, [false, false, false]),
            (RequestCredentialsMode::SameOrigin, [true, false, false]),
        ] {
            let client = FetchClient::new(
                &credentials.fetch_config(),
                new_shared_browser_cookie_store(),
            );
            let request = Request::get(server.url.join("/redirect-cross")?.as_str())?
                .with_initiator_url(&server.url)
                .with_request_origin(moli_url::WebOrigin::from_url(&server.url))
                .with_request_mode(RequestMode::Cors)
                .with_credentials_mode(mode);
            transport.fetch(&client, request).await?;
            let observed = std::mem::take(&mut *server.requests.lock());
            assert_eq!(
                observed
                    .iter()
                    .map(|request| request.path.as_str())
                    .collect::<Vec<_>>(),
                ["/redirect-cross", "/redirect-back", "/whoami"]
            );
            for (request, authenticated) in observed.iter().zip(expected_authentication) {
                let expected: Vec<_> = authenticated
                    .then(|| credentials.client_certificate.clone())
                    .into_iter()
                    .collect();
                assert_eq!(
                    request.client_certificates, expected,
                    "{transport:?} {mode:?} {}",
                    request.path
                );
            }
            assert!(client.shutdown().is_clean());
        }
    }
    Ok(())
}

#[tokio::test]
async fn tls_client_certificate_preserves_service_worker_redirect_taint() -> Result<()> {
    let credentials = TlsCredentials::new()?;
    let server = TlsServer::spawn(&credentials).await?;
    let mut cross_origin = server.url.clone();
    cross_origin.set_host(Some("localhost"))?;
    for transport in Transport::ALL {
        let client = FetchClient::new(
            &credentials.fetch_config(),
            new_shared_browser_cookie_store(),
        );
        // Populate an authenticated keep-alive connection before handing a
        // redirected same-origin-credentials request to this same endpoint.
        transport
            .fetch(&client, Request::get(server.url.as_str())?)
            .await?;
        let request = Request::get(server.url.as_str())?
            .with_initiator_url(&server.url)
            .with_request_origin(moli_url::WebOrigin::from_url(&server.url))
            .with_request_mode(RequestMode::Cors)
            .with_credentials_mode(RequestCredentialsMode::SameOrigin)
            .with_redirect_chain(vec![RedirectInfo {
                source: RedirectSource::ServiceWorker,
                from_url: cross_origin.clone(),
                to_url: server.url.clone(),
                status: 302,
                headers: vec![(
                    "Location".to_owned(),
                    server.url.as_str().as_bytes().to_vec(),
                )],
                network_extra_info_available: false,
                request_extra_info: None,
                response_extra_info: None,
                redirect_has_extra_info: false,
                request_cookie_report: None,
                cookie_set_reports: Vec::new(),
                from_cache: false,
                negotiated_http_version: None,
            }]);
        transport.fetch(&client, request).await?;
        let observed = std::mem::take(&mut *server.requests.lock());
        assert_eq!(observed.len(), 2, "{transport:?}");
        assert_eq!(
            observed[0].client_certificates.as_slice(),
            std::slice::from_ref(&credentials.client_certificate)
        );
        assert!(
            observed[1].client_certificates.is_empty(),
            "{transport:?}: {observed:?}"
        );
        assert_ne!(observed[0].connection_id, observed[1].connection_id);
        assert!(client.shutdown().is_clean());
    }
    Ok(())
}
