use anyhow::Result;
use std::{
    path::PathBuf,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::oneshot,
    time::{Duration, sleep, timeout},
};
use url::Url;

use moli_cookie_jar::{
    BrowserCookieFacadeContext, NetworkCookieRequestContext, new_shared_browser_cookie_store,
};
use moli_fetch::{
    BrowserRequestMetadata, FetchCancelHandle, RawResponse, RedirectInfo, RequestCredentialsMode,
    RequestResourceType, ResponseHead, ScriptFetchRequestMetadata, StreamingRawResponse,
    cookie_header_for_request,
};

use super::{
    FetchConfig, Request, ResourceRequestClient, raw_subresource_memory_cache_key,
    streaming_raw_response_from_cached_subresource,
};
use crate::network::{BrowserResourceRuntimeOwner, BrowserResourceRuntimeOwnerRoot};
use crate::protocol_types::OptionalResourceFetchMask;
use crate::types::SubresourceResourceType;

#[test]
fn loader_clones_share_one_browser_resource_runtime() {
    let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
    let clone = loader.clone();

    assert!(loader.shares_resource_runtime_with(&clone));
    assert!(loader.shares_page_network_policy_with(&clone));
    assert_eq!(
        loader.resource_runtime_diagnostics().runtime_id,
        clone.resource_runtime_diagnostics().runtime_id
    );
}

#[test]
fn page_loader_fork_shares_backend_but_isolates_mutable_policy() {
    let parent_owner = ResourceRequestClient::new(&FetchConfig::default()).expect("parent loader");
    let parent = parent_owner.handle().with_browser_site_context(
        BrowserCookieFacadeContext::default()
            .with_site_for_cookies_url(&Url::parse("https://parent.test/").unwrap())
            .with_top_frame_origin_url(&Url::parse("https://parent.test/").unwrap()),
    );
    parent.set_extra_http_headers(&[("x-page".to_owned(), "parent".to_owned())]);
    let page = parent.fork_with_isolated_page_network_policy();

    assert!(parent.shares_resource_runtime_with(&page));
    assert!(!parent.shares_page_network_policy_with(&page));
    assert_ne!(
        parent.page_network_policy().memory_cache_partition_id(),
        page.page_network_policy().memory_cache_partition_id(),
        "a new Page target must own a distinct renderer memory-cache partition"
    );
    assert!(page.browser_site_context().is_none());

    page.set_network_offline(true);
    page.set_extra_http_headers(&[("x-page".to_owned(), "child".to_owned())]);

    assert!(!parent.page_network_policy().snapshot().network_offline());
    assert!(page.page_network_policy().snapshot().network_offline());
}

#[test]
fn document_loader_fork_keeps_page_memory_cache_ownership() {
    let owner = ResourceRequestClient::new(&FetchConfig::default()).expect("Page loader");
    let document = owner.fork_with_isolated_document_network_policy();

    assert!(owner.shares_resource_runtime_with(&document));
    assert!(!owner.shares_page_network_policy_with(&document));
    assert_eq!(
        owner.page_network_policy().memory_cache_partition_id(),
        document.page_network_policy().memory_cache_partition_id(),
        "replacement Documents must retain their Page's memory-cache partition"
    );
}

#[test]
fn worker_loader_fork_preserves_creator_browser_site_context() {
    let context = BrowserCookieFacadeContext::default()
        .with_site_for_cookies_url(&Url::parse("https://top.test/").unwrap())
        .with_top_frame_origin_url(&Url::parse("https://top.test/").unwrap());
    let parent_owner = ResourceRequestClient::new(&FetchConfig::default()).expect("parent loader");
    let parent = parent_owner
        .handle()
        .with_browser_site_context(context.clone());

    let worker = parent.fork_with_isolated_worker_network_policy();

    assert!(parent.shares_resource_runtime_with(&worker));
    assert!(!parent.shares_page_network_policy_with(&worker));
    assert!(parent.shares_memory_cache_partition_with(&worker));
    assert_eq!(worker.browser_site_context(), Some(&context));
}

#[test]
fn independently_created_loaders_use_isolated_browser_resource_runtimes() {
    let left = ResourceRequestClient::new(&FetchConfig::default()).expect("left loader");
    let right = ResourceRequestClient::new(&FetchConfig::default()).expect("right loader");

    assert!(!left.shares_resource_runtime_with(&right));
    assert_ne!(
        left.resource_runtime_diagnostics().runtime_id,
        right.resource_runtime_diagnostics().runtime_id
    );
}

fn unique_test_cache_dir() -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "moli-renderer-v8-cache-test-{}-{unique}",
        std::process::id()
    ))
}

#[tokio::test(flavor = "multi_thread")]
async fn memory_cache_tee_drop_after_body_eof_cancels_pending_completion_and_releases_exact_runtime()
-> Result<()> {
    let registration = BrowserResourceRuntimeOwner::new(
        &FetchConfig::default(),
        new_shared_browser_cookie_store(),
    );
    let (root, binding) = BrowserResourceRuntimeOwnerRoot::new(registration);
    let registrar = root.registrar();
    let old_runtime = binding.current();
    let client = ResourceRequestClient::from_browser_resource_runtime(old_runtime.clone());
    let request = Request::get("https://cache.test/last-chunk.css")?
        .with_resource_type(RequestResourceType::CssStyleSheet);
    let cache_key = raw_subresource_memory_cache_key(&request)
        .expect("stylesheet request should use the raw subresource memory cache");

    let (body_tx, body_rx) = tokio::sync::mpsc::unbounded_channel();
    body_tx
        .send(b"last".to_vec())
        .expect("fake upstream body receiver should be live");
    drop(body_tx);
    let cancel_handle = FetchCancelHandle::new();
    let (mut completion_tx, completion_rx) = oneshot::channel();
    let inner = StreamingRawResponse::new_with_head(
        ResponseHead {
            final_url: request.url.clone(),
            status: 200,
            headers: vec![("cache-control".to_owned(), "max-age=60".to_owned())],
            request_cookie_report: None,
            cookie_set_reports: Vec::new(),
            redirected: false,
            redirect_chain: Vec::new(),
            from_cache: false,
            negotiated_http_version: None,
        },
        body_rx,
        cancel_handle.clone(),
        completion_rx,
    )
    .with_lifetime_lease(old_runtime.clone());
    let (body_eof_tx, body_eof_rx) = oneshot::channel();
    let mut outward = client.tee_raw_subresource_response_for_memory_cache_with_body_eof_observer(
        request,
        cache_key,
        inner,
        Some(body_eof_tx),
    );

    let replacement = registrar
        .replace_owned(BrowserResourceRuntimeOwner::new(
            &FetchConfig::default(),
            new_shared_browser_cookie_store(),
        ))
        .expect("replacement runtime should register on the same root");
    drop(replacement);
    drop(client);
    drop(old_runtime);
    assert_eq!(root.owner_count_for_testing(), 2);

    assert_eq!(outward.next_chunk().await.as_deref(), Some(&b"last"[..]));
    body_eof_rx
        .await
        .expect("tee should observe upstream body EOF before completion");
    assert!(
        !completion_tx.is_closed(),
        "upstream completion must remain pending at the test barrier"
    );

    drop(outward);
    completion_tx.closed().await;
    assert!(
        cancel_handle.is_cancelled(),
        "dropping the outward response must cancel the pending inner response"
    );

    let retired_reports = root.reap_retired();
    assert_eq!(retired_reports.len(), 1);
    assert!(retired_reports[0].is_clean());
    assert_eq!(root.owner_count_for_testing(), 1);
    let active_reports = root.shutdown_and_join_reports_for_testing();
    assert_eq!(active_reports.len(), 1);
    assert!(active_reports[0].is_clean());
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn dropping_cacheable_stream_stalled_between_chunks_cancels_real_fetch_and_reaps_exact_owner()
-> Result<()> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    let (disconnected_tx, disconnected_rx) = oneshot::channel();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await?;
        read_http_request_head(&mut stream).await?;
        stream
            .write_all(
                concat!(
                    "HTTP/1.1 200 OK\r\n",
                    "Content-Type: text/css\r\n",
                    "Cache-Control: max-age=60\r\n",
                    "Transfer-Encoding: chunked\r\n",
                    "\r\n",
                    "4\r\n",
                    "a{}b\r\n",
                )
                .as_bytes(),
            )
            .await?;

        // The next response chunk is deliberately never published. A read is
        // an exact cancellation witness: no further HTTP request bytes are
        // expected, so EOF means the client closed the in-progress transfer.
        let mut byte = [0_u8; 1];
        let disconnected = stream.read(&mut byte).await? == 0;
        let _ = disconnected_tx.send(disconnected);
        Ok::<_, std::io::Error>(())
    });

    let registration = BrowserResourceRuntimeOwner::new(
        &FetchConfig::default(),
        new_shared_browser_cookie_store(),
    );
    let (root, binding) = BrowserResourceRuntimeOwnerRoot::new(registration);
    let registrar = root.registrar();
    let old_runtime = binding.current();
    let client = ResourceRequestClient::from_browser_resource_runtime(old_runtime.clone());
    let cancel_handle = FetchCancelHandle::new();
    let request = Request::get(&format!("http://{addr}/stalled.css"))?
        .with_resource_type(RequestResourceType::CssStyleSheet);
    assert!(
        raw_subresource_memory_cache_key(&request).is_some(),
        "the request must exercise the renderer memory-cache tee"
    );
    let mut outward = client
        .fetch_raw_stream_with_cancel(request, cancel_handle.clone())
        .await?;
    assert_eq!(outward.next_chunk().await.as_deref(), Some(&b"a{}b"[..]));

    let replacement = registrar
        .replace_owned(BrowserResourceRuntimeOwner::new(
            &FetchConfig::default(),
            new_shared_browser_cookie_store(),
        ))
        .expect("replacement runtime should register on the same root");
    drop(replacement);
    drop(client);
    drop(old_runtime);
    assert_eq!(root.owner_count_for_testing(), 2);

    drop(outward);
    assert!(
        timeout(Duration::from_secs(3), disconnected_rx).await??,
        "the server must observe cancellation while stalled between chunks"
    );
    assert!(
        cancel_handle.is_cancelled(),
        "dropping the outward tee must cancel its exact inner fetch"
    );

    let retired_reports = root.reap_retired();
    assert_eq!(retired_reports.len(), 1);
    assert!(
        retired_reports[0].is_clean(),
        "the exact retired fetch owner must join cleanly: {:?}",
        retired_reports[0]
    );
    let active_reports = root.shutdown_and_join_reports_for_testing();
    assert_eq!(active_reports.len(), 1);
    assert!(active_reports[0].is_clean());
    server.await??;
    Ok(())
}

async fn read_http_request_head(stream: &mut tokio::net::TcpStream) -> std::io::Result<()> {
    let mut request = Vec::new();
    let mut byte = [0_u8; 1];
    loop {
        let read = stream.read(&mut byte).await?;
        if read == 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "client closed before sending complete request",
            ));
        }
        request.push(byte[0]);
        if request.ends_with(b"\r\n\r\n") {
            return Ok(());
        }
    }
}

async fn read_http_request_text(stream: &mut tokio::net::TcpStream) -> std::io::Result<String> {
    let mut request = Vec::new();
    let mut byte = [0_u8; 1];
    loop {
        let read = stream.read(&mut byte).await?;
        if read == 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "client closed before sending complete request",
            ));
        }
        request.push(byte[0]);
        if request.ends_with(b"\r\n\r\n") {
            return String::from_utf8(request).map_err(|error| {
                std::io::Error::new(std::io::ErrorKind::InvalidData, error.to_string())
            });
        }
    }
}

#[test]
fn merge_loader_network_policy_headers_uses_header_name_keys_and_request_order() {
    let policy = super::PageNetworkPolicy::default();
    policy.set_extra_http_headers(&[
        ("X-Test".to_owned(), "context".to_owned()),
        ("Accept".to_owned(), "text/html".to_owned()),
    ]);
    let mut request = Request::get("https://example.test/headers")
        .unwrap()
        .with_page_network_policy();
    request.request_headers = vec![
        ("x-test".to_owned(), "request".to_owned()),
        ("X-New".to_owned(), "new".to_owned()),
    ];
    let merged = policy
        .snapshot()
        .apply_to_request(request)
        .unwrap()
        .request_headers;

    assert_eq!(
        merged,
        vec![
            ("Accept".to_owned(), "text/html".to_owned()),
            ("x-test".to_owned(), "request".to_owned()),
            ("X-New".to_owned(), "new".to_owned()),
        ]
    );
}

#[test]
fn csp_report_subresource_maps_to_csp_report_request_type() {
    assert_eq!(
        crate::network::request_resource_type_for_subresource(SubresourceResourceType::CspReport),
        Some(RequestResourceType::CspReport)
    );
    assert_eq!(
        SubresourceResourceType::CspReport.as_cdp_type(),
        "CSPViolationReport"
    );
}

#[test]
fn dictionary_subresource_maps_to_dictionary_request_type_and_other_cdp_type() {
    assert_eq!(
        crate::network::request_resource_type_for_subresource(SubresourceResourceType::Dictionary),
        Some(RequestResourceType::Dictionary)
    );
    assert_eq!(SubresourceResourceType::Dictionary.as_cdp_type(), "Other");
}

#[test]
fn audio_video_subresources_keep_media_request_type_and_cdp_type() {
    assert_eq!(
        crate::network::request_resource_type_for_subresource(SubresourceResourceType::Audio),
        Some(RequestResourceType::Media)
    );
    assert_eq!(
        crate::network::request_resource_type_for_subresource(SubresourceResourceType::Video),
        Some(RequestResourceType::Media)
    );
    assert_eq!(SubresourceResourceType::Audio.as_cdp_type(), "Media");
    assert_eq!(SubresourceResourceType::Video.as_cdp_type(), "Media");
}

#[test]
fn loader_disables_every_optional_resource_family_by_default() {
    let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
    let optional = [
        SubresourceResourceType::Image,
        SubresourceResourceType::Font,
        SubresourceResourceType::Audio,
        SubresourceResourceType::Video,
        SubresourceResourceType::Media,
        SubresourceResourceType::TextTrack,
    ];

    assert_eq!(
        loader.optional_resource_fetch_mask(),
        OptionalResourceFetchMask::NONE
    );
    for resource_type in optional {
        assert!(
            !loader.optional_resource_fetch_enabled(resource_type),
            "{resource_type:?} must be disabled on a default loader"
        );
    }
    for resource_type in [
        SubresourceResourceType::Script,
        SubresourceResourceType::Stylesheet,
        SubresourceResourceType::Fetch,
        SubresourceResourceType::Xhr,
    ] {
        assert!(
            loader.optional_resource_fetch_enabled(resource_type),
            "{resource_type:?} must remain outside this policy"
        );
    }
}

#[test]
fn loader_preserves_each_optional_resource_bit_without_aliasing() {
    let resources = [
        (
            SubresourceResourceType::Image,
            OptionalResourceFetchMask::IMAGE,
        ),
        (
            SubresourceResourceType::Font,
            OptionalResourceFetchMask::FONT,
        ),
        (
            SubresourceResourceType::Audio,
            OptionalResourceFetchMask::AUDIO,
        ),
        (
            SubresourceResourceType::Video,
            OptionalResourceFetchMask::VIDEO,
        ),
        (
            SubresourceResourceType::Media,
            OptionalResourceFetchMask::MEDIA,
        ),
        (
            SubresourceResourceType::TextTrack,
            OptionalResourceFetchMask::TEXT_TRACK,
        ),
    ];
    let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("loader");

    for (enabled_type, enabled_bit) in resources {
        loader.set_optional_resource_fetch_mask(enabled_bit);
        assert_eq!(loader.optional_resource_fetch_mask(), enabled_bit);
        for (observed_type, _) in resources {
            assert_eq!(
                loader.optional_resource_fetch_enabled(observed_type),
                observed_type == enabled_type,
                "enabling {enabled_type:?} changed {observed_type:?}"
            );
        }
    }
}

#[test]
fn loader_image_compatibility_switch_preserves_other_resource_bits() {
    let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
    let preserved = OptionalResourceFetchMask::AUDIO | OptionalResourceFetchMask::FONT;
    loader.set_optional_resource_fetch_mask(preserved);

    loader.set_image_fetch_enabled(true);
    assert_eq!(
        loader.optional_resource_fetch_mask(),
        preserved | OptionalResourceFetchMask::IMAGE
    );

    loader.set_image_fetch_enabled(false);
    assert_eq!(loader.optional_resource_fetch_mask(), preserved);
}

#[test]
fn script_text_memory_cache_key_partitions_credentials_and_cookie_context() -> Result<()> {
    let url = Url::parse("https://scripts.test/app.js")?;
    let base_request = || {
        Request::get_with_url(url.clone())
            .with_script_fetch_metadata(ScriptFetchRequestMetadata::default())
    };

    let include_key = super::script_text_cache_key(
        &base_request().with_credentials_mode(RequestCredentialsMode::Include),
    );
    let omit_key = super::script_text_cache_key(
        &base_request().with_credentials_mode(RequestCredentialsMode::Omit),
    );
    assert_ne!(
        include_key, omit_key,
        "script text cache must not share responses across credentials modes"
    );

    let same_origin = Url::parse("https://scripts.test/page")?;
    let cross_origin = Url::parse("https://other.test/page")?;
    let mut same_site_request = base_request();
    same_site_request.cookie_context =
        NetworkCookieRequestContext::subresource("GET").with_initiator_url(&url, &same_origin);
    let mut cross_site_request = base_request();
    cross_site_request.cookie_context =
        NetworkCookieRequestContext::subresource("GET").with_initiator_url(&url, &cross_origin);

    assert_ne!(
        super::script_text_cache_key(&same_site_request),
        super::script_text_cache_key(&cross_site_request),
        "script text cache must not share responses across cookie contexts"
    );

    let first_page = Url::parse("https://scripts.test/first-page")?;
    let second_page = Url::parse("https://scripts.test/second-page")?;
    let mut first_page_request = base_request();
    first_page_request.cookie_context =
        NetworkCookieRequestContext::subresource("GET").with_initiator_url(&url, &first_page);
    let mut second_page_request = base_request();
    second_page_request.cookie_context =
        NetworkCookieRequestContext::subresource("GET").with_initiator_url(&url, &second_page);
    assert_eq!(
        super::script_text_cache_key(&first_page_request),
        super::script_text_cache_key(&second_page_request),
        "same-site page URL changes should not partition reusable script/module responses"
    );

    let first_partition_key = base_request().with_network_partition_key(Some(
        "storage-key:v1;origin=https://scripts.test;top-level-site=https://app.example.test;opaque-nonce=1"
            .to_owned(),
    ));
    let second_partition_key = base_request().with_network_partition_key(Some(
        "storage-key:v1;origin=https://scripts.test;top-level-site=https://app.example.test;opaque-nonce=2"
            .to_owned(),
    ));
    assert_ne!(
        super::script_text_cache_key(&first_partition_key),
        super::script_text_cache_key(&second_partition_key),
        "script text cache must not share responses across network partition keys"
    );

    let no_integrity_key = super::script_text_cache_key(&base_request());
    let integrity_key = super::script_text_cache_key(&base_request().with_script_fetch_metadata(
        ScriptFetchRequestMetadata {
            integrity: Some("sha384-integrity".to_owned()),
            ..ScriptFetchRequestMetadata::default()
        },
    ));
    assert_ne!(
        no_integrity_key, integrity_key,
        "script text cache must not share responses across integrity metadata"
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn streaming_fetch_commits_set_cookie_before_body_completion() -> Result<()> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        read_http_request_head(&mut stream).await.unwrap();
        stream
            .write_all(
                concat!(
                    "HTTP/1.1 200 OK\r\n",
                    "Content-Type: text/html; charset=utf-8\r\n",
                    "Set-Cookie: sid=server; Path=/\r\n",
                    "Transfer-Encoding: chunked\r\n",
                    "\r\n",
                    "c\r\n",
                    "hello world!\r\n",
                )
                .as_bytes(),
            )
            .await
            .unwrap();
        sleep(Duration::from_millis(100)).await;
        stream.write_all(b"0\r\n\r\n").await.unwrap();
    });

    let cookie_store = new_shared_browser_cookie_store();
    let loader = ResourceRequestClient::new_with_cookie_store(
        &FetchConfig::default(),
        Arc::clone(&cookie_store),
    )?;
    let request = Request::get(&format!("http://{addr}/stream"))?;
    let mut response = loader
        .fetch_raw_stream_with_cancel(request, FetchCancelHandle::new())
        .await?;

    assert_eq!(
        cookie_header_for_request(
            &cookie_store,
            &Url::parse(&format!("http://{addr}/subresource"))?,
            NetworkCookieRequestContext::subresource("GET"),
        )?,
        Some("sid=server".to_owned())
    );
    assert_eq!(
        response.next_chunk().await.as_deref(),
        Some(&b"hello world!"[..])
    );

    response.finish().await?;
    server.await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn dropping_streaming_response_cancels_the_source_transfer() -> Result<()> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    let (disconnect_tx, disconnect_rx) = oneshot::channel();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        read_http_request_head(&mut stream).await.unwrap();
        stream
            .write_all(
                concat!(
                    "HTTP/1.1 200 OK\r\n",
                    "Content-Type: text/html; charset=utf-8\r\n",
                    "Transfer-Encoding: chunked\r\n",
                    "\r\n",
                    "1\r\n",
                    "a\r\n",
                )
                .as_bytes(),
            )
            .await
            .unwrap();
        let mut disconnected = false;
        for _ in 0..200 {
            sleep(Duration::from_millis(10)).await;
            if stream.write_all(b"1\r\na\r\n").await.is_err() {
                disconnected = true;
                break;
            }
        }
        let _ = disconnect_tx.send(disconnected);
    });

    let loader = ResourceRequestClient::new(&FetchConfig::default())?;
    let request = Request::get(&format!("http://{addr}/stream"))?;
    let mut response = loader
        .fetch_raw_stream_with_cancel(request, FetchCancelHandle::new())
        .await?;

    assert_eq!(response.next_chunk().await.as_deref(), Some(&b"a"[..]));
    drop(response);

    assert!(
        timeout(Duration::from_secs(3), disconnect_rx).await??,
        "server never observed the client cancelling the streaming response"
    );

    server.await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn text_stream_fetch_uses_disk_cache_for_safe_gets() -> Result<()> {
    let cache_dir = unique_test_cache_dir();
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        read_http_request_head(&mut stream).await.unwrap();
        let body = "cached-renderer-text-stream";
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nCache-Control: max-age=60\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        stream.write_all(response.as_bytes()).await.unwrap();
    });

    let mut config = FetchConfig::default();
    config.set_http_cache_dir(Some(cache_dir.display().to_string()));
    let loader = ResourceRequestClient::new(&config)?;
    let url = format!("http://{addr}/cacheable.txt");

    let first = loader
        .fetch_text_stream(Request::get(&url)?.with_page_network_policy())
        .await?;
    let second = loader
        .fetch_text_stream(Request::get(&url)?.with_page_network_policy())
        .await?;

    assert_eq!(first.body_text(), "cached-renderer-text-stream");
    assert_eq!(second.body_text(), "cached-renderer-text-stream");

    server.await?;
    let _ = std::fs::remove_dir_all(cache_dir);
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn script_text_fetch_uses_shared_memory_resource_cache_with_fresh_cache_headers() -> Result<()>
{
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        read_http_request_head(&mut stream).await.unwrap();
        let body = "window.scriptMemoryCacheHit = true;";
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/javascript\r\nCache-Control: max-age=60\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        stream.write_all(response.as_bytes()).await.unwrap();

        assert!(
            timeout(Duration::from_millis(100), listener.accept())
                .await
                .is_err(),
            "second script fetch should be served from the loader memory cache"
        );
    });

    let loader = ResourceRequestClient::new(&FetchConfig::default())?;
    let url = format!("http://{addr}/app.js");
    let request = || {
        Request::get(&url).map(|request| {
            request.with_script_fetch_metadata(ScriptFetchRequestMetadata::default())
        })
    };

    let first = loader
        .fetch_cacheable_script_text_stream(request()?)
        .await?;
    let second = loader
        .fetch_cacheable_script_text_stream(request()?)
        .await?;

    assert_eq!(first.body_text(), "window.scriptMemoryCacheHit = true;");
    assert_eq!(second.body_text(), "window.scriptMemoryCacheHit = true;");
    assert!(!first.from_cache);
    assert!(second.from_cache);

    server.await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn cache_bypass_replaces_script_text_memory_entry() -> Result<()> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    let server = tokio::spawn(async move {
        for body in [
            "window.cacheGeneration = 1;",
            "window.cacheGeneration = 1;",
            "window.cacheGeneration = 2;",
        ] {
            let (mut stream, _) = listener.accept().await.unwrap();
            read_http_request_head(&mut stream).await.unwrap();
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/javascript\r\nCache-Control: max-age=60\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream.write_all(response.as_bytes()).await.unwrap();
        }
        assert!(
            timeout(Duration::from_millis(100), listener.accept())
                .await
                .is_err(),
            "normal loading after bypass should retain both Page-local script entries"
        );
    });

    let loader = ResourceRequestClient::new(&FetchConfig::default())?;
    let peer = loader.fork_with_isolated_page_network_policy();
    let url = format!("http://{addr}/app.js");
    let request = || {
        Request::get(&url).map(|request| {
            request
                .with_page_network_policy()
                .with_script_fetch_metadata(ScriptFetchRequestMetadata::default())
        })
    };

    let first = loader
        .fetch_cacheable_script_text_stream(request()?)
        .await?;
    let peer_first = peer.fetch_cacheable_script_text_stream(request()?).await?;
    loader.set_cache_disabled(true);
    let bypassed = loader
        .fetch_cacheable_script_text_stream(request()?)
        .await?;
    loader.set_cache_disabled(false);
    let restored = loader
        .fetch_cacheable_script_text_stream(request()?)
        .await?;
    let peer_restored = peer.fetch_cacheable_script_text_stream(request()?).await?;

    assert_eq!(first.body_text(), "window.cacheGeneration = 1;");
    assert_eq!(peer_first.body_text(), "window.cacheGeneration = 1;");
    assert_eq!(bypassed.body_text(), "window.cacheGeneration = 2;");
    assert_eq!(restored.body_text(), "window.cacheGeneration = 2;");
    assert_eq!(peer_restored.body_text(), "window.cacheGeneration = 1;");
    assert!(!first.from_cache);
    assert!(!peer_first.from_cache);
    assert!(!bypassed.from_cache);
    assert!(restored.from_cache);
    assert!(peer_restored.from_cache);

    server.await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn unique_script_text_fetches_stay_within_one_loader_memory_budget() -> Result<()> {
    const SCRIPT_COUNT: usize = 40;
    const SCRIPT_BYTES: usize = 256 * 1024;

    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    let server = tokio::spawn(async move {
        let body = format!("/*{}*/", "x".repeat(SCRIPT_BYTES - 4));
        for _ in 0..SCRIPT_COUNT {
            let (mut stream, _) = listener.accept().await.unwrap();
            read_http_request_head(&mut stream).await.unwrap();
            let response_head = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/javascript\r\nCache-Control: max-age=3600\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            stream.write_all(response_head.as_bytes()).await.unwrap();
            stream.write_all(body.as_bytes()).await.unwrap();
        }
    });

    let loader = ResourceRequestClient::new(&FetchConfig::default())?;
    for index in 0..SCRIPT_COUNT {
        let url = format!("http://{addr}/script-{index}.js");
        let request =
            Request::get(&url)?.with_script_fetch_metadata(ScriptFetchRequestMetadata::default());
        let response = loader.fetch_cacheable_script_text_stream(request).await?;
        assert_eq!(response.body_bytes().len(), SCRIPT_BYTES);
    }

    server.await?;
    let diagnostics = loader.memory_cache_diagnostics();
    assert!(
        diagnostics.retained_bytes <= diagnostics.retained_bytes_limit,
        "loader memory cache exceeded its shared budget: {diagnostics:?}"
    );
    assert!(
        diagnostics.completed_script_entry_count < SCRIPT_COUNT,
        "unique completed scripts should evict older strong references: {diagnostics:?}"
    );
    assert_eq!(diagnostics.pending_script_entry_count, 0);
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn stylesheet_text_stream_uses_shared_memory_resource_cache_without_http_cache_dir()
-> Result<()> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        read_http_request_head(&mut stream).await.unwrap();
        let body = "body { color: rgb(1, 2, 3); }";
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/css\r\nCache-Control: max-age=60\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        stream.write_all(response.as_bytes()).await.unwrap();

        assert!(
            timeout(Duration::from_millis(100), listener.accept())
                .await
                .is_err(),
            "second stylesheet text stream should be served from the loader memory cache"
        );
    });

    let loader = ResourceRequestClient::new(&FetchConfig::default())?;
    let url = format!("http://{addr}/cached.css");
    let request = || {
        Request::get(&url).map(|request| {
            request
                .with_page_network_policy()
                .with_resource_type(RequestResourceType::CssStyleSheet)
        })
    };

    let first = loader.fetch_text_stream(request()?).await?;
    let second = loader.fetch_text_stream(request()?).await?;

    assert_eq!(first.body_text(), "body { color: rgb(1, 2, 3); }");
    assert_eq!(second.body_text(), "body { color: rgb(1, 2, 3); }");
    assert!(!first.from_cache);
    assert!(second.from_cache);

    server.await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn text_stream_fetch_handles_local_data_stylesheet_urls() -> Result<()> {
    let loader = ResourceRequestClient::new(&FetchConfig::default())?;
    let response = loader
        .fetch_text_stream(
            Request::get("data:text/css,:root%7Bbackground:green%7D")?
                .with_page_network_policy()
                .with_resource_type(RequestResourceType::CssStyleSheet),
        )
        .await?;

    assert_eq!(response.status, 200);
    assert_eq!(
        response.final_url.as_str(),
        "data:text/css,:root%7Bbackground:green%7D"
    );
    assert!(
        response
            .headers
            .iter()
            .any(|(name, value)| name.eq_ignore_ascii_case("content-type") && value == "text/css")
    );
    assert_eq!(response.body_text(), ":root{background:green}");
    assert!(!response.from_cache);
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn concurrent_script_text_waiter_preserves_owner_cache_state() -> Result<()> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    let (release_tx, release_rx) = oneshot::channel();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        read_http_request_head(&mut stream).await.unwrap();
        let body = "export default function fromCacheCoalescing() {}";
        let response_head = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/javascript\r\nCache-Control: max-age=60\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        stream.write_all(response_head.as_bytes()).await.unwrap();
        let _ = release_rx.await;
        stream.write_all(body.as_bytes()).await.unwrap();

        assert!(
            timeout(Duration::from_millis(100), listener.accept())
                .await
                .is_err(),
            "coalesced and completed-cache script loads should share the single network request"
        );
    });

    let loader = ResourceRequestClient::new(&FetchConfig::default())?;
    let url = format!("http://{addr}/module.js");
    let request = || {
        Request::get(&url).map(|request| {
            request.with_script_fetch_metadata(ScriptFetchRequestMetadata::default())
        })
    };

    let first_request = request()?;
    let second_request = request()?;
    let first = loader.fetch_cacheable_script_text_stream(first_request);
    let second = loader.fetch_cacheable_script_text_stream(second_request);
    let release = async move {
        sleep(Duration::from_millis(50)).await;
        let _ = release_tx.send(());
    };

    let (first, second, _) = tokio::join!(first, second, release);
    let first = first?;
    let second = second?;
    assert_eq!(
        first.body_text(),
        "export default function fromCacheCoalescing() {}"
    );
    assert_eq!(
        second.body_text(),
        "export default function fromCacheCoalescing() {}"
    );
    assert!(!first.from_cache);
    assert!(
        !second.from_cache,
        "in-flight coalescing should preserve the owner's network provenance"
    );

    let third = loader
        .fetch_cacheable_script_text_stream(request()?)
        .await?;
    assert_eq!(
        third.body_text(),
        "export default function fromCacheCoalescing() {}"
    );
    assert!(
        third.from_cache,
        "completed script text cache hits should still report memory-cache provenance"
    );

    server.await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn script_text_fetch_respects_configured_request_timeout() -> Result<()> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        read_http_request_head(&mut stream).await.unwrap();
        stream
            .write_all(
                concat!(
                    "HTTP/1.1 200 OK\r\n",
                    "Content-Type: application/javascript\r\n",
                    "Content-Length: 32\r\n",
                    "Connection: close\r\n",
                    "\r\n",
                )
                .as_bytes(),
            )
            .await
            .unwrap();
        sleep(Duration::from_millis(500)).await;
        let _ = stream.write_all(b"window.scriptLoaded = true;\n").await;
    });

    let mut config = FetchConfig::default();
    config.set_request_timeout_ms(100);
    let loader = ResourceRequestClient::new(&config)?;
    let request = Request::get(&format!("http://{addr}/slow-script.js"))?
        .with_script_fetch_metadata(ScriptFetchRequestMetadata::default());
    let error = timeout(
        Duration::from_secs(2),
        loader.fetch_cacheable_script_text_stream(request),
    )
    .await
    .expect("script fetch should complete with the configured request timeout")
    .unwrap_err();

    assert!(
        error
            .chain()
            .any(|cause| cause.to_string().contains("Timeout was reached")),
        "expected configured curl request timeout, got: {error:#}"
    );
    server.await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn compatibility_fetch_uses_streaming_disk_cache_for_safe_gets() -> Result<()> {
    let cache_dir = unique_test_cache_dir();
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        read_http_request_head(&mut stream).await.unwrap();
        let body = "cached-renderer-compat-fetch";
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nCache-Control: max-age=60\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        stream.write_all(response.as_bytes()).await.unwrap();
    });

    let mut config = FetchConfig::default();
    config.set_http_cache_dir(Some(cache_dir.display().to_string()));
    let loader = ResourceRequestClient::new(&config)?;
    let url = format!("http://{addr}/cacheable.txt");

    let first = loader
        .fetch(Request::get(&url)?.with_page_network_policy())
        .await?;
    let second = loader
        .fetch(Request::get(&url)?.with_page_network_policy())
        .await?;

    assert_eq!(first.body_text(), "cached-renderer-compat-fetch");
    assert_eq!(second.body_text(), "cached-renderer-compat-fetch");

    server.await?;
    let _ = std::fs::remove_dir_all(cache_dir);
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn compatibility_raw_fetch_uses_streaming_disk_cache_for_safe_gets() -> Result<()> {
    let cache_dir = unique_test_cache_dir();
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        read_http_request_head(&mut stream).await.unwrap();
        let body = b"cached-renderer-compat-raw\xff";
        let response_head = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nCache-Control: max-age=60\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        stream.write_all(response_head.as_bytes()).await.unwrap();
        stream.write_all(body).await.unwrap();
    });

    let mut config = FetchConfig::default();
    config.set_http_cache_dir(Some(cache_dir.display().to_string()));
    let loader = ResourceRequestClient::new(&config)?;
    let url = format!("http://{addr}/cacheable.bin");

    let first = loader
        .fetch_raw(Request::get(&url)?.with_page_network_policy())
        .await?;
    let second = loader
        .fetch_raw(Request::get(&url)?.with_page_network_policy())
        .await?;

    assert_eq!(first.body_bytes(), b"cached-renderer-compat-raw\xff");
    assert_eq!(second.body_bytes(), b"cached-renderer-compat-raw\xff");

    server.await?;
    let _ = std::fs::remove_dir_all(cache_dir);
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn image_raw_stream_uses_shared_memory_resource_cache_without_http_cache_dir() -> Result<()> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        read_http_request_head(&mut stream).await.unwrap();
        let body = b"cached-image-body";
        let response_head = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nCache-Control: max-age=60\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        stream.write_all(response_head.as_bytes()).await.unwrap();
        stream.write_all(body).await.unwrap();

        assert!(
            timeout(Duration::from_millis(100), listener.accept())
                .await
                .is_err(),
            "second image fetch should be served from the loader memory cache"
        );
    });

    let loader = ResourceRequestClient::new(&FetchConfig::default())?;
    let url = format!("http://{addr}/cached.png");
    let request = || {
        Request::get(&url).map(|request| {
            request
                .with_page_network_policy()
                .with_resource_type(RequestResourceType::Image)
        })
    };

    let mut first = loader
        .fetch_raw_stream_with_cancel(request()?, FetchCancelHandle::new())
        .await?;
    assert!(!first.from_cache);
    let mut first_body = Vec::new();
    while let Some(chunk) = first.next_chunk().await {
        first_body.extend_from_slice(&chunk);
    }
    first.finish().await?;

    let mut second = loader
        .fetch_raw_stream_with_cancel(request()?, FetchCancelHandle::new())
        .await?;
    assert!(second.from_cache);
    let mut second_body = Vec::new();
    while let Some(chunk) = second.next_chunk().await {
        second_body.extend_from_slice(&chunk);
    }
    second.finish().await?;

    assert_eq!(first_body, b"cached-image-body");
    assert_eq!(second_body, b"cached-image-body");

    server.await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn cache_bypass_replaces_raw_subresource_memory_entry() -> Result<()> {
    async fn body(mut response: StreamingRawResponse) -> Result<(Vec<u8>, bool)> {
        let from_cache = response.from_cache;
        let mut bytes = Vec::new();
        while let Some(chunk) = response.next_chunk().await {
            bytes.extend_from_slice(&chunk);
        }
        response.finish().await?;
        Ok((bytes, from_cache))
    }

    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    let server = tokio::spawn(async move {
        for payload in [
            b"image-v1".as_slice(),
            b"image-v1".as_slice(),
            b"image-v2".as_slice(),
        ] {
            let (mut stream, _) = listener.accept().await.unwrap();
            read_http_request_head(&mut stream).await.unwrap();
            let response_head = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nCache-Control: max-age=60\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                payload.len()
            );
            stream.write_all(response_head.as_bytes()).await.unwrap();
            stream.write_all(payload).await.unwrap();
        }
        assert!(
            timeout(Duration::from_millis(100), listener.accept())
                .await
                .is_err(),
            "normal loading after bypass should retain both Page-local image entries"
        );
    });

    let loader = ResourceRequestClient::new(&FetchConfig::default())?;
    let peer = loader.fork_with_isolated_page_network_policy();
    let url = format!("http://{addr}/cached.png");
    let request = || {
        Request::get(&url).map(|request| {
            request
                .with_page_network_policy()
                .with_resource_type(RequestResourceType::Image)
        })
    };

    let first = body(
        loader
            .fetch_raw_stream_with_cancel(request()?, FetchCancelHandle::new())
            .await?,
    )
    .await?;
    let peer_first = body(
        peer.fetch_raw_stream_with_cancel(request()?, FetchCancelHandle::new())
            .await?,
    )
    .await?;
    loader.set_cache_disabled(true);
    let bypassed = body(
        loader
            .fetch_raw_stream_with_cancel(request()?, FetchCancelHandle::new())
            .await?,
    )
    .await?;
    loader.set_cache_disabled(false);
    let restored = body(
        loader
            .fetch_raw_stream_with_cancel(request()?, FetchCancelHandle::new())
            .await?,
    )
    .await?;
    let peer_restored = body(
        peer.fetch_raw_stream_with_cancel(request()?, FetchCancelHandle::new())
            .await?,
    )
    .await?;

    assert_eq!(first, (b"image-v1".to_vec(), false));
    assert_eq!(peer_first, (b"image-v1".to_vec(), false));
    assert_eq!(bypassed, (b"image-v2".to_vec(), false));
    assert_eq!(restored, (b"image-v2".to_vec(), true));
    assert_eq!(peer_restored, (b"image-v1".to_vec(), true));

    server.await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn browser_fetch_raw_stream_uses_shared_memory_resource_cache_without_http_cache_dir()
-> Result<()> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        read_http_request_head(&mut stream).await.unwrap();
        let body = b"cached-fetch-body";
        let response_head = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nCache-Control: max-age=60\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        stream.write_all(response_head.as_bytes()).await.unwrap();
        stream.write_all(body).await.unwrap();

        assert!(
            timeout(Duration::from_millis(100), listener.accept())
                .await
                .is_err(),
            "second browser fetch raw stream should be served from the loader memory cache"
        );
    });

    let loader = ResourceRequestClient::new(&FetchConfig::default())?;
    let url = format!("http://{addr}/cached-fetch.txt");
    let request = || {
        Request::get(&url).map(|request| {
            request
                .with_page_network_policy()
                .with_browser_request_metadata(BrowserRequestMetadata::Fetch)
        })
    };

    let mut first = loader
        .fetch_raw_stream_with_cancel(request()?, FetchCancelHandle::new())
        .await?;
    assert!(!first.from_cache);
    let mut first_body = Vec::new();
    while let Some(chunk) = first.next_chunk().await {
        first_body.extend_from_slice(&chunk);
    }
    first.finish().await?;

    let mut second = loader
        .fetch_raw_stream_with_cancel(request()?, FetchCancelHandle::new())
        .await?;
    assert!(second.from_cache);
    let mut second_body = Vec::new();
    while let Some(chunk) = second.next_chunk().await {
        second_body.extend_from_slice(&chunk);
    }
    second.finish().await?;

    assert_eq!(first_body, b"cached-fetch-body");
    assert_eq!(second_body, b"cached-fetch-body");

    server.await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn cached_raw_subresource_marks_redirect_hops_from_cache() -> Result<()> {
    let start_url = Url::parse("http://example.test/redirect.txt")?;
    let final_url = Url::parse("http://example.test/final.txt")?;
    let response = RawResponse::from_head_and_body(
        ResponseHead {
            final_url: final_url.clone(),
            status: 200,
            headers: vec![("content-type".to_owned(), "text/plain".to_owned())],
            request_cookie_report: None,
            cookie_set_reports: Vec::new(),
            redirected: true,
            redirect_chain: vec![RedirectInfo {
                from_url: start_url,
                to_url: final_url,
                status: 301,
                headers: vec![("location".to_owned(), "/final.txt".to_owned())],
                network_extra_info_available: true,
                request_extra_info: None,
                response_extra_info: None,
                redirect_has_extra_info: true,
                request_cookie_report: None,
                cookie_set_reports: Vec::new(),
                from_cache: false,
                negotiated_http_version: None,
            }],
            from_cache: false,
            negotiated_http_version: None,
        },
        b"cached-final-body".to_vec(),
    );

    let mut response = streaming_raw_response_from_cached_subresource(response)?;

    assert!(response.from_cache);
    assert_eq!(response.redirect_chain.len(), 1);
    assert!(response.redirect_chain[0].from_cache);
    assert!(!response.redirect_chain[0].network_extra_info_available);
    let mut body = Vec::new();
    while let Some(chunk) = response.next_chunk().await {
        body.extend_from_slice(&chunk);
    }
    response.finish().await?;
    assert_eq!(body, b"cached-final-body");
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn loader_applies_network_policy_only_to_opt_in_requests() -> Result<()> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    let server = tokio::spawn(async move {
        let (mut first, _) = listener.accept().await.unwrap();
        let first_request = read_http_request_text(&mut first).await.unwrap();
        let first_seen = first_request
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("x-cdp-test")
                    .then(|| value.trim().to_owned())
            })
            .unwrap_or_default();
        let first_body = format!("plain:{first_seen}");
        let first_response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            first_body.len(),
            first_body
        );
        first.write_all(first_response.as_bytes()).await.unwrap();

        let (mut second, _) = listener.accept().await.unwrap();
        let second_request = read_http_request_text(&mut second).await.unwrap();
        let second_seen = second_request
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("x-cdp-test")
                    .then(|| value.trim().to_owned())
            })
            .unwrap_or_default();
        let second_body = format!("optin:{second_seen}");
        let second_response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            second_body.len(),
            second_body
        );
        second.write_all(second_response.as_bytes()).await.unwrap();
    });

    let loader = ResourceRequestClient::new(&FetchConfig::default())?;
    loader.set_extra_http_headers(&[("x-cdp-test".to_owned(), "loader-policy".to_owned())]);

    let plain = loader
        .fetch(Request::get(&format!("http://{addr}/plain"))?)
        .await?;
    assert_eq!(plain.body_text(), "plain:");

    let opt_in = loader
        .fetch(Request::get(&format!("http://{addr}/optin"))?.with_page_network_policy())
        .await?;
    assert_eq!(opt_in.body_text(), "optin:loader-policy");

    server.await?;
    Ok(())
}

#[test]
fn service_worker_bypass_isolated_between_page_request_clients() -> Result<()> {
    let shared = ResourceRequestClient::new(&FetchConfig::default())?;
    let first_page = shared.fork_with_isolated_page_network_policy();
    let second_page = shared.fork_with_isolated_page_network_policy();
    first_page.set_bypass_service_worker(true);
    second_page.set_bypass_service_worker(false);

    assert!(!shared.bypass_service_worker());
    assert!(first_page.bypass_service_worker());
    assert!(!second_page.bypass_service_worker());

    first_page.set_bypass_service_worker(false);
    second_page.set_bypass_service_worker(true);

    assert!(!shared.bypass_service_worker());
    assert!(!first_page.bypass_service_worker());
    assert!(second_page.bypass_service_worker());
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn loader_opt_in_network_policy_enforces_blocked_and_offline() -> Result<()> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let _ = read_http_request_head(&mut stream).await;
        let body = "reachable";
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        stream.write_all(response.as_bytes()).await.unwrap();
    });

    let loader = ResourceRequestClient::new(&FetchConfig::default())?;
    let blocked_url = format!("http://{addr}/blocked");
    loader.set_blocked_url_patterns(std::slice::from_ref(&blocked_url));
    let blocked = loader
        .fetch(Request::get(&blocked_url)?.with_page_network_policy())
        .await
        .unwrap_err()
        .to_string();
    assert!(blocked.contains("net::ERR_BLOCKED_BY_CLIENT"));

    let plain = loader
        .fetch(Request::get(&format!("http://{addr}/plain"))?)
        .await?;
    assert_eq!(plain.body_text(), "reachable");

    loader.set_network_offline(true);
    let offline = loader
        .fetch(Request::get(&format!("http://{addr}/offline"))?.with_page_network_policy())
        .await
        .unwrap_err()
        .to_string();
    assert!(offline.contains("Network emulation offline"));

    server.await?;
    Ok(())
}
