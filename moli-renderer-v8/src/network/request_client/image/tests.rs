use futures_util::{future::join_all, poll};
use moli_fetch::{FetchConfig, RequestCacheMode};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::{mpsc, oneshot},
    task::{JoinHandle, JoinSet},
    time::{Duration, timeout},
};

use super::*;

const DEADLINE: Duration = Duration::from_secs(5);

struct ImageRequestGate {
    response: oneshot::Sender<()>,
    closed: oneshot::Receiver<()>,
}

struct ImageServer {
    url: String,
    requests: mpsc::UnboundedReceiver<ImageRequestGate>,
    task: JoinHandle<()>,
}

impl Drop for ImageServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl ImageServer {
    async fn start() -> Result<Self> {
        Self::with_early_headers(false).await
    }

    async fn with_early_headers(early_headers: bool) -> Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let url = format!("http://{}/shared.png", listener.local_addr()?);
        let (requests_tx, requests) = mpsc::unbounded_channel();
        let task = tokio::spawn(async move {
            let mut connections = JoinSet::new();
            loop {
                tokio::select! {
                    accepted = listener.accept() => {
                        let (mut stream, _) = accepted.expect("accept image request");
                        let requests_tx = requests_tx.clone();
                        connections.spawn(async move {
                            let mut headers = Vec::new();
                            while !headers.ends_with(b"\r\n\r\n") {
                                headers.push(stream.read_u8().await.expect("image request headers"));
                                assert!(headers.len() < 64 * 1024);
                            }
                            const RESPONSE_HEAD: &[u8] = b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n";
                            if early_headers {
                                stream.write_all(RESPONSE_HEAD).await.expect("early image headers");
                            }
                            let (response_tx, response) = oneshot::channel();
                            let (closed_tx, closed) = oneshot::channel();
                            requests_tx.send(ImageRequestGate { response: response_tx, closed })
                                .expect("image request observer");
                            let mut byte = [0];
                            tokio::select! {
                                received = stream.read(&mut byte) => {
                                    assert_eq!(received.expect("read cancelled socket"), 0);
                                    let _ = closed_tx.send(());
                                }
                                response = response => {
                                    if response.is_ok() {
                                        if !early_headers {
                                            stream.write_all(RESPONSE_HEAD).await.expect("image headers");
                                        }
                                        stream.write_all(b"body")
                                            .await.expect("image response");
                                    }
                                }
                            }
                        });
                    }
                    finished = connections.join_next(), if !connections.is_empty() => {
                        finished.expect("connection task").expect("connection finished");
                    }
                }
            }
        });
        Ok(Self {
            url,
            requests,
            task,
        })
    }

    fn request(&self) -> Request {
        Request::get(&self.url)
            .unwrap()
            .with_request_origin(moli_url::WebOrigin::from_url(
                &url::Url::parse(&self.url).unwrap(),
            ))
            .with_resource_type(RequestResourceType::Image)
            .with_browser_request_metadata(BrowserRequestMetadata::Image)
            .with_request_mode(RequestMode::NoCors)
            .with_page_network_policy()
    }

    async fn next(&mut self) -> ImageRequestGate {
        timeout(DEADLINE, self.requests.recv())
            .await
            .expect("image reached server")
            .expect("request gate")
    }
}

#[tokio::test]
async fn no_store_headers_prevent_later_consumers_joining_a_pending_image() -> Result<()> {
    use std::future::{Future, poll_fn};
    let mut server = ImageServer::with_early_headers(true).await?;
    let owner = ResourceRequestClient::new(&FetchConfig::default())?;
    let first = owner
        .fetch_image_with_cancel_and_network_metadata(server.request(), FetchCancelHandle::new());
    tokio::pin!(first);
    assert!(poll!(&mut first).is_pending());
    let early = server.next().await;
    let allow_late_join = owner
        .pending_images
        .entries
        .lock()
        .values()
        .next()
        .unwrap()
        .allow_late_join
        .clone();
    timeout(
        DEADLINE,
        poll_fn(|cx| {
            assert!(
                first.as_mut().poll(cx).is_pending(),
                "image body is still gated"
            );
            if allow_late_join.load(Ordering::Acquire) {
                std::task::Poll::Pending
            } else {
                std::task::Poll::Ready(())
            }
        }),
    )
    .await?;
    let second = owner
        .fetch_image_with_cancel_and_network_metadata(server.request(), FetchCancelHandle::new());
    tokio::pin!(second);
    assert!(poll!(&mut second).is_pending());
    let late = server.next().await;
    early.response.send(()).unwrap();
    late.response.send(()).unwrap();
    let (first, second) = timeout(DEADLINE, async { tokio::join!(first, second) }).await?;
    assert_eq!(first?.response().body_bytes(), b"body");
    assert_eq!(second?.response().body_bytes(), b"body");
    Ok(())
}

#[tokio::test]
async fn concurrent_image_consumers_share_one_transfer_and_exact_response() -> Result<()> {
    let mut server = ImageServer::start().await?;
    let owner = ResourceRequestClient::new(&FetchConfig::default())?;
    let client = owner.frozen_request_client();
    let mut loads: Vec<_> = (0..5)
        .map(|_| {
            Box::pin(client.fetch_image_with_cancel_and_network_metadata(
                server.request(),
                FetchCancelHandle::new(),
            ))
        })
        .collect();
    for load in &mut loads {
        assert!(poll!(load.as_mut()).is_pending());
    }
    server.next().await.response.send(()).unwrap();
    let responses = timeout(DEADLINE, join_all(loads)).await?;
    for response in &responses {
        let response = response.as_ref().expect("image response");
        assert_eq!(response.response().body_bytes(), b"body");
        assert_eq!(response.response().status, 200);
        assert!(!response.response().from_cache);
        assert!(response.request_observation().is_some());
        assert_eq!(
            response.observation_journal(),
            responses[0].as_ref().unwrap().observation_journal()
        );
    }
    assert!(server.requests.try_recv().is_err());
    Ok(())
}

#[tokio::test]
async fn streaming_image_consumers_join_materialized_loads_and_preserve_observations() -> Result<()>
{
    let mut server = ImageServer::start().await?;
    let owner = ResourceRequestClient::new(&FetchConfig::default())?;
    let client = owner.frozen_request_client();
    let streaming = client.fetch_raw_stream_with_cancel_and_network_metadata(
        server.request(),
        FetchCancelHandle::new(),
    );
    let materialized = owner
        .fetch_image_with_cancel_and_network_metadata(server.request(), FetchCancelHandle::new());
    tokio::pin!(streaming, materialized);
    assert!(poll!(&mut streaming).is_pending());
    assert!(poll!(&mut materialized).is_pending());
    server.next().await.response.send(()).unwrap();
    let (streaming, materialized) =
        timeout(DEADLINE, async { tokio::join!(streaming, materialized) }).await?;
    let streaming = streaming?;
    let materialized = materialized?;
    assert!(streaming.request_observation().is_some());
    assert_eq!(
        streaming.response().final_url,
        materialized.response().final_url
    );
    assert_eq!(streaming.response().status, 200);
    assert_eq!(
        streaming.response().headers,
        materialized.response().headers
    );
    assert!(!streaming.response().from_cache);
    let (_, encoded) = crate::network_host::collect_image_response_into_parkable(
        streaming,
        moli_parkable_image::ParkableImageManager::default(),
    )
    .await
    .map_err(anyhow::Error::msg)?;
    assert_eq!(
        encoded.snapshot()?.as_ref(),
        materialized.response().body_bytes()
    );
    assert!(matches!(
        server.requests.try_recv(),
        Err(mpsc::error::TryRecvError::Empty)
    ));
    Ok(())
}

#[tokio::test]
async fn cancelling_one_image_consumer_keeps_the_other_transfer_alive() -> Result<()> {
    let mut server = ImageServer::start().await?;
    let owner = ResourceRequestClient::new(&FetchConfig::default())?;
    let peer = owner.frozen_request_client();
    let cancel = FetchCancelHandle::new();
    let first =
        owner.fetch_image_with_cancel_and_network_metadata(server.request(), cancel.clone());
    let second = peer
        .fetch_image_with_cancel_and_network_metadata(server.request(), FetchCancelHandle::new());
    tokio::pin!(first, second);
    assert!(poll!(&mut first).is_pending());
    assert!(poll!(&mut second).is_pending());
    let gate = server.next().await;
    cancel.cancel();
    assert!(
        timeout(DEADLINE, first)
            .await?
            .unwrap_err()
            .to_string()
            .contains("cancelled")
    );
    gate.response.send(()).unwrap();
    assert_eq!(
        timeout(DEADLINE, second).await??.response().body_bytes(),
        b"body"
    );
    assert!(server.requests.try_recv().is_err());
    Ok(())
}

#[tokio::test]
async fn last_image_consumer_cancels_transport_and_next_request_starts_fresh() -> Result<()> {
    for explicit_cancel in [true, false] {
        let mut server = ImageServer::start().await?;
        let owner = ResourceRequestClient::new(&FetchConfig::default())?;
        let cancel = FetchCancelHandle::new();
        let mut first = Box::pin(
            owner.fetch_image_with_cancel_and_network_metadata(server.request(), cancel.clone()),
        );
        assert!(poll!(first.as_mut()).is_pending());
        let gate = server.next().await;
        if explicit_cancel {
            cancel.cancel();
            assert!(timeout(DEADLINE, &mut first).await?.is_err());
        }
        drop(first);
        timeout(DEADLINE, gate.closed).await??;
        let second = owner.fetch_image_with_cancel_and_network_metadata(
            server.request(),
            FetchCancelHandle::new(),
        );
        tokio::pin!(second);
        assert!(poll!(&mut second).is_pending());
        server.next().await.response.send(()).unwrap();
        assert_eq!(
            timeout(DEADLINE, second).await??.response().body_bytes(),
            b"body"
        );
    }
    Ok(())
}

#[tokio::test]
async fn failed_image_load_is_shared_but_not_cached() -> Result<()> {
    let mut server = ImageServer::start().await?;
    let owner = ResourceRequestClient::new(&FetchConfig::default())?;
    let first = owner
        .fetch_image_with_cancel_and_network_metadata(server.request(), FetchCancelHandle::new());
    let second = owner
        .fetch_image_with_cancel_and_network_metadata(server.request(), FetchCancelHandle::new());
    tokio::pin!(first, second);
    assert!(poll!(&mut first).is_pending());
    assert!(poll!(&mut second).is_pending());
    drop(server.next().await.response);
    let (first, second) = timeout(DEADLINE, async { tokio::join!(first, second) }).await?;
    let first = first.unwrap_err();
    let second = second.unwrap_err();
    assert_eq!(format!("{first:#}"), format!("{second:#}"));
    assert!(
        first.chain().count() > 1,
        "preserve the transport cause chain"
    );
    let third = owner
        .fetch_image_with_cancel_and_network_metadata(server.request(), FetchCancelHandle::new());
    tokio::pin!(third);
    assert!(poll!(&mut third).is_pending());
    server.next().await.response.send(()).unwrap();
    assert_eq!(
        timeout(DEADLINE, third).await??.response().body_bytes(),
        b"body"
    );
    Ok(())
}

#[tokio::test]
async fn image_sharing_does_not_bypass_current_network_policy() -> Result<()> {
    let mut server = ImageServer::start().await?;
    let owner = ResourceRequestClient::new(&FetchConfig::default())?;
    let first = owner
        .fetch_image_with_cancel_and_network_metadata(server.request(), FetchCancelHandle::new());
    tokio::pin!(first);
    assert!(poll!(&mut first).is_pending());
    let gate = server.next().await;
    owner.set_network_offline(true);
    assert!(
        owner
            .fetch_image_with_cancel_and_network_metadata(
                server.request(),
                FetchCancelHandle::new()
            )
            .await
            .is_err()
    );
    gate.response.send(()).unwrap();
    assert_eq!(
        timeout(DEADLINE, first).await??.response().body_bytes(),
        b"body"
    );
    Ok(())
}

#[test]
fn pending_images_are_document_local_but_shared_by_frozen_request_views() -> Result<()> {
    let owner = ResourceRequestClient::new(&FetchConfig::default())?;
    let frozen = owner.frozen_request_client();
    assert!(Arc::ptr_eq(&owner.pending_images, &frozen.pending_images));
    for isolated in [
        owner.fork_with_isolated_document_network_policy(),
        owner.fork_with_isolated_page_network_policy(),
    ] {
        assert!(!Arc::ptr_eq(
            &owner.pending_images,
            &isolated.pending_images
        ));
    }
    Ok(())
}

#[test]
fn image_sharing_key_keeps_request_modes_and_cache_bypasses_separate() -> Result<()> {
    let owner = ResourceRequestClient::new(&FetchConfig::default())?;
    let base = Request::get("https://example.test/image.png")?
        .with_resource_type(RequestResourceType::Image)
        .with_browser_request_metadata(BrowserRequestMetadata::Image)
        .with_request_mode(RequestMode::NoCors);
    let key = ImageFetchKey::new(&owner, &base).unwrap();
    assert!(
        ImageFetchKey::new(&owner, &base.clone().with_request_mode(RequestMode::Cors)).unwrap()
            != key
    );
    assert!(
        ImageFetchKey::new(
            &owner,
            &base
                .clone()
                .with_credentials_mode(RequestCredentialsMode::Omit)
        )
        .unwrap()
            != key
    );
    assert!(
        ImageFetchKey::new(
            &owner,
            &base.clone().with_resource_type(RequestResourceType::Raw)
        )
        .is_none()
    );
    assert!(
        ImageFetchKey::new(
            &owner,
            &base
                .clone()
                .with_browser_request_metadata(BrowserRequestMetadata::Xhr)
        )
        .is_none()
    );
    for mode in [
        RequestCacheMode::Bypass,
        RequestCacheMode::NoStore,
        RequestCacheMode::Validate,
    ] {
        assert!(ImageFetchKey::new(&owner, &base.clone().with_cache_mode(mode)).is_none());
    }
    let mut with_headers = base;
    with_headers
        .request_headers
        .push(("x-variant".into(), "different".into()));
    assert!(ImageFetchKey::new(&owner, &with_headers).is_none());
    Ok(())
}
