//! Drive a real Quinn session entirely through the poll surface.
//!
//! Nothing here calls an `async` method on the transport. Every operation goes
//! through `web_transport_trait::poll`, stepped with `poll_fn` so the trait's own
//! `Poll` contract is what is under test — including that a `Pending` open or accept
//! is actually resumed rather than restarted, which is the part that breaks if a
//! retained future is rebuilt per poll.

use std::{
    future::{poll_fn, Future},
    net::Ipv4Addr,
};

use anyhow::{Context as _, Result};
use rcgen::{CertifiedKey, KeyPair};
use web_transport_proto::ConnectRequest;
use web_transport_quinn::quinn::rustls::pki_types::{PrivateKeyDer, PrivatePkcs8KeyDer};
use web_transport_quinn::{ClientBuilder, ServerBuilder};
use web_transport_trait::{
    poll::{RecvStream, SendStream, Session},
    Error as _,
};

/// With both crypto features enabled — as `--all-features` does — the crate cannot
/// pick a provider for us, so the test installs one. Harmless when only one feature
/// is on, since a default is already set by then.
fn install_crypto_provider() {
    #[cfg(all(feature = "aws-lc-rs", feature = "ring"))]
    {
        let _ = web_transport_quinn::quinn::rustls::crypto::aws_lc_rs::default_provider()
            .install_default();
    }
}

/// A client and server session, already connected.
async fn pair() -> Result<(web_transport_quinn::Session, web_transport_quinn::Session)> {
    install_crypto_provider();

    let CertifiedKey { cert, signing_key } =
        rcgen::generate_simple_self_signed(vec!["localhost".into(), "127.0.0.1".into()])?;
    let chain = vec![cert.der().clone()];
    let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(KeyPair::serialize_der(
        &signing_key,
    )));

    let mut server = ServerBuilder::new()
        .with_addr((Ipv4Addr::LOCALHOST, 0).into())
        .with_certificate(chain.clone(), key)?;

    let addr = server.local_addr()?;

    let server_task = tokio::spawn(async move {
        let request = server.accept().await.context("no connection")?;
        let session = request.ok().await?;
        anyhow::Ok(session)
    });

    let url: url::Url = format!("https://127.0.0.1:{}/", addr.port()).parse()?;
    let client = ClientBuilder::new().with_server_certificates(chain)?;
    let client = client.connect(ConnectRequest::new(url)).await?;

    let server = server_task.await??;
    Ok((client, server))
}

/// Open, write, finish, accept and read — all through `poll_*`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_uni_stream_round_trips_through_the_poll_surface() -> Result<()> {
    let (mut client, mut server) = pair().await?;

    let mut send = poll_fn(|cx| client.poll_open_uni(cx)).await?;
    let size = poll_fn(|cx| send.poll_write(cx, b"hello")).await?;
    assert_eq!(size, 5);
    send.finish()?;

    let mut recv = poll_fn(|cx| server.poll_accept_uni(cx)).await?;

    let mut dst = [0u8; 16];
    let size = poll_fn(|cx| recv.poll_read(cx, &mut dst))
        .await?
        .context("stream ended early")?;
    assert_eq!(&dst[..size], b"hello");

    // Drained and finished.
    assert_eq!(poll_fn(|cx| recv.poll_read(cx, &mut dst)).await?, None);

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_bi_stream_round_trips_through_the_poll_surface() -> Result<()> {
    let (mut client, mut server) = pair().await?;

    let (mut client_send, mut client_recv) = poll_fn(|cx| client.poll_open_bi(cx)).await?;
    poll_fn(|cx| client_send.poll_write(cx, b"ping")).await?;

    let (mut server_send, mut server_recv) = poll_fn(|cx| server.poll_accept_bi(cx)).await?;

    let mut dst = [0u8; 16];
    let size = poll_fn(|cx| server_recv.poll_read(cx, &mut dst))
        .await?
        .context("stream ended early")?;
    assert_eq!(&dst[..size], b"ping");

    poll_fn(|cx| server_send.poll_write(cx, b"pong")).await?;
    let size = poll_fn(|cx| client_recv.poll_read(cx, &mut dst))
        .await?
        .context("stream ended early")?;
    assert_eq!(&dst[..size], b"pong");

    Ok(())
}

/// The datagram path, including that `poll_send_datagram` is retried with the same
/// payload rather than taking ownership.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn datagrams_round_trip_through_the_poll_surface() -> Result<()> {
    let (mut client, mut server) = pair().await?;

    poll_fn(|cx| client.poll_send_datagram(cx, b"dgram")).await?;

    let received = poll_fn(|cx| server.poll_recv_datagram(cx)).await?;
    assert_eq!(&received[..], b"dgram");

    Ok(())
}

/// A `Pending` operation must be *resumed*, not restarted. Poll `poll_closed` once
/// against a waker that goes nowhere, then drive it to completion on a different one:
/// if the implementation rebuilt its close-wait future per poll it would have dropped
/// Quinn's notifier registration and this would hang.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_pending_operation_is_resumed_not_restarted() -> Result<()> {
    let (mut client, mut server) = pair().await?;

    let mut closed = std::pin::pin!(poll_fn(|cx| client.poll_closed(cx)));

    // Register, then abandon that poll.
    let noop = std::task::Waker::noop();
    assert!(
        closed
            .as_mut()
            .poll(&mut std::task::Context::from_waker(noop))
            .is_pending(),
        "the session should still be open"
    );

    // Disambiguate from the inherent `close`, which takes bytes.
    Session::close(&mut server, 42, "bye");

    // Now drive it on the real task waker.
    let err = closed.await;
    assert!(
        err.session_error().is_some(),
        "expected an application close, got {err:?}"
    );

    Ok(())
}
