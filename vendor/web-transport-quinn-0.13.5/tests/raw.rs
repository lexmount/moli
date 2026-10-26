//! Raw QUIC sessions: a QUIC connection using a non-HTTP/3 ALPN, wrapped in a
//! [`Session`] so callers can treat WebTransport and raw QUIC uniformly.
//!
//! There is no HTTP/3 exchange on this path, so there is neither a request URL nor
//! a response. The session reports the negotiated ALPN as its protocol instead.

// A crypto provider is needed to establish the connection, matching the gate on
// the crate's own builders.
#![cfg(any(feature = "aws-lc-rs", feature = "ring"))]

use std::{
    net::{Ipv4Addr, SocketAddr},
    sync::Arc,
    time::Duration,
};

use anyhow::{Context, Result};
use rcgen::{CertifiedKey, KeyPair};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
// `Session::raw` and the inherent `protocol()` accessor are used directly.
use web_transport_quinn::Session;

/// A raw QUIC ALPN, i.e. anything other than the `h3` used by WebTransport.
const RAW_ALPN: &str = "moq-00";

fn self_signed() -> Result<(CertificateDer<'static>, PrivateKeyDer<'static>)> {
    let CertifiedKey { cert, signing_key } =
        rcgen::generate_simple_self_signed(vec!["localhost".into()]).context("rcgen")?;

    let cert_der = CertificateDer::from(cert.der().to_vec());
    let key_der = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(KeyPair::serialize_der(
        &signing_key,
    )));

    Ok((cert_der, key_der))
}

/// The provider is passed explicitly rather than relying on the process-level
/// default, which rustls cannot determine when both backend features are enabled
/// (as `--all-features` does).
fn crypto_provider() -> Arc<rustls::crypto::CryptoProvider> {
    #[cfg(feature = "aws-lc-rs")]
    {
        Arc::new(rustls::crypto::aws_lc_rs::default_provider())
    }
    #[cfg(all(feature = "ring", not(feature = "aws-lc-rs")))]
    {
        Arc::new(rustls::crypto::ring::default_provider())
    }
}

/// Bind a server endpoint and a client endpoint, both offering only `RAW_ALPN`.
fn endpoints() -> Result<(quinn::Endpoint, quinn::Endpoint, SocketAddr)> {
    let (cert, key) = self_signed()?;
    let provider = crypto_provider();

    let mut server_crypto = rustls::ServerConfig::builder_with_provider(provider.clone())
        .with_protocol_versions(&[&rustls::version::TLS13])?
        .with_no_client_auth()
        .with_single_cert(vec![cert.clone()], key)?;
    server_crypto.alpn_protocols = vec![RAW_ALPN.as_bytes().to_vec()];
    server_crypto.max_early_data_size = u32::MAX;

    let server_config = quinn::ServerConfig::with_crypto(Arc::new(
        quinn::crypto::rustls::QuicServerConfig::try_from(server_crypto)?,
    ));

    let bind: SocketAddr = (Ipv4Addr::LOCALHOST, 0).into();
    let server = quinn::Endpoint::server(server_config, bind)?;
    let server_addr = server.local_addr()?;

    let mut roots = rustls::RootCertStore::empty();
    roots.add(cert)?;

    let mut client_crypto = rustls::ClientConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&rustls::version::TLS13])?
        .with_root_certificates(roots)
        .with_no_client_auth();
    client_crypto.alpn_protocols = vec![RAW_ALPN.as_bytes().to_vec()];
    // Needed for the client-side 0-RTT test below; harmless elsewhere. The default
    // `resumption` is an in-memory store, so one endpoint can resume its own ticket.
    client_crypto.enable_early_data = true;

    let client_config = quinn::ClientConfig::new(Arc::new(
        quinn::crypto::rustls::QuicClientConfig::try_from(client_crypto)?,
    ));

    let mut client = quinn::Endpoint::client((Ipv4Addr::LOCALHOST, 0).into())?;
    client.set_default_client_config(client_config);

    Ok((server, client, server_addr))
}

/// Bind a QUIC server and connect a client to it, both fully handshaken.
async fn connect_raw() -> Result<(quinn::Connection, quinn::Connection)> {
    let (server, client, server_addr) = endpoints()?;

    let accept = tokio::spawn(async move {
        let conn = server.accept().await.context("no connection")?.await?;
        anyhow::Ok(conn)
    });

    let client_conn = client.connect(server_addr, "localhost")?.await?;
    let server_conn = accept.await??;

    Ok((client_conn, server_conn))
}

/// The ALPN the peers negotiated, as an accept path would read it.
fn negotiated_alpn(conn: &quinn::Connection) -> Result<String> {
    let data = conn
        .handshake_data()
        .context("no handshake data")?
        .downcast::<quinn::crypto::rustls::HandshakeData>()
        .ok()
        .context("unexpected handshake data")?;

    Ok(String::from_utf8(data.protocol.context("no ALPN")?)?)
}

/// A raw session has no CONNECT request, so it has no URL. Before this was an
/// `Option`, callers had to synthesize a fake request to construct one.
#[tokio::test]
async fn raw_session_has_no_request() -> Result<()> {
    let (client_conn, _server_conn) = connect_raw().await?;

    let session = Session::raw(client_conn);
    assert!(session.request().is_none());
    assert!(session.response().is_none());

    Ok(())
}

/// `protocol()` reports the ALPN the QUIC handshake negotiated, read off the
/// connection rather than supplied by the caller.
#[tokio::test]
async fn raw_session_reports_alpn_as_protocol() -> Result<()> {
    let (client_conn, server_conn) = connect_raw().await?;

    // Independently confirm what was negotiated, so the assertion below is checking
    // the session against the connection rather than against itself.
    assert_eq!(negotiated_alpn(&client_conn)?, RAW_ALPN);

    // Both directions: the server reads the ALPN from its own handshake data too.
    assert_eq!(Session::raw(client_conn).protocol(), Some(RAW_ALPN));
    assert_eq!(Session::raw(server_conn).protocol(), Some(RAW_ALPN));

    Ok(())
}

/// Wrapping a connection that is still handshaking must not pin `protocol()` to
/// `None` for the life of the session.
///
/// A resuming client gets its [`quinn::Connection`] from `into_0rtt` before the server
/// has answered, so the ALPN genuinely is not negotiated yet. Reading `protocol()` in
/// that window must not stop a later read from seeing the real value.
#[tokio::test]
async fn a_handshaking_session_reports_its_alpn_once_it_completes() -> Result<()> {
    let (server, client, server_addr) = endpoints()?;

    // Accept connections for the whole test; 0-RTT needs a prior session to resume.
    let accepting = tokio::spawn(async move {
        while let Some(incoming) = server.accept().await {
            tokio::spawn(async move {
                if let Ok(conn) = incoming.await {
                    conn.closed().await;
                }
            });
        }
    });

    // 0-RTT needs a ticket from an earlier session, and the ticket arrives after that
    // handshake with no completion to await. Connect until one is available rather than
    // sleeping on a guess: the first attempt is what produces the ticket, and a later
    // one resumes it.
    let mut zero_rtt = None;
    for _ in 0..10 {
        match client.connect(server_addr, "localhost")?.into_0rtt() {
            Ok(resumed) => {
                zero_rtt = Some(resumed);
                break;
            }
            Err(connecting) => {
                let conn = connecting.await?;
                // The ticket arrives on this connection shortly after the handshake,
                // with nothing to await on, so hold it open long enough to receive
                // one. Retrying is what makes this robust — not the wait itself.
                tokio::time::sleep(Duration::from_millis(50)).await;
                conn.close(0u32.into(), b"");
                conn.closed().await;
            }
        }
    }

    let (conn, accepted) = zero_rtt.context("0-RTT never became available")?;

    let session = Session::raw(conn);

    // The ALPN is not negotiated yet, so this read has nothing to report...
    assert_eq!(
        session.protocol(),
        None,
        "expected no ALPN before the handshake completed"
    );

    // ...and it must not have poisoned the answer for later reads.
    assert!(accepted.await, "0-RTT was rejected by the server");
    assert_eq!(
        session.protocol(),
        Some(RAW_ALPN),
        "an early read pinned protocol() to None for the life of the session"
    );

    accepting.abort();

    Ok(())
}

/// A raw session must not write the WebTransport stream header, since the peer
/// is reading plain QUIC.
#[tokio::test]
async fn raw_session_streams_omit_webtransport_header() -> Result<()> {
    let (client_conn, server_conn) = connect_raw().await?;

    let client = Session::raw(client_conn);
    let server = Session::raw(server_conn);

    let mut send = client.open_uni().await?;
    send.write_all(b"hello").await?;
    send.finish()?;

    let mut recv = server.accept_uni().await?;
    let data = recv.read_to_end(1024).await?;
    assert_eq!(&data, b"hello");

    Ok(())
}

#[tokio::test]
async fn raw_peer_close_code_is_reported_by_session_and_streams() -> Result<()> {
    use web_transport_trait::Error as _;
    let (client, peer) = connect_raw().await?;
    let session = Session::raw(client);
    let mut send = session.open_uni().await?;
    send.write_all(b"hello").await?;
    let mut peer_recv = peer.accept_uni().await?;
    let mut hello = [0; 5];
    peer_recv.read_exact(&mut hello).await?;
    let mut peer_send = peer.open_uni().await?;
    peer_send.write_all(b"hello").await?;
    let mut recv = session.accept_uni().await?;
    recv.read_exact(&mut hello).await?;
    peer.close(4075u32.into(), b"application close");
    let closed = session.closed().await;
    assert_eq!(
        closed.session_error(),
        Some((4075, "application close".into()))
    );
    assert_eq!(
        session.close_reason().unwrap().session_error(),
        closed.session_error()
    );
    assert_eq!(
        send.write_all(b"after close")
            .await
            .unwrap_err()
            .session_error(),
        closed.session_error()
    );
    assert_eq!(
        recv.read(&mut hello).await.unwrap_err().session_error(),
        closed.session_error()
    );
    let stopped = futures::future::poll_fn(|cx| {
        web_transport_trait::poll::SendStream::poll_closed(&mut send, cx)
    })
    .await
    .unwrap_err();
    assert_eq!(stopped.session_error(), closed.session_error());
    Ok(())
}

#[tokio::test]
async fn raw_peer_close_survives_local_close() -> Result<()> {
    use web_transport_trait::Error as _;
    let (client, peer) = connect_raw().await?;
    let session = Session::raw(client.clone());
    peer.close(4075u32.into(), b"application close");
    // Wait on the quinn connection so the session never observes the close first.
    client.closed().await;
    session.close(1, b"cleanup");
    assert_eq!(
        session.close_reason().unwrap().session_error(),
        Some((4075, "application close".into()))
    );
    Ok(())
}

#[tokio::test]
async fn raw_peer_close_above_u32_is_not_a_session_error() -> Result<()> {
    use web_transport_trait::Error as _;
    // Includes a code that would decode as WebTransport code 7 under the HTTP/3 mapping.
    for code in [u32::MAX as u64 + 1, web_transport_proto::error_to_http3(7)] {
        let (client, peer) = connect_raw().await?;
        let session = Session::raw(client);
        peer.close(quinn::VarInt::from_u64(code)?, b"too big");
        assert_eq!(
            session.closed().await.session_error(),
            None,
            "code {code:#x}"
        );
    }
    Ok(())
}

/// A raw session sends RESET_STREAM and STOP_SENDING codes as is, so a plain QUIC
/// peer agrees on them, and still reads the HTTP/3-mapped codes an older raw peer sends.
#[tokio::test]
async fn raw_session_stream_codes_are_not_mapped() -> Result<()> {
    use web_transport_trait::Error as _;

    const CODE: u32 = 5;

    let (client_conn, plain) = connect_raw().await?;
    let session = Session::raw(client_conn);
    let legacy = quinn::VarInt::try_from(web_transport_proto::error_to_http3(CODE))?;

    tokio::time::timeout(Duration::from_secs(10), async {
        // Our reset and stop reach the plain peer as is.
        let mut send = session.open_uni().await?;
        send.write_all(b"x").await?;
        let mut recv = plain.accept_uni().await?;
        recv.read_exact(&mut [0u8; 1]).await?;
        send.reset(CODE)?;
        let err = recv.read(&mut [0u8; 1]).await.unwrap_err();
        assert!(
            matches!(err, quinn::ReadError::Reset(code) if code == CODE.into()),
            "{err:?}"
        );

        let mut send = plain.open_uni().await?;
        send.write_all(b"x").await?;
        let mut recv = session.accept_uni().await?;
        recv.read_exact(&mut [0u8; 1]).await?;
        recv.stop(CODE)?;
        assert_eq!(send.stopped().await?, Some(CODE.into()));

        // The plain peer's codes, and a legacy peer's mapped ones, read back as the code.
        for code in [CODE.into(), legacy] {
            let mut send = plain.open_uni().await?;
            send.write_all(b"x").await?;
            let mut recv = session.accept_uni().await?;
            recv.read_exact(&mut [0u8; 1]).await?;
            send.reset(code)?;
            let err = recv.read(&mut [0u8; 1]).await.unwrap_err();
            assert_eq!(err.stream_error(), Some(CODE), "{err}");

            let mut send = session.open_uni().await?;
            send.write_all(b"x").await?;
            let mut recv = plain.accept_uni().await?;
            recv.read_exact(&mut [0u8; 1]).await?;
            recv.stop(code)?;
            let err = loop {
                if let Err(err) = send.write(&[0u8; 1024]).await {
                    break err;
                }
            };
            assert_eq!(err.stream_error(), Some(CODE), "{err}");
            assert_eq!(send.stopped().await?, Some(CODE));
        }

        anyhow::Ok(())
    })
    .await?
}
