//! Accepters come and go; the accept queue must not leak them or lose them.
//!
//! `SessionAccept` keeps a list of parked accepters so that every one of them is woken
//! when a stream arrives. That list used to be a `Vec<Waker>` with no way to say "I'm
//! gone", cleared only by a stream arriving or the connection failing, so a caller that
//! started an accept and dropped the future — a `tokio::time::timeout` around
//! `accept_uni()`, say — left its waker there forever. Distinct tasks doing that on a
//! quiet connection grew the list without bound, retaining a task allocation apiece.
//!
//! The wake path had the mirror-image problem: the inner accept future was polled with
//! the caller's waker, so the last caller to poll owned the only path an arrival had
//! back into this crate. When that caller walked away, the accepters still parked here
//! were never woken.

use std::{
    future::Future,
    net::Ipv4Addr,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    task::{Context, Wake, Waker},
    time::Duration,
};

use anyhow::{Context as _, Result};
use rcgen::{CertifiedKey, KeyPair};
use tokio::time::timeout;
use web_transport_proto::ConnectRequest;
use web_transport_quinn::quinn::rustls::pki_types::{PrivateKeyDer, PrivatePkcs8KeyDer};
use web_transport_quinn::{ClientBuilder, ServerBuilder};

/// How many accepters give up, one after another. Well past any plausible bound on
/// concurrent accepters, so a list that keeps every registration is obvious.
const ABANDONED: usize = 256;

/// A waker that records whether it was woken. The test also watches its `Arc` count:
/// anything still holding a clone of the waker is retaining this allocation.
#[derive(Default)]
struct FlagWaker {
    woken: AtomicBool,
}

impl FlagWaker {
    fn woken(&self) -> bool {
        self.woken.load(Ordering::SeqCst)
    }
}

impl Wake for FlagWaker {
    fn wake(self: Arc<Self>) {
        self.woken.store(true, Ordering::SeqCst);
    }

    fn wake_by_ref(self: &Arc<Self>) {
        self.woken.store(true, Ordering::SeqCst);
    }
}

/// With both crypto features enabled — as `--all-features` does — the crate cannot
/// pick a provider for us, so the test installs one.
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

/// Register an accepter, abandon it, and repeat — on a connection where no stream ever
/// arrives, which is the case that used to grow forever. Nothing here delivers a
/// stream: an arrival drains the list, which would hide exactly what is under test.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn abandoned_accepters_release_their_wakers() -> Result<()> {
    let (client, server) = pair().await?;

    // Each iteration is a distinct task's worth of waker, registered by a single poll
    // and abandoned when the future drops at the end of the scope.
    let mut abandoned = Vec::with_capacity(ABANDONED);
    for _ in 0..ABANDONED {
        let flag = Arc::new(FlagWaker::default());
        let waker = Waker::from(flag.clone());

        let mut accept = std::pin::pin!(server.accept_uni());
        assert!(
            accept
                .as_mut()
                .poll(&mut Context::from_waker(&waker))
                .is_pending(),
            "no stream should be available yet"
        );

        abandoned.push(flag);
    }

    // Every abandoned waker should be uniquely owned by the test again: the shared
    // accept futures are polled with the accept state's own waker, never the caller's,
    // so there is nothing else that could still be holding one.
    let retained = abandoned
        .iter()
        .filter(|flag| Arc::strong_count(flag) > 1)
        .count();
    assert_eq!(
        retained, 0,
        "{retained} of {ABANDONED} abandoned accepters are still parked in the accept list"
    );

    client.close(0, b"bye");
    Ok(())
}

/// Reclaiming an abandoned accepter's slot must not cost a live accepter its wakeup.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn churn_does_not_lose_a_live_accepter() -> Result<()> {
    let (client, server) = pair().await?;

    // One accepter that sticks around, registered before the churn.
    let parked = Arc::new(FlagWaker::default());
    let parked_waker = Waker::from(parked.clone());
    let mut waiting = std::pin::pin!(server.accept_uni());
    assert!(
        waiting
            .as_mut()
            .poll(&mut Context::from_waker(&parked_waker))
            .is_pending(),
        "no stream should be available yet"
    );

    for _ in 0..ABANDONED {
        let waker = Waker::from(Arc::new(FlagWaker::default()));
        let mut accept = std::pin::pin!(server.accept_uni());
        assert!(accept
            .as_mut()
            .poll(&mut Context::from_waker(&waker))
            .is_pending());
    }

    let mut send = client.open_uni().await?;
    send.write_all(b"hi").await?;
    send.finish()?;

    // A second accepter takes the stream; the first should have been woken to retry.
    let recv = timeout(Duration::from_secs(10), server.accept_uni())
        .await
        .context("the stream never arrived")??;
    drop(recv);

    assert!(
        parked.woken(),
        "the accepter that was still waiting lost its registration to the churn"
    );

    client.close(0, b"bye");
    Ok(())
}

/// The last accepter to poll is the one the inner accept future holds a waker for. If it
/// gives up, the arrival has to reach whoever is still waiting.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_departing_accepter_does_not_strand_the_others() -> Result<()> {
    let (client, server) = pair().await?;

    // A parks first.
    let a = Arc::new(FlagWaker::default());
    let a_waker = Waker::from(a.clone());
    let mut first = std::pin::pin!(server.accept_uni());
    assert!(first
        .as_mut()
        .poll(&mut Context::from_waker(&a_waker))
        .is_pending());

    // B polls after A, so B's waker is the one the inner future holds, then gives up.
    {
        let b = Waker::from(Arc::new(FlagWaker::default()));
        let mut second = std::pin::pin!(server.accept_uni());
        assert!(second
            .as_mut()
            .poll(&mut Context::from_waker(&b))
            .is_pending());
    }

    let mut send = client.open_uni().await?;
    send.write_all(b"hi").await?;
    send.finish()?;

    // Nothing here polls the accept path again: A has to be woken by the arrival.
    for _ in 0..500 {
        if a.woken() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(a.woken(), "the remaining accepter was never woken");

    client.close(0, b"bye");
    Ok(())
}
