use std::{sync::Arc, time::Duration};

use bytes::Bytes;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use tokio::time::timeout;
use url::Url;
use web_transport_quinn::{SessionError, WebTransportError};

use crate::{CloseInfo, CloseReason, Config, ConnectOptions, Owner};

const DEADLINE: Duration = Duration::from_secs(10);

struct Fixture {
    endpoint: quinn::Endpoint,
    config: Config,
    options: ConnectOptions,
    _certificates: tempfile::TempDir,
}

impl Fixture {
    fn new() -> Self {
        let cert = rcgen::generate_simple_self_signed(vec!["wt.test".to_owned()]).unwrap();
        let certificates = tempfile::tempdir().unwrap();
        let ca = certificates.path().join("ca.pem");
        std::fs::write(&ca, cert.cert.pem()).unwrap();
        let mut tls = rustls::ServerConfig::builder_with_provider(Arc::new(
            rustls::crypto::aws_lc_rs::default_provider(),
        ))
        .with_protocol_versions(&[&rustls::version::TLS13])
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(
            vec![CertificateDer::from(cert.cert.der().to_vec())],
            PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(cert.key_pair.serialize_der())),
        )
        .unwrap();
        tls.alpn_protocols = vec![web_transport_quinn::ALPN.as_bytes().to_vec()];
        let crypto = quinn::crypto::rustls::QuicServerConfig::try_from(tls).unwrap();
        let config = quinn::ServerConfig::with_crypto(Arc::new(crypto));
        let endpoint = quinn::Endpoint::server(config, "127.0.0.1:0".parse().unwrap()).unwrap();
        let port = endpoint.local_addr().unwrap().port();
        let config = Config {
            tls: moli_curl::CurlTlsConfig {
                ca_cert: Some(ca),
                ..Default::default()
            },
            proxy: Some(String::new()),
            host_resolve: vec![format!("wt.test:{port}:127.0.0.1")],
            ..Default::default()
        };
        let options = ConnectOptions::new(
            Url::parse(&format!("https://wt.test:{port}/session")).unwrap(),
            "https://origin.test".to_owned(),
        );
        Self {
            endpoint,
            config,
            options,
            _certificates: certificates,
        }
    }

    fn server(&self) -> web_transport_quinn::Server {
        web_transport_quinn::Server::new(self.endpoint.clone())
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.endpoint.close(quinn::VarInt::from_u32(0x100), b"");
    }
}

#[tokio::test]
async fn real_session_echoes_byte_streams_datagrams_and_close_capsules() {
    timeout(DEADLINE, async {
        let fixture = Fixture::new();
        let mut server = fixture.server();
        let peer = tokio::spawn(async move {
            let request = server.accept().await.unwrap();
            assert_eq!(request.headers[http::header::ORIGIN], "https://origin.test");
            assert_eq!(request.headers["x-session"], "native");
            assert_eq!(request.protocols, ["first", "second"]);
            let response = web_transport_quinn::proto::ConnectResponse::OK.with_protocol("second");
            let session = request.respond(response).await.unwrap();
            let (mut send, mut recv) = session.accept_bi().await.unwrap();
            let bytes = recv.read_to_end(128 * 1024).await.unwrap();
            send.write_all(&bytes).await.unwrap();
            send.finish().unwrap();
            let mut recv = session.accept_uni().await.unwrap();
            let bytes = recv.read_to_end(128 * 1024).await.unwrap();
            let mut send = session.open_uni().await.unwrap();
            send.write_all(&bytes).await.unwrap();
            send.finish().unwrap();
            let datagram = session.read_datagram().await.unwrap();
            session.send_datagram(datagram).unwrap();
            assert!(matches!(session.closed().await,
                SessionError::WebTransportError(WebTransportError::Closed(42, reason)) if reason == "bye-💡"));
        });
        let owner = Owner::new(fixture.config.clone());
        let mut options = fixture.options.clone();
        options.protocols = vec!["first".into(), "second".into()];
        options.headers.insert("x-session", "native".parse().unwrap());
        options.headers.insert(http::header::ORIGIN, "https://forged.test".parse().unwrap());
        let session = owner.connector().connect(options).await.unwrap();
        assert_eq!(session.protocol(), Some("second"));
        let payload: Vec<u8> = (0u8..=255).cycle().take(64 * 1024).collect();
        let (mut send, mut recv) = session.open_bi().await.unwrap();
        send.write_all(&payload).await.unwrap();
        send.finish().unwrap();
        assert_eq!(recv.read_to_end(payload.len()).await.unwrap(), payload);
        let mut send = session.open_uni().await.unwrap();
        send.write_all(&payload).await.unwrap();
        send.finish().unwrap();
        let mut recv = session.accept_uni().await.unwrap();
        assert_eq!(recv.read_to_end(payload.len()).await.unwrap(), payload);
        session.send_datagram(Bytes::from_static(b"native\0\xff")).unwrap();
        assert_eq!(session.read_datagram().await.unwrap().as_ref(), b"native\0\xff");
        assert!(session.send_datagram(Bytes::from(vec![0; session.max_datagram_size() + 1])).is_err());
        session.close(CloseInfo { code: 42, reason: "bye-💡".into() });
        assert!(matches!(session.closed().await,
            CloseReason::Local(CloseInfo { code: 42, reason }) if reason == "bye-💡"));
        peer.await.unwrap();
    }).await.unwrap();
}

#[tokio::test]
async fn native_streams_retain_their_session_until_the_streams_are_dropped() {
    timeout(DEADLINE, async {
        let fixture = Fixture::new();
        let mut server = fixture.server();
        let peer = tokio::spawn(async move {
            let session = server.accept().await.unwrap().ok().await.unwrap();
            let (mut send, mut recv) = session.accept_bi().await.unwrap();
            let data = recv.read_to_end(1024).await.unwrap();
            send.write_all(&data).await.unwrap();
            send.finish().unwrap();
            session.closed().await;
        });
        let owner = Owner::new(fixture.config.clone());
        let session = owner
            .connector()
            .connect(fixture.options.clone())
            .await
            .unwrap();
        let (mut send, mut recv) = session.open_bi().await.unwrap();
        drop(session);
        send.write_all(b"retained native streams").await.unwrap();
        send.finish().unwrap();
        assert_eq!(
            recv.read_to_end(1024).await.unwrap(),
            b"retained native streams"
        );
        drop(send);
        drop(recv);
        peer.await.unwrap();
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn graceful_close_survives_dropping_the_last_session_wrapper() {
    timeout(DEADLINE, async {
        let fixture = Fixture::new();
        let mut server = fixture.server();
        let peer = tokio::spawn(async move {
            let session = server.accept().await.unwrap().ok().await.unwrap();
            assert!(matches!(session.closed().await,
                SessionError::WebTransportError(WebTransportError::Closed(7, reason)) if reason == "collected"));
        });
        let owner = Owner::new(fixture.config.clone());
        let session = owner.connector().connect(fixture.options.clone()).await.unwrap();
        session.close(CloseInfo { code: 7, reason: "collected".into() });
        drop(session);
        peer.await.unwrap();
    }).await.unwrap();
}

#[tokio::test]
async fn owner_retirement_closes_a_live_peer_and_invalidates_retained_connectors() {
    timeout(DEADLINE, async {
        let fixture = Fixture::new();
        let mut server = fixture.server();
        let peer = tokio::spawn(async move {
            let session = server.accept().await.unwrap().ok().await.unwrap();
            session.closed().await;
        });
        let owner = Owner::new(fixture.config.clone());
        let connector = owner.connector();
        let session = connector.connect(fixture.options.clone()).await.unwrap();
        drop(owner);
        assert!(connector.is_closed());
        assert!(matches!(session.closed().await, CloseReason::OwnerRetired));
        assert!(
            connector
                .connect(fixture.options.clone())
                .await
                .err()
                .unwrap()
                .to_string()
                .contains("owner retired")
        );
        peer.await.unwrap();
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn retiring_owner_interrupts_a_pending_quic_handshake() {
    timeout(DEADLINE, async {
        let fixture = Fixture::new();
        let receiver = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let port = receiver.local_addr().unwrap().port();
        let mut config = fixture.config.clone();
        config.connect_timeout = None;
        config.host_resolve = vec![format!("wt.test:{port}:127.0.0.1")];
        let owner = Owner::new(config);
        let connector = owner.connector();
        let mut options = fixture.options.clone();
        options.url.set_port(Some(port)).unwrap();
        let mut connect = Box::pin(connector.connect(options));
        let mut packet = [0; 1500];
        tokio::select! {
            result = &mut connect => { drop(result); panic!("peer has not answered TLS"); },
            packet = receiver.recv_from(&mut packet) => { packet.unwrap(); },
        }
        drop(owner);
        assert!(
            connect
                .await
                .err()
                .unwrap()
                .to_string()
                .contains("owner retired")
        );
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn cancelling_connect_releases_an_established_quic_connection_before_http3_completes() {
    timeout(DEADLINE, async {
        let fixture = Fixture::new();
        let endpoint = fixture.endpoint.clone();
        let (connected_tx, connected_rx) = tokio::sync::oneshot::channel();
        let peer = tokio::spawn(async move {
            let connection = endpoint.accept().await.unwrap().await.unwrap();
            connected_tx.send(()).unwrap();
            connection.closed().await;
        });
        let owner = Owner::new(fixture.config.clone());
        let connector = owner.connector();
        let mut connect = Box::pin(connector.connect(fixture.options.clone()));
        tokio::select! {
            result = &mut connect => { drop(result); panic!("peer has not answered HTTP/3 SETTINGS"); },
            connected = connected_rx => connected.unwrap(),
        }
        drop(connect);
        peer.await.unwrap();
    }).await.unwrap();
}

#[tokio::test]
async fn all_host_override_addresses_are_admitted_before_any_connection() {
    let fixture = Fixture::new();
    let mut config = fixture.config.clone();
    let port = fixture.options.url.port().unwrap();
    config.host_resolve = vec![format!("wt.test:{port}:127.0.0.1,10.0.0.1")];
    config.address_policy =
        moli_curl::NetworkAddressPolicy::new(false, vec!["10.0.0.0/8".parse().unwrap()]);
    config.tls.ca_cert = Some(fixture._certificates.path().join("does-not-exist.pem"));
    let owner = Owner::new(config);
    let error = owner
        .connector()
        .connect(fixture.options.clone())
        .await
        .err()
        .unwrap();
    assert!(error.to_string().contains("10.0.0.1"));
    assert!(error.to_string().contains("blocked address"));
}

#[tokio::test]
async fn selected_proxy_is_rejected_before_direct_dns_or_tls_work() {
    let fixture = Fixture::new();
    let mut config = fixture.config.clone();
    config.proxy = Some("http://proxy.invalid:1080".into());
    config.no_proxy = Some(String::new());
    config.tls.ca_cert = Some(fixture._certificates.path().join("does-not-exist.pem"));
    let owner = Owner::new(config);
    let error = owner
        .connector()
        .connect(fixture.options.clone())
        .await
        .err()
        .unwrap();
    assert!(
        error
            .to_string()
            .contains("cannot use the selected HTTP/SOCKS proxy")
    );
}

#[tokio::test]
async fn configured_proxy_bypass_reaches_the_original_tls_hostname() {
    timeout(DEADLINE, async {
        let fixture = Fixture::new();
        let mut server = fixture.server();
        let peer = tokio::spawn(async move {
            let session = server.accept().await.unwrap().ok().await.unwrap();
            session.close(13, b"bypass");
            session.closed().await;
        });
        let mut config = fixture.config.clone();
        config.proxy = Some("http://proxy.invalid:1080".into());
        config.no_proxy = Some("wt.test".into());
        let owner = Owner::new(config);
        let session = owner
            .connector()
            .connect(fixture.options.clone())
            .await
            .unwrap();
        assert!(matches!(session.closed().await,
            CloseReason::Remote(CloseInfo { code: 13, reason }) if reason == "bypass"));
        peer.await.unwrap();
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn ca_trust_does_not_disable_hostname_validation() {
    timeout(DEADLINE, async {
        let fixture = Fixture::new();
        let mut server = fixture.server();
        let peer = tokio::spawn(async move { server.accept().await });
        let mut config = fixture.config.clone();
        let port = fixture.options.url.port().unwrap();
        config.host_resolve = vec![format!("wrong.test:{port}:127.0.0.1")];
        let owner = Owner::new(config);
        let mut options = fixture.options.clone();
        options.url.set_host(Some("wrong.test")).unwrap();
        let error = owner.connector().connect(options).await.err().unwrap();
        assert!(error.to_string().contains("not valid for name"));
        peer.abort();
        assert!(peer.await.err().unwrap().is_cancelled());
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn an_unresponsive_address_does_not_block_another_admitted_address() {
    timeout(DEADLINE, async {
        let fixture = Fixture::new();
        let port = fixture.options.url.port().unwrap();
        let _silent = tokio::net::UdpSocket::bind((std::net::Ipv4Addr::new(127, 0, 0, 2), port))
            .await
            .unwrap();
        let mut server = fixture.server();
        let peer = tokio::spawn(async move {
            let session = server.accept().await.unwrap().ok().await.unwrap();
            session.close(9, b"reachable");
            session.closed().await;
        });
        let mut config = fixture.config.clone();
        config.host_resolve = vec![format!("wt.test:{port}:127.0.0.2,127.0.0.1")];
        let owner = Owner::new(config);
        let session = owner
            .connector()
            .connect(fixture.options.clone())
            .await
            .unwrap();
        assert!(matches!(session.closed().await,
            CloseReason::Remote(CloseInfo { code: 9, reason }) if reason == "reachable"));
        peer.await.unwrap();
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn long_close_reason_is_retained_locally_and_truncated_only_on_the_wire() {
    timeout(DEADLINE, async {
        let fixture = Fixture::new();
        let mut server = fixture.server();
        let peer = tokio::spawn(async move {
            let session = server.accept().await.unwrap().ok().await.unwrap();
            assert!(matches!(session.closed().await,
                SessionError::WebTransportError(WebTransportError::Closed(11, reason))
                    if reason == "€".repeat(341)));
        });
        let owner = Owner::new(fixture.config.clone());
        let session = owner
            .connector()
            .connect(fixture.options.clone())
            .await
            .unwrap();
        let reason = "€".repeat(400);
        session.close(CloseInfo {
            code: 11,
            reason: reason.clone(),
        });
        session.close(CloseInfo {
            code: 22,
            reason: "must not replace the first close".into(),
        });
        assert!(matches!(session.closed().await,
            CloseReason::Local(CloseInfo { code: 11, reason: actual }) if actual == reason));
        peer.await.unwrap();
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn retiring_one_session_does_not_retire_its_sibling_or_shared_connector() {
    timeout(DEADLINE, async {
        let fixture = Fixture::new();
        let mut server = fixture.server();
        let peer = tokio::spawn(async move {
            let first = server.accept().await.unwrap().ok().await.unwrap();
            let second = server.accept().await.unwrap().ok().await.unwrap();
            let bytes = first.read_datagram().await.unwrap();
            first.send_datagram(bytes).unwrap();
            let raw = std::ops::Deref::deref(&first);
            // The peer library can reciprocate a CONNECT-stream terminal with
            // a local QUIC close. The owner contract requires actual transport
            // termination here, followed by real traffic on the sibling.
            raw.closed().await;
            let bytes = second.read_datagram().await.unwrap();
            second.send_datagram(bytes).unwrap();
            assert!(matches!(second.closed().await,
                SessionError::WebTransportError(WebTransportError::Closed(77, reason))
                    if reason == "sibling still active"));
        });
        let owner = Owner::new(fixture.config.clone());
        let connector = owner.connector();
        let first = connector.connect(fixture.options.clone()).await.unwrap();
        let second = connector.connect(fixture.options.clone()).await.unwrap();
        first
            .send_datagram(Bytes::from_static(b"first still active before retirement"))
            .unwrap();
        assert_eq!(
            first.read_datagram().await.unwrap().as_ref(),
            b"first still active before retirement"
        );
        first.cancel();
        first.cancel();
        assert!(matches!(first.closed().await, CloseReason::OwnerRetired));
        assert!(!connector.is_closed());
        second
            .send_datagram(Bytes::from_static(b"shared owner, independent sessions"))
            .unwrap();
        assert_eq!(
            second.read_datagram().await.unwrap().as_ref(),
            b"shared owner, independent sessions"
        );
        second.close(CloseInfo {
            code: 77,
            reason: "sibling still active".into(),
        });
        peer.await.unwrap();
    })
    .await
    .unwrap();
}
