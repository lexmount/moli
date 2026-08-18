mod cookie_context;
mod cookie_store;
mod support;

use anyhow::{Context, Result};
use curl::easy::Handler;
use moli_browser_profile::DEFAULT_ACCEPT_LANGUAGE;
use moli_cookie_jar::{NetworkCookieRequestContext, new_shared_browser_cookie_store};
use moli_http_cache::{HttpCacheEntryMetadata, HttpCacheStore};
use std::{
    collections::BTreeSet,
    fs,
    io::Read,
    num::NonZeroU32,
    process::Command,
    sync::{Arc, mpsc as std_mpsc},
    thread,
    time::{Duration, Instant},
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::{mpsc, oneshot};
use url::Url;

use crate::{
    BrowserNavigationRequestKind, BrowserRequestMetadata, FetchCancelHandle, FetchClient,
    FetchClientHandle, FetchConfig, FetchPriorityHint, FetchRuntimeJoinStatus,
    NegotiatedHttpVersion, NetworkFetchFailureContext, RawResponse, Request, RequestAuth,
    RequestAuthScheme, RequestAuthTarget, RequestCacheMode, RequestCredentialsMode, RequestMode,
    RequestRedirectMode, RequestResourceType, Response, ResponseBody, ResponseHead,
    ScriptFetchRequestMetadata, ScriptFetchSchedulerPriority, StreamingResponseCollector,
    SubresourceRequestMetadata, WebBotAuthProfile, WebBotAuthSigner, http_cache_stats,
    runtime::FetchRuntimeOwner,
};

use self::support::{
    EmptyHttpHttpsUpgradeServer, Http2ProtocolFallbackServer, ScriptedH2Server, ScriptedHttpServer,
    ScriptedHttps11Server, ScriptedResponse, unique_test_cache_dir, wait_for_runtime_owner_count,
};

const ENV_PROXY_CHILD_TEST: &str = "MOLI_FETCH_ENV_PROXY_CHILD";
const ENV_PROXY_URL: &str = "MOLI_FETCH_ENV_PROXY_URL";
const TEST_HIGH_ENTROPY_CLIENT_HINTS: &str = "Sec-CH-UA-Full-Version, Sec-CH-UA-Full-Version-List, Sec-CH-UA-Arch, Sec-CH-UA-Bitness, Sec-CH-UA-Platform-Version, Sec-CH-UA-Model, Sec-CH-UA-WoW64";
const RFC_9421_ED25519_PRIVATE_KEY: &str = "-----BEGIN PRIVATE KEY-----\n\
MC4CAQAwBQYDK2VwBCIEIJ+DYvh6SEqVTm50DFtMDoQikTmiCqirVv9mWG9qfSnF\n\
-----END PRIVATE KEY-----\n";

fn test_web_bot_auth_signer() -> WebBotAuthSigner {
    WebBotAuthSigner::from_pem(
        RFC_9421_ED25519_PRIVATE_KEY.as_bytes(),
        "bot.example",
        Some("poqkLGiymh_W0uP6PZFw-dvez3QJT5SolqXBCW38r0U"),
        WebBotAuthProfile::Cloudflare,
    )
    .unwrap()
}

fn sample_response_head() -> ResponseHead {
    ResponseHead {
        final_url: Url::parse("http://example.test/final").unwrap(),
        status: 203,
        headers: vec![("content-type".to_owned(), "text/plain".to_owned())],
        request_cookie_report: None,
        cookie_set_reports: Vec::new(),
        redirected: false,
        redirect_chain: Vec::new(),
        from_cache: false,
        negotiated_http_version: None,
    }
}

fn load_test_cache_body(store: &HttpCacheStore, key: &str) -> Result<Option<Vec<u8>>> {
    let Some(mut entry) = store.load_reader(key)? else {
        return Ok(None);
    };
    let mut body = Vec::new();
    entry.body.read_to_end(&mut body)?;
    Ok(Some(body))
}

fn store_test_cache_body(
    store: &HttpCacheStore,
    key: &str,
    metadata: HttpCacheEntryMetadata,
    body: &[u8],
) -> Result<()> {
    let mut writer = store.create_body_writer(key)?;
    writer.write_all(body)?;
    writer.finish(metadata)
}

fn fetch_response_for_test(client: &FetchClientHandle, request: Request) -> Result<Response> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?
        .block_on(client.fetch(request))
}

fn fetch_raw_with_network_metadata_for_test(
    client: &FetchClient,
    request: Request,
) -> Result<crate::NetworkFetchResult<RawResponse>> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?
        .block_on(client.fetch_raw_with_network_metadata(request))
}

#[test]
fn response_head_round_trips_through_text_and_raw_materialized_responses() {
    let head = sample_response_head();
    let response =
        Response::from_head_and_body(head.clone(), "hello".to_owned(), b"hello".to_vec());

    let raw = response.into_materialized_raw_response();
    assert_eq!(raw.final_url, head.final_url);
    assert_eq!(raw.status, head.status);
    assert_eq!(raw.headers, head.headers);
    assert_eq!(raw.body_bytes(), b"hello");

    let text = raw.into_lossy_materialized_text_response();
    assert_eq!(text.final_url, head.final_url);
    assert_eq!(text.status, head.status);
    assert_eq!(text.headers, head.headers);
    assert_eq!(text.body_text(), "hello");
    assert_eq!(text.body_bytes(), b"hello");
}

#[test]
fn request_redirect_mode_controls_follow_flag() {
    let request = Request::new("GET", "https://example.test/start", None, Vec::new())
        .unwrap()
        .with_redirect_mode(RequestRedirectMode::Manual);

    assert_eq!(request.redirect_mode, RequestRedirectMode::Manual);
    assert!(!request.follow_redirects);
}

#[tokio::test]
async fn http_transport_rejects_file_urls_in_every_response_mode() -> Result<()> {
    const FILE_URL: &str = "file:///moli-policy-must-not-open";
    const EXPECTED: &str = "URL scheme \"file\" is not supported by the HTTP network transport.";

    let mut config = FetchConfig::default();
    config.set_network_blocking(true, Vec::new());
    let client = FetchClient::new(&config, new_shared_browser_cookie_store());

    let buffered_error = client
        .fetch(Request::get(FILE_URL)?.with_follow_redirects(false))
        .await
        .expect_err("buffered transport must reject file URL");
    assert!(format!("{buffered_error:#}").contains(EXPECTED));

    let html_stream_error = client
        .fetch_html_stream(Request::get(FILE_URL)?)
        .await
        .expect_err("HTML streaming transport must reject file URL");
    assert!(format!("{html_stream_error:#}").contains(EXPECTED));

    let raw_stream_error = client
        .fetch_raw_stream_with_cancel(Request::get(FILE_URL)?, FetchCancelHandle::new())
        .await
        .expect_err("raw streaming transport must reject file URL");
    assert!(format!("{raw_stream_error:#}").contains(EXPECTED));
    Ok(())
}

#[test]
fn request_apply_redirect_status_rewrites_post_to_get_and_drops_body_headers() {
    let mut request = Request::new(
        "POST",
        "https://example.test/target",
        Some("payload".to_owned()),
        vec![
            ("Content-Type".to_owned(), "text/plain".to_owned()),
            ("X-Keep".to_owned(), "1".to_owned()),
        ],
    )
    .unwrap();

    request.apply_redirect_status(303);

    assert_eq!(request.method, "GET");
    assert!(request.body.is_none());
    assert!(
        !request
            .request_headers
            .iter()
            .any(|(name, _)| name.eq_ignore_ascii_case("content-type"))
    );
    assert!(
        request
            .request_headers
            .iter()
            .any(|(name, _)| name.eq_ignore_ascii_case("x-keep"))
    );
}

#[test]
fn request_with_request_mode_keeps_expected_mode() {
    let request = Request::new("GET", "https://example.test/script.js", None, Vec::new())
        .unwrap()
        .with_request_mode(RequestMode::NoCors);

    assert_eq!(request.request_mode, RequestMode::NoCors);
}

#[test]
fn raw_response_head_helpers_keep_body_separate() {
    let head = sample_response_head();
    let raw = RawResponse::from_head_and_body(head.clone(), vec![0, 255, b'a']);
    let (split_head, body) = raw.into_parts();

    assert_eq!(split_head.final_url, head.final_url);
    assert_eq!(split_head.status, head.status);
    assert_eq!(split_head.headers, head.headers);
    assert_eq!(
        body.try_into_materialized_bytes()
            .expect("raw body should remain materialized"),
        vec![0, 255, b'a']
    );
}

#[test]
fn response_body_marks_materialized_and_streaming_shapes() {
    let body = ResponseBody::materialized_text("hello".to_owned(), Vec::new());
    assert!(!body.is_streaming());
    assert_eq!(
        body.try_into_materialized_bytes()
            .expect("materialized body should convert to bytes"),
        b"hello"
    );

    let (body_tx, body_rx) = mpsc::unbounded_channel();
    drop(body_tx);
    let (_completion_tx, completion_rx) = oneshot::channel();
    let stream = crate::StreamingHtmlResponse::new(
        Url::parse("http://example.test/stream").unwrap(),
        200,
        Vec::new(),
        body_rx,
        FetchCancelHandle::new(),
        completion_rx,
    );
    let (_head, streaming_body) = stream.into_body();
    assert!(streaming_body.is_streaming());
    assert!(streaming_body.try_into_materialized_bytes().is_err());
}

#[tokio::test]
async fn response_body_materializes_streaming_text_source() {
    let (body_tx, body_rx) = mpsc::unbounded_channel();
    body_tx.send("hel".to_owned()).unwrap();
    body_tx.send("lo".to_owned()).unwrap();
    drop(body_tx);
    let (completion_tx, completion_rx) = oneshot::channel();
    completion_tx.send(Ok(())).unwrap();
    let cancel_handle = FetchCancelHandle::new();

    let stream = crate::StreamingHtmlResponse::new_with_head(
        sample_response_head(),
        body_rx,
        cancel_handle.clone(),
        completion_rx,
    );
    let (text, bytes) = stream
        .into_body()
        .1
        .into_lossy_materialized_text()
        .await
        .expect("streaming text body should materialize");

    assert_eq!(text, "hello");
    assert_eq!(bytes, b"hello");
    assert!(
        !cancel_handle.is_cancelled(),
        "observing a successful streaming completion must disarm Drop cancellation"
    );
}

#[tokio::test]
async fn streaming_raw_response_materializes_through_response_body_source() {
    let (body_tx, body_rx) = mpsc::unbounded_channel();
    body_tx.send(vec![0, 255]).unwrap();
    body_tx.send(vec![b'a']).unwrap();
    drop(body_tx);
    let (completion_tx, completion_rx) = oneshot::channel();
    completion_tx.send(Ok(())).unwrap();
    let cancel_handle = FetchCancelHandle::new();

    let raw = crate::StreamingRawResponse::new(
        sample_response_head().final_url,
        200,
        Vec::new(),
        None,
        Vec::new(),
        false,
        Vec::new(),
        body_rx,
        cancel_handle.clone(),
        completion_rx,
    )
    .into_materialized_raw_response()
    .await
    .expect("streaming raw body should materialize through ResponseBody");

    assert_eq!(raw.body_bytes(), vec![0, 255, b'a']);
    assert!(
        !cancel_handle.is_cancelled(),
        "observing a successful streaming completion must disarm Drop cancellation"
    );
}

#[test]
fn request_credentials_mode_uses_standard_webidl_labels() {
    use std::str::FromStr;

    assert_eq!(
        RequestCredentialsMode::from_str("include"),
        Ok(RequestCredentialsMode::Include)
    );
    assert_eq!(
        RequestCredentialsMode::from_str("same-origin"),
        Ok(RequestCredentialsMode::SameOrigin)
    );
    assert_eq!(
        RequestCredentialsMode::from_str("omit"),
        Ok(RequestCredentialsMode::Omit)
    );
    assert_eq!(RequestCredentialsMode::Include.as_ref(), "include");
    assert_eq!(RequestCredentialsMode::SameOrigin.as_ref(), "same-origin");
    assert_eq!(RequestCredentialsMode::Omit.as_ref(), "omit");
    assert!(RequestCredentialsMode::from_str("credentialless").is_err());
}

fn fetch_with_config_for_test(config: &FetchConfig, request: Request) -> Result<Response> {
    let client = FetchClient::new(config, new_shared_browser_cookie_store());
    fetch_response_for_test(&client, request)
}

#[test]
fn request_min_timeout_can_extend_short_configured_deadline() {
    let server = ScriptedHttpServer::spawn(vec![ScriptedResponse::ok("slow").with_delay_ms(150)]);
    let mut config = FetchConfig::default();
    config.set_request_timeout_ms(50);

    let response = fetch_with_config_for_test(
        &config,
        Request::get(&server.url())
            .unwrap()
            .with_min_request_timeout(Duration::from_secs(1)),
    )
    .unwrap();

    assert_eq!(response.body_text(), "slow");
    assert_eq!(server.hits(), 1);
    server.shutdown();
}

async fn read_http_request_head(
    stream: &mut tokio::net::TcpStream,
) -> Result<String, std::io::Error> {
    let mut buffer = Vec::new();
    let mut byte = [0u8; 1];
    while !buffer.ends_with(b"\r\n\r\n") {
        let read = stream.read(&mut byte).await?;
        if read == 0 {
            break;
        }
        buffer.push(byte[0]);
    }
    Ok(String::from_utf8_lossy(&buffer).into_owned())
}

async fn spawn_raw_body_server(
    body: &'static [u8],
    content_type: &'static str,
) -> Result<(String, tokio::task::JoinHandle<()>)> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.expect("accept raw fetch client");
        let _ = read_http_request_head(&mut stream)
            .await
            .expect("read raw fetch request");
        let headers = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: {content_type}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        stream
            .write_all(headers.as_bytes())
            .await
            .expect("write raw fetch headers");
        stream.write_all(body).await.expect("write raw fetch body");
    });
    Ok((format!("http://{addr}/raw.bin"), server))
}

async fn spawn_delayed_raw_body_server(
    body: &'static [u8],
    content_type: &'static str,
    delay: Duration,
) -> Result<(String, tokio::task::JoinHandle<()>)> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener
            .accept()
            .await
            .expect("accept delayed raw fetch client");
        let _ = read_http_request_head(&mut stream)
            .await
            .expect("read delayed raw fetch request");
        let headers = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: {content_type}\r\nContent-Disposition: attachment; filename=\"streamed.bin\"\r\nConnection: close\r\n\r\n",
            body.len()
        );
        stream
            .write_all(headers.as_bytes())
            .await
            .expect("write delayed raw fetch headers");
        tokio::time::sleep(delay).await;
        stream
            .write_all(body)
            .await
            .expect("write delayed raw fetch body");
    });
    Ok((format!("http://{addr}/raw-stream.bin"), server))
}

#[test]
fn request_lowers_script_fetch_metadata_into_owned_request_fields() -> Result<()> {
    let metadata = ScriptFetchRequestMetadata {
        cross_origin: Some("anonymous".to_owned()),
        referrer_policy: Some("no-referrer".to_owned()),
        document_referrer_policy: Some("strict-origin".to_owned()),
        charset: Some("utf-8".to_owned()),
        integrity: Some("sha256-test".to_owned()),
        nonce: Some("nonce-1".to_owned()),
        fetch_priority: Some(FetchPriorityHint::High),
        scheduler_priority: Some(ScriptFetchSchedulerPriority::High),
    };
    let request = Request::new("GET", "https://example.test/app.js", None, Vec::new())?
        .with_script_fetch_metadata(metadata);

    assert_eq!(
        request.subresource_request_metadata(),
        Some(&SubresourceRequestMetadata {
            referrer_policy: Some("no-referrer".to_owned()),
            document_referrer_policy: Some("strict-origin".to_owned()),
            integrity: Some("sha256-test".to_owned()),
        })
    );
    assert_eq!(
        request.priority_hints.fetch_priority,
        Some(FetchPriorityHint::High)
    );
    assert_eq!(
        request.script_scheduler_priority(),
        Some(ScriptFetchSchedulerPriority::High)
    );
    assert_eq!(request.resource_type, RequestResourceType::Script);
    Ok(())
}

#[test]
fn fetch_priority_hint_parsing_keeps_webidl_strict_and_html_case_insensitive() {
    assert_eq!(
        "high".parse::<FetchPriorityHint>().ok(),
        Some(FetchPriorityHint::High)
    );
    assert!(
        "HIGH".parse::<FetchPriorityHint>().is_err(),
        "RequestInit priority is a WebIDL enum and must stay case-sensitive"
    );
    assert_eq!(
        FetchPriorityHint::from_attribute(Some(" HIGH ")),
        Some(FetchPriorityHint::High),
        "HTML fetchpriority attribute keywords are ASCII case-insensitive"
    );
}

#[tokio::test]
async fn streaming_response_collector_ignores_interim_headers_before_final_start() -> Result<()> {
    let (start_tx, mut start_rx) = oneshot::channel();
    let (body_tx, _body_rx) = mpsc::unbounded_channel();
    let cancel_handle = FetchCancelHandle::new();
    let mut collector = StreamingResponseCollector::new(
        new_shared_browser_cookie_store(),
        start_tx,
        body_tx,
        cancel_handle,
    );
    let current_url = Url::parse("http://example.test/stream")?;
    collector.begin_request(
        None,
        current_url.clone(),
        NetworkCookieRequestContext::top_level_navigation("GET"),
        None,
        true,
        vec![],
        None,
    );

    assert!(collector.header(b"HTTP/1.1 103 Early Hints\r\n"));
    assert!(collector.header(b"Link: </style.css>; rel=preload; as=style\r\n"));
    assert!(collector.header(b"\r\n"));
    assert!(!collector.started());
    assert!(matches!(
        start_rx.try_recv(),
        Err(tokio::sync::oneshot::error::TryRecvError::Empty)
    ));

    assert!(collector.header(b"HTTP/1.1 200 OK\r\n"));
    assert!(collector.header(b"Content-Type: text/html; charset=utf-8\r\n"));
    assert!(collector.header(b"\r\n"));
    assert!(collector.started());

    let started = start_rx.await??;
    assert_eq!(started.status, 200);
    assert_eq!(started.final_url, current_url);
    assert_eq!(
        started.headers,
        vec![(
            "content-type".to_owned(),
            "text/html; charset=utf-8".to_owned()
        )]
    );
    Ok(())
}

#[tokio::test]
async fn streaming_response_collector_drops_redirect_body_before_final_response() -> Result<()> {
    let (start_tx, mut start_rx) = oneshot::channel();
    let (body_tx, mut body_rx) = mpsc::unbounded_channel();
    let cancel_handle = FetchCancelHandle::new();
    let mut collector = StreamingResponseCollector::new(
        new_shared_browser_cookie_store(),
        start_tx,
        body_tx,
        cancel_handle,
    );
    let redirected_url = Url::parse("http://example.test/redirect")?;
    let final_url = Url::parse("http://example.test/final")?;

    collector.begin_request(
        None,
        redirected_url.clone(),
        NetworkCookieRequestContext::top_level_navigation("GET"),
        None,
        true,
        vec![],
        None,
    );
    assert!(collector.header(b"HTTP/1.1 302 Found\r\n"));
    assert!(collector.header(b"Location: /final\r\n"));
    assert!(collector.header(b"Content-Type: text/html; charset=utf-8\r\n"));
    assert!(collector.header(b"\r\n"));
    assert!(!collector.started());
    assert_eq!(
        collector
            .write(b"<a href=\"/final\">Found</a>")
            .expect("redirect body write should succeed"),
        26
    );
    assert!(matches!(
        start_rx.try_recv(),
        Err(tokio::sync::oneshot::error::TryRecvError::Empty)
    ));
    assert!(matches!(
        body_rx.try_recv(),
        Err(tokio::sync::mpsc::error::TryRecvError::Empty)
    ));

    collector.begin_request(
        None,
        final_url.clone(),
        NetworkCookieRequestContext::top_level_navigation("GET"),
        None,
        true,
        vec![],
        None,
    );
    assert!(collector.header(b"HTTP/1.1 200 OK\r\n"));
    assert!(collector.header(b"Content-Type: text/html; charset=utf-8\r\n"));
    assert!(collector.header(b"\r\n"));
    assert!(collector.started());
    assert_eq!(
        collector
            .write(b"<!doctype html><html><body>ok</body></html>")
            .expect("final body write should succeed"),
        43
    );
    collector.finish_streaming_body();

    let started = start_rx.await??;
    assert_eq!(started.status, 200);
    assert_eq!(started.final_url, final_url);
    assert_eq!(
        body_rx.recv().await.as_deref(),
        Some("<!doctype html><html><body>ok</body></html>")
    );
    assert!(body_rx.recv().await.is_none());
    Ok(())
}

#[tokio::test]
async fn streaming_response_collector_treats_connection_established_reason_as_normal_response()
-> Result<()> {
    let (start_tx, start_rx) = oneshot::channel();
    let (body_tx, mut body_rx) = mpsc::unbounded_channel();
    let cancel_handle = FetchCancelHandle::new();
    let mut collector = StreamingResponseCollector::new(
        new_shared_browser_cookie_store(),
        start_tx,
        body_tx,
        cancel_handle,
    );
    let current_url = Url::parse("https://example.test/final")?;

    collector.begin_request(
        None,
        current_url.clone(),
        NetworkCookieRequestContext::top_level_navigation("GET"),
        None,
        true,
        vec![],
        None,
    );
    assert!(collector.header(b"HTTP/1.1 200 Connection established\r\n"));
    assert!(collector.header(b"Content-Type: text/html; charset=utf-8\r\n"));
    assert!(collector.header(b"\r\n"));
    assert!(collector.started());
    assert_eq!(
        collector
            .write(b"<!doctype html><html><body>ok</body></html>")
            .expect("final body write should succeed"),
        43
    );
    collector.finish_streaming_body();

    let started = start_rx.await??;
    assert_eq!(started.status, 200);
    assert_eq!(started.final_url, current_url);
    assert_eq!(
        body_rx.recv().await.as_deref(),
        Some("<!doctype html><html><body>ok</body></html>")
    );
    assert!(body_rx.recv().await.is_none());
    Ok(())
}

#[tokio::test]
async fn streaming_response_collector_respects_response_credentials_gate() -> Result<()> {
    let (start_tx, start_rx) = oneshot::channel();
    let (body_tx, _body_rx) = mpsc::unbounded_channel();
    let cookie_store = new_shared_browser_cookie_store();
    let mut collector = StreamingResponseCollector::new(
        cookie_store.clone(),
        start_tx,
        body_tx,
        FetchCancelHandle::new(),
    );
    let current_url = Url::parse("http://example.test/stream")?;

    collector.begin_request(
        None,
        current_url.clone(),
        NetworkCookieRequestContext::top_level_navigation("GET"),
        None,
        false,
        vec![],
        None,
    );
    assert!(collector.header(b"HTTP/1.1 200 OK\r\n"));
    assert!(collector.header(b"Set-Cookie: blocked=1; Path=/\r\n"));
    assert!(collector.header(b"\r\n"));

    let started = start_rx.await??;
    assert!(started.cookie_set_reports.is_empty());
    let cookie_header = crate::cookie_header_for_request(
        &cookie_store,
        &current_url,
        Request::get(current_url.as_str())?.cookie_context,
    )?;
    assert!(cookie_header.is_none());
    Ok(())
}

#[tokio::test]
async fn streaming_response_collector_writes_cache_body_only_with_writer() -> Result<()> {
    let (start_tx, _start_rx) = oneshot::channel();
    let (body_tx, mut body_rx) = mpsc::unbounded_channel();
    let cancel_handle = FetchCancelHandle::new();
    let mut collector = StreamingResponseCollector::new(
        new_shared_browser_cookie_store(),
        start_tx,
        body_tx,
        cancel_handle,
    );
    let current_url = Url::parse("http://example.test/stream")?;

    collector.begin_request(
        None,
        current_url.clone(),
        NetworkCookieRequestContext::top_level_navigation("GET"),
        None,
        true,
        vec![],
        None,
    );
    assert!(collector.header(b"HTTP/1.1 200 OK\r\n"));
    assert!(collector.header(b"Content-Type: text/html; charset=utf-8\r\n"));
    assert!(collector.header(b"\r\n"));
    assert_eq!(
        collector
            .write(b"<!doctype html><html><body>ok</body></html>")
            .expect("streaming body write should succeed"),
        43
    );
    assert_eq!(
        body_rx.recv().await.as_deref(),
        Some("<!doctype html><html><body>ok</body></html>")
    );
    assert!(collector.take_cache_body_writer().is_none());

    let (start_tx, _start_rx) = oneshot::channel();
    let (body_tx, _body_rx) = mpsc::unbounded_channel();
    let mut collector = StreamingResponseCollector::new(
        new_shared_browser_cookie_store(),
        start_tx,
        body_tx,
        FetchCancelHandle::new(),
    );
    let cache_dir = unique_test_cache_dir();
    let store = HttpCacheStore::new(&cache_dir);
    let cache_key = HttpCacheStore::key_for_url(current_url.as_str());
    let cache_body_writer = store.create_body_writer(&cache_key)?;
    collector.begin_request(
        None,
        current_url.clone(),
        NetworkCookieRequestContext::top_level_navigation("GET"),
        None,
        true,
        vec![],
        Some(cache_body_writer),
    );
    assert!(collector.header(b"HTTP/1.1 200 OK\r\n"));
    assert!(collector.header(b"Content-Type: text/html; charset=utf-8\r\n"));
    assert!(collector.header(b"\r\n"));
    assert_eq!(
        collector
            .write(b"<!doctype html><html><body>cached</body></html>")
            .expect("cacheable streaming body write should succeed"),
        47
    );
    let cache_body_writer = collector
        .take_cache_body_writer()
        .expect("cache body writer should remain attached");
    cache_body_writer.finish(HttpCacheEntryMetadata::new(
        current_url.to_string(),
        current_url.to_string(),
        200,
        Vec::new(),
        1,
        Some(2),
        Vec::new(),
    ))?;
    let cached = load_test_cache_body(&store, &cache_key)?.expect("cache body should be published");
    assert_eq!(cached, b"<!doctype html><html><body>cached</body></html>");
    let _ = std::fs::remove_dir_all(cache_dir);
    Ok(())
}

#[tokio::test]
async fn streaming_response_collector_caches_original_body_bytes() -> Result<()> {
    let (start_tx, _start_rx) = oneshot::channel();
    let (body_tx, mut body_rx) = mpsc::unbounded_channel();
    let cancel_handle = FetchCancelHandle::new();
    let mut collector = StreamingResponseCollector::new(
        new_shared_browser_cookie_store(),
        start_tx,
        body_tx,
        cancel_handle,
    );
    let current_url = Url::parse("http://example.test/binary-stream")?;
    let cache_dir = unique_test_cache_dir();
    let store = HttpCacheStore::new(&cache_dir);
    let cache_key = HttpCacheStore::key_for_url(current_url.as_str());
    let cache_body_writer = store.create_body_writer(&cache_key)?;
    let body = [b'a', 0xff, 0x80, b'b'];

    collector.begin_request(
        None,
        current_url.clone(),
        NetworkCookieRequestContext::top_level_navigation("GET"),
        None,
        true,
        vec![],
        Some(cache_body_writer),
    );
    assert!(collector.header(b"HTTP/1.1 200 OK\r\n"));
    assert!(collector.header(b"Content-Type: application/octet-stream\r\n"));
    assert!(collector.header(b"\r\n"));
    assert_eq!(
        collector
            .write(&body)
            .expect("binary streaming body write should succeed"),
        body.len()
    );
    collector.finish_streaming_body();

    let cache_body_writer = collector
        .take_cache_body_writer()
        .expect("cache body writer should remain attached");
    cache_body_writer.finish(HttpCacheEntryMetadata::new(
        current_url.to_string(),
        current_url.to_string(),
        200,
        Vec::new(),
        1,
        Some(2),
        Vec::new(),
    ))?;
    let cached = load_test_cache_body(&store, &cache_key)?.expect("cache body should be published");
    assert_eq!(cached, body);
    assert_eq!(body_rx.recv().await.as_deref(), Some("a"));
    assert_eq!(body_rx.recv().await.as_deref(), Some("\u{FFFD}"));
    assert_eq!(body_rx.recv().await.as_deref(), Some("\u{FFFD}"));
    assert_eq!(body_rx.recv().await.as_deref(), Some("b"));
    assert!(body_rx.recv().await.is_none());

    let _ = std::fs::remove_dir_all(cache_dir);
    Ok(())
}

#[tokio::test]
async fn fetch_html_stream_drops_redirect_body_chunks() -> Result<()> {
    let server = ScriptedHttpServer::spawn(vec![
        ScriptedResponse::status(302, "<a href=\"/final\">Found</a>")
            .with_header("Location", "/final")
            .with_header("Content-Type", "text/html; charset=utf-8"),
        ScriptedResponse::ok("<!doctype html><html><body>ok</body></html>")
            .with_header("Content-Type", "text/html; charset=utf-8"),
    ]);

    let client = FetchClient::new(&FetchConfig::default(), new_shared_browser_cookie_store());
    let mut response = client
        .fetch_html_stream(Request::get(&server.url()).unwrap())
        .await?;

    let mut body = String::new();
    while let Some(chunk) = response.next_chunk().await {
        body.push_str(&chunk);
    }
    response.finish().await?;

    assert_eq!(response.status, 200);
    assert!(response.redirected);
    assert_eq!(response.redirect_chain.len(), 1);
    let head = response.head();
    assert_eq!(head.status, 200);
    assert!(head.redirected);
    assert_eq!(head.redirect_chain.len(), 1);
    assert_eq!(body, "<!doctype html><html><body>ok</body></html>");

    server.shutdown();
    Ok(())
}

#[tokio::test]
async fn fetch_html_stream_uses_disk_cache_for_safe_gets() -> Result<()> {
    let cache_dir = unique_test_cache_dir();
    let server = ScriptedHttpServer::spawn(vec![
        ScriptedResponse::ok("<!doctype html><html><body>hit-1</body></html>")
            .with_header("Content-Type", "text/html; charset=utf-8")
            .with_header("Cache-Control", "max-age=60"),
    ]);

    let mut config = FetchConfig::default();
    config.set_http_cache_dir(Some(cache_dir.display().to_string()));

    let client = FetchClient::new(&config, new_shared_browser_cookie_store());
    let mut first = client
        .fetch_html_stream(Request::get(&server.url()).unwrap())
        .await?;
    let mut first_body = String::new();
    while let Some(chunk) = first.next_chunk().await {
        first_body.push_str(&chunk);
    }
    first.finish().await?;

    let client = FetchClient::new(&config, new_shared_browser_cookie_store());
    let mut second = client
        .fetch_html_stream(Request::get(&server.url()).unwrap())
        .await?;
    let mut second_body = String::new();
    while let Some(chunk) = second.next_chunk().await {
        second_body.push_str(&chunk);
    }
    second.finish().await?;

    assert_eq!(first_body, "<!doctype html><html><body>hit-1</body></html>");
    assert_eq!(
        second_body,
        "<!doctype html><html><body>hit-1</body></html>"
    );
    assert_eq!(server.hits(), 1);

    server.shutdown();
    let _ = std::fs::remove_dir_all(cache_dir);
    Ok(())
}

#[tokio::test]
async fn fetch_html_stream_does_not_create_cache_entry_for_no_store() -> Result<()> {
    let cache_dir = unique_test_cache_dir();
    let server = ScriptedHttpServer::spawn(vec![
        ScriptedResponse::ok("<!doctype html><html><body>hit-1</body></html>")
            .with_header("Content-Type", "text/html; charset=utf-8")
            .with_header("Cache-Control", "no-store"),
    ]);

    let mut config = FetchConfig::default();
    config.set_http_cache_dir(Some(cache_dir.display().to_string()));

    let client = FetchClient::new(&config, new_shared_browser_cookie_store());
    let mut response = client
        .fetch_html_stream(Request::get(&server.url()).unwrap())
        .await?;
    while response.next_chunk().await.is_some() {}
    response.finish().await?;

    assert_eq!(server.hits(), 1);
    assert!(
        !cache_dir.exists() || fs::read_dir(&cache_dir)?.next().is_none(),
        "uncacheable streaming responses should not create temp entry dirs"
    );

    server.shutdown();
    let _ = std::fs::remove_dir_all(cache_dir);
    Ok(())
}

#[tokio::test]
async fn fetch_raw_preserves_binary_body_bytes() -> Result<()> {
    const BODY: &[u8] = b"\x00\xffraw-download\x80\xfe";
    let (url, server) = spawn_raw_body_server(BODY, "application/octet-stream").await?;
    let client = FetchClient::new(&FetchConfig::default(), new_shared_browser_cookie_store());

    let response = client.fetch_raw(Request::get(&url)?).await?;

    assert_eq!(response.status, 200);
    assert!(
        response
            .headers
            .iter()
            .any(|(name, value)| { name == "content-type" && value == "application/octet-stream" })
    );
    assert_eq!(response.body_bytes(), BODY);

    server.await.expect("raw fetch server should finish");
    Ok(())
}

#[tokio::test]
async fn fetch_preserves_binary_body_bytes_on_text_response() -> Result<()> {
    const BODY: &[u8] = b"\x00\xfffetch-download\x80\xfe";
    let (url, server) = spawn_raw_body_server(BODY, "application/octet-stream").await?;
    let client = FetchClient::new(&FetchConfig::default(), new_shared_browser_cookie_store());

    let response = client.fetch(Request::get(&url)?).await?;

    assert_eq!(response.status, 200);
    assert_eq!(response.body_bytes(), BODY);
    assert_ne!(
        response.body_text().as_bytes(),
        BODY,
        "lossy text preview should not be used as the byte source"
    );

    server
        .await
        .expect("binary text response server should finish");
    Ok(())
}

#[tokio::test]
async fn fetch_raw_stream_yields_headers_before_delayed_body_and_preserves_bytes() -> Result<()> {
    const BODY: &[u8] = b"\x00\xffstreamed-download\x80\xfe";
    let (url, server) =
        spawn_delayed_raw_body_server(BODY, "application/octet-stream", Duration::from_millis(150))
            .await?;
    let client = FetchClient::new(&FetchConfig::default(), new_shared_browser_cookie_store());
    let start = Instant::now();
    let mut response = client
        .fetch_raw_stream_with_cancel(Request::get(&url)?, FetchCancelHandle::new())
        .await?;

    assert!(
        start.elapsed() < Duration::from_millis(120),
        "streaming raw response should become available from headers before the delayed body arrives"
    );
    assert_eq!(response.status, 200);
    assert_eq!(response.final_url.as_str(), url);
    assert!(!response.redirected);
    assert!(response.redirect_chain.is_empty());
    assert!(response.cookie_set_reports.is_empty());
    assert!(response.headers.iter().any(|(name, value)| {
        name == "content-disposition" && value == "attachment; filename=\"streamed.bin\""
    }));

    let mut body = Vec::new();
    while let Some(chunk) = response.next_chunk().await {
        body.extend_from_slice(&chunk);
    }
    response.finish().await?;
    assert_eq!(body, BODY);

    server
        .await
        .expect("delayed raw streaming fetch server should finish");
    Ok(())
}

#[tokio::test]
async fn fetch_raw_stream_preserves_redirect_metadata() -> Result<()> {
    let server = ScriptedHttpServer::spawn(vec![
        ScriptedResponse::status(302, "Found").with_header("Location", "/final.bin"),
        ScriptedResponse::ok("final-body").with_header("Content-Type", "application/octet-stream"),
    ]);
    let client = FetchClient::new(&FetchConfig::default(), new_shared_browser_cookie_store());
    let mut response = client
        .fetch_raw_stream_with_cancel(
            Request::get(&server.url_path("/redirect.bin"))?,
            FetchCancelHandle::new(),
        )
        .await?;

    assert_eq!(response.status, 200);
    assert_eq!(response.final_url.as_str(), server.url_path("/final.bin"));
    assert!(response.redirected);
    assert_eq!(response.redirect_chain.len(), 1);
    assert_eq!(response.redirect_chain[0].status, 302);
    assert_eq!(
        response.redirect_chain[0].to_url.as_str(),
        server.url_path("/final.bin")
    );

    let mut body = Vec::new();
    while let Some(chunk) = response.next_chunk().await {
        body.extend_from_slice(&chunk);
    }
    response.finish().await?;
    assert_eq!(body, b"final-body");
    server.shutdown();
    Ok(())
}

#[tokio::test]
async fn followed_http_redirect_to_file_is_rejected_without_opening_the_target() -> Result<()> {
    let server = ScriptedHttpServer::spawn(vec![
        ScriptedResponse::status(302, "Found")
            .with_header("Location", "file:///moli-policy-must-not-open"),
    ]);
    let client = FetchClient::new(&FetchConfig::default(), new_shared_browser_cookie_store());

    let error = client
        .fetch_raw_stream_with_cancel(
            Request::get(&server.url_path("/redirect-to-file"))?,
            FetchCancelHandle::new(),
        )
        .await
        .expect_err("followed redirect to file must fail");

    assert!(
        error
            .to_string()
            .contains("URL scheme \"file\" is not supported by the HTTP network transport.")
    );
    assert_eq!(server.hits(), 1);
    server.shutdown();
    Ok(())
}

#[tokio::test]
async fn manual_http_redirect_to_file_remains_observable_without_being_followed() -> Result<()> {
    let server = ScriptedHttpServer::spawn(vec![
        ScriptedResponse::status(302, "Found")
            .with_header("Location", "file:///moli-policy-must-not-open"),
    ]);
    let client = FetchClient::new(&FetchConfig::default(), new_shared_browser_cookie_store());

    let response = client
        .fetch(
            Request::get(&server.url_path("/manual-redirect-to-file"))?
                .with_redirect_mode(RequestRedirectMode::Manual),
        )
        .await?;

    assert_eq!(response.status, 302);
    assert!(response.headers.iter().any(|(name, value)| {
        name.eq_ignore_ascii_case("location") && value == "file:///moli-policy-must-not-open"
    }));
    assert_eq!(server.hits(), 1);
    server.shutdown();
    Ok(())
}

#[tokio::test]
async fn fetch_raw_stream_manual_redirect_returns_redirect_response() -> Result<()> {
    let server = ScriptedHttpServer::spawn(vec![
        ScriptedResponse::status(302, "Found")
            .with_header("Location", "/final.bin")
            .with_body("redirect-body"),
        ScriptedResponse::ok("final-body").with_header("Content-Type", "application/octet-stream"),
    ]);
    let client = FetchClient::new(&FetchConfig::default(), new_shared_browser_cookie_store());
    let mut response = client
        .fetch_raw_stream_with_cancel(
            Request::get(&server.url_path("/redirect.bin"))?
                .with_redirect_mode(RequestRedirectMode::Manual),
            FetchCancelHandle::new(),
        )
        .await?;

    assert_eq!(response.status, 302);
    assert_eq!(
        response.final_url.as_str(),
        server.url_path("/redirect.bin")
    );
    assert!(!response.redirected);
    assert!(response.redirect_chain.is_empty());
    assert!(
        response.headers.iter().any(|(name, value)| {
            name.eq_ignore_ascii_case("location") && value == "/final.bin"
        })
    );

    let mut body = Vec::new();
    while let Some(chunk) = response.next_chunk().await {
        body.extend_from_slice(&chunk);
    }
    response.finish().await?;
    assert!(body.is_empty());
    assert_eq!(
        server.requests().len(),
        1,
        "manual raw redirect must not follow to the final URL"
    );
    server.shutdown();
    Ok(())
}

#[tokio::test]
async fn fetch_raw_stream_treats_switching_protocols_as_final_response() -> Result<()> {
    let server = ScriptedHttpServer::spawn(vec![
        ScriptedResponse::status(101, "Switching Protocols").with_body("HTTP Response Status"),
    ]);
    let client = FetchClient::new(&FetchConfig::default(), new_shared_browser_cookie_store());
    let mut response = client
        .fetch_raw_stream_with_cancel(Request::get(&server.url())?, FetchCancelHandle::new())
        .await?;

    assert_eq!(response.status, 101);
    assert_eq!(response.final_url.as_str(), server.url());
    assert!(!response.redirected);
    assert!(response.redirect_chain.is_empty());

    let mut body = Vec::new();
    while let Some(chunk) = response.next_chunk().await {
        body.extend_from_slice(&chunk);
    }
    response.finish().await?;
    assert!(body.is_empty());
    server.shutdown();
    Ok(())
}

#[tokio::test]
async fn fetch_raw_stream_treats_https11_switching_protocols_as_final_response() -> Result<()> {
    let server = ScriptedHttps11Server::spawn(vec![
        ScriptedResponse::status(101, "Switching Protocols").with_body("HTTP Response Status"),
    ]);
    let mut config = FetchConfig::default();
    config.set_tls_verify_host(false);
    let client = FetchClient::new(&config, new_shared_browser_cookie_store());
    let mut response = client
        .fetch_raw_stream_with_cancel(Request::get(&server.url())?, FetchCancelHandle::new())
        .await?;

    assert_eq!(response.status, 101);
    assert_eq!(response.final_url.as_str(), server.url());
    assert!(!response.redirected);
    assert!(response.redirect_chain.is_empty());

    let mut body = Vec::new();
    while let Some(chunk) = response.next_chunk().await {
        body.extend_from_slice(&chunk);
    }
    response.finish().await?;
    assert!(body.is_empty());
    assert_eq!(server.hits(), 1);
    assert_eq!(server.requests(), ["/cache".to_owned()]);
    server.shutdown();
    Ok(())
}

#[test]
fn web_bot_auth_resigns_restarts_redirects_and_subresources() -> Result<()> {
    let server = ScriptedHttps11Server::spawn(vec![
        ScriptedResponse::status(403, "Challenge")
            .with_header("Accept-CH", "Sec-CH-UA-Arch")
            .with_header("Critical-CH", "Sec-CH-UA-Arch"),
        ScriptedResponse::status(302, "Found").with_header("Location", "/final"),
        ScriptedResponse::ok("final"),
        ScriptedResponse::ok("asset"),
    ]);
    let mut config = FetchConfig::default();
    config.set_tls_verify_host(false);
    config.set_web_bot_auth(Some(test_web_bot_auth_signer()));
    let client = FetchClient::new(&config, new_shared_browser_cookie_store());

    let response = fetch_response_for_test(&client, Request::get(&server.url_path("/start"))?)?;
    assert_eq!(response.body_text(), "final");
    assert_eq!(response.redirect_chain.len(), 2);
    assert_eq!(response.redirect_chain[0].status, 307);
    assert_eq!(response.redirect_chain[1].status, 302);

    let final_url = Url::parse(&server.url_path("/final"))?;
    let subresource = Request::new(
        "POST",
        &server.url_path("/asset"),
        Some("probe".to_owned()),
        Vec::new(),
    )?
    .with_browser_request_metadata(BrowserRequestMetadata::Fetch)
    .with_initiator_url(&final_url);
    assert_eq!(
        fetch_response_for_test(&client, subresource)?.body_text(),
        "asset"
    );

    assert_eq!(
        server.requests(),
        [
            "/start".to_owned(),
            "/start".to_owned(),
            "/final".to_owned(),
            "/asset".to_owned(),
        ]
    );
    let request_heads = server.request_heads();
    assert_eq!(request_heads.len(), 4);
    for request_head in &request_heads {
        assert_eq!(
            request_head_header_value(request_head, "Signature-Agent"),
            Some("\"https://bot.example\"")
        );
        let signature_input = request_head_header_value(request_head, "Signature-Input")
            .expect("signed request should include Signature-Input");
        assert!(
            signature_input.contains("(\"@authority\" \"@method\" \"@path\" \"signature-agent\")")
        );
        assert!(request_head_header_value(request_head, "Signature").is_some());
    }
    assert!(request_heads[3].starts_with("POST /asset HTTP/1.1\r\n"));

    let nonces = request_heads
        .iter()
        .map(|request_head| {
            nonce_from_signature_input(
                request_head_header_value(request_head, "Signature-Input").unwrap(),
            )
            .to_owned()
        })
        .collect::<BTreeSet<_>>();
    assert_eq!(nonces.len(), request_heads.len());

    let restart = &response.redirect_chain[0];
    assert_request_extra_signature_matches_wire(
        &restart
            .response_extra_info
            .as_ref()
            .expect("restart response extra info")
            .request_extra_info,
        &request_heads[0],
    );
    assert_request_extra_signature_matches_wire(
        restart
            .request_extra_info
            .as_ref()
            .expect("restarted request extra info"),
        &request_heads[1],
    );
    let redirect = &response.redirect_chain[1];
    assert_request_extra_signature_matches_wire(
        &redirect
            .response_extra_info
            .as_ref()
            .expect("redirect response extra info")
            .request_extra_info,
        &request_heads[1],
    );
    assert_request_extra_signature_matches_wire(
        redirect
            .request_extra_info
            .as_ref()
            .expect("redirect target request extra info"),
        &request_heads[2],
    );
    assert_request_extra_signature_matches_wire(
        response
            .network_request_extra_info()
            .expect("final request extra info"),
        &request_heads[2],
    );

    server.shutdown();
    Ok(())
}

#[test]
fn web_bot_auth_is_not_sent_over_plain_http() -> Result<()> {
    let server = ScriptedHttpServer::spawn(vec![ScriptedResponse::ok("plain")]);
    let mut config = FetchConfig::default();
    config.set_web_bot_auth(Some(test_web_bot_auth_signer()));

    let response = fetch_with_config_for_test(&config, Request::get(&server.url())?)?;
    assert_eq!(response.body_text(), "plain");
    let requests = server.requests();
    assert_eq!(requests.len(), 1);
    for name in ["Signature-Agent", "Signature-Input", "Signature"] {
        assert_eq!(request_head_header_value(&requests[0], name), None);
    }

    server.shutdown();
    Ok(())
}

#[test]
fn web_bot_auth_bypasses_shared_http_cache() -> Result<()> {
    let cache_dir = unique_test_cache_dir();
    let server = ScriptedHttps11Server::spawn(vec![
        ScriptedResponse::ok("first").with_header("Cache-Control", "max-age=60"),
        ScriptedResponse::ok("second").with_header("Cache-Control", "max-age=60"),
    ]);
    let mut config = FetchConfig::default();
    config.set_tls_verify_host(false);
    config.set_http_cache_dir(Some(cache_dir.display().to_string()));
    config.set_web_bot_auth(Some(test_web_bot_auth_signer()));

    let first = fetch_with_config_for_test(&config, Request::get(&server.url())?)?;
    let second = fetch_with_config_for_test(&config, Request::get(&server.url())?)?;
    assert_eq!(first.body_text(), "first");
    assert_eq!(second.body_text(), "second");
    assert!(!first.from_cache);
    assert!(!second.from_cache);
    assert_eq!(server.hits(), 2);
    let request_heads = server.request_heads();
    assert_ne!(
        request_head_header_value(&request_heads[0], "Signature-Input"),
        request_head_header_value(&request_heads[1], "Signature-Input")
    );
    assert!(
        !cache_dir.exists() || fs::read_dir(&cache_dir)?.next().is_none(),
        "authenticated responses must not enter the shared disk cache"
    );

    server.shutdown();
    let _ = fs::remove_dir_all(cache_dir);
    Ok(())
}

#[tokio::test]
async fn fetch_raw_stream_finishes_switching_protocols_body_without_connection_close() -> Result<()>
{
    let server = ScriptedHttps11Server::spawn(vec![
        ScriptedResponse::status(101, "Switching Protocols")
            .with_body("HTTP Response Status")
            .with_hold_open_ms(2_000),
    ]);
    let mut config = FetchConfig::default();
    config.set_tls_verify_host(false);
    let client = FetchClient::new(&config, new_shared_browser_cookie_store());
    let mut response = client
        .fetch_raw_stream_with_cancel(Request::get(&server.url())?, FetchCancelHandle::new())
        .await?;

    assert_eq!(response.status, 101);
    assert_eq!(response.final_url.as_str(), server.url());

    let body = tokio::time::timeout(Duration::from_millis(500), async {
        let mut body = Vec::new();
        while let Some(chunk) = response.next_chunk().await {
            body.extend_from_slice(&chunk);
        }
        response.finish().await?;
        anyhow::Ok(body)
    })
    .await
    .expect("101 streaming body should finish before the keep-alive connection closes")?;
    assert!(body.is_empty());
    assert_eq!(server.hits(), 1);
    server.shutdown();
    Ok(())
}

#[tokio::test]
async fn fetch_raw_stream_finishes_null_body_status_without_connection_close() -> Result<()> {
    for (status, reason) in [(204, "No Content"), (205, "Reset Content")] {
        let server = ScriptedHttps11Server::spawn(vec![
            ScriptedResponse::status(status, reason)
                .with_body("HTTP Response Status")
                .with_hold_open_ms(2_000),
        ]);
        let mut config = FetchConfig::default();
        config.set_tls_verify_host(false);
        let client = FetchClient::new(&config, new_shared_browser_cookie_store());
        let mut response = client
            .fetch_raw_stream_with_cancel(Request::get(&server.url())?, FetchCancelHandle::new())
            .await?;

        assert_eq!(response.status, status);

        let body = tokio::time::timeout(Duration::from_millis(500), async {
            let mut body = Vec::new();
            while let Some(chunk) = response.next_chunk().await {
                body.extend_from_slice(&chunk);
            }
            response.finish().await?;
            anyhow::Ok(body)
        })
        .await
        .with_context(|| format!("{status} streaming body should finish at headers"))??;
        assert!(body.is_empty());
        assert_eq!(server.hits(), 1);
        server.shutdown();
    }
    Ok(())
}

#[tokio::test]
async fn fetch_redirect_303_rewrites_post_to_get_and_drops_body_headers() -> Result<()> {
    let server = ScriptedHttpServer::spawn(vec![
        ScriptedResponse::status(303, "See Other").with_header("Location", "/final"),
        ScriptedResponse::ok("final-body"),
    ]);
    let client = FetchClient::new(&FetchConfig::default(), new_shared_browser_cookie_store());
    let response = client
        .fetch(Request::new(
            "POST",
            &server.url_path("/redirect"),
            Some("payload".to_owned()),
            vec![
                ("Content-Type".to_owned(), "text/plain".to_owned()),
                ("X-Keep".to_owned(), "1".to_owned()),
            ],
        )?)
        .await?;

    assert_eq!(response.status, 200);
    assert_eq!(response.body_text(), "final-body");
    assert!(response.redirected);

    let requests = server.requests();
    assert_eq!(requests.len(), 2);
    assert!(requests[0].starts_with("POST /redirect HTTP/1.1"));
    assert!(requests[1].starts_with("GET /final HTTP/1.1"));
    assert!(
        !requests[1]
            .to_ascii_lowercase()
            .contains("content-type: text/plain"),
        "rewritten GET request should not keep Content-Type: {}",
        requests[1]
    );
    assert!(
        !requests[1].contains("payload"),
        "rewritten GET request should not keep the POST body: {}",
        requests[1]
    );
    assert!(
        requests[1].contains("X-Keep: 1"),
        "non body header should be preserved: {}",
        requests[1]
    );
    server.shutdown();
    Ok(())
}

#[tokio::test]
async fn fetch_redirect_307_preserves_post_method_and_body_headers() -> Result<()> {
    let server = ScriptedHttpServer::spawn(vec![
        ScriptedResponse::status(307, "Temporary Redirect").with_header("Location", "/final"),
        ScriptedResponse::ok("final-body"),
    ]);
    let client = FetchClient::new(&FetchConfig::default(), new_shared_browser_cookie_store());
    let response = client
        .fetch(Request::new(
            "POST",
            &server.url_path("/redirect"),
            Some("payload".to_owned()),
            vec![("Content-Type".to_owned(), "text/plain".to_owned())],
        )?)
        .await?;

    assert_eq!(response.status, 200);
    assert_eq!(response.body_text(), "final-body");
    assert!(response.redirected);

    let requests = server.requests();
    assert_eq!(requests.len(), 2);
    assert!(requests[0].starts_with("POST /redirect HTTP/1.1"));
    assert!(requests[1].starts_with("POST /final HTTP/1.1"));
    assert!(
        requests[1]
            .to_ascii_lowercase()
            .contains("content-type: text/plain"),
        "307 redirect should preserve request body headers: {}",
        requests[1]
    );
    server.shutdown();
    Ok(())
}

#[tokio::test]
async fn fetch_raw_stream_uses_disk_cache_for_safe_gets() -> Result<()> {
    let cache_dir = unique_test_cache_dir();
    let server = ScriptedHttpServer::spawn(vec![
        ScriptedResponse::ok("cached-raw-stream")
            .with_header("Content-Type", "application/octet-stream")
            .with_header("Cache-Control", "max-age=60"),
    ]);
    let mut config = FetchConfig::default();
    config.set_http_cache_dir(Some(cache_dir.display().to_string()));

    let client = FetchClient::new(&config, new_shared_browser_cookie_store());
    let mut first = client
        .fetch_raw_stream_with_cancel(Request::get(&server.url())?, FetchCancelHandle::new())
        .await?;
    assert!(!first.from_cache);
    let mut first_body = Vec::new();
    while let Some(chunk) = first.next_chunk().await {
        first_body.extend_from_slice(&chunk);
    }
    first.finish().await?;

    let client = FetchClient::new(&config, new_shared_browser_cookie_store());
    let mut second = client
        .fetch_raw_stream_with_cancel(Request::get(&server.url())?, FetchCancelHandle::new())
        .await?;
    assert!(second.from_cache);
    let mut second_body = Vec::new();
    while let Some(chunk) = second.next_chunk().await {
        second_body.extend_from_slice(&chunk);
    }
    second.finish().await?;

    assert_eq!(first_body, b"cached-raw-stream");
    assert_eq!(second_body, b"cached-raw-stream");
    assert_eq!(server.hits(), 1);

    server.shutdown();
    let _ = std::fs::remove_dir_all(cache_dir);
    Ok(())
}

#[tokio::test]
async fn cache_mode_bypass_skips_fresh_cache_read_and_replaces_entry() -> Result<()> {
    let cache_dir = unique_test_cache_dir();
    let server = ScriptedHttpServer::spawn(vec![
        ScriptedResponse::ok("cached-v1").with_header("Cache-Control", "max-age=60"),
        ScriptedResponse::ok("network-v2").with_header("Cache-Control", "max-age=60"),
    ]);
    let mut config = FetchConfig::default();
    config.set_http_cache_dir(Some(cache_dir.display().to_string()));
    let client = FetchClient::new(&config, new_shared_browser_cookie_store());

    let first = client.fetch_raw(Request::get(&server.url())?).await?;
    let bypassed = client
        .fetch_raw(Request::get(&server.url())?.with_cache_mode(crate::RequestCacheMode::Bypass))
        .await?;
    let after_bypass = client.fetch_raw(Request::get(&server.url())?).await?;

    assert_eq!(first.body_bytes(), b"cached-v1");
    assert_eq!(bypassed.body_bytes(), b"network-v2");
    assert!(!bypassed.from_cache);
    assert_eq!(after_bypass.body_bytes(), b"network-v2");
    assert!(after_bypass.from_cache);
    assert_eq!(server.hits(), 2);

    server.shutdown();
    let _ = std::fs::remove_dir_all(cache_dir);
    Ok(())
}

#[tokio::test]
async fn fetch_raw_stream_uses_disk_cache_for_redirect_hops() -> Result<()> {
    let cache_dir = unique_test_cache_dir();
    let server = ScriptedHttpServer::spawn(vec![
        ScriptedResponse::status(301, "Moved Permanently")
            .with_header("Location", "/final")
            .with_header("Cache-Control", "max-age=60"),
        ScriptedResponse::ok("final-body")
            .with_header("Content-Type", "application/octet-stream")
            .with_header("Cache-Control", "max-age=60"),
    ]);
    let mut config = FetchConfig::default();
    config.set_http_cache_dir(Some(cache_dir.display().to_string()));

    let client = FetchClient::new(&config, new_shared_browser_cookie_store());
    let mut first = client
        .fetch_raw_stream_with_cancel(
            Request::get(&server.url_path("/redirect"))?,
            FetchCancelHandle::new(),
        )
        .await?;
    assert!(!first.from_cache);
    assert_eq!(first.redirect_chain.len(), 1);
    assert!(!first.redirect_chain[0].from_cache);
    let mut first_body = Vec::new();
    while let Some(chunk) = first.next_chunk().await {
        first_body.extend_from_slice(&chunk);
    }
    first.finish().await?;

    let client = FetchClient::new(&config, new_shared_browser_cookie_store());
    let mut second = client
        .fetch_raw_stream_with_cancel(
            Request::get(&server.url_path("/redirect"))?,
            FetchCancelHandle::new(),
        )
        .await?;
    assert!(second.from_cache);
    assert_eq!(second.redirect_chain.len(), 1);
    assert!(second.redirect_chain[0].from_cache);
    let mut second_body = Vec::new();
    while let Some(chunk) = second.next_chunk().await {
        second_body.extend_from_slice(&chunk);
    }
    second.finish().await?;

    assert_eq!(first_body, b"final-body");
    assert_eq!(second_body, b"final-body");
    assert_eq!(server.hits(), 2);

    server.shutdown();
    let _ = std::fs::remove_dir_all(cache_dir);
    Ok(())
}

#[tokio::test]
async fn fetch_raw_stream_delivers_response_when_cache_entry_limit_is_exceeded() -> Result<()> {
    let cache_dir = unique_test_cache_dir();
    let server = ScriptedHttpServer::spawn(vec![
        ScriptedResponse::ok("too-large-for-cache")
            .with_header("Content-Type", "application/octet-stream")
            .with_header("Cache-Control", "max-age=60"),
        ScriptedResponse::ok("network-refetch")
            .with_header("Content-Type", "application/octet-stream")
            .with_header("Cache-Control", "max-age=60"),
    ]);
    let mut config = FetchConfig::default();
    config.set_http_cache_dir(Some(cache_dir.display().to_string()));
    config.set_http_cache_max_bytes(Some(4));

    let client = FetchClient::new(&config, new_shared_browser_cookie_store());
    let mut first = client
        .fetch_raw_stream_with_cancel(Request::get(&server.url())?, FetchCancelHandle::new())
        .await?;
    let mut first_body = Vec::new();
    while let Some(chunk) = first.next_chunk().await {
        first_body.extend_from_slice(&chunk);
    }
    first.finish().await?;

    let mut second = client
        .fetch_raw_stream_with_cancel(Request::get(&server.url())?, FetchCancelHandle::new())
        .await?;
    let mut second_body = Vec::new();
    while let Some(chunk) = second.next_chunk().await {
        second_body.extend_from_slice(&chunk);
    }
    second.finish().await?;

    assert_eq!(first_body, b"too-large-for-cache");
    assert_eq!(second_body, b"network-refetch");
    assert_eq!(server.hits(), 2);

    server.shutdown();
    let _ = std::fs::remove_dir_all(cache_dir);
    Ok(())
}

#[test]
fn fetch_client_startup_trims_existing_disk_cache_to_quota() -> Result<()> {
    let cache_dir = unique_test_cache_dir();
    let seed_store = HttpCacheStore::new(&cache_dir);
    let first_url = "http://example.test/first";
    let second_url = "http://example.test/second";
    let third_url = "http://example.test/third";
    let body = vec![b'x'; 1024];
    for (url, stored_at) in [(first_url, 1), (second_url, 2), (third_url, 3)] {
        store_test_cache_body(
            &seed_store,
            &HttpCacheStore::key_for_url(url),
            HttpCacheEntryMetadata::new(
                url.to_owned(),
                url.to_owned(),
                200,
                Vec::new(),
                stored_at,
                Some(stored_at + 60),
                Vec::new(),
            ),
            &body,
        )?;
    }
    assert_eq!(seed_store.entries()?.len(), 3);

    let mut config = FetchConfig::default();
    config.set_http_cache_dir(Some(cache_dir.display().to_string()));
    config.set_http_cache_max_bytes(Some(3_000));
    let _client = FetchClient::new(&config, new_shared_browser_cookie_store());

    let trimming_store = HttpCacheStore::with_max_bytes(&cache_dir, Some(3_000));
    assert!(
        trimming_store
            .load_reader(&HttpCacheStore::key_for_url(first_url))?
            .is_none(),
        "oldest existing cache entry should be pruned during client startup"
    );
    assert!(
        trimming_store
            .load_reader(&HttpCacheStore::key_for_url(second_url))?
            .is_some()
    );
    assert!(
        trimming_store
            .load_reader(&HttpCacheStore::key_for_url(third_url))?
            .is_some()
    );

    let _ = std::fs::remove_dir_all(cache_dir);
    Ok(())
}

#[tokio::test]
async fn fetch_raw_compat_uses_streaming_disk_cache_for_safe_gets() -> Result<()> {
    let cache_dir = unique_test_cache_dir();
    let server = ScriptedHttpServer::spawn(vec![
        ScriptedResponse::ok("cached-raw-compat")
            .with_header("Content-Type", "application/octet-stream")
            .with_header("Cache-Control", "max-age=60"),
    ]);
    let mut config = FetchConfig::default();
    config.set_http_cache_dir(Some(cache_dir.display().to_string()));

    let client = FetchClient::new(&config, new_shared_browser_cookie_store());
    let first = client.fetch_raw(Request::get(&server.url())?).await?;

    let client = FetchClient::new(&config, new_shared_browser_cookie_store());
    let second = client.fetch_raw(Request::get(&server.url())?).await?;

    assert_eq!(first.body_bytes(), b"cached-raw-compat");
    assert_eq!(second.body_bytes(), b"cached-raw-compat");
    assert_eq!(server.hits(), 1);

    server.shutdown();
    let _ = std::fs::remove_dir_all(cache_dir);
    Ok(())
}

#[test]
fn fetch_client_rejects_private_network_targets() {
    let mut config = FetchConfig::default();
    config.set_network_blocking(true, vec![]);

    let error = fetch_with_config_for_test(&config, Request::get("http://127.0.0.1/").unwrap())
        .unwrap_err();
    let error_chain = format!("{error:#}");

    assert!(
        error_chain.contains("blocked private network address `127.0.0.1`"),
        "unexpected error: {error_chain}"
    );
}

#[test]
fn fetch_client_rejects_configured_cidrs() {
    let mut config = FetchConfig::default();
    config.set_network_blocking(false, vec!["198.18.0.0/15".parse().unwrap()]);

    let error = fetch_with_config_for_test(&config, Request::get("http://198.18.0.1/").unwrap())
        .unwrap_err();
    let error_chain = format!("{error:#}");

    assert!(
        error_chain.contains("matches `198.18.0.0/15`"),
        "unexpected error: {error_chain}"
    );
}

#[test]
fn fetch_client_rejects_http_bad_ports_before_network_io() {
    let error = fetch_with_config_for_test(
        &FetchConfig::default(),
        Request::get("http://example.test:25/").unwrap(),
    )
    .unwrap_err();
    let error_chain = format!("{error:#}");

    assert!(
        error_chain.contains("blocked bad port for `http://example.test:25/`"),
        "unexpected error: {error_chain}"
    );
}

#[test]
fn fetch_client_preserves_libcurl_env_proxy_fallback() {
    let proxy = ScriptedHttpServer::spawn(vec![ScriptedResponse::ok("proxied-env")]);
    let proxy_origin = proxy.origin();
    let output = Command::new(std::env::current_exe().expect("test binary path"))
        .arg("--exact")
        .arg("tests::env_proxy_child_uses_libcurl_env_proxy_fallback")
        .arg("--ignored")
        .arg("--nocapture")
        .env(ENV_PROXY_CHILD_TEST, "1")
        .env(ENV_PROXY_URL, &proxy_origin)
        .env("http_proxy", &proxy_origin)
        .env_remove("HTTP_PROXY")
        .env_remove("HTTPS_PROXY")
        .env_remove("https_proxy")
        .env_remove("ALL_PROXY")
        .env_remove("all_proxy")
        .env_remove("NO_PROXY")
        .env_remove("no_proxy")
        .output()
        .expect("child proxy fallback test should run");

    assert!(
        output.status.success(),
        "child proxy fallback test failed with status {:?}\nstdout:\n{}\nstderr:\n{}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(proxy.hits(), 1);
    let requests = proxy.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(
        request_path(&requests[0]),
        "http://env-proxy-fallback.invalid/from-env"
    );

    proxy.shutdown();
}

#[test]
#[ignore = "spawned by fetch_client_preserves_libcurl_env_proxy_fallback"]
fn env_proxy_child_uses_libcurl_env_proxy_fallback() -> Result<()> {
    if std::env::var_os(ENV_PROXY_CHILD_TEST).is_none() {
        return Ok(());
    }

    let proxy_origin = std::env::var(ENV_PROXY_URL)?;
    assert_eq!(std::env::var("http_proxy")?, proxy_origin);
    let mut config = FetchConfig::default();
    config.set_request_timeout_ms(1_000);
    config.set_connect_timeout_ms(Some(1_000));

    let response = fetch_with_config_for_test(
        &config,
        Request::get("http://env-proxy-fallback.invalid/from-env")?,
    )?;
    assert_eq!(response.body_text(), "proxied-env");
    Ok(())
}

#[tokio::test]
async fn direct_localhost_uses_shared_dns_across_all_curl_transports() -> Result<()> {
    let server = ScriptedHttpServer::spawn(vec![
        ScriptedResponse::ok("buffered-dns"),
        ScriptedResponse::ok("html-dns"),
        ScriptedResponse::ok("raw-dns"),
    ]);
    let port = Url::parse(&server.url())?
        .port()
        .context("scripted server URL should contain a port")?;
    let mut config = FetchConfig::default();
    // Disable ambient proxy environment variables so this test proves the
    // direct-origin DNS residence rather than curl's proxy resolver path.
    config.set_http_proxy(Some(String::new()));
    let client = FetchClient::new(&config, new_shared_browser_cookie_store());

    let buffered = client
        .fetch(
            Request::get(&format!("http://localhost:{port}/buffered"))?
                .with_follow_redirects(false),
        )
        .await?;
    assert_eq!(buffered.body_text(), "buffered-dns");

    let mut html = client
        .fetch_html_stream(Request::get(&format!("http://localhost:{port}/html"))?)
        .await?;
    let mut html_body = String::new();
    while let Some(chunk) = html.next_chunk().await {
        html_body.push_str(&chunk);
    }
    html.finish().await?;
    assert_eq!(html_body, "html-dns");

    let mut raw = client
        .fetch_raw_stream_with_cancel(
            Request::get(&format!("http://localhost:{port}/raw"))?,
            FetchCancelHandle::new(),
        )
        .await?;
    let mut raw_body = Vec::new();
    while let Some(chunk) = raw.next_chunk().await {
        raw_body.extend_from_slice(&chunk);
    }
    raw.finish().await?;
    assert_eq!(raw_body, b"raw-dns");
    assert_eq!(server.hits(), 3);

    assert!(client.shutdown().is_clean());
    server.shutdown();
    Ok(())
}

#[test]
fn fetch_client_uses_disk_cache_for_safe_gets() {
    let cache_dir = unique_test_cache_dir();
    let server = ScriptedHttpServer::spawn(vec![
        ScriptedResponse::ok("hit-1").with_header("Cache-Control", "max-age=60"),
    ]);

    let mut config = FetchConfig::default();
    config.set_http_cache_dir(Some(cache_dir.display().to_string()));

    let first = fetch_with_config_for_test(&config, Request::get(&server.url()).unwrap()).unwrap();
    let second = fetch_with_config_for_test(&config, Request::get(&server.url()).unwrap()).unwrap();

    assert_eq!(first.body_text(), "hit-1");
    assert_eq!(second.body_text(), "hit-1");
    assert_eq!(server.hits(), 1);

    server.shutdown();
    let _ = std::fs::remove_dir_all(cache_dir);
}

#[tokio::test]
async fn fetch_raw_stream_reuses_https_cache_when_tls_verification_is_disabled() -> Result<()> {
    let cache_dir = unique_test_cache_dir();
    let server = ScriptedHttps11Server::spawn(vec![
        ScriptedResponse::ok("cached-https-raw-stream")
            .with_header("Content-Type", "image/png")
            .with_header("Cache-Control", "max-age=60"),
    ]);

    let mut config = FetchConfig::default();
    config.set_http_cache_dir(Some(cache_dir.display().to_string()));
    config.set_tls_verify_host(false);

    let client = FetchClient::new(&config, new_shared_browser_cookie_store());
    let mut first = client
        .fetch_raw_stream_with_cancel(Request::get(&server.url())?, FetchCancelHandle::new())
        .await?;
    assert!(!first.from_cache);
    let mut first_body = Vec::new();
    while let Some(chunk) = first.next_chunk().await {
        first_body.extend_from_slice(&chunk);
    }
    first.finish().await?;

    let client = FetchClient::new(&config, new_shared_browser_cookie_store());
    let mut second = client
        .fetch_raw_stream_with_cancel(Request::get(&server.url())?, FetchCancelHandle::new())
        .await?;
    assert!(second.from_cache);
    let mut second_body = Vec::new();
    while let Some(chunk) = second.next_chunk().await {
        second_body.extend_from_slice(&chunk);
    }
    second.finish().await?;

    assert_eq!(first_body, b"cached-https-raw-stream");
    assert_eq!(second_body, b"cached-https-raw-stream");
    assert_eq!(server.hits(), 1);

    server.shutdown();
    let _ = std::fs::remove_dir_all(cache_dir);
    Ok(())
}

#[test]
fn fetch_client_disk_cache_reuses_entries_across_fragment_variants() {
    let cache_dir = unique_test_cache_dir();
    let server = ScriptedHttpServer::spawn(vec![
        ScriptedResponse::ok("hit-1").with_header("Cache-Control", "max-age=60"),
    ]);

    let mut config = FetchConfig::default();
    config.set_http_cache_dir(Some(cache_dir.display().to_string()));
    let first_url = format!("{}#one", server.url());
    let second_url = format!("{}#two", server.url());

    let first = fetch_with_config_for_test(&config, Request::get(&first_url).unwrap()).unwrap();
    let second = fetch_with_config_for_test(&config, Request::get(&second_url).unwrap()).unwrap();

    assert_eq!(first.body_text(), "hit-1");
    assert_eq!(second.body_text(), "hit-1");
    assert_eq!(second.final_url.as_str(), second_url);
    assert_eq!(server.hits(), 1);

    server.shutdown();
    let _ = std::fs::remove_dir_all(cache_dir);
}

#[test]
fn fetch_client_treats_oversized_cached_body_as_miss() {
    let cache_dir = unique_test_cache_dir();
    let server = ScriptedHttpServer::spawn(vec![
        ScriptedResponse::ok("cached-body-too-large").with_header("Cache-Control", "max-age=60"),
        ScriptedResponse::ok("ok").with_header("Cache-Control", "max-age=60"),
    ]);

    let mut write_config = FetchConfig::default();
    write_config.set_http_cache_dir(Some(cache_dir.display().to_string()));

    let mut read_config = FetchConfig::default();
    read_config.set_http_cache_dir(Some(cache_dir.display().to_string()));
    read_config.set_connection_limits(None, None, Some(4));

    let first =
        fetch_with_config_for_test(&write_config, Request::get(&server.url()).unwrap()).unwrap();
    let second =
        fetch_with_config_for_test(&read_config, Request::get(&server.url()).unwrap()).unwrap();

    assert_eq!(first.body_text(), "cached-body-too-large");
    assert_eq!(second.body_text(), "ok");
    assert_eq!(
        server.hits(),
        2,
        "oversized buffered cache hits should fall through to the network"
    );

    server.shutdown();
    let _ = std::fs::remove_dir_all(cache_dir);
}

#[test]
fn fetch_client_does_not_revalidate_oversized_cached_body() {
    let cache_dir = unique_test_cache_dir();
    let server = ScriptedHttpServer::spawn(vec![
        ScriptedResponse::ok("cached-body-too-large")
            .with_header("Cache-Control", "max-age=0")
            .with_header("ETag", "\"large\""),
        ScriptedResponse::ok("ok").with_header("Cache-Control", "max-age=60"),
    ]);

    let mut write_config = FetchConfig::default();
    write_config.set_http_cache_dir(Some(cache_dir.display().to_string()));

    let mut read_config = FetchConfig::default();
    read_config.set_http_cache_dir(Some(cache_dir.display().to_string()));
    read_config.set_connection_limits(None, None, Some(4));

    let first =
        fetch_with_config_for_test(&write_config, Request::get(&server.url()).unwrap()).unwrap();
    let second =
        fetch_with_config_for_test(&read_config, Request::get(&server.url()).unwrap()).unwrap();

    assert_eq!(first.body_text(), "cached-body-too-large");
    assert_eq!(second.body_text(), "ok");
    assert_eq!(server.hits(), 2);
    let requests = server.requests();
    assert!(
        !requests[1].to_ascii_lowercase().contains("if-none-match"),
        "oversized stale cache entries should miss instead of sending conditional validation"
    );

    server.shutdown();
    let _ = std::fs::remove_dir_all(cache_dir);
}

#[test]
fn fetch_client_refetches_when_304_cached_body_replay_exceeds_limit() {
    let cache_dir = unique_test_cache_dir();
    let server = ScriptedHttpServer::spawn(vec![
        ScriptedResponse::ok("small")
            .with_header("Cache-Control", "max-age=0")
            .with_header("ETag", "\"small\""),
        ScriptedResponse::status(304, "Not Modified")
            .with_header("Cache-Control", "max-age=60")
            .with_delay_ms(200),
        ScriptedResponse::ok("ok").with_header("Cache-Control", "max-age=60"),
    ]);

    let mut write_config = FetchConfig::default();
    write_config.set_http_cache_dir(Some(cache_dir.display().to_string()));

    let mut read_config = FetchConfig::default();
    read_config.set_http_cache_dir(Some(cache_dir.display().to_string()));
    read_config.set_connection_limits(None, None, Some(8));

    let first =
        fetch_with_config_for_test(&write_config, Request::get(&server.url()).unwrap()).unwrap();
    assert_eq!(first.body_text(), "small");

    let body_path = fs::read_dir(&cache_dir)
        .unwrap()
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .find(|path| path.is_dir() && path.extension().is_some_and(|ext| ext == "entry"))
        .and_then(|entry_dir| {
            fs::read_dir(entry_dir)
                .ok()?
                .filter_map(|entry| entry.ok())
                .find_map(|entry| {
                    let path = entry.path();
                    path.file_name()
                        .and_then(|name| name.to_str())
                        .is_some_and(|name| name.starts_with("body.") && name.ends_with(".bin"))
                        .then_some(path)
                })
        })
        .expect("cached body path should exist");

    let request_url = server.url();
    let fetch_handle = thread::spawn(move || {
        fetch_with_config_for_test(&read_config, Request::get(&request_url).unwrap())
            .expect("post-304 cached body replay failure should refetch")
    });

    let deadline = Instant::now() + Duration::from_secs(2);
    while server.hits() < 2 && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(server.hits(), 2, "second request should be revalidating");
    fs::write(&body_path, b"cached-body-too-large").unwrap();

    let second = fetch_handle.join().expect("fetch thread should not panic");

    assert_eq!(second.body_text(), "ok");
    assert_eq!(server.hits(), 3);
    let requests = server.requests();
    assert!(
        requests[1].to_ascii_lowercase().contains("if-none-match"),
        "second request should revalidate the stale cache entry: {}",
        requests[1]
    );
    assert!(
        !requests[2].to_ascii_lowercase().contains("if-none-match"),
        "retry after failed 304 merge must be an unconditional network fetch: {}",
        requests[2]
    );

    server.shutdown();
    let _ = std::fs::remove_dir_all(cache_dir);
}

#[test]
fn fetch_client_persists_disk_cache_files_to_configured_dir() {
    let cache_dir = unique_test_cache_dir();
    let server = ScriptedHttpServer::spawn(vec![
        ScriptedResponse::ok("hit-1").with_header("Cache-Control", "max-age=60"),
    ]);

    let mut config = FetchConfig::default();
    config.set_http_cache_dir(Some(cache_dir.display().to_string()));

    let response =
        fetch_with_config_for_test(&config, Request::get(&server.url()).unwrap()).unwrap();

    assert_eq!(response.body_text(), "hit-1");
    let entries = std::fs::read_dir(&cache_dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect::<Vec<_>>();
    assert_eq!(entries.len(), 1, "cache dir should contain one entry dir");
    assert!(
        entries[0].is_dir(),
        "cache should use Chromium-like directory entries"
    );
    assert!(entries[0].join("meta.json").is_file());
    assert!(
        std::fs::read_dir(&entries[0]).unwrap().any(|entry| entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with("body.")),
        "entry dir should contain a separately stored body stream"
    );

    server.shutdown();
    let _ = std::fs::remove_dir_all(cache_dir);
}

#[test]
fn fetch_client_rejects_disk_cache_entry_when_request_url_differs() {
    let cache_dir = unique_test_cache_dir();
    let server = ScriptedHttpServer::spawn(vec![
        ScriptedResponse::ok("hit-1").with_header("Cache-Control", "max-age=60"),
        ScriptedResponse::ok("hit-2").with_header("Cache-Control", "max-age=60"),
    ]);

    let mut config = FetchConfig::default();
    config.set_http_cache_dir(Some(cache_dir.display().to_string()));

    let first = fetch_with_config_for_test(&config, Request::get(&server.url()).unwrap()).unwrap();
    let entry_dir = std::fs::read_dir(&cache_dir)
        .unwrap()
        .next()
        .expect("cache entry should exist")
        .unwrap()
        .path();
    let meta_path = entry_dir.join("meta.json");
    let metadata = std::fs::read_to_string(&meta_path).unwrap();
    let rewritten_metadata = metadata.replace(
        &format!("\"request_url\":\"{}\"", server.url()),
        "\"request_url\":\"http://example.invalid/other\"",
    );
    assert_ne!(metadata, rewritten_metadata);
    std::fs::write(&meta_path, rewritten_metadata).unwrap();

    let second = fetch_with_config_for_test(&config, Request::get(&server.url()).unwrap()).unwrap();

    assert_eq!(first.body_text(), "hit-1");
    assert_eq!(second.body_text(), "hit-2");
    assert_eq!(server.hits(), 2);

    server.shutdown();
    let _ = std::fs::remove_dir_all(cache_dir);
}

#[test]
fn fetch_client_skips_disk_cache_for_vary_star() {
    let cache_dir = unique_test_cache_dir();
    let server = ScriptedHttpServer::spawn(vec![
        ScriptedResponse::ok("hit-1")
            .with_header("Cache-Control", "max-age=60")
            .with_header("Vary", "*"),
        ScriptedResponse::ok("hit-2")
            .with_header("Cache-Control", "max-age=60")
            .with_header("Vary", "*"),
    ]);

    let mut config = FetchConfig::default();
    config.set_http_cache_dir(Some(cache_dir.display().to_string()));

    let first = fetch_with_config_for_test(&config, Request::get(&server.url()).unwrap()).unwrap();
    let second = fetch_with_config_for_test(&config, Request::get(&server.url()).unwrap()).unwrap();

    assert_eq!(first.body_text(), "hit-1");
    assert_eq!(second.body_text(), "hit-2");
    assert_eq!(server.hits(), 2);

    server.shutdown();
    let _ = std::fs::remove_dir_all(cache_dir);
}

#[test]
fn fetch_client_skips_disk_cache_when_any_vary_header_is_star() {
    let cache_dir = unique_test_cache_dir();
    let server = ScriptedHttpServer::spawn(vec![
        ScriptedResponse::ok("hit-1")
            .with_header("Cache-Control", "max-age=60")
            .with_header("Vary", "Accept-Encoding")
            .with_header("Vary", "*"),
        ScriptedResponse::ok("hit-2")
            .with_header("Cache-Control", "max-age=60")
            .with_header("Vary", "Accept-Encoding")
            .with_header("Vary", "*"),
    ]);

    let mut config = FetchConfig::default();
    config.set_http_cache_dir(Some(cache_dir.display().to_string()));

    let first = fetch_with_config_for_test(&config, Request::get(&server.url()).unwrap()).unwrap();
    let second = fetch_with_config_for_test(&config, Request::get(&server.url()).unwrap()).unwrap();

    assert_eq!(first.body_text(), "hit-1");
    assert_eq!(second.body_text(), "hit-2");
    assert_eq!(
        server.hits(),
        2,
        "all Vary header fields must be considered before storing"
    );

    server.shutdown();
    let _ = std::fs::remove_dir_all(cache_dir);
}

#[test]
fn fetch_client_cache_vary_user_agent_partitions_hits() {
    let cache_dir = unique_test_cache_dir();
    let server = ScriptedHttpServer::spawn(vec![
        ScriptedResponse::ok("hit-1")
            .with_header("Cache-Control", "max-age=60")
            .with_header("Vary", "User-Agent"),
        ScriptedResponse::ok("hit-2")
            .with_header("Cache-Control", "max-age=60")
            .with_header("Vary", "User-Agent"),
    ]);

    let mut first_config = FetchConfig::default();
    first_config.set_http_cache_dir(Some(cache_dir.display().to_string()));
    first_config.set_user_agent("Moli/Test-UA-1");
    let mut second_config = FetchConfig::default();
    second_config.set_http_cache_dir(Some(cache_dir.display().to_string()));
    second_config.set_user_agent("Moli/Test-UA-2");

    let first =
        fetch_with_config_for_test(&first_config, Request::get(&server.url()).unwrap()).unwrap();
    let second =
        fetch_with_config_for_test(&second_config, Request::get(&server.url()).unwrap()).unwrap();

    assert_eq!(first.body_text(), "hit-1");
    assert_eq!(second.body_text(), "hit-2");
    assert_eq!(server.hits(), 2);

    server.shutdown();
    let _ = std::fs::remove_dir_all(cache_dir);
}

#[test]
fn fetch_client_cache_vary_accept_language_records_effective_header() {
    let cache_dir = unique_test_cache_dir();
    let server = ScriptedHttpServer::spawn(vec![
        ScriptedResponse::ok("hit-1")
            .with_header("Cache-Control", "max-age=60")
            .with_header("Vary", "Accept-Language"),
    ]);

    let mut config = FetchConfig::default();
    config.set_http_cache_dir(Some(cache_dir.display().to_string()));

    let response =
        fetch_with_config_for_test(&config, Request::get(&server.url()).unwrap()).unwrap();

    assert_eq!(response.body_text(), "hit-1");
    let entry_dir = fs::read_dir(&cache_dir)
        .unwrap()
        .next()
        .expect("cache entry should exist")
        .unwrap()
        .path();
    let metadata = fs::read_to_string(entry_dir.join("meta.json")).unwrap();
    assert!(
        metadata.contains("\"name\":\"accept-language\""),
        "metadata should record Accept-Language vary key: {metadata}"
    );
    assert!(
        metadata.contains(DEFAULT_ACCEPT_LANGUAGE),
        "metadata should record effective Accept-Language value: {metadata}"
    );

    server.shutdown();
    let _ = std::fs::remove_dir_all(cache_dir);
}

#[test]
fn fetch_client_cache_vary_referer_partitions_hits() {
    let cache_dir = unique_test_cache_dir();
    let server = ScriptedHttpServer::spawn(vec![
        ScriptedResponse::ok("hit-1")
            .with_header("Cache-Control", "max-age=60")
            .with_header("Vary", "Referer"),
        ScriptedResponse::ok("hit-2")
            .with_header("Cache-Control", "max-age=60")
            .with_header("Vary", "Referer"),
    ]);

    let mut config = FetchConfig::default();
    config.set_http_cache_dir(Some(cache_dir.display().to_string()));

    let request_url = Url::parse(&server.url()).unwrap();
    let first_referrer = request_url.join("/page-one").unwrap();
    let second_referrer = request_url.join("/page-two").unwrap();
    let fetch_for_referrer = |referrer: &Url| {
        fetch_with_config_for_test(
            &config,
            Request::new("GET", &server.url(), None, Vec::new())
                .unwrap()
                .with_browser_request_metadata(BrowserRequestMetadata::Fetch)
                .with_initiator_url(referrer),
        )
        .unwrap()
    };

    let first = fetch_for_referrer(&first_referrer);
    let second = fetch_for_referrer(&first_referrer);
    let third = fetch_for_referrer(&second_referrer);

    assert_eq!(first.body_text(), "hit-1");
    assert_eq!(second.body_text(), "hit-1");
    assert_eq!(third.body_text(), "hit-2");
    assert_eq!(
        server.hits(),
        2,
        "Vary: Referer should hit only when the effective Referer matches"
    );

    let entry_dir = fs::read_dir(&cache_dir)
        .unwrap()
        .next()
        .expect("cache entry should exist")
        .unwrap()
        .path();
    let metadata = fs::read_to_string(entry_dir.join("meta.json")).unwrap();
    assert!(
        metadata.contains("\"name\":\"referer\""),
        "metadata should record Referer vary key: {metadata}"
    );
    assert!(
        metadata.contains("/page-two"),
        "metadata should record the latest effective Referer value: {metadata}"
    );

    server.shutdown();
    let _ = std::fs::remove_dir_all(cache_dir);
}

#[test]
fn fetch_client_cache_vary_navigation_browser_headers_are_cacheable() {
    let cache_dir = unique_test_cache_dir();
    let server = ScriptedHttpServer::spawn(vec![
        ScriptedResponse::ok("hit-1")
            .with_header("Cache-Control", "max-age=60")
            .with_header(
                "Vary",
                "Accept, Sec-Fetch-Mode, Sec-Fetch-Dest, Sec-Fetch-Site, Sec-Fetch-User, Upgrade-Insecure-Requests",
            ),
    ]);

    let mut config = FetchConfig::default();
    config.set_http_cache_dir(Some(cache_dir.display().to_string()));

    let first = fetch_with_config_for_test(&config, Request::get(&server.url()).unwrap()).unwrap();
    let second = fetch_with_config_for_test(&config, Request::get(&server.url()).unwrap()).unwrap();

    assert_eq!(first.body_text(), "hit-1");
    assert_eq!(second.body_text(), "hit-1");
    assert_eq!(
        server.hits(),
        1,
        "Vary over synthesized navigation headers should remain cacheable"
    );

    let entry_dir = fs::read_dir(&cache_dir)
        .unwrap()
        .next()
        .expect("cache entry should exist")
        .unwrap()
        .path();
    let metadata = fs::read_to_string(entry_dir.join("meta.json")).unwrap();
    for expected in [
        "\"name\":\"accept\"",
        "\"name\":\"sec-fetch-mode\"",
        "\"name\":\"sec-fetch-dest\"",
        "\"name\":\"sec-fetch-site\"",
        "\"name\":\"sec-fetch-user\"",
        "\"name\":\"upgrade-insecure-requests\"",
    ] {
        assert!(
            metadata.contains(expected),
            "metadata should record supported Vary key {expected}: {metadata}"
        );
    }

    server.shutdown();
    let _ = std::fs::remove_dir_all(cache_dir);
}

#[test]
fn fetch_client_cache_vary_client_hints_are_cacheable() {
    let cache_dir = unique_test_cache_dir();
    let server = ScriptedHttpServer::spawn(vec![
        ScriptedResponse::ok("hit-1")
            .with_header("Cache-Control", "max-age=60")
            .with_header("Vary", "Sec-CH-UA, Sec-CH-UA-Mobile, Sec-CH-UA-Platform"),
    ]);

    let mut config = FetchConfig::default();
    config.set_http_cache_dir(Some(cache_dir.display().to_string()));

    let first = fetch_with_config_for_test(&config, Request::get(&server.url()).unwrap()).unwrap();
    let second = fetch_with_config_for_test(&config, Request::get(&server.url()).unwrap()).unwrap();

    assert_eq!(first.body_text(), "hit-1");
    assert_eq!(second.body_text(), "hit-1");
    assert_eq!(
        server.hits(),
        1,
        "Vary over synthesized client hints should remain cacheable"
    );

    let entry_dir = fs::read_dir(&cache_dir)
        .unwrap()
        .next()
        .expect("cache entry should exist")
        .unwrap()
        .path();
    let metadata = fs::read_to_string(entry_dir.join("meta.json")).unwrap();
    for expected in [
        "\"name\":\"sec-ch-ua\"",
        "\"name\":\"sec-ch-ua-mobile\"",
        "\"name\":\"sec-ch-ua-platform\"",
    ] {
        assert!(
            metadata.contains(expected),
            "metadata should record supported client-hint Vary key {expected}: {metadata}"
        );
    }

    server.shutdown();
    let _ = std::fs::remove_dir_all(cache_dir);
}

#[test]
fn critical_client_hints_restart_navigation_before_exposing_the_first_response() -> Result<()> {
    let server = ScriptedHttpServer::spawn(vec![
        ScriptedResponse::status(403, "Challenge")
            .with_body("discarded challenge")
            .with_header("Accept-CH", TEST_HIGH_ENTROPY_CLIENT_HINTS)
            .with_header("Critical-CH", TEST_HIGH_ENTROPY_CLIENT_HINTS),
        ScriptedResponse::ok("restarted navigation"),
        ScriptedResponse::ok("subresource"),
    ]);
    let config = FetchConfig::default();
    let client = FetchClient::new(&config, new_shared_browser_cookie_store());

    let navigation = fetch_response_for_test(&client, Request::get(&server.url())?)?;
    assert_eq!(navigation.body_text(), "restarted navigation");
    assert_eq!(navigation.redirect_chain.len(), 1);
    let restart = &navigation.redirect_chain[0];
    assert_eq!(restart.from_url, restart.to_url);
    assert_eq!(restart.status, 307);
    assert_eq!(
        restart
            .headers
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case("location"))
            .map(|(_, value)| value.as_str()),
        Some(restart.to_url.as_str())
    );
    assert!(!restart.redirect_has_extra_info);
    assert!(restart.cookie_set_reports.is_empty());
    assert_eq!(
        restart
            .response_extra_info
            .as_ref()
            .map(|extra| extra.status),
        Some(403)
    );
    let initial_headers = &restart
        .response_extra_info
        .as_ref()
        .expect("discarded server response extra info")
        .request_extra_info
        .headers;
    assert!(
        !initial_headers
            .iter()
            .any(|(name, _)| name.eq_ignore_ascii_case("sec-ch-ua-arch"))
    );
    let restarted_headers = &restart
        .request_extra_info
        .as_ref()
        .expect("restarted request extra info")
        .headers;
    assert!(restarted_headers.iter().any(|(name, value)| {
        name.eq_ignore_ascii_case("sec-ch-ua-arch") && value == "\"x86\""
    }));
    assert!(navigation.network_request_extra_info().is_some());

    let navigation_url = Url::parse(&server.url())?;
    let subresource = Request::new("POST", &server.url(), Some("probe".to_owned()), Vec::new())?
        .with_browser_request_metadata(BrowserRequestMetadata::Fetch)
        .with_initiator_url(&navigation_url);
    let subresource = fetch_response_for_test(&client, subresource)?;
    assert_eq!(subresource.body_text(), "subresource");

    let requests = server.requests();
    assert_eq!(requests.len(), 3, "unexpected request chain: {requests:#?}");
    let first = requests[0].to_ascii_lowercase();
    let restarted = requests[1].to_ascii_lowercase();
    let later = requests[2].to_ascii_lowercase();
    assert!(!first.contains("sec-ch-ua-arch:"));
    for expected in [
        "sec-ch-ua-full-version: \"145.0.0.0\"",
        "sec-ch-ua-full-version-list:",
        "sec-ch-ua-arch: \"x86\"",
        "sec-ch-ua-bitness: \"64\"",
        "sec-ch-ua-platform-version: \"19.0.0\"",
        "sec-ch-ua-model: \"\"",
        "sec-ch-ua-wow64: ?0",
    ] {
        assert!(
            restarted.contains(expected),
            "restarted navigation missing {expected}: {}",
            requests[1]
        );
        assert!(
            later.contains(expected),
            "later same-origin request missing persisted {expected}: {}",
            requests[2]
        );
    }

    server.shutdown();
    Ok(())
}

#[test]
fn critical_client_hints_restart_redirect_hop_before_following_location() -> Result<()> {
    let redirect = || {
        ScriptedResponse::status(302, "Found")
            .with_header("Location", "/final")
            .with_header("Accept-CH", "Sec-CH-UA-Arch")
            .with_header("Critical-CH", "Sec-CH-UA-Arch")
    };
    let server =
        ScriptedHttpServer::spawn(vec![redirect(), redirect(), ScriptedResponse::ok("ok")]);
    let config = FetchConfig::default();
    let client = FetchClient::new(&config, new_shared_browser_cookie_store());

    let response = fetch_response_for_test(&client, Request::get(&server.url())?)?;
    assert_eq!(response.body_text(), "ok");
    assert_eq!(response.redirect_chain.len(), 2);
    let restart = &response.redirect_chain[0];
    assert_eq!(restart.status, 307);
    assert_eq!(restart.from_url, restart.to_url);
    assert!(!restart.redirect_has_extra_info);
    assert_eq!(
        restart
            .response_extra_info
            .as_ref()
            .map(|extra| extra.status),
        Some(302)
    );
    assert!(restart.request_extra_info.is_some());
    let server_redirect = &response.redirect_chain[1];
    assert_eq!(server_redirect.status, 302);
    assert!(server_redirect.redirect_has_extra_info);
    assert_eq!(
        server_redirect
            .response_extra_info
            .as_ref()
            .map(|extra| extra.status),
        Some(302)
    );

    let requests = server.requests();
    assert_eq!(requests.len(), 3, "unexpected request chain: {requests:#?}");
    assert!(requests[0].starts_with("GET /cache "));
    assert!(requests[1].starts_with("GET /cache "));
    assert!(requests[2].starts_with("GET /final "));
    assert!(!requests[0].to_ascii_lowercase().contains("sec-ch-ua-arch:"));
    assert!(
        requests[1]
            .to_ascii_lowercase()
            .contains("sec-ch-ua-arch: \"x86\"")
    );
    assert!(
        requests[2]
            .to_ascii_lowercase()
            .contains("sec-ch-ua-arch: \"x86\"")
    );

    server.shutdown();
    Ok(())
}

#[test]
fn critical_client_hint_vary_remains_cacheable_after_restart() -> Result<()> {
    let cache_dir = unique_test_cache_dir();
    let server = ScriptedHttpServer::spawn(vec![
        ScriptedResponse::status(403, "Challenge")
            .with_header("Accept-CH", "Sec-CH-UA-Arch")
            .with_header("Critical-CH", "Sec-CH-UA-Arch"),
        ScriptedResponse::ok("cached")
            .with_header("Cache-Control", "max-age=60")
            .with_header("Vary", "Sec-CH-UA-Arch"),
    ]);
    let mut config = FetchConfig::default();
    config.set_http_cache_dir(Some(cache_dir.display().to_string()));
    let client = FetchClient::new(&config, new_shared_browser_cookie_store());

    let first = fetch_response_for_test(&client, Request::get(&server.url())?)?;
    let second = fetch_response_for_test(&client, Request::get(&server.url())?)?;
    assert_eq!(first.body_text(), "cached");
    assert_eq!(second.body_text(), "cached");
    assert_eq!(
        server.hits(),
        2,
        "the post-restart response should be reused"
    );

    let entry_dir = fs::read_dir(&cache_dir)
        .unwrap()
        .next()
        .expect("cache entry should exist")
        .unwrap()
        .path();
    let metadata = fs::read_to_string(entry_dir.join("meta.json"))?;
    assert!(
        metadata.contains("\"name\":\"sec-ch-ua-arch\""),
        "{metadata}"
    );
    assert!(metadata.contains("\\\"x86\\\""), "{metadata}");

    server.shutdown();
    let _ = fs::remove_dir_all(cache_dir);
    Ok(())
}

#[test]
fn fetch_client_partitions_disk_cache_by_top_frame_origin() {
    let cache_dir = unique_test_cache_dir();
    let server = ScriptedHttpServer::spawn(vec![
        ScriptedResponse::ok("hit-1").with_header("Cache-Control", "max-age=60"),
        ScriptedResponse::ok("hit-2").with_header("Cache-Control", "max-age=60"),
    ]);
    let first_top_frame = Url::parse("https://app.example.test/").unwrap();
    let second_top_frame = Url::parse("https://other.example.test/").unwrap();

    let mut config = FetchConfig::default();
    config.set_http_cache_dir(Some(cache_dir.display().to_string()));

    let first = fetch_with_config_for_test(
        &config,
        Request::get(&server.url())
            .unwrap()
            .with_top_frame_origin_url(&first_top_frame),
    )
    .unwrap();
    let second = fetch_with_config_for_test(
        &config,
        Request::get(&server.url())
            .unwrap()
            .with_top_frame_origin_url(&second_top_frame),
    )
    .unwrap();
    let third = fetch_with_config_for_test(
        &config,
        Request::get(&server.url())
            .unwrap()
            .with_top_frame_origin_url(&first_top_frame),
    )
    .unwrap();

    assert_eq!(first.body_text(), "hit-1");
    assert_eq!(second.body_text(), "hit-2");
    assert_eq!(third.body_text(), "hit-1");
    assert_eq!(server.hits(), 2);

    server.shutdown();
    let _ = std::fs::remove_dir_all(cache_dir);
}

#[test]
fn fetch_client_treats_disk_cache_store_failure_as_best_effort() {
    let cache_file = unique_test_cache_dir();
    fs::write(&cache_file, b"not a directory").unwrap();
    let server = ScriptedHttpServer::spawn(vec![
        ScriptedResponse::ok("hit-1").with_header("Cache-Control", "max-age=60"),
    ]);

    let mut config = FetchConfig::default();
    config.set_http_cache_dir(Some(cache_file.display().to_string()));

    let response =
        fetch_with_config_for_test(&config, Request::get(&server.url()).unwrap()).unwrap();

    assert_eq!(response.body_text(), "hit-1");
    assert_eq!(server.hits(), 1);

    server.shutdown();
    let _ = std::fs::remove_file(cache_file);
}

#[test]
fn fetch_client_caches_script_referrer_and_priority_metadata_requests() {
    let cache_dir = unique_test_cache_dir();
    let server = ScriptedHttpServer::spawn(vec![
        ScriptedResponse::ok("hit-1").with_header("Cache-Control", "max-age=60"),
    ]);

    let mut config = FetchConfig::default();
    config.set_http_cache_dir(Some(cache_dir.display().to_string()));
    let script_metadata = ScriptFetchRequestMetadata {
        referrer_policy: Some("origin".to_owned()),
        fetch_priority: Some(FetchPriorityHint::High),
        ..ScriptFetchRequestMetadata::default()
    };

    let first = fetch_with_config_for_test(
        &config,
        Request::get(&server.url())
            .unwrap()
            .with_script_fetch_metadata(script_metadata.clone()),
    )
    .unwrap();
    let second = fetch_with_config_for_test(
        &config,
        Request::get(&server.url())
            .unwrap()
            .with_script_fetch_metadata(script_metadata),
    )
    .unwrap();

    assert_eq!(first.body_text(), "hit-1");
    assert_eq!(second.body_text(), "hit-1");
    assert!(second.head().from_cache);
    assert_eq!(
        server.hits(),
        1,
        "referrer policy and scheduling hints do not change response identity without Vary"
    );

    server.shutdown();
    let _ = std::fs::remove_dir_all(cache_dir);
}

#[test]
fn fetch_client_uses_disk_cache_for_default_script_metadata_requests() {
    let cache_dir = unique_test_cache_dir();
    let server = ScriptedHttpServer::spawn(vec![
        ScriptedResponse::ok("hit-1").with_header("Cache-Control", "max-age=60"),
    ]);

    let mut config = FetchConfig::default();
    config.set_http_cache_dir(Some(cache_dir.display().to_string()));

    let first = fetch_with_config_for_test(
        &config,
        Request::get(&server.url())
            .unwrap()
            .with_script_fetch_metadata(ScriptFetchRequestMetadata::default()),
    )
    .unwrap();
    let second = fetch_with_config_for_test(
        &config,
        Request::get(&server.url())
            .unwrap()
            .with_script_fetch_metadata(ScriptFetchRequestMetadata::default()),
    )
    .unwrap();

    assert_eq!(first.body_text(), "hit-1");
    assert_eq!(second.body_text(), "hit-1");
    assert_eq!(
        server.hits(),
        1,
        "default script metadata should remain eligible for HTTP cache reuse"
    );

    server.shutdown();
    let _ = std::fs::remove_dir_all(cache_dir);
}

fn stylesheet_cache_request(url: &str, metadata: SubresourceRequestMetadata) -> Request {
    Request::new("GET", url, None, Vec::new())
        .unwrap()
        .with_resource_type(RequestResourceType::CssStyleSheet)
        .with_request_mode(RequestMode::NoCors)
        .with_credentials_mode(RequestCredentialsMode::Include)
        .with_browser_request_metadata(BrowserRequestMetadata::Style)
        .with_subresource_request_metadata(metadata)
}

#[test]
fn fetch_client_uses_disk_cache_for_default_stylesheet_metadata() {
    let cache_dir = unique_test_cache_dir();
    let server = ScriptedHttpServer::spawn(vec![
        ScriptedResponse::ok("body { color: green; }")
            .with_header("Cache-Control", "max-age=60")
            .with_header("Content-Type", "text/css"),
    ]);
    let mut config = FetchConfig::default();
    config.set_http_cache_dir(Some(cache_dir.display().to_string()));

    let first = fetch_with_config_for_test(
        &config,
        stylesheet_cache_request(&server.url(), SubresourceRequestMetadata::default()),
    )
    .unwrap();
    let second = fetch_with_config_for_test(
        &config,
        stylesheet_cache_request(&server.url(), SubresourceRequestMetadata::default()),
    )
    .unwrap();

    assert_eq!(first.body_text(), "body { color: green; }");
    assert_eq!(second.body_text(), "body { color: green; }");
    assert!(second.head().from_cache);
    assert_eq!(server.hits(), 1);
    let request = server.requests()[0].to_ascii_lowercase().replace(' ', "");
    assert!(
        request.contains("sec-fetch-dest:style"),
        "stylesheet request should carry style destination: {request}"
    );
    assert!(
        request.contains("sec-fetch-mode:no-cors"),
        "stylesheet request should carry no-cors mode: {request}"
    );

    server.shutdown();
    let _ = std::fs::remove_dir_all(cache_dir);
}

#[test]
fn fetch_client_excludes_unvalidated_stylesheet_integrity_from_cache() {
    let cache_dir = unique_test_cache_dir();
    let server = ScriptedHttpServer::spawn(vec![
        ScriptedResponse::ok("first")
            .with_header("Cache-Control", "max-age=60")
            .with_header("Content-Type", "text/css"),
        ScriptedResponse::ok("second")
            .with_header("Cache-Control", "max-age=60")
            .with_header("Content-Type", "text/css"),
    ]);
    let mut config = FetchConfig::default();
    config.set_http_cache_dir(Some(cache_dir.display().to_string()));
    let metadata = SubresourceRequestMetadata {
        integrity: Some("sha256-test".to_owned()),
        ..SubresourceRequestMetadata::default()
    };

    let first = fetch_with_config_for_test(
        &config,
        stylesheet_cache_request(&server.url(), metadata.clone()),
    )
    .unwrap();
    let second =
        fetch_with_config_for_test(&config, stylesheet_cache_request(&server.url(), metadata))
            .unwrap();

    assert_eq!(first.body_text(), "first");
    assert_eq!(second.body_text(), "second");
    assert_eq!(
        server.hits(),
        2,
        "integrity stays excluded until stylesheet cache hits revalidate each client"
    );

    server.shutdown();
    let _ = std::fs::remove_dir_all(cache_dir);
}

#[test]
fn fetch_client_stylesheet_referrer_policy_uses_vary_referer_on_cache_hits() {
    let cache_dir = unique_test_cache_dir();
    let server = ScriptedHttpServer::spawn(vec![
        ScriptedResponse::ok("origin-policy")
            .with_header("Cache-Control", "max-age=60")
            .with_header("Content-Type", "text/css")
            .with_header("Vary", "Referer"),
        ScriptedResponse::ok("no-referrer-policy")
            .with_header("Cache-Control", "max-age=60")
            .with_header("Content-Type", "text/css")
            .with_header("Vary", "Referer"),
    ]);
    let mut config = FetchConfig::default();
    config.set_http_cache_dir(Some(cache_dir.display().to_string()));
    let document_url = Url::parse("https://document.example/path/page.html").unwrap();
    let request = |policy: &str| {
        stylesheet_cache_request(
            &server.url(),
            SubresourceRequestMetadata {
                referrer_policy: Some(policy.to_owned()),
                ..SubresourceRequestMetadata::default()
            },
        )
        .with_initiator_url(&document_url)
    };

    let first = fetch_with_config_for_test(&config, request("origin")).unwrap();
    let second = fetch_with_config_for_test(&config, request("origin")).unwrap();
    let third = fetch_with_config_for_test(&config, request("no-referrer")).unwrap();
    let fourth = fetch_with_config_for_test(&config, request("no-referrer")).unwrap();

    assert_eq!(first.body_text(), "origin-policy");
    assert_eq!(second.body_text(), "origin-policy");
    assert!(second.head().from_cache);
    assert_eq!(third.body_text(), "no-referrer-policy");
    assert_eq!(fourth.body_text(), "no-referrer-policy");
    assert!(fourth.head().from_cache);
    assert_eq!(server.hits(), 2);
    assert!(
        server.requests()[0]
            .to_ascii_lowercase()
            .contains("referer: https://document.example/")
    );
    assert!(
        !server.requests()[1]
            .to_ascii_lowercase()
            .contains("referer:")
    );

    server.shutdown();
    let _ = std::fs::remove_dir_all(cache_dir);
}

#[test]
fn fetch_client_cache_ignores_script_producer_and_scheduling_only_metadata() {
    let cases = [
        (
            "cross-origin-attribute",
            ScriptFetchRequestMetadata {
                cross_origin: Some("anonymous".to_owned()),
                ..ScriptFetchRequestMetadata::default()
            },
        ),
        (
            "charset",
            ScriptFetchRequestMetadata {
                charset: Some("utf-8".to_owned()),
                ..ScriptFetchRequestMetadata::default()
            },
        ),
        (
            "nonce",
            ScriptFetchRequestMetadata {
                nonce: Some("nonce-test".to_owned()),
                ..ScriptFetchRequestMetadata::default()
            },
        ),
        (
            "fetch-priority",
            ScriptFetchRequestMetadata {
                fetch_priority: Some(FetchPriorityHint::High),
                ..ScriptFetchRequestMetadata::default()
            },
        ),
        (
            "scheduler-priority",
            ScriptFetchRequestMetadata {
                scheduler_priority: Some(ScriptFetchSchedulerPriority::High),
                ..ScriptFetchRequestMetadata::default()
            },
        ),
    ];

    for (label, metadata) in cases {
        let cache_dir = unique_test_cache_dir();
        let server = ScriptedHttpServer::spawn(vec![
            ScriptedResponse::ok("first")
                .with_header("Cache-Control", "max-age=60")
                .with_header("Content-Type", "text/css"),
        ]);
        let mut config = FetchConfig::default();
        config.set_http_cache_dir(Some(cache_dir.display().to_string()));
        let request = || {
            stylesheet_cache_request(&server.url(), SubresourceRequestMetadata::default())
                .with_script_fetch_metadata(metadata.clone())
                .with_resource_type(RequestResourceType::CssStyleSheet)
        };

        let first = fetch_with_config_for_test(&config, request()).unwrap();
        let second = fetch_with_config_for_test(&config, request()).unwrap();

        assert_eq!(first.body_text(), "first", "{label}");
        assert_eq!(second.body_text(), "first", "{label}");
        assert!(second.head().from_cache, "{label}");
        assert_eq!(
            server.hits(),
            1,
            "{label} must not split HTTP response identity"
        );

        server.shutdown();
        let _ = std::fs::remove_dir_all(cache_dir);
    }
}

#[test]
fn fetch_client_partitions_stylesheet_cache_by_credentials_mode() {
    let cache_dir = unique_test_cache_dir();
    let server = ScriptedHttpServer::spawn(vec![
        ScriptedResponse::ok("include")
            .with_header("Cache-Control", "max-age=60")
            .with_header("Content-Type", "text/css"),
        ScriptedResponse::ok("same-origin")
            .with_header("Cache-Control", "max-age=60")
            .with_header("Content-Type", "text/css"),
    ]);
    let mut config = FetchConfig::default();
    config.set_http_cache_dir(Some(cache_dir.display().to_string()));
    let request = |credentials_mode| {
        stylesheet_cache_request(&server.url(), SubresourceRequestMetadata::default())
            .with_credentials_mode(credentials_mode)
    };

    let first =
        fetch_with_config_for_test(&config, request(RequestCredentialsMode::Include)).unwrap();
    let second =
        fetch_with_config_for_test(&config, request(RequestCredentialsMode::SameOrigin)).unwrap();
    let third =
        fetch_with_config_for_test(&config, request(RequestCredentialsMode::Include)).unwrap();

    assert_eq!(first.body_text(), "include");
    assert_eq!(second.body_text(), "same-origin");
    assert_eq!(third.body_text(), "include");
    assert!(third.head().from_cache);
    assert_eq!(server.hits(), 2);

    server.shutdown();
    let _ = std::fs::remove_dir_all(cache_dir);
}

#[test]
fn fetch_client_skips_stylesheet_cache_for_vary_origin() {
    let cache_dir = unique_test_cache_dir();
    let server = ScriptedHttpServer::spawn(vec![
        ScriptedResponse::ok("first")
            .with_header("Cache-Control", "max-age=60")
            .with_header("Content-Type", "text/css")
            .with_header("Vary", "Origin"),
        ScriptedResponse::ok("second")
            .with_header("Cache-Control", "max-age=60")
            .with_header("Content-Type", "text/css")
            .with_header("Vary", "Origin"),
    ]);
    let mut config = FetchConfig::default();
    config.set_http_cache_dir(Some(cache_dir.display().to_string()));
    let initiator = Url::parse("https://document.example/page").unwrap();
    let request = || {
        stylesheet_cache_request(&server.url(), SubresourceRequestMetadata::default())
            .with_request_mode(RequestMode::Cors)
            .with_credentials_mode(RequestCredentialsMode::SameOrigin)
            .with_initiator_url(&initiator)
    };

    let first = fetch_with_config_for_test(&config, request()).unwrap();
    let second = fetch_with_config_for_test(&config, request()).unwrap();

    assert_eq!(first.body_text(), "first");
    assert_eq!(second.body_text(), "second");
    assert_eq!(server.hits(), 2);
    assert!(
        server.requests()[0]
            .to_ascii_lowercase()
            .contains("origin: https://document.example")
    );

    server.shutdown();
    let _ = std::fs::remove_dir_all(cache_dir);
}

#[test]
fn fetch_client_cache_mode_validate_revalidates_fresh_script_metadata_hit() {
    let cache_dir = unique_test_cache_dir();
    let server = ScriptedHttpServer::spawn(vec![
        ScriptedResponse::ok("hit-1")
            .with_header("Cache-Control", "max-age=60")
            .with_header("ETag", "\"v1\""),
        ScriptedResponse::status(304, "Not Modified")
            .with_header("Cache-Control", "max-age=60")
            .with_header("ETag", "\"v1\""),
    ]);

    let mut config = FetchConfig::default();
    config.set_http_cache_dir(Some(cache_dir.display().to_string()));

    let first = fetch_with_config_for_test(
        &config,
        Request::get(&server.url())
            .unwrap()
            .with_script_fetch_metadata(ScriptFetchRequestMetadata::default()),
    )
    .unwrap();
    let second = fetch_with_config_for_test(
        &config,
        Request::get(&server.url())
            .unwrap()
            .with_cache_mode(RequestCacheMode::Validate)
            .with_script_fetch_metadata(ScriptFetchRequestMetadata::default()),
    )
    .unwrap();

    assert_eq!(first.body_text(), "hit-1");
    assert_eq!(second.body_text(), "hit-1");
    assert_eq!(
        server.hits(),
        2,
        "Validate cache mode must revalidate a fresh script cache entry"
    );
    let second_request = server.requests()[1].to_ascii_lowercase();
    assert!(
        second_request.contains("if-none-match: \"v1\""),
        "Validate cache mode should attach cached validators: {}",
        server.requests()[1]
    );

    server.shutdown();
    let _ = std::fs::remove_dir_all(cache_dir);
}

#[test]
fn fetch_client_cache_matches_effective_explicit_user_agent() {
    let cache_dir = unique_test_cache_dir();
    let server = ScriptedHttpServer::spawn(vec![
        ScriptedResponse::ok("hit-1")
            .with_header("Cache-Control", "max-age=60")
            .with_header("Vary", "User-Agent"),
        ScriptedResponse::ok("hit-2")
            .with_header("Cache-Control", "max-age=60")
            .with_header("Vary", "User-Agent"),
    ]);

    let mut config = FetchConfig::default();
    config.set_http_cache_dir(Some(cache_dir.display().to_string()));
    config.set_user_agent("Moli/Base-UA");
    let request_with_user_agent = |user_agent: &str| {
        Request::new(
            "GET",
            &server.url(),
            None,
            vec![("User-Agent".to_owned(), user_agent.to_owned())],
        )
        .unwrap()
    };

    let first =
        fetch_with_config_for_test(&config, request_with_user_agent("Moli/Protocol-UA-1")).unwrap();
    let second =
        fetch_with_config_for_test(&config, request_with_user_agent("Moli/Protocol-UA-1")).unwrap();
    let third =
        fetch_with_config_for_test(&config, request_with_user_agent("Moli/Protocol-UA-2")).unwrap();
    let fourth =
        fetch_with_config_for_test(&config, request_with_user_agent("Moli/Protocol-UA-2")).unwrap();

    assert_eq!(first.body_text(), "hit-1");
    assert_eq!(second.body_text(), "hit-1");
    assert_eq!(third.body_text(), "hit-2");
    assert_eq!(fourth.body_text(), "hit-2");
    assert_eq!(
        server.hits(),
        2,
        "Vary: User-Agent should match the effective explicit header"
    );

    server.shutdown();
    let _ = std::fs::remove_dir_all(cache_dir);
}

fn assert_explicit_request_headers_bypass_disk_cache(
    request_headers: Vec<(String, String)>,
    reason: &str,
) {
    let cache_dir = unique_test_cache_dir();
    let server = ScriptedHttpServer::spawn(vec![
        ScriptedResponse::ok("hit-1").with_header("Cache-Control", "max-age=60"),
        ScriptedResponse::ok("hit-2").with_header("Cache-Control", "max-age=60"),
    ]);

    let mut config = FetchConfig::default();
    config.set_http_cache_dir(Some(cache_dir.display().to_string()));
    let request = || Request::new("GET", &server.url(), None, request_headers.clone()).unwrap();

    let first = fetch_with_config_for_test(&config, request()).unwrap();
    let second = fetch_with_config_for_test(&config, request()).unwrap();

    assert_eq!(first.body_text(), "hit-1");
    assert_eq!(second.body_text(), "hit-2");
    assert_eq!(server.hits(), 2, "{reason}");

    server.shutdown();
    let _ = std::fs::remove_dir_all(cache_dir);
}

#[test]
fn fetch_client_skips_disk_cache_for_unsupported_explicit_request_headers() {
    assert_explicit_request_headers_bypass_disk_cache(
        vec![("X-Custom-Cache-Input".to_owned(), "one".to_owned())],
        "unsupported custom request headers must remain outside the disk cache",
    );
}

#[test]
fn fetch_client_skips_disk_cache_for_request_cache_control_no_store() {
    assert_explicit_request_headers_bypass_disk_cache(
        vec![("Cache-Control".to_owned(), "no-store".to_owned())],
        "request Cache-Control: no-store must bypass disk cache storage and reuse",
    );
}

#[test]
fn fetch_client_skips_disk_cache_for_duplicate_supported_request_headers() {
    assert_explicit_request_headers_bypass_disk_cache(
        vec![
            ("Accept-Language".to_owned(), "en-US".to_owned()),
            ("accept-language".to_owned(), "fr".to_owned()),
        ],
        "duplicate supported headers must remain outside last-value Vary matching",
    );
}

#[test]
fn fetch_client_skips_disk_cache_for_default_request_headers() {
    let cache_dir = unique_test_cache_dir();
    let server = ScriptedHttpServer::spawn(vec![
        ScriptedResponse::ok("hit-1").with_header("Cache-Control", "max-age=60"),
        ScriptedResponse::ok("hit-2").with_header("Cache-Control", "max-age=60"),
    ]);

    let mut config = FetchConfig::default();
    config.set_http_cache_dir(Some(cache_dir.display().to_string()));
    config.set_default_request_headers(vec![("X-Test".to_owned(), "one".to_owned())]);

    let first = fetch_with_config_for_test(&config, Request::get(&server.url()).unwrap()).unwrap();
    let second = fetch_with_config_for_test(&config, Request::get(&server.url()).unwrap()).unwrap();

    assert_eq!(first.body_text(), "hit-1");
    assert_eq!(second.body_text(), "hit-2");
    assert_eq!(
        server.hits(),
        2,
        "requests with configured default headers must not use the URL-only disk cache"
    );

    server.shutdown();
    let _ = std::fs::remove_dir_all(cache_dir);
}

#[test]
fn fetch_client_skips_disk_cache_for_post_requests() {
    let cache_dir = unique_test_cache_dir();
    let server = ScriptedHttpServer::spawn(vec![
        ScriptedResponse::ok("hit-1").with_header("Cache-Control", "max-age=60"),
        ScriptedResponse::ok("hit-2").with_header("Cache-Control", "max-age=60"),
    ]);

    let mut config = FetchConfig::default();
    config.set_http_cache_dir(Some(cache_dir.display().to_string()));

    let first = fetch_with_config_for_test(
        &config,
        Request::new(
            "POST",
            &server.url(),
            Some("payload".to_owned()),
            Vec::new(),
        )
        .unwrap(),
    )
    .unwrap();
    let second = fetch_with_config_for_test(
        &config,
        Request::new(
            "POST",
            &server.url(),
            Some("payload".to_owned()),
            Vec::new(),
        )
        .unwrap(),
    )
    .unwrap();

    assert_eq!(first.body_text(), "hit-1");
    assert_eq!(second.body_text(), "hit-2");
    assert_eq!(server.hits(), 2);

    server.shutdown();
    let _ = std::fs::remove_dir_all(cache_dir);
}

#[test]
fn fetch_client_skips_disk_cache_for_get_requests_with_body() {
    let cache_dir = unique_test_cache_dir();
    let server = ScriptedHttpServer::spawn(vec![
        ScriptedResponse::ok("hit-1").with_header("Cache-Control", "max-age=60"),
        ScriptedResponse::ok("hit-2").with_header("Cache-Control", "max-age=60"),
    ]);

    let mut config = FetchConfig::default();
    config.set_http_cache_dir(Some(cache_dir.display().to_string()));

    let first = fetch_with_config_for_test(
        &config,
        Request::new("GET", &server.url(), Some("payload".to_owned()), Vec::new()).unwrap(),
    )
    .unwrap();
    let second = fetch_with_config_for_test(
        &config,
        Request::new("GET", &server.url(), Some("payload".to_owned()), Vec::new()).unwrap(),
    )
    .unwrap();

    assert_eq!(first.body_text(), "hit-1");
    assert_eq!(second.body_text(), "hit-2");
    assert_eq!(server.hits(), 2);

    server.shutdown();
    let _ = std::fs::remove_dir_all(cache_dir);
}

#[test]
fn fetch_client_skips_disk_cache_for_auth_requests() {
    let cache_dir = unique_test_cache_dir();
    let server = ScriptedHttpServer::spawn(vec![
        ScriptedResponse::ok("hit-1").with_header("Cache-Control", "max-age=60"),
        ScriptedResponse::ok("hit-2").with_header("Cache-Control", "max-age=60"),
    ]);

    let mut config = FetchConfig::default();
    config.set_http_cache_dir(Some(cache_dir.display().to_string()));
    let auth = RequestAuth {
        target: RequestAuthTarget::Server,
        scheme: RequestAuthScheme::Basic,
        username: "user".to_owned(),
        password: "pass".to_owned(),
    };

    let first = fetch_with_config_for_test(
        &config,
        Request::get(&server.url()).unwrap().with_auth(auth.clone()),
    )
    .unwrap();
    let second = fetch_with_config_for_test(
        &config,
        Request::get(&server.url()).unwrap().with_auth(auth),
    )
    .unwrap();

    assert_eq!(first.body_text(), "hit-1");
    assert_eq!(second.body_text(), "hit-2");
    assert_eq!(server.hits(), 2);

    server.shutdown();
    let _ = std::fs::remove_dir_all(cache_dir);
}

#[test]
fn fetch_client_cache_respects_no_store() {
    let cache_dir = unique_test_cache_dir();
    let server = ScriptedHttpServer::spawn(vec![
        ScriptedResponse::ok("hit-1").with_header("Cache-Control", "no-store"),
        ScriptedResponse::ok("hit-2").with_header("Cache-Control", "no-store"),
    ]);

    let mut config = FetchConfig::default();
    config.set_http_cache_dir(Some(cache_dir.display().to_string()));

    let first = fetch_with_config_for_test(&config, Request::get(&server.url()).unwrap()).unwrap();
    let second = fetch_with_config_for_test(&config, Request::get(&server.url()).unwrap()).unwrap();

    assert_eq!(first.body_text(), "hit-1");
    assert_eq!(second.body_text(), "hit-2");
    assert_eq!(server.hits(), 2);

    server.shutdown();
    let _ = std::fs::remove_dir_all(cache_dir);
}

#[test]
fn fetch_client_streams_preemptive_basic_server_auth() {
    let server = ScriptedHttpServer::spawn(vec![ScriptedResponse::ok("basic-ok")]);
    let auth = RequestAuth {
        target: RequestAuthTarget::Server,
        scheme: RequestAuthScheme::Basic,
        username: "user".to_owned(),
        password: "pass".to_owned(),
    };

    let response = fetch_with_config_for_test(
        &FetchConfig::default(),
        Request::get(&server.url()).unwrap().with_auth(auth),
    )
    .unwrap();

    assert_eq!(response.body_text(), "basic-ok");
    assert_eq!(server.hits(), 1);
    assert!(
        server.requests()[0].to_ascii_lowercase().contains(
            "authorization: basic dxnlcjpwYXNz"
                .to_ascii_lowercase()
                .as_str()
        ),
        "basic auth should be sent as a preemptive header on the streaming path: {}",
        server.requests()[0]
    );

    server.shutdown();
}

#[test]
fn fetch_client_cache_respects_no_cache_response() {
    let cache_dir = unique_test_cache_dir();
    let server = ScriptedHttpServer::spawn(vec![
        ScriptedResponse::ok("hit-1").with_header("Cache-Control", "no-cache"),
        ScriptedResponse::ok("hit-2").with_header("Cache-Control", "no-cache"),
    ]);

    let mut config = FetchConfig::default();
    config.set_http_cache_dir(Some(cache_dir.display().to_string()));

    let first = fetch_with_config_for_test(&config, Request::get(&server.url()).unwrap()).unwrap();
    let second = fetch_with_config_for_test(&config, Request::get(&server.url()).unwrap()).unwrap();

    assert_eq!(first.body_text(), "hit-1");
    assert_eq!(second.body_text(), "hit-2");
    assert_eq!(server.hits(), 2);

    server.shutdown();
    let _ = std::fs::remove_dir_all(cache_dir);
}

#[test]
fn fetch_client_cache_revalidates_no_cache_response_with_etag() {
    // Cache-Control: no-cache permits storage, but every reuse must validate.
    // A 304 response should merge with the stored body instead of surfacing an
    // empty Not Modified response to the caller.
    let cache_dir = unique_test_cache_dir();
    let server = ScriptedHttpServer::spawn(vec![
        ScriptedResponse::ok("hit-1")
            .with_header("Cache-Control", "no-cache")
            .with_header("ETag", "\"v1\""),
        ScriptedResponse::status(304, "Not Modified")
            .with_header("Cache-Control", "no-cache")
            .with_header("ETag", "\"v1\""),
    ]);

    let mut config = FetchConfig::default();
    config.set_http_cache_dir(Some(cache_dir.display().to_string()));

    let first = fetch_with_config_for_test(&config, Request::get(&server.url()).unwrap()).unwrap();
    let second = fetch_with_config_for_test(&config, Request::get(&server.url()).unwrap()).unwrap();

    assert_eq!(first.body_text(), "hit-1");
    assert_eq!(second.body_text(), "hit-1");
    assert_eq!(server.hits(), 2);
    assert!(
        server.requests()[1].contains("If-None-Match: \"v1\""),
        "no-cache reuse should validate with the stored ETag: {}",
        server.requests()[1]
    );

    server.shutdown();
    let _ = std::fs::remove_dir_all(cache_dir);
}

#[test]
fn network_metadata_captures_transport_generated_request_headers() {
    let server = ScriptedHttpServer::spawn(vec![
        ScriptedResponse::ok("wire headers").with_header("Cache-Control", "no-store"),
    ]);
    let client = FetchClient::new(&FetchConfig::default(), new_shared_browser_cookie_store());
    let request = Request::new(
        "GET",
        &server.url(),
        None,
        vec![("X-Observed".to_owned(), "yes".to_owned())],
    )
    .unwrap();

    let observed = fetch_raw_with_network_metadata_for_test(&client, request).unwrap();
    let journal = observed.observation_journal();

    assert_eq!(journal.exchanges().len(), 1);
    let request_headers = journal.exchanges()[0].request().headers();
    assert_header_present(request_headers, "Host");
    assert_header_present(request_headers, "Accept-Encoding");
    assert_header_present(request_headers, "User-Agent");
    assert_eq!(header_value(request_headers, "X-Observed"), Some("yes"));
    let response = journal.exchanges()[0]
        .response()
        .expect("raw response observation");
    assert_eq!(response.status(), 200);
    assert_eq!(
        header_value(response.headers(), "Cache-Control"),
        Some("no-store")
    );

    assert!(client.shutdown().is_clean());
    server.shutdown();
}

#[tokio::test]
async fn network_metadata_failure_preserves_request_sent_before_empty_response() -> Result<()> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener
            .accept()
            .await
            .expect("empty response server should accept request");
        read_http_request_head(&mut stream)
            .await
            .expect("empty response server should read request");
    });
    let url = Url::parse(&format!("http://{addr}/empty"))?;
    let cookie_store = new_shared_browser_cookie_store();
    cookie_store
        .lock()
        .set_document_cookie(&url, "failedRequestCookie=present; Path=/");
    let client = FetchClient::new(&FetchConfig::default(), cookie_store);

    let error = client
        .fetch_raw_with_network_metadata(
            Request::get(url.as_str())?.with_top_level_navigation_cookie_context(),
        )
        .await
        .expect_err("empty response should fail");
    let failure = error
        .downcast_ref::<NetworkFetchFailureContext>()
        .expect("metadata fetch failure should retain its typed observation journal");
    let [exchange] = failure.observation_journal().exchanges() else {
        panic!("expected exactly one observed exchange");
    };

    assert!(exchange.response().is_none());
    assert_header_present(exchange.request().headers(), "Host");
    assert_header_present(exchange.request().headers(), "Accept-Encoding");
    let cookie_report = exchange
        .request()
        .cookie_report()
        .expect("actual request should retain its cookie access report");
    assert_eq!(cookie_report.included_cookies.len(), 1);

    assert!(client.shutdown().is_clean());
    server.await.expect("empty response server should finish");
    Ok(())
}

#[tokio::test]
async fn network_metadata_failure_preserves_redirect_chain_and_final_request() -> Result<()> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    let server = tokio::spawn(async move {
        let (mut first, _) = listener
            .accept()
            .await
            .expect("redirect server should accept initial request");
        read_http_request_head(&mut first)
            .await
            .expect("redirect server should read initial request");
        first
            .write_all(
                b"HTTP/1.1 302 Found\r\nLocation: /reset\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            )
            .await
            .expect("redirect server should write redirect response");
        first
            .shutdown()
            .await
            .expect("redirect server should close initial connection");

        let (mut second, _) = listener
            .accept()
            .await
            .expect("redirect server should accept final request");
        read_http_request_head(&mut second)
            .await
            .expect("redirect server should read final request");
    });
    let initial_url = Url::parse(&format!("http://{addr}/start"))?;
    let final_url = Url::parse(&format!("http://{addr}/reset"))?;
    let client = FetchClient::new(&FetchConfig::default(), new_shared_browser_cookie_store());

    let error = client
        .fetch_raw_with_network_metadata(Request::get(initial_url.as_str())?)
        .await
        .expect_err("redirect final request should fail without a response");
    let failure = error
        .downcast_ref::<NetworkFetchFailureContext>()
        .expect("metadata fetch failure should retain typed request context");
    let context = failure
        .request_context()
        .expect("runtime failure should retain final request context");

    assert_eq!(context.current_url(), &final_url);
    assert_eq!(context.request_method(), "GET");
    assert_eq!(context.request_body(), None);
    assert_eq!(context.redirect_chain().len(), 1);
    assert_eq!(context.redirect_chain()[0].from_url, initial_url);
    assert_eq!(context.redirect_chain()[0].to_url, final_url);
    assert_eq!(context.redirect_chain()[0].status, 302);
    let [initial_exchange, final_exchange] = failure.observation_journal().exchanges() else {
        panic!("expected redirect response and request-only final exchange");
    };
    assert_eq!(
        initial_exchange
            .response()
            .expect("redirect response observation")
            .status(),
        302
    );
    assert!(final_exchange.response().is_none());
    assert_header_present(final_exchange.request().headers(), "Host");

    assert!(client.shutdown().is_clean());
    server.await.expect("redirect reset server should finish");
    Ok(())
}

#[test]
fn network_metadata_preserves_each_redirect_exchange() {
    let server = ScriptedHttpServer::spawn(vec![
        ScriptedResponse::status(302, "Found")
            .with_header("Location", "/final")
            .with_header("Cache-Control", "no-store"),
        ScriptedResponse::ok("redirected").with_header("Cache-Control", "no-store"),
    ]);
    let client = FetchClient::new(&FetchConfig::default(), new_shared_browser_cookie_store());

    let observed =
        fetch_raw_with_network_metadata_for_test(&client, Request::get(&server.url()).unwrap())
            .unwrap();
    let journal = observed.observation_journal();

    assert_eq!(journal.exchanges().len(), 2);
    assert_eq!(
        journal.exchanges()[0]
            .response()
            .expect("redirect response")
            .status(),
        302
    );
    assert_eq!(
        journal.exchanges()[1]
            .response()
            .expect("final response")
            .status(),
        200
    );
    assert_header_present(journal.exchanges()[0].request().headers(), "Host");
    assert_header_present(journal.exchanges()[1].request().headers(), "Host");

    assert!(client.shutdown().is_clean());
    server.shutdown();
}

#[test]
fn network_metadata_keeps_raw_304_separate_from_merged_cached_response() {
    let cache_dir = unique_test_cache_dir();
    let server = ScriptedHttpServer::spawn(vec![
        ScriptedResponse::ok("cached")
            .with_header("Cache-Control", "no-cache")
            .with_header("ETag", "\"v1\""),
        ScriptedResponse::status(304, "Not Modified")
            .with_header("Cache-Control", "no-cache")
            .with_header("ETag", "\"v1\""),
    ]);
    let mut config = FetchConfig::default();
    config.set_http_cache_dir(Some(cache_dir.display().to_string()));
    let client = FetchClient::new(&config, new_shared_browser_cookie_store());

    let first =
        fetch_raw_with_network_metadata_for_test(&client, Request::get(&server.url()).unwrap())
            .unwrap();
    assert_eq!(first.response().status, 200);
    let second =
        fetch_raw_with_network_metadata_for_test(&client, Request::get(&server.url()).unwrap())
            .unwrap();

    assert_eq!(second.response().status, 200);
    assert_eq!(second.response().body_bytes(), b"cached");
    let exchange = second
        .observation_journal()
        .exchanges()
        .last()
        .expect("revalidation exchange");
    assert_eq!(
        header_value(exchange.request().headers(), "If-None-Match"),
        Some("\"v1\"")
    );
    assert_eq!(
        exchange
            .response()
            .expect("raw revalidation response")
            .status(),
        304
    );

    assert!(client.shutdown().is_clean());
    server.shutdown();
    let _ = std::fs::remove_dir_all(cache_dir);
}

fn assert_header_present(headers: &[(String, String)], name: &str) {
    assert!(
        header_value(headers, name).is_some(),
        "missing {name} in observed headers: {headers:?}"
    );
}

fn header_value<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(header_name, _)| header_name.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.as_str())
}

fn request_head_header_value<'a>(request_head: &'a str, name: &str) -> Option<&'a str> {
    request_head.lines().skip(1).find_map(|line| {
        let (header_name, value) = line.split_once(':')?;
        header_name.eq_ignore_ascii_case(name).then(|| value.trim())
    })
}

fn nonce_from_signature_input(signature_input: &str) -> &str {
    signature_input
        .split(";nonce=\"")
        .nth(1)
        .and_then(|value| value.split('"').next())
        .expect("Signature-Input should contain a nonce parameter")
}

fn assert_request_extra_signature_matches_wire(
    extra_info: &crate::NetworkRequestExtraInfo,
    request_head: &str,
) {
    for name in ["Signature-Agent", "Signature-Input", "Signature"] {
        assert_eq!(
            header_value(&extra_info.headers, name),
            request_head_header_value(request_head, name),
            "network extra info and wire request differ for {name}"
        );
    }
}

#[test]
fn fetch_client_cache_respects_pragma_no_cache_response() {
    let cache_dir = unique_test_cache_dir();
    let server = ScriptedHttpServer::spawn(vec![
        ScriptedResponse::ok("hit-1").with_header("Pragma", "no-cache"),
        ScriptedResponse::ok("hit-2").with_header("Pragma", "no-cache"),
    ]);

    let mut config = FetchConfig::default();
    config.set_http_cache_dir(Some(cache_dir.display().to_string()));

    let first = fetch_with_config_for_test(&config, Request::get(&server.url()).unwrap()).unwrap();
    let second = fetch_with_config_for_test(&config, Request::get(&server.url()).unwrap()).unwrap();

    assert_eq!(first.body_text(), "hit-1");
    assert_eq!(second.body_text(), "hit-2");
    assert_eq!(server.hits(), 2);

    server.shutdown();
    let _ = std::fs::remove_dir_all(cache_dir);
}

#[test]
fn fetch_client_cache_respects_private_response() {
    let cache_dir = unique_test_cache_dir();
    let server = ScriptedHttpServer::spawn(vec![
        ScriptedResponse::ok("hit-1").with_header("Cache-Control", "private, max-age=60"),
        ScriptedResponse::ok("hit-2").with_header("Cache-Control", "private, max-age=60"),
    ]);

    let mut config = FetchConfig::default();
    config.set_http_cache_dir(Some(cache_dir.display().to_string()));

    let first = fetch_with_config_for_test(&config, Request::get(&server.url()).unwrap()).unwrap();
    let second = fetch_with_config_for_test(&config, Request::get(&server.url()).unwrap()).unwrap();

    assert_eq!(first.body_text(), "hit-1");
    assert_eq!(second.body_text(), "hit-2");
    assert_eq!(server.hits(), 2);

    server.shutdown();
    let _ = std::fs::remove_dir_all(cache_dir);
}

#[test]
fn fetch_client_cache_respects_set_cookie_response() {
    let cache_dir = unique_test_cache_dir();
    let server = ScriptedHttpServer::spawn(vec![
        ScriptedResponse::ok("hit-1")
            .with_header("Cache-Control", "max-age=60")
            .with_header("Set-Cookie", "session=one; Path=/"),
        ScriptedResponse::ok("hit-2")
            .with_header("Cache-Control", "max-age=60")
            .with_header("Set-Cookie", "session=two; Path=/"),
    ]);

    let mut config = FetchConfig::default();
    config.set_http_cache_dir(Some(cache_dir.display().to_string()));

    let first = fetch_with_config_for_test(&config, Request::get(&server.url()).unwrap()).unwrap();
    let second = fetch_with_config_for_test(&config, Request::get(&server.url()).unwrap()).unwrap();

    assert_eq!(first.body_text(), "hit-1");
    assert_eq!(second.body_text(), "hit-2");
    assert_eq!(server.hits(), 2);

    server.shutdown();
    let _ = std::fs::remove_dir_all(cache_dir);
}

#[test]
fn fetch_response_stores_partitioned_cookie_under_request_top_level_site() -> Result<()> {
    let server = ScriptedHttps11Server::spawn(vec![ScriptedResponse::ok("ok").with_header(
        "Set-Cookie",
        "chip=one; Path=/; Secure; SameSite=None; Partitioned",
    )]);
    let mut config = FetchConfig::default();
    config.set_tls_verify_host(false);
    let cookie_store = new_shared_browser_cookie_store();
    let client = FetchClient::new(&config, Arc::clone(&cookie_store));
    let request_url = Url::parse(&server.url())?;
    let first_top_level = Url::parse("https://first.example/page")?;
    let second_top_level = Url::parse("https://second.example/page")?;

    let response = fetch_response_for_test(
        &client,
        Request::new("GET", request_url.as_str(), None, Vec::new())?
            .with_initiator_url(&first_top_level),
    )?;

    assert_eq!(response.status, 200);
    let first_context = NetworkCookieRequestContext::subresource("GET")
        .with_initiator_url(&request_url, &first_top_level);
    let second_context = NetworkCookieRequestContext::subresource("GET")
        .with_initiator_url(&request_url, &second_top_level);
    assert_eq!(
        crate::cookie_header_for_request(&cookie_store, &request_url, first_context)?,
        Some("chip=one".to_owned())
    );
    assert_eq!(
        crate::cookie_header_for_request(&cookie_store, &request_url, second_context)?,
        None
    );

    server.shutdown();
    Ok(())
}

#[test]
fn fetch_client_skips_disk_cache_for_non_success_status() {
    let cache_dir = unique_test_cache_dir();
    let server = ScriptedHttpServer::spawn(vec![
        ScriptedResponse::status(404, "Not Found").with_header("Cache-Control", "max-age=60"),
        ScriptedResponse::status(404, "Not Found").with_header("Cache-Control", "max-age=60"),
    ]);

    let mut config = FetchConfig::default();
    config.set_http_cache_dir(Some(cache_dir.display().to_string()));

    let first = fetch_with_config_for_test(&config, Request::get(&server.url()).unwrap()).unwrap();
    let second = fetch_with_config_for_test(&config, Request::get(&server.url()).unwrap()).unwrap();

    assert_eq!(first.status, 404);
    assert_eq!(second.status, 404);
    assert_eq!(server.hits(), 2);

    server.shutdown();
    let _ = std::fs::remove_dir_all(cache_dir);
}

#[test]
fn fetch_client_caches_redirect_final_response_under_final_url() {
    let cache_dir = unique_test_cache_dir();
    let server = ScriptedHttpServer::spawn(vec![
        ScriptedResponse::status(302, "Found")
            .with_header("Cache-Control", "max-age=60")
            .with_header("Location", "/final"),
        ScriptedResponse::ok("hit-1").with_header("Cache-Control", "max-age=60"),
    ]);

    let mut config = FetchConfig::default();
    config.set_http_cache_dir(Some(cache_dir.display().to_string()));

    let first =
        fetch_with_config_for_test(&config, Request::get(&server.url_path("/start")).unwrap())
            .unwrap();
    let second =
        fetch_with_config_for_test(&config, Request::get(&server.url_path("/final")).unwrap())
            .unwrap();

    assert_eq!(first.body_text(), "hit-1");
    assert!(first.redirected);
    assert_eq!(second.body_text(), "hit-1");
    assert_eq!(server.hits(), 2);
    assert!(
        second.from_cache,
        "follow-up final URL request should reuse the cached final response"
    );

    server.shutdown();
    let _ = std::fs::remove_dir_all(cache_dir);
}

#[test]
fn fetch_client_cache_respects_max_age_expiry() {
    let cache_dir = unique_test_cache_dir();
    let server = ScriptedHttpServer::spawn(vec![
        ScriptedResponse::ok("hit-1").with_header("Cache-Control", "max-age=1"),
        ScriptedResponse::ok("hit-2").with_header("Cache-Control", "max-age=1"),
    ]);

    let mut config = FetchConfig::default();
    config.set_http_cache_dir(Some(cache_dir.display().to_string()));

    let first = fetch_with_config_for_test(&config, Request::get(&server.url()).unwrap()).unwrap();
    std::thread::sleep(Duration::from_millis(1_100));
    let second = fetch_with_config_for_test(&config, Request::get(&server.url()).unwrap()).unwrap();

    assert_eq!(first.body_text(), "hit-1");
    assert_eq!(second.body_text(), "hit-2");
    assert_eq!(server.hits(), 2);

    server.shutdown();
    let _ = std::fs::remove_dir_all(cache_dir);
}

#[test]
fn fetch_client_cache_age_header_can_make_max_age_stale() {
    let cache_dir = unique_test_cache_dir();
    let server = ScriptedHttpServer::spawn(vec![
        ScriptedResponse::ok("hit-1")
            .with_header("Cache-Control", "max-age=60")
            .with_header("Age", "120")
            .with_header("ETag", "\"v1\""),
        ScriptedResponse::status(304, "Not Modified").with_header("Cache-Control", "max-age=60"),
    ]);

    let mut config = FetchConfig::default();
    config.set_http_cache_dir(Some(cache_dir.display().to_string()));

    let first = fetch_with_config_for_test(&config, Request::get(&server.url()).unwrap()).unwrap();
    let second = fetch_with_config_for_test(&config, Request::get(&server.url()).unwrap()).unwrap();

    assert_eq!(first.body_text(), "hit-1");
    assert_eq!(second.body_text(), "hit-1");
    assert_eq!(server.hits(), 2);
    assert!(
        server.requests()[1]
            .to_ascii_lowercase()
            .contains("if-none-match: \"v1\""),
        "Age should make the cached entry stale enough to validate: {}",
        server.requests()[1]
    );

    server.shutdown();
    let _ = std::fs::remove_dir_all(cache_dir);
}

#[test]
fn fetch_client_cache_date_header_can_make_max_age_stale() {
    let cache_dir = unique_test_cache_dir();
    let server = ScriptedHttpServer::spawn(vec![
        ScriptedResponse::ok("hit-1")
            .with_header("Cache-Control", "max-age=60")
            .with_header("Date", "Wed, 21 Oct 2015 07:28:00 GMT")
            .with_header("ETag", "\"v1\""),
        ScriptedResponse::status(304, "Not Modified").with_header("Cache-Control", "max-age=60"),
    ]);

    let mut config = FetchConfig::default();
    config.set_http_cache_dir(Some(cache_dir.display().to_string()));

    let first = fetch_with_config_for_test(&config, Request::get(&server.url()).unwrap()).unwrap();
    let second = fetch_with_config_for_test(&config, Request::get(&server.url()).unwrap()).unwrap();

    assert_eq!(first.body_text(), "hit-1");
    assert_eq!(second.body_text(), "hit-1");
    assert_eq!(server.hits(), 2);
    assert!(
        server.requests()[1]
            .to_ascii_lowercase()
            .contains("if-none-match: \"v1\""),
        "old Date should make the cached entry stale enough to validate: {}",
        server.requests()[1]
    );

    server.shutdown();
    let _ = std::fs::remove_dir_all(cache_dir);
}

#[test]
fn fetch_client_cache_expired_expires_header_revalidates() {
    let cache_dir = unique_test_cache_dir();
    let server = ScriptedHttpServer::spawn(vec![
        ScriptedResponse::ok("hit-1")
            .with_header("Expires", "Wed, 21 Oct 2015 07:28:00 GMT")
            .with_header("ETag", "\"v1\""),
        ScriptedResponse::status(304, "Not Modified")
            .with_header("Expires", "Wed, 07 Sep 2033 21:46:42 GMT"),
    ]);

    let mut config = FetchConfig::default();
    config.set_http_cache_dir(Some(cache_dir.display().to_string()));

    let first = fetch_with_config_for_test(&config, Request::get(&server.url()).unwrap()).unwrap();
    let second = fetch_with_config_for_test(&config, Request::get(&server.url()).unwrap()).unwrap();
    let third = fetch_with_config_for_test(&config, Request::get(&server.url()).unwrap()).unwrap();

    assert_eq!(first.body_text(), "hit-1");
    assert_eq!(second.body_text(), "hit-1");
    assert_eq!(third.body_text(), "hit-1");
    assert_eq!(
        server.hits(),
        2,
        "304 Expires update should make the third request a fresh cache hit"
    );

    server.shutdown();
    let _ = std::fs::remove_dir_all(cache_dir);
}

#[test]
fn fetch_client_cache_revalidates_with_etag() {
    let cache_dir = unique_test_cache_dir();
    let server = ScriptedHttpServer::spawn(vec![
        ScriptedResponse::ok("hit-1")
            .with_header("Cache-Control", "max-age=0")
            .with_header("ETag", "\"v1\""),
        ScriptedResponse::status(304, "Not Modified"),
    ]);

    let mut config = FetchConfig::default();
    config.set_http_cache_dir(Some(cache_dir.display().to_string()));

    let first = fetch_with_config_for_test(&config, Request::get(&server.url()).unwrap()).unwrap();
    let second = fetch_with_config_for_test(&config, Request::get(&server.url()).unwrap()).unwrap();

    assert_eq!(first.body_text(), "hit-1");
    assert_eq!(second.body_text(), "hit-1");
    assert_eq!(server.hits(), 2);
    assert!(
        server.requests()[1]
            .to_ascii_lowercase()
            .contains("if-none-match: \"v1\""),
        "unexpected request headers: {}",
        server.requests()[1]
    );

    server.shutdown();
    let _ = std::fs::remove_dir_all(cache_dir);
}

#[test]
fn fetch_client_cache_updates_freshness_after_not_modified() {
    let cache_dir = unique_test_cache_dir();
    let server = ScriptedHttpServer::spawn(vec![
        ScriptedResponse::ok("hit-1")
            .with_header("Cache-Control", "max-age=0")
            .with_header("ETag", "\"v1\""),
        ScriptedResponse::status(304, "Not Modified")
            .with_header("Cache-Control", "max-age=60")
            .with_header("ETag", "\"v1\""),
        ScriptedResponse::ok("hit-3").with_header("Cache-Control", "max-age=60"),
    ]);

    let mut config = FetchConfig::default();
    config.set_http_cache_dir(Some(cache_dir.display().to_string()));

    let first = fetch_with_config_for_test(&config, Request::get(&server.url()).unwrap()).unwrap();
    let second = fetch_with_config_for_test(&config, Request::get(&server.url()).unwrap()).unwrap();
    let third = fetch_with_config_for_test(&config, Request::get(&server.url()).unwrap()).unwrap();

    assert_eq!(first.body_text(), "hit-1");
    assert_eq!(second.body_text(), "hit-1");
    assert_eq!(third.body_text(), "hit-1");
    assert_eq!(
        server.hits(),
        2,
        "304 metadata update should make the next cache lookup fresh"
    );

    server.shutdown();
    let _ = std::fs::remove_dir_all(cache_dir);
}

#[test]
fn fetch_client_cache_revalidates_same_document_reload() {
    // Mirrors Chromium net/http/http_cache_unittest.cc LoadValidateCacheImplicit:
    // a request-level max-age=0 directive must validate an otherwise fresh hit.
    let cache_dir = unique_test_cache_dir();
    let server = ScriptedHttpServer::spawn(vec![
        ScriptedResponse::ok("hit-1")
            .with_header("Cache-Control", "max-age=60")
            .with_header("ETag", "\"v1\""),
        ScriptedResponse::status(304, "Not Modified")
            .with_header("Cache-Control", "max-age=60")
            .with_header("ETag", "\"v1\""),
    ]);

    let mut config = FetchConfig::default();
    config.set_http_cache_dir(Some(cache_dir.display().to_string()));
    let reload_url = Url::parse(&server.url()).unwrap();

    let first = fetch_with_config_for_test(&config, Request::get(&server.url()).unwrap()).unwrap();
    let second = fetch_with_config_for_test(
        &config,
        Request::get(&server.url())
            .unwrap()
            .with_browser_navigation_kind(BrowserNavigationRequestKind::Reload)
            .with_initiator_url(&reload_url),
    )
    .unwrap();

    assert_eq!(first.body_text(), "hit-1");
    assert_eq!(second.body_text(), "hit-1");
    assert_eq!(server.hits(), 2);
    let reload_request = server.requests()[1].to_ascii_lowercase();
    assert!(
        reload_request.contains("cache-control: max-age=0"),
        "reload should synthesize Cache-Control validation: {}",
        server.requests()[1]
    );
    assert!(
        reload_request.contains("if-none-match: \"v1\""),
        "reload should revalidate a fresh cached entry instead of serving it directly: {}",
        server.requests()[1]
    );

    server.shutdown();
    let _ = std::fs::remove_dir_all(cache_dir);
}

#[test]
fn fetch_client_cache_immutable_ignores_same_document_reload_validation_when_fresh() {
    let cache_dir = unique_test_cache_dir();
    let server = ScriptedHttpServer::spawn(vec![
        ScriptedResponse::ok("hit-1")
            .with_header("Cache-Control", "max-age=60, immutable")
            .with_header("ETag", "\"v1\""),
    ]);

    let mut config = FetchConfig::default();
    config.set_http_cache_dir(Some(cache_dir.display().to_string()));
    let reload_url = Url::parse(&server.url()).unwrap();

    let first = fetch_with_config_for_test(&config, Request::get(&server.url()).unwrap()).unwrap();
    let second = fetch_with_config_for_test(
        &config,
        Request::get(&server.url())
            .unwrap()
            .with_browser_navigation_kind(BrowserNavigationRequestKind::Reload)
            .with_initiator_url(&reload_url),
    )
    .unwrap();

    assert_eq!(first.body_text(), "hit-1");
    assert_eq!(second.body_text(), "hit-1");
    assert_eq!(
        server.hits(),
        1,
        "fresh immutable cache hits should satisfy reload without revalidation"
    );

    server.shutdown();
    let _ = std::fs::remove_dir_all(cache_dir);
}

#[test]
fn fetch_client_cache_immutable_revalidates_after_expiry() {
    let cache_dir = unique_test_cache_dir();
    let server = ScriptedHttpServer::spawn(vec![
        ScriptedResponse::ok("hit-1")
            .with_header("Cache-Control", "max-age=0, immutable")
            .with_header("ETag", "\"v1\""),
        ScriptedResponse::ok("hit-2").with_header("Cache-Control", "max-age=60, immutable"),
    ]);

    let mut config = FetchConfig::default();
    config.set_http_cache_dir(Some(cache_dir.display().to_string()));
    let reload_url = Url::parse(&server.url()).unwrap();

    let first = fetch_with_config_for_test(&config, Request::get(&server.url()).unwrap()).unwrap();
    let second = fetch_with_config_for_test(
        &config,
        Request::get(&server.url())
            .unwrap()
            .with_browser_navigation_kind(BrowserNavigationRequestKind::Reload)
            .with_initiator_url(&reload_url),
    )
    .unwrap();

    assert_eq!(first.body_text(), "hit-1");
    assert_eq!(second.body_text(), "hit-2");
    assert_eq!(
        server.hits(),
        2,
        "immutable must not allow stale entries to bypass network validation"
    );

    server.shutdown();
    let _ = std::fs::remove_dir_all(cache_dir);
}

#[test]
fn fetch_client_cache_removes_entry_when_304_returns_no_store() {
    // Mirrors Chromium CacheControlNoStore3 / ConditionalRequest304NoStore:
    // a 304 that says no-store invalidates the previously stored entry.
    let cache_dir = unique_test_cache_dir();
    let server = ScriptedHttpServer::spawn(vec![
        ScriptedResponse::ok("hit-1")
            .with_header("Cache-Control", "max-age=0")
            .with_header("ETag", "\"v1\""),
        ScriptedResponse::status(304, "Not Modified").with_header("Cache-Control", "no-store"),
        ScriptedResponse::ok("hit-3").with_header("Cache-Control", "max-age=60"),
    ]);

    let mut config = FetchConfig::default();
    config.set_http_cache_dir(Some(cache_dir.display().to_string()));

    let first = fetch_with_config_for_test(&config, Request::get(&server.url()).unwrap()).unwrap();
    let second = fetch_with_config_for_test(&config, Request::get(&server.url()).unwrap()).unwrap();
    let third = fetch_with_config_for_test(&config, Request::get(&server.url()).unwrap()).unwrap();

    assert_eq!(first.body_text(), "hit-1");
    assert_eq!(second.body_text(), "hit-1");
    assert_eq!(third.body_text(), "hit-3");
    assert_eq!(server.hits(), 3);
    assert!(
        !server.requests()[2]
            .to_ascii_lowercase()
            .contains("if-none-match"),
        "304 no-store should evict the old cache entry before the next request: {}",
        server.requests()[2]
    );

    server.shutdown();
    let _ = std::fs::remove_dir_all(cache_dir);
}

#[test]
fn fetch_client_cache_304_updates_vary_header_metadata() {
    let cache_dir = unique_test_cache_dir();
    let server = ScriptedHttpServer::spawn(vec![
        ScriptedResponse::ok("hit-1")
            .with_header("Cache-Control", "max-age=0")
            .with_header("ETag", "\"v1\"")
            .with_header("Vary", "User-Agent"),
        ScriptedResponse::status(304, "Not Modified")
            .with_header("Cache-Control", "max-age=60")
            .with_header("ETag", "\"v1\"")
            .with_header("Vary", "Accept-Language"),
        ScriptedResponse::ok("hit-3").with_header("Cache-Control", "max-age=60"),
    ]);

    let mut first_config = FetchConfig::default();
    first_config.set_http_cache_dir(Some(cache_dir.display().to_string()));
    first_config.set_user_agent("Moli/Test-UA-1");
    let mut second_config = FetchConfig::default();
    second_config.set_http_cache_dir(Some(cache_dir.display().to_string()));
    second_config.set_user_agent("Moli/Test-UA-2");

    let first =
        fetch_with_config_for_test(&first_config, Request::get(&server.url()).unwrap()).unwrap();
    let second =
        fetch_with_config_for_test(&first_config, Request::get(&server.url()).unwrap()).unwrap();
    let third =
        fetch_with_config_for_test(&second_config, Request::get(&server.url()).unwrap()).unwrap();

    assert_eq!(first.body_text(), "hit-1");
    assert_eq!(second.body_text(), "hit-1");
    assert_eq!(third.body_text(), "hit-1");
    assert_eq!(
        server.hits(),
        2,
        "304 Vary update should replace the cached Vary key for later lookups"
    );

    server.shutdown();
    let _ = std::fs::remove_dir_all(cache_dir);
}

#[test]
fn fetch_client_cache_304_without_vary_preserves_existing_vary_metadata() {
    let cache_dir = unique_test_cache_dir();
    let server = ScriptedHttpServer::spawn(vec![
        ScriptedResponse::ok("hit-1")
            .with_header("Cache-Control", "max-age=0")
            .with_header("ETag", "\"v1\"")
            .with_header("Vary", "User-Agent"),
        ScriptedResponse::status(304, "Not Modified")
            .with_header("Cache-Control", "max-age=60")
            .with_header("ETag", "\"v1\""),
        ScriptedResponse::ok("hit-3").with_header("Cache-Control", "max-age=60"),
    ]);

    let mut first_config = FetchConfig::default();
    first_config.set_http_cache_dir(Some(cache_dir.display().to_string()));
    first_config.set_user_agent("Moli/Test-UA-1");
    let mut second_config = FetchConfig::default();
    second_config.set_http_cache_dir(Some(cache_dir.display().to_string()));
    second_config.set_user_agent("Moli/Test-UA-2");

    let first =
        fetch_with_config_for_test(&first_config, Request::get(&server.url()).unwrap()).unwrap();
    let second =
        fetch_with_config_for_test(&first_config, Request::get(&server.url()).unwrap()).unwrap();
    let third =
        fetch_with_config_for_test(&second_config, Request::get(&server.url()).unwrap()).unwrap();

    assert_eq!(first.body_text(), "hit-1");
    assert_eq!(second.body_text(), "hit-1");
    assert_eq!(third.body_text(), "hit-3");
    assert_eq!(
        server.hits(),
        3,
        "304 without Vary should not delete the original cached Vary key"
    );

    server.shutdown();
    let _ = std::fs::remove_dir_all(cache_dir);
}

#[test]
fn fetch_client_cache_preserves_content_length_when_merging_304() {
    let cache_dir = unique_test_cache_dir();
    let server = ScriptedHttpServer::spawn(vec![
        ScriptedResponse::ok("hit-1")
            .with_header("Cache-Control", "max-age=0")
            .with_header("ETag", "\"v1\""),
        ScriptedResponse::status(304, "Not Modified")
            .with_header("Cache-Control", "max-age=60")
            .with_header("ETag", "\"v1\""),
    ]);

    let mut config = FetchConfig::default();
    config.set_http_cache_dir(Some(cache_dir.display().to_string()));

    let first = fetch_with_config_for_test(&config, Request::get(&server.url()).unwrap()).unwrap();
    let second = fetch_with_config_for_test(&config, Request::get(&server.url()).unwrap()).unwrap();

    assert_eq!(first.body_text(), "hit-1");
    assert_eq!(second.body_text(), "hit-1");
    assert_eq!(
        second
            .headers
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
            .map(|(_, value)| value.as_str()),
        Some("5"),
        "304 Content-Length must not replace the cached 200 Content-Length"
    );

    server.shutdown();
    let _ = std::fs::remove_dir_all(cache_dir);
}

#[test]
fn fetch_client_cache_skips_connection_nominated_headers_when_merging_304() {
    let cache_dir = unique_test_cache_dir();
    let server = ScriptedHttpServer::spawn(vec![
        ScriptedResponse::ok("hit-1")
            .with_header("Cache-Control", "max-age=0")
            .with_header("ETag", "\"v1\"")
            .with_header("X-Transient", "cached-value"),
        ScriptedResponse::status(304, "Not Modified")
            .with_header("Cache-Control", "max-age=60")
            .with_header("ETag", "\"v1\"")
            .with_header("Connection", "X-Transient")
            .with_header("X-Transient", "not-modified-value"),
        ScriptedResponse::ok("hit-3").with_header("Cache-Control", "max-age=60"),
    ]);

    let mut config = FetchConfig::default();
    config.set_http_cache_dir(Some(cache_dir.display().to_string()));

    let first = fetch_with_config_for_test(&config, Request::get(&server.url()).unwrap()).unwrap();
    let second = fetch_with_config_for_test(&config, Request::get(&server.url()).unwrap()).unwrap();
    let third = fetch_with_config_for_test(&config, Request::get(&server.url()).unwrap()).unwrap();

    assert_eq!(first.body_text(), "hit-1");
    assert_eq!(second.body_text(), "hit-1");
    assert_eq!(third.body_text(), "hit-1");
    assert_eq!(server.hits(), 2);
    assert_eq!(
        third
            .headers
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case("x-transient"))
            .map(|(_, value)| value.as_str()),
        Some("cached-value"),
        "304 Connection-nominated fields must not replace cached metadata"
    );

    server.shutdown();
    let _ = std::fs::remove_dir_all(cache_dir);
}

#[test]
fn fetch_client_cache_uses_expires_freshness_without_max_age() {
    let cache_dir = unique_test_cache_dir();
    let server = ScriptedHttpServer::spawn(vec![
        ScriptedResponse::ok("hit-1").with_header("Expires", "Wed, 07 Sep 2033 21:46:42 GMT"),
    ]);

    let mut config = FetchConfig::default();
    config.set_http_cache_dir(Some(cache_dir.display().to_string()));

    let first = fetch_with_config_for_test(&config, Request::get(&server.url()).unwrap()).unwrap();
    let second = fetch_with_config_for_test(&config, Request::get(&server.url()).unwrap()).unwrap();

    assert_eq!(first.body_text(), "hit-1");
    assert_eq!(second.body_text(), "hit-1");
    assert_eq!(server.hits(), 1);

    server.shutdown();
    let _ = std::fs::remove_dir_all(cache_dir);
}

#[test]
fn fetch_client_cache_skips_unknown_vary_header() {
    let cache_dir = unique_test_cache_dir();
    let server = ScriptedHttpServer::spawn(vec![
        ScriptedResponse::ok("hit-1")
            .with_header("Cache-Control", "max-age=60")
            .with_header("Vary", "Foo"),
        ScriptedResponse::ok("hit-2")
            .with_header("Cache-Control", "max-age=60")
            .with_header("Vary", "Foo"),
    ]);

    let mut config = FetchConfig::default();
    config.set_http_cache_dir(Some(cache_dir.display().to_string()));

    let first = fetch_with_config_for_test(&config, Request::get(&server.url()).unwrap()).unwrap();
    let second = fetch_with_config_for_test(&config, Request::get(&server.url()).unwrap()).unwrap();

    assert_eq!(first.body_text(), "hit-1");
    assert_eq!(second.body_text(), "hit-2");
    assert_eq!(server.hits(), 2);

    server.shutdown();
    let _ = std::fs::remove_dir_all(cache_dir);
}

#[test]
fn fetch_client_prunes_disk_cache_to_configured_quota() {
    let cache_dir = unique_test_cache_dir();
    let large_a = "a".repeat(1024);
    let large_b = "b".repeat(1024);
    let large_c = "c".repeat(1024);
    let large_d = "d".repeat(1024);
    let server = ScriptedHttpServer::spawn(vec![
        ScriptedResponse::ok(&large_a).with_header("Cache-Control", "max-age=60"),
        ScriptedResponse::ok(&large_b).with_header("Cache-Control", "max-age=60"),
        ScriptedResponse::ok(&large_c).with_header("Cache-Control", "max-age=60"),
        ScriptedResponse::ok(&large_d).with_header("Cache-Control", "max-age=60"),
    ]);

    let mut config = FetchConfig::default();
    config.set_http_cache_dir(Some(cache_dir.display().to_string()));
    config.set_http_cache_max_bytes(Some(3_000));

    let first =
        fetch_with_config_for_test(&config, Request::get(&server.url_path("/one")).unwrap())
            .unwrap();
    let second =
        fetch_with_config_for_test(&config, Request::get(&server.url_path("/two")).unwrap())
            .unwrap();
    let third =
        fetch_with_config_for_test(&config, Request::get(&server.url_path("/three")).unwrap())
            .unwrap();
    let first_again =
        fetch_with_config_for_test(&config, Request::get(&server.url_path("/one")).unwrap())
            .unwrap();

    assert_eq!(first.body_text(), large_a);
    assert_eq!(second.body_text(), large_b);
    assert_eq!(third.body_text(), large_c);
    assert_eq!(first_again.body_text(), large_d);
    assert_eq!(
        server.hits(),
        4,
        "small quota should evict the oldest cached entry"
    );

    server.shutdown();
    let _ = std::fs::remove_dir_all(cache_dir);
}

#[test]
fn http_cache_stats_reports_profile_cache_growth() -> Result<()> {
    let cache_dir = unique_test_cache_dir();
    let store = HttpCacheStore::new(&cache_dir);
    let readable_url = "http://example.test/readable";
    let orphan_url = "http://example.test/orphan";
    store_test_cache_body(
        &store,
        &HttpCacheStore::key_for_url(readable_url),
        HttpCacheEntryMetadata::new(
            readable_url.to_owned(),
            readable_url.to_owned(),
            200,
            Vec::new(),
            1,
            Some(2),
            Vec::new(),
        ),
        b"readable",
    )?;
    let orphan_dir = cache_dir.join(format!("{}.entry", HttpCacheStore::key_for_url(orphan_url)));
    fs::create_dir_all(&orphan_dir)?;
    fs::write(orphan_dir.join("body.orphan.bin"), b"orphaned")?;

    let mut config = FetchConfig::default();
    config.set_http_cache_dir(Some(cache_dir.display().to_string()));
    let stats = http_cache_stats(&config)?;

    assert_eq!(stats.entry_count, 1);
    assert_eq!(stats.unreadable_entry_count, 1);
    assert_eq!(stats.readable_body_bytes, 8);
    assert!(stats.total_bytes >= 16);

    let _ = std::fs::remove_dir_all(cache_dir);
    Ok(())
}

#[tokio::test]
async fn fetch_html_stream_treats_disk_cache_store_failure_as_best_effort() -> Result<()> {
    let cache_file = unique_test_cache_dir();
    fs::write(&cache_file, b"not a directory").unwrap();
    let server = ScriptedHttpServer::spawn(vec![
        ScriptedResponse::ok("<!doctype html><html><body>hit-1</body></html>")
            .with_header("Content-Type", "text/html; charset=utf-8")
            .with_header("Cache-Control", "max-age=60"),
    ]);

    let mut config = FetchConfig::default();
    config.set_http_cache_dir(Some(cache_file.display().to_string()));

    let client = FetchClient::new(&config, new_shared_browser_cookie_store());
    let mut response = client
        .fetch_html_stream(Request::get(&server.url()).unwrap())
        .await?;
    let mut body = String::new();
    while let Some(chunk) = response.next_chunk().await {
        body.push_str(&chunk);
    }
    response.finish().await?;

    assert_eq!(body, "<!doctype html><html><body>hit-1</body></html>");
    assert_eq!(server.hits(), 1);

    server.shutdown();
    let _ = std::fs::remove_file(cache_file);
    Ok(())
}

#[test]
fn fetch_client_cache_revalidates_with_last_modified() {
    let cache_dir = unique_test_cache_dir();
    let server = ScriptedHttpServer::spawn(vec![
        ScriptedResponse::ok("hit-1")
            .with_header("Cache-Control", "max-age=0")
            .with_header("Last-Modified", "Wed, 21 Oct 2015 07:28:00 GMT"),
        ScriptedResponse::status(304, "Not Modified"),
    ]);

    let mut config = FetchConfig::default();
    config.set_http_cache_dir(Some(cache_dir.display().to_string()));

    let first = fetch_with_config_for_test(&config, Request::get(&server.url()).unwrap()).unwrap();
    let second = fetch_with_config_for_test(&config, Request::get(&server.url()).unwrap()).unwrap();

    assert_eq!(first.body_text(), "hit-1");
    assert_eq!(second.body_text(), "hit-1");
    assert_eq!(server.hits(), 2);
    assert!(
        server.requests()[1]
            .to_ascii_lowercase()
            .contains("if-modified-since: wed, 21 oct 2015 07:28:00 gmt"),
        "unexpected request headers: {}",
        server.requests()[1]
    );

    server.shutdown();
    let _ = std::fs::remove_dir_all(cache_dir);
}

#[test]
fn fetch_client_reuses_existing_runtime_owner_for_sequential_requests() {
    let server = ScriptedHttpServer::spawn(vec![
        ScriptedResponse::ok("hit-1"),
        ScriptedResponse::ok("hit-2"),
    ]);
    let client = FetchClient::new(&FetchConfig::default(), new_shared_browser_cookie_store());

    let first = fetch_response_for_test(&client, Request::get(&server.url()).unwrap()).unwrap();
    let second = fetch_response_for_test(&client, Request::get(&server.url()).unwrap()).unwrap();

    assert_eq!(first.body_text(), "hit-1");
    assert_eq!(second.body_text(), "hit-2");
    assert_eq!(server.hits(), 2);
    assert_eq!(client.runtime_owner_count_for_testing(), 1);

    server.shutdown();
}

#[tokio::test]
async fn fetch_with_cancel_aborts_inflight_streaming_transfer() -> Result<()> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    let (disconnect_tx, disconnect_rx) = oneshot::channel();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.expect("cancel test accept");
        let _request = read_http_request_head(&mut stream)
            .await
            .expect("cancel test should read request");
        stream
            .write_all(
                b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nConnection: close\r\n\r\nhello",
            )
            .await
            .expect("cancel test should write initial response bytes");
        tokio::time::sleep(Duration::from_millis(150)).await;
        let mut tail = [0u8; 1];
        let disconnected = matches!(
            tokio::time::timeout(Duration::from_secs(2), stream.read(&mut tail)).await,
            Ok(Ok(0))
        );
        let _ = disconnect_tx.send(disconnected);
    });

    let client = FetchClient::new(&FetchConfig::default(), new_shared_browser_cookie_store());
    let cancel_handle = FetchCancelHandle::new();
    let request = Request::get(&format!("http://{addr}/slow"))?;
    let fetch_task = {
        let client = client.clone();
        let cancel_handle = cancel_handle.clone();
        tokio::spawn(async move { client.fetch_with_cancel(request, cancel_handle).await })
    };

    tokio::time::sleep(Duration::from_millis(20)).await;
    cancel_handle.cancel();

    let error = fetch_task
        .await
        .expect("fetch task should join")
        .unwrap_err();
    let error_chain = format!("{error:#}");
    assert!(
        error_chain.contains("fetch runtime request cancelled")
            || error_chain.contains("Callback aborted"),
        "unexpected cancel error: {error_chain}"
    );

    let disconnected = tokio::time::timeout(Duration::from_secs(3), disconnect_rx)
        .await
        .expect("server disconnect observation timed out")
        .expect("server disconnect observation channel closed");
    assert!(
        disconnected,
        "expected client transport to close after fetch cancellation"
    );

    server.await.expect("cancel test server should finish");
    Ok(())
}

#[tokio::test]
async fn fetch_raw_stream_with_cancel_aborts_inflight_raw_transfer() -> Result<()> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    let (disconnect_tx, disconnect_rx) = oneshot::channel();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.expect("raw cancel test accept");
        let _request = read_http_request_head(&mut stream)
            .await
            .expect("raw cancel test should read request");
        stream
            .write_all(
                b"HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nConnection: close\r\n\r\nhello",
            )
            .await
            .expect("raw cancel test should write initial response bytes");
        tokio::time::sleep(Duration::from_millis(150)).await;
        let mut tail = [0u8; 1];
        let disconnected = matches!(
            tokio::time::timeout(Duration::from_secs(2), stream.read(&mut tail)).await,
            Ok(Ok(0))
        );
        let _ = disconnect_tx.send(disconnected);
    });

    let client = FetchClient::new(&FetchConfig::default(), new_shared_browser_cookie_store());
    let cancel_handle = FetchCancelHandle::new();
    let request = Request::get(&format!("http://{addr}/slow-raw"))?;
    let mut response = client
        .fetch_raw_stream_with_cancel(request, cancel_handle.clone())
        .await?;
    assert_eq!(response.status, 200);

    tokio::time::sleep(Duration::from_millis(20)).await;
    cancel_handle.cancel();

    let error = response.finish().await.unwrap_err();
    let error_chain = format!("{error:#}");
    assert!(
        error_chain.contains("fetch runtime request cancelled")
            || error_chain.contains("Callback aborted"),
        "unexpected cancel error: {error_chain}"
    );

    let disconnected = tokio::time::timeout(Duration::from_secs(3), disconnect_rx)
        .await
        .expect("raw server disconnect observation timed out")
        .expect("raw server disconnect observation channel closed");
    assert!(
        disconnected,
        "expected client transport to close after raw fetch cancellation"
    );

    server.await.expect("raw cancel test server should finish");
    Ok(())
}

#[test]
fn fetch_client_runs_parallel_requests_on_one_runtime_owner() {
    let server = ScriptedHttpServer::spawn(vec![
        ScriptedResponse::ok("hit-1").with_delay_ms(250),
        ScriptedResponse::ok("hit-2").with_delay_ms(250),
    ]);
    let mut config = FetchConfig::default();
    config.set_connection_limits(NonZeroU32::new(2), None, None);
    let client = FetchClient::new(&config, new_shared_browser_cookie_store());
    let start = Arc::new(std::sync::Barrier::new(3));

    let left_client = client.clone();
    let left_start = Arc::clone(&start);
    let left_url = server.url();
    let left = thread::spawn(move || {
        left_start.wait();
        fetch_response_for_test(&left_client, Request::get(&left_url).unwrap())
    });

    let right_client = client.clone();
    let right_start = Arc::clone(&start);
    let right_url = server.url();
    let right = thread::spawn(move || {
        right_start.wait();
        fetch_response_for_test(&right_client, Request::get(&right_url).unwrap())
    });

    start.wait();
    wait_for_runtime_owner_count(&client, 1);
    let deadline = Instant::now() + Duration::from_secs(1);
    while Instant::now() < deadline && server.hits() < 2 {
        thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(
        server.hits(),
        2,
        "multi owner should start both transfers concurrently"
    );

    let mut bodies = [
        left.join().unwrap().unwrap().body_text().to_owned(),
        right.join().unwrap().unwrap().body_text().to_owned(),
    ];
    bodies.sort();
    assert_eq!(bodies, ["hit-1".to_owned(), "hit-2".to_owned()]);

    server.shutdown();
}

#[test]
fn fetch_runtime_wakes_active_owner_for_new_parallel_request() -> Result<()> {
    let server = ScriptedHttpServer::spawn(vec![
        ScriptedResponse::ok("slow").with_delay_ms(300),
        ScriptedResponse::ok("fast"),
    ]);
    let mut config = FetchConfig::default();
    config.set_connection_limits(NonZeroU32::new(2), None, None);
    let runtime = FetchRuntimeOwner::new(&config, new_shared_browser_cookie_store());

    let slow_rx = runtime.submit(Request::get(&server.url_path("/slow"))?)?;
    let deadline = Instant::now() + Duration::from_secs(1);
    while Instant::now() < deadline && server.hits() < 1 {
        thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(server.hits(), 1, "slow transfer should be active");

    let fast_rx = runtime.submit(Request::get(&server.url_path("/fast"))?)?;
    let deadline = Instant::now() + Duration::from_millis(150);
    while Instant::now() < deadline && server.hits() < 2 {
        thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(
        server.hits(),
        2,
        "active curl wait should wake and start newly submitted transfer"
    );

    assert_eq!(fast_rx.blocking_recv()??.body_text(), "fast");
    assert_eq!(slow_rx.blocking_recv()??.body_text(), "slow");

    server.shutdown();
    Ok(())
}

#[test]
fn fetch_client_respects_host_transfer_cap_for_parallel_same_host_requests() {
    let server = ScriptedHttpServer::spawn(vec![
        ScriptedResponse::ok("hit-1").with_delay_ms(250),
        ScriptedResponse::ok("hit-2").with_delay_ms(250),
    ]);
    let mut config = FetchConfig::default();
    config.set_connection_limits(NonZeroU32::new(2), NonZeroU32::new(1), None);
    let client = FetchClient::new(&config, new_shared_browser_cookie_store());
    let start = Arc::new(std::sync::Barrier::new(3));

    let left_client = client.clone();
    let left_start = Arc::clone(&start);
    let left_url = server.url();
    let left = thread::spawn(move || {
        left_start.wait();
        fetch_response_for_test(&left_client, Request::get(&left_url).unwrap())
    });

    let right_client = client.clone();
    let right_start = Arc::clone(&start);
    let right_url = server.url();
    let right = thread::spawn(move || {
        right_start.wait();
        fetch_response_for_test(&right_client, Request::get(&right_url).unwrap())
    });

    start.wait();
    wait_for_runtime_owner_count(&client, 1);
    let deadline = Instant::now() + Duration::from_secs(1);
    while Instant::now() < deadline && server.hits() < 1 {
        thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(server.hits(), 1, "first same-host transfer should start");
    thread::sleep(Duration::from_millis(100));
    assert_eq!(
        server.hits(),
        1,
        "host cap should keep the second same-host transfer pending"
    );

    let mut bodies = [
        left.join().unwrap().unwrap().body_text().to_owned(),
        right.join().unwrap().unwrap().body_text().to_owned(),
    ];
    bodies.sort();
    assert_eq!(bodies, ["hit-1".to_owned(), "hit-2".to_owned()]);

    server.shutdown();
}

#[test]
fn fetch_runtime_starts_queued_high_priority_request_before_low_priority_request() -> Result<()> {
    let server = ScriptedHttpServer::spawn(vec![
        ScriptedResponse::ok("first").with_delay_ms(250),
        ScriptedResponse::ok("high"),
        ScriptedResponse::ok("low"),
    ]);
    let mut config = FetchConfig::default();
    config.set_connection_limits(NonZeroU32::new(1), None, None);
    let runtime = FetchRuntimeOwner::new(&config, new_shared_browser_cookie_store());

    let first_rx = runtime.submit(Request::get(&server.url_path("/first"))?)?;
    let deadline = Instant::now() + Duration::from_secs(1);
    while Instant::now() < deadline && server.hits() < 1 {
        thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(server.hits(), 1, "first transfer should be active");

    let low_request = Request::new("GET", &server.url_path("/low"), None, Vec::new())?
        .with_script_fetch_metadata(ScriptFetchRequestMetadata {
            fetch_priority: Some(FetchPriorityHint::Low),
            ..ScriptFetchRequestMetadata::default()
        });
    let high_request = Request::new("GET", &server.url_path("/high"), None, Vec::new())?
        .with_script_fetch_metadata(ScriptFetchRequestMetadata {
            fetch_priority: Some(FetchPriorityHint::High),
            ..ScriptFetchRequestMetadata::default()
        });
    let low_rx = runtime.submit(low_request)?;
    let high_rx = runtime.submit(high_request)?;

    let first = first_rx
        .blocking_recv()
        .expect("first response channel should complete")?;
    let high = high_rx
        .blocking_recv()
        .expect("high response channel should complete")?;
    let low = low_rx
        .blocking_recv()
        .expect("low response channel should complete")?;

    assert_eq!(first.body_text(), "first");
    assert_eq!(high.body_text(), "high");
    assert_eq!(low.body_text(), "low");
    let requests = server.requests();
    let request_paths = requests
        .iter()
        .map(|request| request_path(request))
        .collect::<Vec<_>>();
    assert_eq!(request_paths, ["/first", "/high", "/low"]);

    server.shutdown();
    Ok(())
}

#[test]
fn fetch_runtime_keeps_browser_fetch_and_auto_script_at_same_default_priority() -> Result<()> {
    let server = ScriptedHttpServer::spawn(vec![
        ScriptedResponse::ok("first").with_delay_ms(250),
        ScriptedResponse::ok("second"),
        ScriptedResponse::ok("third"),
    ]);
    let mut config = FetchConfig::default();
    config.set_connection_limits(NonZeroU32::new(1), None, None);
    let runtime = FetchRuntimeOwner::new(&config, new_shared_browser_cookie_store());

    let first_rx = runtime.submit(Request::get(&server.url_path("/first"))?)?;
    let deadline = Instant::now() + Duration::from_secs(1);
    while Instant::now() < deadline && server.hits() < 1 {
        thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(server.hits(), 1, "first transfer should be active");

    let script_request = Request::new("GET", &server.url_path("/script"), None, Vec::new())?
        .with_script_fetch_metadata(ScriptFetchRequestMetadata::default());
    let fetch_request = Request::new("GET", &server.url_path("/fetch"), None, Vec::new())?
        .with_browser_request_metadata(BrowserRequestMetadata::Fetch);
    let script_rx = runtime.submit(script_request)?;
    let fetch_rx = runtime.submit(fetch_request)?;

    let first = first_rx
        .blocking_recv()
        .expect("first response channel should complete")?;
    let fetch = fetch_rx
        .blocking_recv()
        .expect("fetch response channel should complete")?;
    let script = script_rx
        .blocking_recv()
        .expect("script response channel should complete")?;

    assert_eq!(first.body_text(), "first");
    assert_eq!(script.body_text(), "second");
    assert_eq!(fetch.body_text(), "third");
    let requests = server.requests();
    let request_paths = requests
        .iter()
        .map(|request| request_path(request))
        .collect::<Vec<_>>();
    assert_eq!(request_paths, ["/first", "/script", "/fetch"]);

    server.shutdown();
    Ok(())
}

#[test]
fn fetch_runtime_negotiates_http2_over_tls() {
    let server = ScriptedH2Server::spawn(vec![ScriptedResponse::ok("h2-ok")]);
    let mut config = FetchConfig::default();
    config.set_tls_verify_host(false);
    let client = FetchClient::new(&config, new_shared_browser_cookie_store());

    let response =
        fetch_response_for_test(&client, Request::get(&server.url_path("/h2")).unwrap()).unwrap();

    assert_eq!(response.body_text(), "h2-ok");
    assert_eq!(
        response.negotiated_http_version,
        Some(NegotiatedHttpVersion::Http2)
    );
    assert_eq!(server.hits(), 1);
    assert_eq!(server.requests(), ["/h2".to_owned()]);

    server.shutdown();
}

#[test]
fn fetch_runtime_reports_http10_from_wire_response() {
    let server = ScriptedHttpServer::spawn(vec![
        ScriptedResponse::ok("http10-ok").with_http_version("1.0"),
    ]);
    let client = FetchClient::new(&FetchConfig::default(), new_shared_browser_cookie_store());

    let response =
        fetch_response_for_test(&client, Request::get(&server.url_path("/http10")).unwrap())
            .unwrap();

    assert_eq!(response.body_text(), "http10-ok");
    assert_eq!(
        response.negotiated_http_version,
        Some(NegotiatedHttpVersion::Http10)
    );
    server.shutdown();
}

#[tokio::test]
async fn fetch_runtime_raw_stream_reports_negotiated_http2() -> Result<()> {
    let server = ScriptedH2Server::spawn(vec![ScriptedResponse::ok("h2-stream")]);
    let mut config = FetchConfig::default();
    config.set_tls_verify_host(false);
    let client = FetchClient::new(&config, new_shared_browser_cookie_store());

    let mut response = client
        .fetch_raw_stream_with_cancel(
            Request::get(&server.url_path("/h2-stream"))?,
            FetchCancelHandle::new(),
        )
        .await?;
    assert_eq!(
        response.negotiated_http_version,
        Some(NegotiatedHttpVersion::Http2)
    );
    let mut body = Vec::new();
    while let Some(chunk) = response.next_chunk().await {
        body.extend_from_slice(&chunk);
    }
    response.finish().await?;

    assert_eq!(body, b"h2-stream");
    server.shutdown();
    Ok(())
}

#[test]
fn fetch_runtime_retries_streaming_html_get_over_http11_after_http2_protocol_error() -> Result<()> {
    let server = Http2ProtocolFallbackServer::spawn();
    let mut config = FetchConfig::default();
    config.set_tls_verify_host(false);
    let client = FetchClient::new(&config, new_shared_browser_cookie_store());
    let body = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?
        .block_on(async {
            let mut response = client
                .fetch_html_stream(Request::get(&server.url())?)
                .await?;
            let mut body = String::new();
            while let Some(chunk) = response.next_chunk().await {
                body.push_str(&chunk);
            }
            response.finish().await?;
            Ok::<_, anyhow::Error>(body)
        })?;

    assert_eq!(body, "http1 fallback");
    assert_eq!(server.h2_hits(), 1);
    assert_eq!(server.http11_hits(), 1);

    server.shutdown();
    Ok(())
}

#[test]
fn fetch_runtime_retries_raw_streaming_get_over_http11_after_http2_protocol_error() -> Result<()> {
    let server = Http2ProtocolFallbackServer::spawn();
    let mut config = FetchConfig::default();
    config.set_tls_verify_host(false);
    let client = FetchClient::new(&config, new_shared_browser_cookie_store());

    let response = fetch_response_for_test(&client, Request::get(&server.url())?)?;

    assert_eq!(response.body_text(), "http1 fallback");
    assert_eq!(server.h2_hits(), 1);
    assert_eq!(server.http11_hits(), 1);

    server.shutdown();
    Ok(())
}

#[test]
fn fetch_runtime_retries_buffered_get_over_http11_after_http2_protocol_error() -> Result<()> {
    let server = Http2ProtocolFallbackServer::spawn();
    let mut config = FetchConfig::default();
    config.set_tls_verify_host(false);
    let client = FetchClient::new(&config, new_shared_browser_cookie_store());
    let request = Request::get(&server.url())?.with_follow_redirects(false);

    let response = fetch_response_for_test(&client, request)?;

    assert_eq!(response.body_text(), "http1 fallback");
    assert_eq!(server.h2_hits(), 1);
    assert_eq!(server.http11_hits(), 1);

    server.shutdown();
    Ok(())
}

#[test]
fn fetch_runtime_does_not_replay_post_after_http2_protocol_error() -> Result<()> {
    let server = Http2ProtocolFallbackServer::spawn();
    let mut config = FetchConfig::default();
    config.set_tls_verify_host(false);
    let client = FetchClient::new(&config, new_shared_browser_cookie_store());
    let request = Request::new("POST", &server.url(), Some("body".to_owned()), Vec::new())?;

    fetch_response_for_test(&client, request)
        .expect_err("POST should not be replayed after an HTTP/2 protocol error");
    assert_eq!(server.h2_hits(), 1);
    assert_eq!(server.http11_hits(), 0);

    server.shutdown();
    Ok(())
}

#[tokio::test]
async fn fetch_runtime_retries_streaming_html_empty_http_navigation_over_https() -> Result<()> {
    let server = EmptyHttpHttpsUpgradeServer::spawn();
    let mut config = FetchConfig::default();
    config.set_tls_verify_host(false);
    config.set_http_host_resolve(vec![server.resolve_entry()]);
    let client = FetchClient::new(&config, new_shared_browser_cookie_store());
    let mut response = client
        .fetch_html_stream(Request::get(&server.url())?)
        .await?;
    let mut body = String::new();
    while let Some(chunk) = response.next_chunk().await {
        body.push_str(&chunk);
    }
    response.finish().await?;

    assert_eq!(body, "https upgrade fallback");
    assert_eq!(response.final_url.scheme(), "https");
    assert!(response.redirected);
    assert_https_upgrade_redirect(&response.redirect_chain);
    assert_eq!(server.http_hits(), 1);
    assert_eq!(server.https_hits(), 1);

    server.shutdown();
    Ok(())
}

#[tokio::test]
async fn fetch_runtime_retries_raw_streaming_empty_http_navigation_over_https() -> Result<()> {
    let server = EmptyHttpHttpsUpgradeServer::spawn();
    let mut config = FetchConfig::default();
    config.set_tls_verify_host(false);
    config.set_http_host_resolve(vec![server.resolve_entry()]);
    let client = FetchClient::new(&config, new_shared_browser_cookie_store());
    let mut response = client
        .fetch_raw_stream_with_cancel(Request::get(&server.url())?, FetchCancelHandle::new())
        .await?;
    let mut body = Vec::new();
    while let Some(chunk) = response.next_chunk().await {
        body.extend_from_slice(&chunk);
    }
    response.finish().await?;

    assert_eq!(body, b"https upgrade fallback");
    assert_eq!(response.final_url.scheme(), "https");
    assert!(response.redirected);
    assert_https_upgrade_redirect(&response.redirect_chain);
    assert_eq!(server.http_hits(), 1);
    assert_eq!(server.https_hits(), 1);

    server.shutdown();
    Ok(())
}

#[test]
fn fetch_runtime_retries_buffered_empty_http_navigation_over_https() -> Result<()> {
    let server = EmptyHttpHttpsUpgradeServer::spawn();
    let mut config = FetchConfig::default();
    config.set_tls_verify_host(false);
    config.set_http_host_resolve(vec![server.resolve_entry()]);
    let client = FetchClient::new(&config, new_shared_browser_cookie_store());
    let request = Request::get(&server.url())?.with_follow_redirects(false);

    let response = fetch_response_for_test(&client, request)?;

    assert_eq!(response.body_text(), "https upgrade fallback");
    assert_eq!(response.final_url.scheme(), "https");
    assert!(response.redirected);
    assert_https_upgrade_redirect(&response.redirect_chain);
    assert_eq!(server.http_hits(), 1);
    assert_eq!(server.https_hits(), 1);

    server.shutdown();
    Ok(())
}

fn assert_https_upgrade_redirect(redirect_chain: &[crate::RedirectInfo]) {
    assert_eq!(redirect_chain.len(), 1);
    let redirect = &redirect_chain[0];
    assert_eq!(redirect.status, 307);
    assert_eq!(redirect.from_url.scheme(), "http");
    assert_eq!(redirect.to_url.scheme(), "https");
    assert!(
        redirect.headers.iter().any(|(name, value)| {
            name == "non-authoritative-reason" && value == "HttpsUpgrades"
        })
    );
}

#[test]
fn fetch_runtime_starts_concurrent_http2_requests_without_waiting_for_reuse() {
    let server = ScriptedH2Server::spawn(vec![
        ScriptedResponse::ok("h2-a").with_delay_ms(250),
        ScriptedResponse::ok("h2-b").with_delay_ms(250),
    ]);
    let mut config = FetchConfig::default();
    config.set_connection_limits(NonZeroU32::new(2), None, None);
    config.set_tls_verify_host(false);
    let client = FetchClient::new(&config, new_shared_browser_cookie_store());
    let start = Arc::new(std::sync::Barrier::new(3));

    let left_client = client.clone();
    let left_start = Arc::clone(&start);
    let left_url = server.url_path("/h2-a");
    let left = thread::spawn(move || {
        left_start.wait();
        fetch_response_for_test(&left_client, Request::get(&left_url).unwrap())
    });

    let right_client = client.clone();
    let right_start = Arc::clone(&start);
    let right_url = server.url_path("/h2-b");
    let right = thread::spawn(move || {
        right_start.wait();
        fetch_response_for_test(&right_client, Request::get(&right_url).unwrap())
    });

    start.wait();
    let deadline = Instant::now() + Duration::from_secs(1);
    while Instant::now() < deadline && server.hits() < 2 {
        thread::sleep(Duration::from_millis(10));
    }

    assert_eq!(
        server.hits(),
        2,
        "concurrent H2 requests should start without waiting for a reusable TLS connection"
    );
    let mut bodies = [
        left.join().unwrap().unwrap().body_text().to_owned(),
        right.join().unwrap().unwrap().body_text().to_owned(),
    ];
    bodies.sort();
    assert_eq!(bodies, ["h2-a".to_owned(), "h2-b".to_owned()]);
    let mut requests = server.requests();
    requests.sort();
    assert_eq!(requests, ["/h2-a".to_owned(), "/h2-b".to_owned()]);
    assert_eq!(
        server.connection_stream_counts().iter().sum::<usize>(),
        2,
        "server should observe both requests as H2 streams"
    );

    server.shutdown();
}

#[test]
fn fetch_runtime_starts_streaming_html_and_fetch_without_waiting_for_h2_reuse() {
    let server = ScriptedH2Server::spawn(vec![
        ScriptedResponse::ok("<!doctype html><p>stream</p>").with_delay_ms(250),
        ScriptedResponse::ok("h2-fetch").with_delay_ms(250),
    ]);
    let mut config = FetchConfig::default();
    config.set_connection_limits(NonZeroU32::new(2), None, None);
    config.set_tls_verify_host(false);
    let client = FetchClient::new(&config, new_shared_browser_cookie_store());
    let start = Arc::new(std::sync::Barrier::new(3));

    let stream_client = client.clone();
    let stream_start = Arc::clone(&start);
    let stream_url = server.url_path("/stream");
    let stream = thread::spawn(move || -> Result<String> {
        stream_start.wait();
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?
            .block_on(async move {
                let mut response = stream_client
                    .fetch_html_stream(Request::get(&stream_url).unwrap())
                    .await?;
                let mut body = String::new();
                while let Some(chunk) = response.next_chunk().await {
                    body.push_str(&chunk);
                }
                response.finish().await?;
                Ok(body)
            })
    });

    let fetch_client = client.clone();
    let fetch_start = Arc::clone(&start);
    let fetch_url = server.url_path("/fetch");
    let fetch = thread::spawn(move || {
        fetch_start.wait();
        fetch_response_for_test(&fetch_client, Request::get(&fetch_url).unwrap())
            .map(|response| response.body_text().to_owned())
    });

    start.wait();
    let deadline = Instant::now() + Duration::from_secs(1);
    while Instant::now() < deadline && server.hits() < 2 {
        thread::sleep(Duration::from_millis(10));
    }

    assert_eq!(
        server.hits(),
        2,
        "streaming HTML and ordinary fetch should start without waiting for a reusable TLS connection"
    );
    let mut bodies = [
        stream.join().unwrap().unwrap(),
        fetch.join().unwrap().unwrap(),
    ];
    bodies.sort();
    assert_eq!(
        bodies,
        [
            "<!doctype html><p>stream</p>".to_owned(),
            "h2-fetch".to_owned()
        ]
    );
    let mut requests = server.requests();
    requests.sort();
    assert_eq!(requests, ["/fetch".to_owned(), "/stream".to_owned()]);
    assert_eq!(
        server.connection_stream_counts().iter().sum::<usize>(),
        2,
        "server should observe both requests as H2 streams"
    );

    server.shutdown();
}

#[test]
fn fetch_client_recovers_after_runtime_owner_panic() -> Result<()> {
    let server = ScriptedHttpServer::spawn(vec![ScriptedResponse::ok("hit-1")]);
    let client = FetchClient::new(&FetchConfig::default(), new_shared_browser_cookie_store());
    let panic_request = Request::new(
        "GET",
        &server.url(),
        None,
        vec![("x-moli-test-panic".to_owned(), "runtime-worker".to_owned())],
    )?;

    let panic_error = fetch_response_for_test(&client, panic_request)
        .unwrap_err()
        .to_string();
    assert!(
        panic_error.contains("fetch runtime owner panicked while handling GET"),
        "unexpected panic error: {panic_error}"
    );

    let follow_up_client = client.clone();
    let follow_up_url = server.url();
    let (result_tx, result_rx) = std_mpsc::channel();
    let follow_up = thread::spawn(move || {
        let result =
            fetch_response_for_test(&follow_up_client, Request::get(&follow_up_url).unwrap());
        let _ = result_tx.send(result);
    });
    let response = result_rx
        .recv_timeout(Duration::from_secs(1))
        .expect("follow-up request should not hang")?;
    assert_eq!(response.body_text(), "hit-1");
    follow_up
        .join()
        .expect("follow-up request should join cleanly");
    assert_eq!(client.runtime_owner_count_for_testing(), 1);

    server.shutdown();
    Ok(())
}

fn request_path(request: &str) -> &str {
    request
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .expect("scripted test request should have a request target")
}

#[test]
fn curl_build_reports_brotli_and_http2_support() {
    let version = curl::Version::get();

    assert!(
        version.feature_http2(),
        "libcurl is missing HTTP/2 support: {:?}",
        version
    );
    assert!(
        version.feature_brotli(),
        "libcurl is missing Brotli support: {:?}",
        version
    );
    assert!(
        version.brotli_version().is_some(),
        "libcurl reports Brotli feature but no Brotli version: {:?}",
        version
    );
}

#[test]
fn dropping_fetch_runtime_cancels_inflight_requests_without_waiting_for_request_timeout() {
    let server = ScriptedHttpServer::spawn(vec![ScriptedResponse::ok("slow").with_delay_ms(2_000)]);
    let mut config = FetchConfig::default();
    config.set_request_timeout_ms(10_000);
    let runtime = FetchRuntimeOwner::new(&config, new_shared_browser_cookie_store());
    let response_rx = runtime
        .submit(Request::get(&server.url()).unwrap())
        .unwrap();

    let deadline = Instant::now() + Duration::from_secs(1);
    while Instant::now() < deadline {
        if runtime.owner_count_for_testing() == 1 && server.hits() >= 1 {
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }

    assert_eq!(runtime.owner_count_for_testing(), 1);
    assert!(
        server.hits() >= 1,
        "fetch runtime owner never started the in-flight request"
    );

    // Dropping the runtime while a request is still running should signal curl
    // cancellation and then join the owner, rather than waiting for network
    // completion or the configured request timeout.
    let drop_started = Instant::now();
    drop(runtime);
    assert!(
        drop_started.elapsed() < Duration::from_secs(1),
        "runtime drop waited too long for in-flight request cancellation"
    );

    let error = response_rx
        .blocking_recv()
        .expect("in-flight request should still respond after shutdown")
        .expect_err("in-flight request should be cancelled during shutdown");
    assert!(
        error.to_string().contains("cancelled during shutdown"),
        "unexpected shutdown error: {error:#}"
    );

    server.shutdown();
}

#[test]
fn explicit_fetch_runtime_shutdown_cancels_inflight_requests_with_live_clones() {
    let server = ScriptedHttpServer::spawn(vec![ScriptedResponse::ok("slow").with_delay_ms(2_000)]);
    let mut config = FetchConfig::default();
    config.set_request_timeout_ms(10_000);
    let runtime = FetchRuntimeOwner::new(&config, new_shared_browser_cookie_store());
    let runtime_clone = runtime.clone();
    let response_rx = runtime
        .submit(Request::get(&server.url()).unwrap())
        .unwrap();

    let deadline = Instant::now() + Duration::from_secs(1);
    while Instant::now() < deadline {
        if runtime.owner_count_for_testing() == 1 && server.hits() >= 1 {
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }

    assert_eq!(runtime.owner_count_for_testing(), 1);
    assert!(
        server.hits() >= 1,
        "fetch runtime owner never started the in-flight request"
    );

    let shutdown_started = Instant::now();
    assert!(runtime.shutdown().is_clean());
    assert!(
        shutdown_started.elapsed() < Duration::from_secs(1),
        "explicit runtime shutdown waited too long for in-flight request cancellation"
    );

    let error = response_rx
        .blocking_recv()
        .expect("in-flight request should still respond after shutdown")
        .expect_err("in-flight request should be cancelled during shutdown");
    assert!(
        error.to_string().contains("cancelled during shutdown"),
        "unexpected shutdown error: {error:#}"
    );

    let submit_after_shutdown = runtime_clone.submit(Request::get(&server.url()).unwrap());
    assert!(
        submit_after_shutdown.is_err(),
        "live clone should reject requests after explicit shutdown"
    );
    server.shutdown();
}

#[test]
fn owner_thread_can_release_request_handle_before_external_owner_join() {
    let server = ScriptedHttpServer::spawn(vec![ScriptedResponse::ok("complete")]);
    let runtime =
        FetchRuntimeOwner::new(&FetchConfig::default(), new_shared_browser_cookie_store());
    let callback_runtime = runtime.clone();
    let (callback_entered_tx, callback_entered_rx) = std_mpsc::channel();
    let (release_callback_tx, release_callback_rx) = std_mpsc::channel();
    let (callback_exited_tx, callback_exited_rx) = std_mpsc::channel();

    runtime
        .submit_with_cancel_callback(
            Request::get(&server.url()).unwrap(),
            FetchCancelHandle::new(),
            Box::new(move |result| {
                callback_entered_tx
                    .send(result.map(|response| response.body_text().to_owned()))
                    .expect("test should still be waiting for the callback");
                release_callback_rx
                    .recv()
                    .expect("test should release the owner-thread callback");
                // This is a request handle, not the JoinHandle-owning runtime
                // owner. Releasing it on lm-fetch-semantics must be inert.
                drop(callback_runtime);
                callback_exited_tx
                    .send(())
                    .expect("test should still be waiting for callback exit");
            }),
        )
        .unwrap();

    let body = callback_entered_rx
        .recv_timeout(Duration::from_secs(1))
        .expect("fetch callback should run on the runtime owner")
        .expect("fetch request should complete successfully");
    assert_eq!(body, "complete");
    release_callback_tx
        .send(())
        .expect("owner callback should still be waiting");
    callback_exited_rx
        .recv_timeout(Duration::from_secs(1))
        .expect("owner callback should release its request handle without self-joining");

    assert!(runtime.shutdown().is_clean());

    server.shutdown();
}

#[test]
fn concurrent_request_handles_can_request_shutdown_before_external_join() {
    let runtime =
        FetchRuntimeOwner::new(&FetchConfig::default(), new_shared_browser_cookie_store());
    let post_shutdown_handle = runtime.clone();

    let start = Arc::new(std::sync::Barrier::new(3));
    let shutdowns = [runtime.clone(), runtime.clone()].map(|handle| {
        let start = Arc::clone(&start);
        thread::spawn(move || {
            start.wait();
            handle.request_shutdown();
        })
    });
    start.wait();
    for shutdown in shutdowns {
        shutdown
            .join()
            .expect("shutdown requester should not panic");
    }

    let external_join_started = Instant::now();
    assert!(runtime.shutdown().is_clean());
    assert!(
        external_join_started.elapsed() < Duration::from_secs(1),
        "external owner join waited too long after concurrent shutdown requests"
    );
    assert!(
        post_shutdown_handle
            .submit(Request::get("https://example.test/").unwrap())
            .is_err(),
        "all request handles should observe the concurrent shutdown"
    );
}

#[test]
fn fetch_runtime_owner_panic_returns_payload_and_identity_and_other_owners_still_join() {
    let mut panicking_owner =
        FetchRuntimeOwner::new(&FetchConfig::default(), new_shared_browser_cookie_store());
    let healthy_owner =
        FetchRuntimeOwner::new(&FetchConfig::default(), new_shared_browser_cookie_store());
    let panic_log_count = panicking_owner.panic_log_count_for_testing();

    panicking_owner.panic_owner_for_testing();
    let report = panicking_owner.join();
    assert_eq!(report, panicking_owner.join(), "join must be idempotent");
    assert_eq!(report.identity().thread_name(), "lm-fetch-semantics");
    assert!(!report.identity().thread_id().is_empty());
    let FetchRuntimeJoinStatus::Panicked(panic) = report.status() else {
        panic!("expected the deterministic owner panic, got {report:?}");
    };
    assert_eq!(panic.payload(), "deterministic fetch runtime panic");
    let location = panic
        .location()
        .expect("semantic owner panic must retain its panic-site location");
    assert!(
        location.contains("moli-fetch/src/runtime.rs"),
        "unexpected panic location: {location}"
    );
    let backtrace = panic
        .backtrace()
        .expect("semantic owner panic must retain its panic-site backtrace");
    assert!(!backtrace.trim().is_empty());
    assert!(
        backtrace.contains("RuntimeOwner::handle_command"),
        "panic backtrace should identify the semantic owner command boundary: {backtrace}"
    );
    assert_eq!(
        panic_log_count.load(std::sync::atomic::Ordering::SeqCst),
        0,
        "explicit join returns evidence without pre-empting the Drop fallback log"
    );

    let healthy_report = healthy_owner.shutdown();
    assert!(healthy_report.is_clean());
    assert_ne!(
        report.identity().runtime_id(),
        healthy_report.identity().runtime_id(),
        "join reports must identify the exact runtime"
    );

    drop(panicking_owner);
    assert_eq!(
        panic_log_count.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "one owner panic must be logged exactly once"
    );
}

#[test]
fn panicking_fetch_runtime_owner_drop_does_not_replace_an_outer_unwind() {
    let runtime =
        FetchRuntimeOwner::new(&FetchConfig::default(), new_shared_browser_cookie_store());
    let panic_log_count = runtime.panic_log_count_for_testing();

    let unwind = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
        runtime.panic_owner_for_testing();
        panic!("outer unwind sentinel");
    }));

    let payload = unwind.expect_err("the outer panic should remain observable");
    assert_eq!(
        payload.downcast_ref::<&'static str>().copied(),
        Some("outer unwind sentinel"),
        "Drop join must not replace the original panic"
    );
    assert_eq!(
        panic_log_count.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "Drop during unwind must log the owner panic once without panicking"
    );
}
