use super::*;
use crate::page_task_queue::RendererResourceCompletionTestHarness;
use anyhow::Result;
use moli_fetch::FetchConfig;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    task::JoinHandle,
};
use url::Url;

#[derive(Clone, Copy)]
enum Reply {
    Allowed,
    Denied,
    Disconnect,
}

async fn server(exchanges: Vec<(&'static str, Reply)>) -> Result<(String, JoinHandle<Result<()>>)> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let url = format!("http://{}/resource", listener.local_addr()?);
    let task = tokio::spawn(async move {
        for (method, reply) in exchanges {
            let (mut stream, _) =
                tokio::time::timeout(std::time::Duration::from_secs(5), listener.accept())
                    .await??;
            let mut bytes = Vec::new();
            loop {
                let mut buffer = [0; 1024];
                let count = stream.read(&mut buffer).await?;
                anyhow::ensure!(count != 0, "incomplete request headers");
                bytes.extend_from_slice(&buffer[..count]);
                if bytes.windows(4).any(|window| window == b"\r\n\r\n") {
                    break;
                }
            }
            let wire = String::from_utf8(bytes)?;
            assert!(
                wire.starts_with(&format!("{method} /resource HTTP/1.1\r\n")),
                "{wire}"
            );
            if matches!(reply, Reply::Disconnect) {
                continue;
            }
            let allow = if matches!(reply, Reply::Allowed) {
                concat!(
                    "Access-Control-Allow-Origin: http://origin.test\r\n",
                    "Access-Control-Allow-Credentials: true\r\n",
                    "Access-Control-Allow-Methods: PUT, DELETE\r\n",
                    "Access-Control-Allow-Headers: x-test, x-extra\r\n",
                    "Access-Control-Max-Age: 600\r\n",
                )
            } else {
                ""
            };
            stream.write_all(format!(
                "HTTP/1.1 200 OK\r\n{allow}Cache-Control: no-store\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok"
            ).as_bytes()).await?;
        }
        Ok(())
    });
    Ok((url, task))
}

fn request(url: &str) -> Request {
    Request::new_browser_bytes(
        "PUT",
        url,
        None,
        vec![("X-Test".to_owned(), "1".to_owned())],
        moli_url::WebOrigin::from_url(&Url::parse("http://origin.test/").unwrap()),
    )
    .unwrap()
    .with_network_partition_key(Some("partition-a".to_owned()))
    .with_credentials_mode(RequestCredentialsMode::Omit)
    .with_browser_request_metadata(BrowserRequestMetadata::Fetch)
}

fn client() -> Result<crate::network::ResourceRequestClientOwner> {
    let mut config = FetchConfig::default();
    config.set_http_no_proxy(Some("*".to_owned()));
    ResourceRequestClient::new(&config)
}

async fn fetch(
    loader: &ResourceRequestClient,
    request: Request,
    streaming: bool,
    observer: Option<&CorsPreflightNetworkObserver>,
) -> Result<(), String> {
    let headers = request.request_headers.to_byte_strings();
    let (head, body) = if streaming {
        fetch_browser_subresource_raw_stream_with_preflight_headers_and_observer(
            loader, request, None, headers, observer, None,
        )
        .await?
        .into_response()
        .into_body()
    } else {
        fetch_browser_subresource_with_preflight_headers_and_observer(
            loader.clone(),
            request,
            None,
            headers,
            observer,
            None,
        )
        .await?
        .into_response()
        .into_body()
    };
    assert_eq!(head.status, 200);
    assert_eq!(
        body.into_materialized_bytes()
            .await
            .map_err(format_network_error)?,
        b"ok"
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn preflight_cache_is_shared_by_buffered_streaming_fetch_and_xhr_without_fake_events()
-> Result<()> {
    let (url, server) = server(vec![
        ("OPTIONS", Reply::Allowed),
        ("PUT", Reply::Allowed),
        ("PUT", Reply::Allowed),
        ("DELETE", Reply::Allowed),
    ])
    .await?;
    let owner = client()?;
    let loader = owner.handle();
    // A later execution context obtains a fresh request client and policy,
    // while sharing the browser-context grant owner.
    let other =
        ResourceRequestClient::from_browser_resource_runtime(loader.browser_resource_runtime());
    let mut queue = RendererResourceCompletionTestHarness::new();
    let observer = CorsPreflightNetworkObserver::new(
        queue.sender(),
        AsyncSubresourceNetworkContext {
            frame_id: Some("frame".to_owned()),
            request_origin: request(&url).browser_origin()?.clone(),
            document_url: Url::parse("http://origin.test/")?,
            resource_type: SubresourceResourceType::Fetch,
            policy_context: Default::default(),
        },
    );
    for (index, (metadata, streaming, method)) in [
        (BrowserRequestMetadata::Fetch, false, "PUT"),
        (BrowserRequestMetadata::Xhr, true, "PUT"),
        (BrowserRequestMetadata::Fetch, true, "DELETE"),
    ]
    .into_iter()
    .enumerate()
    {
        let mut next = request(&url).with_browser_request_metadata(metadata);
        next.method = method.to_owned();
        if index == 1 {
            next.request_headers = vec![
                ("x-TEST".to_owned(), "different".to_owned()),
                ("x-extra".to_owned(), "allowed in first response".to_owned()),
            ]
            .into();
        }
        fetch(
            if index == 0 { &loader } else { &other },
            next,
            streaming,
            Some(&observer),
        )
        .await
        .map_err(anyhow::Error::msg)?;
        if index == 0 {
            let Some(AsyncSubresourceFetchEvent::ObservedNetworkRecord(record)) =
                queue.pop_next_async_subresource_event()
            else {
                anyhow::bail!("missing real OPTIONS record");
            };
            assert_eq!(record.method(), "OPTIONS");
        }
        assert!(
            queue.pop_next_async_subresource_event().is_none(),
            "a cache hit must not invent an OPTIONS exchange"
        );
    }
    server.await??;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn preflight_cache_rechecks_credentials_partitions_page_policy_and_browser_owner()
-> Result<()> {
    let (url, server) = server(vec![
        ("OPTIONS", Reply::Allowed),
        ("PUT", Reply::Allowed), // omit grant
        ("OPTIONS", Reply::Allowed),
        ("PUT", Reply::Allowed), // include needs its own grant
        ("PUT", Reply::Allowed), // credentialed grant can serve omit
        ("OPTIONS", Reply::Allowed),
        ("PUT", Reply::Allowed), // other partition
        ("OPTIONS", Reply::Allowed),
        ("PUT", Reply::Allowed), // disabled
        ("OPTIONS", Reply::Allowed),
        ("PUT", Reply::Allowed), // disabled again
        ("PUT", Reply::Allowed), // re-enabled
        ("OPTIONS", Reply::Allowed),
        ("PUT", Reply::Allowed), // explicit browser-cache clear
        ("OPTIONS", Reply::Allowed),
        ("PUT", Reply::Allowed), // different browser owner
    ])
    .await?;
    let owner = client()?;
    let loader = owner.handle();
    let first = request(&url);
    fetch(&loader, first.clone(), false, None)
        .await
        .map_err(anyhow::Error::msg)?;
    fetch(
        &loader,
        first
            .clone()
            .with_credentials_mode(RequestCredentialsMode::Include),
        true,
        None,
    )
    .await
    .map_err(anyhow::Error::msg)?;
    fetch(&loader, first, true, None)
        .await
        .map_err(anyhow::Error::msg)?;
    let partitioned = request(&url).with_network_partition_key(Some("partition-b".to_owned()));
    fetch(&loader, partitioned.clone(), false, None)
        .await
        .map_err(anyhow::Error::msg)?;
    loader.set_cache_disabled(true);
    for streaming in [false, true] {
        fetch(&loader, partitioned.clone(), streaming, None)
            .await
            .map_err(anyhow::Error::msg)?;
    }
    loader.set_cache_disabled(false);
    fetch(&loader, partitioned.clone(), false, None)
        .await
        .map_err(anyhow::Error::msg)?;
    loader
        .browser_resource_runtime()
        .clear_cors_preflight_cache(None);
    fetch(&loader, partitioned.clone(), true, None)
        .await
        .map_err(anyhow::Error::msg)?;
    let different_owner = client()?;
    fetch(&different_owner.handle(), partitioned, false, None)
        .await
        .map_err(anyhow::Error::msg)?;
    server.await??;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn cache_hits_still_check_actual_cors_and_failures_retire_the_old_grant() -> Result<()> {
    let (url, server) = server(vec![
        ("OPTIONS", Reply::Allowed),
        ("PUT", Reply::Allowed),
        ("PUT", Reply::Denied), // cached permission cannot bypass CORS response check
        ("OPTIONS", Reply::Allowed),
        ("PUT", Reply::Allowed),
        ("PUT", Reply::Disconnect), // failed actual transport also retires the grant
        ("OPTIONS", Reply::Allowed),
        ("PUT", Reply::Allowed),
        ("OPTIONS", Reply::Allowed), // insufficient allow-headers for replacement
        ("OPTIONS", Reply::Allowed),
        ("PUT", Reply::Allowed),
    ])
    .await?;
    let owner = client()?;
    let loader = owner.handle();
    let original = request(&url);
    fetch(&loader, original.clone(), false, None)
        .await
        .map_err(anyhow::Error::msg)?;
    let error = fetch(&loader, original.clone(), true, None)
        .await
        .unwrap_err();
    assert!(error.contains("CORS check failed:"), "{error}");
    fetch(&loader, original.clone(), false, None)
        .await
        .map_err(anyhow::Error::msg)?;
    assert!(fetch(&loader, original.clone(), false, None).await.is_err());
    fetch(&loader, original.clone(), true, None)
        .await
        .map_err(anyhow::Error::msg)?;
    let mut unsupported = original.clone();
    unsupported.request_headers = vec![("x-not-granted".to_owned(), "1".to_owned())].into();
    let error = fetch(&loader, unsupported, false, None).await.unwrap_err();
    assert!(error.contains("does not allow x-not-granted"), "{error}");
    fetch(&loader, original, true, None)
        .await
        .map_err(anyhow::Error::msg)?;
    server.await??;
    Ok(())
}
