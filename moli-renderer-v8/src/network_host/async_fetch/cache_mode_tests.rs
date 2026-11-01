use super::*;
use anyhow::Result;
use moli_fetch::{FetchConfig, RawResponse, RequestCacheMode, ResponseCacheState};
use std::time::Duration;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    task::JoinHandle,
    time::timeout,
};

fn reply(status: u16, headers: &[(&str, &str)], body: &[u8]) -> Vec<u8> {
    let mut response = format!(
        "HTTP/1.1 {status} Test\r\nContent-Length: {}\r\nConnection: close\r\n",
        body.len()
    )
    .into_bytes();
    for (name, value) in headers {
        response.extend_from_slice(format!("{name}: {value}\r\n").as_bytes());
    }
    response.extend_from_slice(b"\r\n");
    response.extend_from_slice(body);
    response
}

type Server = JoinHandle<Result<Vec<String>>>;

async fn server(responses: Vec<Vec<u8>>) -> Result<(url::Url, Server)> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let url = url::Url::parse(&format!("http://{}/start", listener.local_addr()?))?;
    let task = tokio::spawn(async move {
        let mut requests = Vec::new();
        for response in responses {
            let (mut stream, _) = timeout(Duration::from_secs(5), listener.accept()).await??;
            let mut head = Vec::new();
            while !head.ends_with(b"\r\n\r\n") {
                anyhow::ensure!(head.len() < 8192, "request head is too large");
                head.push(timeout(Duration::from_secs(5), stream.read_u8()).await??);
            }
            requests.push(String::from_utf8(head)?);
            stream.write_all(&response).await?;
        }
        Ok(requests)
    });
    Ok((url, task))
}

fn client() -> Result<crate::network::ResourceRequestClientOwner> {
    let mut config = FetchConfig::default();
    config.set_http_no_proxy(Some("*".to_owned()));
    crate::network::ResourceRequestClient::new(&config)
}

fn request(url: &url::Url, mode: RequestMode) -> Request {
    Request::new_browser("GET", url.clone(), None, Vec::new(), url.into())
        .with_initiator_url(url)
        .with_request_mode(mode)
        .with_credentials_mode(RequestCredentialsMode::SameOrigin)
        .with_browser_request_metadata(BrowserRequestMetadata::Fetch)
}

async fn load(
    loader: &ResourceRequestClient,
    request: Request,
) -> Result<(RawResponse, NetworkObservationJournal)> {
    let observed = fetch_browser_subresource_raw_stream_with_cors_redirect_checks(
        loader,
        request,
        None,
        Vec::new(),
        None,
        None,
    )
    .await
    .map_err(anyhow::Error::msg)?;
    let (response, journal) = observed.into_parts_with_observation_journal();
    Ok((response.into_materialized_raw_response().await?, journal))
}

#[tokio::test]
async fn cache_modes_reuse_stale_same_origin_hops_without_network() -> Result<()> {
    for control in ["max-age=0, must-revalidate", "no-cache"] {
        let (url, server) = server(vec![reply(
            200,
            &[("Cache-Control", control), ("ETag", "\"version\"")],
            b"stored",
        )])
        .await?;
        let owner = client()?;
        let loader = owner.handle();
        let seed = request(&url, RequestMode::Cors);
        assert_eq!(load(&loader, seed.clone()).await?.0.body_bytes(), b"stored");
        // The listening socket is gone before these reads: revalidation or a
        // cache miss cannot accidentally pass by receiving another response.
        assert_eq!(server.await??.len(), 1);
        for (mode, cache) in [
            (RequestMode::Cors, RequestCacheMode::ForceCache),
            (RequestMode::SameOrigin, RequestCacheMode::ForceCache),
            (RequestMode::SameOrigin, RequestCacheMode::OnlyIfCached),
        ] {
            let (hit, journal) = load(
                &loader,
                seed.clone().with_request_mode(mode).with_cache_mode(cache),
            )
            .await?;
            assert_eq!(hit.body_bytes(), b"stored");
            assert_eq!(hit.cache_state, ResponseCacheState::Local);
            assert!(journal.is_empty());
        }
    }
    Ok(())
}

#[tokio::test]
async fn cache_modes_keep_vary_credentials_origin_and_partition_boundaries() -> Result<()> {
    let (url, server) = server(vec![reply(
        200,
        &[
            ("Cache-Control", "max-age=600"),
            ("Vary", "X-Representation"),
        ],
        b"first representation",
    )])
    .await?;
    let owner = client()?;
    let loader = owner.handle();
    let mut seed = request(&url, RequestMode::Cors);
    seed.request_headers = vec![("X-Representation".to_owned(), "first".to_owned())].into();
    load(&loader, seed.clone()).await?;
    assert_eq!(server.await??.len(), 1);
    let cache_only = seed
        .with_request_mode(RequestMode::SameOrigin)
        .with_cache_mode(RequestCacheMode::OnlyIfCached);
    let mut varied = cache_only.clone();
    varied.request_headers = vec![("X-Representation".to_owned(), "second".to_owned())].into();
    for isolated in [
        varied,
        cache_only
            .clone()
            .with_network_partition_key(Some("peer".to_owned())),
        cache_only
            .clone()
            .with_credentials_mode(RequestCredentialsMode::Omit),
    ] {
        let error = load(&loader, isolated).await.unwrap_err();
        assert!(
            error.to_string().contains("net::ERR_CACHE_MISS"),
            "{error:#}"
        );
    }
    let error = load(
        &loader,
        cache_only
            .clone()
            .with_request_origin(moli_url::WebOrigin::Opaque),
    )
    .await
    .unwrap_err();
    assert!(error.to_string().contains("same-origin"), "{error:#}");
    let (retained, journal) = load(&loader, cache_only).await?;
    assert_eq!(retained.body_bytes(), b"first representation");
    assert_eq!(retained.cache_state, ResponseCacheState::Local);
    assert!(journal.is_empty());
    Ok(())
}

#[tokio::test]
async fn cache_modes_cached_same_origin_redirects_rebuild_the_consumer_url_list() -> Result<()> {
    for control in ["max-age=600", "max-age=0, must-revalidate"] {
        let (url, server) = server(vec![
            reply(
                302,
                &[
                    ("Cache-Control", control),
                    ("ETag", "\"redirect\""),
                    ("Location", "/final"),
                ],
                b"",
            ),
            reply(
                200,
                &[("Cache-Control", control), ("ETag", "\"body\"")],
                b"final body",
            ),
        ])
        .await?;
        let owner = client()?;
        let loader = owner.handle();
        let (seed, _) = load(&loader, request(&url, RequestMode::Cors)).await?;
        assert!(seed.redirected);
        assert_eq!(seed.redirect_chain.len(), 1);
        assert_eq!(server.await??.len(), 2);
        let (hit, journal) = load(
            &loader,
            request(&url, RequestMode::SameOrigin).with_cache_mode(RequestCacheMode::OnlyIfCached),
        )
        .await?;
        assert_eq!(hit.body_bytes(), b"final body");
        assert_eq!(hit.final_url.path(), "/final");
        assert!(hit.redirected);
        assert_eq!(hit.redirect_chain.len(), 1);
        assert_eq!(hit.cache_state, ResponseCacheState::Local);
        assert!(journal.is_empty());
    }
    Ok(())
}

#[tokio::test]
async fn cache_modes_cached_redirects_cannot_bypass_same_origin_policy() -> Result<()> {
    let (target, target_server) = server(vec![reply(
        200,
        &[
            ("Cache-Control", "max-age=600"),
            ("Access-Control-Allow-Origin", "*"),
        ],
        b"foreign body",
    )])
    .await?;
    let (source, source_server) = server(vec![reply(
        302,
        &[
            ("Cache-Control", "max-age=600"),
            ("Location", target.as_str()),
        ],
        b"",
    )])
    .await?;
    let owner = client()?;
    let loader = owner.handle();
    let (seed, _) = load(&loader, request(&source, RequestMode::Cors)).await?;
    assert_eq!(seed.body_bytes(), b"foreign body");
    assert_eq!(source_server.await??.len(), 1);
    assert_eq!(target_server.await??.len(), 1);
    let error = load(
        &loader,
        request(&source, RequestMode::SameOrigin).with_cache_mode(RequestCacheMode::OnlyIfCached),
    )
    .await
    .unwrap_err();
    // The redirect itself must have hit the cache; the target is rejected by
    // the current consumer's policy before any cache lookup or network access.
    assert!(error.to_string().contains("same-origin"), "{error:#}");
    assert!(!error.to_string().contains("ERR_CACHE_MISS"), "{error:#}");
    Ok(())
}
