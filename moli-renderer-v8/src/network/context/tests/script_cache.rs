use super::*;
use anyhow::Result;
use moli_cookie_jar::new_shared_browser_cookie_store;
use moli_fetch::{Request, Response, ScriptFetchRequestMetadata};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::oneshot,
    time::{Duration, timeout},
};

use crate::network::{BrowserResourceRuntimeOwner, BrowserResourceRuntimeOwnerRoot};
use crate::network::{ResourceResponseHead, ResourceResponseObserver, ResourceResponseResult};

#[derive(Debug)]
enum ScriptProgress {
    Head(std::sync::Arc<ResourceResponseHead>),
    Data(usize),
}

struct ScriptProgressObserver(tokio::sync::mpsc::UnboundedSender<ScriptProgress>);

impl ResourceResponseObserver for ScriptProgressObserver {
    fn response_started(&self, response: std::sync::Arc<ResourceResponseHead>) {
        let _ = self.0.send(ScriptProgress::Head(response));
    }

    fn data_received(&self, bytes: &[u8]) {
        let _ = self.0.send(ScriptProgress::Data(bytes.len()));
    }

    fn cancelled(&self, _: &crate::network::ResourceResponseFailure) {}
}

fn start_observed_script(
    load: crate::network::loads::ResourceLoadLease,
    url: &str,
) -> Result<(
    tokio::sync::mpsc::UnboundedReceiver<ScriptProgress>,
    oneshot::Receiver<ResourceResponseResult>,
)> {
    let request = Request::get(url)?
        .with_request_origin(moli_url::WebOrigin::from_url(&url::Url::parse(url)?))
        .with_page_network_policy()
        .with_script_fetch_metadata(ScriptFetchRequestMetadata::default());
    let (progress_send, progress) = tokio::sync::mpsc::unbounded_channel();
    let (send, receive) = oneshot::channel();
    load.request_client()
        .fetch_cacheable_script_text_callback_with_load(
            request,
            load,
            Some(std::sync::Arc::new(ScriptProgressObserver(progress_send))),
            move |result| {
                let _ = send.send(result);
            },
        )?;
    Ok((progress, receive))
}

#[tokio::test]
async fn observed_shared_script_late_waiter_survives_owner_cancellation() -> Result<()> {
    check_observed_shared_script(false).await
}

#[tokio::test]
async fn observed_shared_script_partial_failure_retains_head_and_bytes() -> Result<()> {
    check_observed_shared_script(true).await
}

async fn check_observed_shared_script(partial_failure: bool) -> Result<()> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let url = format!("http://{}/observed.js", listener.local_addr()?);
    let owner = ResourceRequestClient::new(&FetchConfig::default())?;
    let document = document_loader((*owner).clone(), 1, &url);
    let first_load = document
        .register_load(
            ResourceLoadKind::Script,
            ResourceLoadDisposition::Ordinary,
            None,
        )
        .unwrap();
    let (mut first_progress, first) = start_observed_script(first_load.clone(), &url)?;
    let (mut stream, _) = timeout(Duration::from_secs(3), listener.accept()).await??;
    read_request(&mut stream).await?;
    stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/javascript\r\nCache-Control: max-age=60\r\nContent-Length: 4\r\nConnection: close\r\n\r\n").await?;
    let Some(ScriptProgress::Head(first_head)) =
        timeout(Duration::from_secs(3), first_progress.recv()).await?
    else {
        panic!("headers must precede body while the server holds every byte")
    };
    assert_eq!(first_head.head.final_url.as_str(), url);
    assert_eq!(first_head.head.status, 200);
    assert!(first_head.network_request_headers.is_some());
    stream.write_all(b"//").await?;
    assert!(matches!(
        timeout(Duration::from_secs(3), first_progress.recv()).await?,
        Some(ScriptProgress::Data(2))
    ));

    let second_load = document
        .register_load(
            ResourceLoadKind::Script,
            ResourceLoadDisposition::Ordinary,
            None,
        )
        .unwrap();
    let (mut second_progress, second) = start_observed_script(second_load, &url)?;
    let Some(ScriptProgress::Head(second_head)) =
        timeout(Duration::from_secs(3), second_progress.recv()).await?
    else {
        panic!("late admission must replay its existing physical response head")
    };
    assert!(std::sync::Arc::ptr_eq(&first_head, &second_head));
    assert!(matches!(
        timeout(Duration::from_secs(3), second_progress.recv()).await?,
        Some(ScriptProgress::Data(2))
    ));

    first_load.cancel();
    assert!(timeout(Duration::from_secs(3), first).await?.is_err());
    assert!(
        timeout(Duration::from_secs(3), first_progress.recv())
            .await?
            .is_none()
    );
    if partial_failure {
        drop(stream);
        let result = timeout(Duration::from_secs(3), second).await??;
        let crate::network::ResourceResponseFailure::PartialBody {
            response,
            body,
            message,
        } = result.expect_err("truncated transport must not become a successful cache entry")
        else {
            panic!("a failed stream must retain its physical response")
        };
        assert!(std::sync::Arc::ptr_eq(&first_head, &response));
        assert_eq!(body.clone_body_bytes(), b"//");
        assert!(!message.is_empty());
    } else {
        stream.write_all(b"ok").await?;
        assert!(matches!(
            timeout(Duration::from_secs(3), second_progress.recv()).await?,
            Some(ScriptProgress::Data(2))
        ));
        let response = timeout(Duration::from_secs(3), second).await???;
        assert_eq!(response.body_bytes(), b"//ok");
        assert!(!response.from_cache);
        let hit = start_script(
            document
                .register_load(
                    ResourceLoadKind::Script,
                    ResourceLoadDisposition::Ordinary,
                    None,
                )
                .unwrap(),
            &url,
        )?;
        let response = timeout(Duration::from_secs(3), hit).await???;
        assert!(response.from_cache);
        assert_eq!(response.body_bytes(), b"//ok");
    }
    assert!(
        timeout(Duration::from_secs(3), second_progress.recv())
            .await?
            .is_none()
    );
    Ok(())
}

#[tokio::test]
async fn observed_shared_script_last_consumer_cancels_held_body() -> Result<()> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let url = format!("http://{}/observed.js", listener.local_addr()?);
    let owner = ResourceRequestClient::new(&FetchConfig::default())?;
    let document = document_loader((*owner).clone(), 1, &url);
    let load = document
        .register_load(
            ResourceLoadKind::Script,
            ResourceLoadDisposition::Ordinary,
            None,
        )
        .unwrap();
    let (mut progress, completed) = start_observed_script(load.clone(), &url)?;
    let (mut stream, _) = timeout(Duration::from_secs(3), listener.accept()).await??;
    read_request(&mut stream).await?;
    stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/javascript\r\nContent-Length: 4\r\nConnection: close\r\n\r\n").await?;
    assert!(matches!(
        timeout(Duration::from_secs(3), progress.recv()).await?,
        Some(ScriptProgress::Head(_))
    ));
    load.cancel();
    assert!(timeout(Duration::from_secs(3), completed).await?.is_err());
    assert!(
        timeout(Duration::from_secs(3), progress.recv())
            .await?
            .is_none()
    );
    assert_eq!(
        timeout(Duration::from_secs(3), stream.read(&mut [0])).await??,
        0
    );
    Ok(())
}

#[derive(Clone, Copy)]
enum ContextChange {
    TransportOnly,
    Document,
    Worker,
}

async fn read_request(stream: &mut TcpStream) -> Result<String> {
    let mut bytes = Vec::new();
    while !bytes.ends_with(b"\r\n\r\n") {
        bytes.push(stream.read_u8().await?);
    }
    Ok(String::from_utf8(bytes)?)
}

async fn respond(stream: &mut TcpStream, body: &str) -> Result<()> {
    stream
        .write_all(
            format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/javascript\r\nCache-Control: max-age=60\r\nVary: User-Agent\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len(),
            )
            .as_bytes(),
        )
        .await?;
    Ok(())
}

// Registering a callback completes cache admission synchronously. The server
// gates response completion separately, so no sleep determines which request
// is still loading when the transport or context changes.
fn start_script(
    load: crate::network::loads::ResourceLoadLease,
    url: &str,
) -> Result<oneshot::Receiver<Result<Response>>> {
    let request = Request::get(url)?
        .with_request_origin(moli_url::WebOrigin::from_url(&url::Url::parse(url)?))
        .with_page_network_policy()
        .with_script_fetch_metadata(ScriptFetchRequestMetadata::default());
    let (send, receive) = oneshot::channel();
    load.request_client()
        .fetch_cacheable_script_text_callback_with_load(request, load, None, move |result| {
            let _ = send.send(result.map_err(anyhow::Error::new));
        })?;
    Ok(receive)
}

#[tokio::test]
async fn streaming_script_owner_cancellation_preserves_shared_callback() -> Result<()> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let url = format!("http://{}/script.js", listener.local_addr()?);
    let owner = ResourceRequestClient::new(&FetchConfig::default())?;
    let document = document_loader((*owner).clone(), 1, &url);
    let request = Request::get(&url)?
        .with_request_origin(moli_url::WebOrigin::from_url(&url::Url::parse(&url)?))
        .with_page_network_policy()
        .with_script_fetch_metadata(ScriptFetchRequestMetadata::default());
    let mut first = Box::pin(document.fetch_script_for_test(request));
    let (mut stream, _) = tokio::select! {
        biased;
        result = &mut first => panic!("the fixture still owns the response: {result:?}"),
        accepted = listener.accept() => accepted?,
    };
    read_request(&mut stream).await?;
    // Synchronous callback admission proves a second consumer exists before
    // the initiating async consumer is canceled.
    let second = start_script(
        document
            .register_load(
                ResourceLoadKind::Script,
                ResourceLoadDisposition::Ordinary,
                None,
            )
            .unwrap(),
        &url,
    )?;
    drop(first);
    respond(&mut stream, "surviving-consumer").await?;
    let result = timeout(Duration::from_secs(3), second).await???;
    assert_eq!(result.body_text(), "surviving-consumer");
    Ok(())
}

#[tokio::test]
async fn streaming_script_last_consumer_cancels_held_transport() -> Result<()> {
    assert_streaming_script_last_consumer_cancels_held_transport(true).await
}

#[tokio::test]
async fn streaming_uncached_script_last_consumer_cancels_held_transport() -> Result<()> {
    assert_streaming_script_last_consumer_cancels_held_transport(false).await
}

async fn assert_streaming_script_last_consumer_cancels_held_transport(
    cacheable: bool,
) -> Result<()> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let url = format!("http://{}/script.js", listener.local_addr()?);
    let owner = ResourceRequestClient::new(&FetchConfig::default())?;
    let document = document_loader((*owner).clone(), 1, &url);
    let mut request = Request::get(&url)?
        .with_request_origin(moli_url::WebOrigin::from_url(&url::Url::parse(&url)?))
        .with_page_network_policy();
    if cacheable {
        request = request.with_script_fetch_metadata(ScriptFetchRequestMetadata::default());
    }
    let mut first = Box::pin(document.fetch_script_for_test(request));
    let (mut stream, _) = tokio::select! {
        biased;
        result = &mut first => panic!("the fixture still owns the response: {result:?}"),
        accepted = listener.accept() => accepted?,
    };
    read_request(&mut stream).await?;
    drop(first);
    let mut byte = [0];
    assert_eq!(
        timeout(Duration::from_secs(3), stream.read(&mut byte)).await??,
        0
    );
    Ok(())
}

#[tokio::test]
async fn native_script_future_drop_cancels_held_transport() -> Result<()> {
    native_resource_future_drop(crate::types::SubresourceResourceType::Script, false, false).await
}

#[tokio::test]
async fn native_stylesheet_future_drop_cancels_held_transport() -> Result<()> {
    native_resource_future_drop(
        crate::types::SubresourceResourceType::Stylesheet,
        false,
        false,
    )
    .await
}

#[tokio::test]
async fn native_script_future_drop_preserves_other_cache_consumer() -> Result<()> {
    native_resource_future_drop(crate::types::SubresourceResourceType::Script, true, false).await
}

#[tokio::test]
async fn native_script_fallback_drop_preserves_other_cache_consumer() -> Result<()> {
    native_resource_future_drop(crate::types::SubresourceResourceType::Script, true, true).await
}

async fn native_resource_future_drop(
    resource_type: crate::types::SubresourceResourceType,
    surviving_consumer: bool,
    service_worker_fallback: bool,
) -> Result<()> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let url = format!("http://{}/script.js", listener.local_addr()?);
    let owner = ResourceRequestClient::new(&FetchConfig::default())?;
    let mut document = document_loader(owner.handle(), 1, &url);
    if service_worker_fallback {
        document.service_worker = Some(std::sync::Arc::new(|_, _, _, _| {
            Some(Box::pin(async { Ok(None) }))
        }));
    }
    let request = Request::get(&url)?
        .with_page_network_policy()
        .with_script_fetch_metadata(ScriptFetchRequestMetadata::default());
    let mut first = Box::pin(document.fetch_resource(
        request,
        resource_type,
        crate::types::SubresourceRequestInitiatorType::Parser,
    ));
    let (mut stream, _) = timeout(Duration::from_secs(3), async {
        tokio::select! {
            biased;
            result = &mut first => panic!("fixture still owns the response: {result:?}"),
            accepted = listener.accept() => accepted,
        }
    })
    .await??;
    read_request(&mut stream).await?;
    let second = surviving_consumer.then(|| {
        start_script(
            document
                .register_load(
                    ResourceLoadKind::Script,
                    ResourceLoadDisposition::Ordinary,
                    None,
                )
                .unwrap(),
            &url,
        )
        .unwrap()
    });
    drop(first);
    if let Some(second) = second {
        respond(&mut stream, "surviving-consumer").await?;
        let response = timeout(Duration::from_secs(3), second).await???;
        assert_eq!(response.body_text(), "surviving-consumer");
        assert_eq!(document.load_diagnostics().active_ordinary_load_count, 0);
    } else {
        let mut byte = [0];
        let closed = timeout(Duration::from_secs(3), stream.read(&mut byte)).await;
        document.begin_detach();
        assert_eq!(
            closed??, 0,
            "dropping the caller must release its original transport"
        );
    }
    Ok(())
}

async fn check_loading_context(change: ContextChange) -> Result<()> {
    let foreign_context = !matches!(change, ContextChange::TransportOnly);
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let url = format!("http://{}/script.js", listener.local_addr()?);
    let (received_send, received) = oneshot::channel();
    let (release, released) = oneshot::channel();
    let server = tokio::spawn(async move {
        let (mut first, _) = listener.accept().await?;
        assert!(read_request(&mut first).await?.contains("FirstAgent"));
        let _ = received_send.send(());
        if foreign_context {
            let (mut second, _) = listener.accept().await?;
            assert!(read_request(&mut second).await?.contains("SecondAgent"));
            respond(&mut second, "second").await?;
        }
        let _ = released.await;
        respond(&mut first, "first").await
    });

    let cookies = new_shared_browser_cookie_store();
    let mut config = FetchConfig::default();
    config.set_user_agent("FirstAgent");
    let (root, binding) = BrowserResourceRuntimeOwnerRoot::new(BrowserResourceRuntimeOwner::new(
        &config,
        cookies.clone(),
    ));
    let original = document_loader(
        ResourceRequestClient::from_browser_resource_runtime(binding.current()),
        1,
        &url,
    );
    let first = start_script(
        original
            .register_load(
                ResourceLoadKind::Script,
                ResourceLoadDisposition::Ordinary,
                None,
            )
            .unwrap(),
        &url,
    )?;
    timeout(Duration::from_secs(3), received).await??;

    config.set_user_agent("SecondAgent");
    let transport = root
        .registrar()
        .replace_owned(BrowserResourceRuntimeOwner::new(&config, cookies))
        .unwrap();
    let current = original.with_replacement_transport(
        ResourceRequestClient::from_browser_resource_runtime(transport),
    );
    let document = match change {
        ContextChange::Document => current.fork_for_document(context(2, &url)),
        _ => current,
    };
    let worker = matches!(change, ContextChange::Worker).then(|| {
        WorkerResourceLoader::new(
            document.request_client().clone(),
            WorkerResourceOwner::Dedicated {
                name: "cache".into(),
            },
            resource_task_runner(),
        )
    });
    let load = if let Some(worker) = &worker {
        worker.register_load(
            ResourceLoadKind::Script,
            ResourceLoadDisposition::Ordinary,
            None,
        )
    } else {
        document.register_load(
            ResourceLoadKind::Script,
            ResourceLoadDisposition::Ordinary,
            None,
        )
    }
    .unwrap();
    let second = start_script(load, &url)?;
    if foreign_context {
        let second = timeout(Duration::from_secs(3), second)
            .await
            .expect("a new loading context must not wait on the outgoing context's request")??;
        assert_eq!(second.body_text(), "second");
        assert!(!second.from_cache);
        release.send(()).unwrap();
    } else {
        release.send(()).unwrap();
        let second = timeout(Duration::from_secs(3), second).await???;
        assert_eq!(second.body_text(), "first");
        assert!(!second.from_cache);
    }
    assert_eq!(
        timeout(Duration::from_secs(3), first).await???.body_text(),
        "first"
    );
    server.await??;

    if foreign_context {
        // A late completion from the old context must neither overwrite the
        // winning response nor prevent the streaming consumer from reusing it.
        let response = document
            .fetch_script_for_test(
                Request::get(&url)?
                    .with_request_origin(moli_url::WebOrigin::from_url(&url::Url::parse(&url)?))
                    .with_page_network_policy()
                    .with_script_fetch_metadata(ScriptFetchRequestMetadata::default()),
            )
            .await?;
        assert_eq!(response.body_text(), "second");
        assert!(response.from_cache);
    }
    Ok(())
}

#[tokio::test]
async fn same_document_coalesces_loading_script_across_transport_replacement() -> Result<()> {
    check_loading_context(ContextChange::TransportOnly).await
}

#[tokio::test]
async fn new_document_does_not_join_loading_script_from_old_document() -> Result<()> {
    check_loading_context(ContextChange::Document).await
}

#[tokio::test]
async fn worker_does_not_join_loading_script_from_creator_document() -> Result<()> {
    check_loading_context(ContextChange::Worker).await
}
