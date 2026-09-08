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
        .with_page_network_policy()
        .with_script_fetch_metadata(ScriptFetchRequestMetadata::default());
    let (send, receive) = oneshot::channel();
    load.request_client()
        .fetch_cacheable_script_text_callback_with_load(request, load, move |result| {
            let _ = send.send(result);
        })?;
    Ok(receive)
}

#[tokio::test]
async fn streaming_script_owner_cancellation_preserves_shared_callback() -> Result<()> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let url = format!("http://{}/script.js", listener.local_addr()?);
    let owner = ResourceRequestClient::new(&FetchConfig::default())?;
    let document = document_loader((*owner).clone(), 1, &url);
    let client = document.request_client();
    let request = Request::get(&url)?
        .with_page_network_policy()
        .with_script_fetch_metadata(ScriptFetchRequestMetadata::default());
    let mut first =
        Box::pin(client.fetch_cacheable_script_text_stream(request, resource_task_runner()));
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
    let client = document.request_client();
    let mut request = Request::get(&url)?.with_page_network_policy();
    if cacheable {
        request = request.with_script_fetch_metadata(ScriptFetchRequestMetadata::default());
    }
    let mut first =
        Box::pin(client.fetch_cacheable_script_text_stream(request, resource_task_runner()));
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
    let client = load.request_client();
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
        let response = client
            .fetch_cacheable_script_text_stream(
                Request::get(&url)?
                    .with_page_network_policy()
                    .with_script_fetch_metadata(ScriptFetchRequestMetadata::default()),
                resource_task_runner(),
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
