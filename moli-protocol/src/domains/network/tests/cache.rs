use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

fn unique_cdp_cache_dir(label: &str) -> std::path::PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock should be after epoch")
        .as_nanos();
    std::env::temp_dir().join(format!("moli-cdp-{label}-{}-{nonce}", std::process::id()))
}

async fn wait_until_cached_request_finished(ctx: &mut TestContext, description: &str) {
    wait_until_messages(ctx, Some("SID-1"), description, |messages| {
        let Some(request_id) = messages.iter().find_map(|message| {
            if message["method"] == json!("Network.requestServedFromCache") {
                message["params"]["requestId"].as_str()
            } else {
                None
            }
        }) else {
            return false;
        };
        messages.iter().any(|message| {
            message["method"] == json!("Network.loadingFinished")
                && message["params"]["requestId"] == json!(request_id)
        })
    })
    .await;
}

fn assert_cached_request_event_order(messages: &[serde_json::Value]) -> &serde_json::Value {
    let cached_index = messages
        .iter()
        .position(|message| message["method"] == json!("Network.requestServedFromCache"))
        .expect("requestServedFromCache event");
    let request_id = messages[cached_index]["params"]["requestId"]
        .as_str()
        .expect("cached request id");
    let event_index = |method: &str| {
        messages
            .iter()
            .position(|message| {
                message["method"] == json!(method)
                    && message["params"]["requestId"] == json!(request_id)
            })
            .unwrap_or_else(|| panic!("cached request {method} event"))
    };
    let request_index = event_index("Network.requestWillBeSent");
    let response_index = event_index("Network.responseReceived");
    let data_index = event_index("Network.dataReceived");
    let finished_index = event_index("Network.loadingFinished");

    assert!(
        request_index < cached_index
            && cached_index < response_index
            && response_index < data_index
            && data_index < finished_index
    );
    assert_eq!(messages[response_index]["params"]["hasExtraInfo"], false);
    assert!(
        !messages.iter().any(|message| {
            matches!(
                message["method"].as_str(),
                Some("Network.requestWillBeSentExtraInfo" | "Network.responseReceivedExtraInfo")
            ) && message["params"]["requestId"] == json!(request_id)
        }),
        "a memory/disk cache hit must not claim raw network extra info"
    );
    &messages[response_index]
}

#[tokio::test(flavor = "multi_thread")]
async fn cached_subresource_reload_emits_served_from_cache_before_response() {
    async fn host(State(_): State<Arc<AtomicUsize>>) -> impl IntoResponse {
        (
            [
                (CONTENT_TYPE.as_str(), "text/html"),
                ("cache-control", "no-store"),
            ],
            r#"<!doctype html><script src="/immutable.js"></script><body>cache host</body>"#,
        )
    }

    async fn immutable_script(State(hits): State<Arc<AtomicUsize>>) -> impl IntoResponse {
        hits.fetch_add(1, Ordering::SeqCst);
        (
            [
                (CONTENT_TYPE.as_str(), "application/javascript"),
                ("cache-control", "public, max-age=31536000, immutable"),
                ("etag", "\"moli-cache-v1\""),
            ],
            "globalThis.__moli_cache_marker = 1;",
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let asset_hits = Arc::new(AtomicUsize::new(0));
    let server_hits = Arc::clone(&asset_hits);
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/host", get(host))
                .route("/immutable.js", get(immutable_script))
                .with_state(server_hits),
        )
        .await
        .unwrap();
    });

    let cache_dir = unique_cdp_cache_dir("subresource-served-from-cache");
    let mut fetch_config = FetchConfig::default();
    fetch_config.set_http_cache_dir(Some(cache_dir.display().to_string()));
    let mut ctx = TestContext::from_conn(CdpConnection::new_with_fetch_config(fetch_config));
    let mut browser_context = ctx.conn.new_browser_context("BID-1".to_owned());
    browser_context.set_active_target_id("TID-1");
    browser_context.attach_active_session("SID-1");
    ctx.conn
        .install_browser_context_fixture_for_test(browser_context);

    ctx.process_async(json!({
        "id": 70,
        "method": "Network.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(70, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 71,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": format!("http://{addr}/host") }
    }))
    .await;
    flush_until_subresource_finished(
        &mut ctx,
        "Script",
        1,
        "initial immutable script network completion",
    )
    .await;
    assert_eq!(asset_hits.load(Ordering::SeqCst), 1);
    assert!(
        !ctx.sent.iter().any(|message| {
            message["method"] == json!("Network.requestServedFromCache")
                && message["sessionId"] == json!("SID-1")
        }),
        "the initial network response must not be reported as a cache hit"
    );
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 72,
        "method": "Page.reload",
        "sessionId": "SID-1",
        "params": {}
    }))
    .await;
    wait_until_cached_request_finished(&mut ctx, "cached immutable script network completion")
        .await;

    let messages = ctx.take_all();
    let response = assert_cached_request_event_order(&messages);
    assert_eq!(response["params"]["response"]["fromDiskCache"], json!(true));
    assert_eq!(
        response["params"]["response"]["url"],
        json!(format!("http://{addr}/immutable.js"))
    );
    assert_eq!(
        asset_hits.load(Ordering::SeqCst),
        1,
        "immutable reload should not contact the origin again"
    );

    ctx.process_async(json!({
        "id": 76,
        "method": "Network.setCacheDisabled",
        "sessionId": "SID-1",
        "params": {"cacheDisabled": true}
    }))
    .await;
    ctx.expect_result(76, json!({}), Some("SID-1"));
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 77,
        "method": "Page.reload",
        "sessionId": "SID-1",
        "params": {}
    }))
    .await;
    flush_until_subresource_finished(
        &mut ctx,
        "Script",
        1,
        "cache-disabled immutable script network completion",
    )
    .await;
    assert_eq!(
        asset_hits.load(Ordering::SeqCst),
        2,
        "cache-disabled reload must contact the origin"
    );
    assert!(
        !ctx.sent.iter().any(|message| {
            message["method"] == json!("Network.requestServedFromCache")
                && message["sessionId"] == json!("SID-1")
        }),
        "cache-disabled requests must not be reported as cache hits"
    );

    ctx.process_async(json!({
        "id": 78,
        "method": "Network.setCacheDisabled",
        "sessionId": "SID-1",
        "params": {"cacheDisabled": false}
    }))
    .await;
    ctx.expect_result(78, json!({}), Some("SID-1"));
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 79,
        "method": "Page.reload",
        "sessionId": "SID-1",
        "params": {}
    }))
    .await;
    wait_until_cached_request_finished(
        &mut ctx,
        "cache re-enabled immutable script network completion",
    )
    .await;
    assert_eq!(
        asset_hits.load(Ordering::SeqCst),
        2,
        "re-enabled cache should reuse the response stored by the bypass load"
    );

    server.abort();
    let _ = fs::remove_dir_all(cache_dir);
}

#[tokio::test(flavor = "multi_thread")]
async fn cached_main_document_navigation_emits_served_from_cache_before_response() {
    async fn cacheable_document(State(hits): State<Arc<AtomicUsize>>) -> impl IntoResponse {
        hits.fetch_add(1, Ordering::SeqCst);
        (
            [
                (CONTENT_TYPE.as_str(), "text/html"),
                ("cache-control", "public, max-age=31536000"),
            ],
            "<!doctype html><html><body>cached document</body></html>",
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let document_hits = Arc::new(AtomicUsize::new(0));
    let server_hits = Arc::clone(&document_hits);
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/document", get(cacheable_document))
                .with_state(server_hits),
        )
        .await
        .unwrap();
    });

    let cache_dir = unique_cdp_cache_dir("main-document-served-from-cache");
    let mut fetch_config = FetchConfig::default();
    fetch_config.set_http_cache_dir(Some(cache_dir.display().to_string()));
    let mut ctx = TestContext::from_conn(CdpConnection::new_with_fetch_config(fetch_config));
    let mut browser_context = ctx.conn.new_browser_context("BID-1".to_owned());
    browser_context.set_active_target_id("TID-1");
    browser_context.attach_active_session("SID-1");
    ctx.conn
        .install_browser_context_fixture_for_test(browser_context);

    ctx.process_async(json!({
        "id": 73,
        "method": "Network.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(73, json!({}), Some("SID-1"));

    let url = format!("http://{addr}/document");
    ctx.process_async(json!({
        "id": 74,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": url }
    }))
    .await;
    wait_until_messages(
        &mut ctx,
        Some("SID-1"),
        "initial main document network completion",
        |messages| {
            let Some(request_id) = messages.iter().find_map(|message| {
                if message["method"] == json!("Network.requestWillBeSent")
                    && message["params"]["type"] == json!("Document")
                {
                    message["params"]["requestId"].as_str()
                } else {
                    None
                }
            }) else {
                return false;
            };
            messages.iter().any(|message| {
                message["method"] == json!("Network.loadingFinished")
                    && message["params"]["requestId"] == json!(request_id)
            })
        },
    )
    .await;
    assert_eq!(document_hits.load(Ordering::SeqCst), 1);
    assert!(
        !ctx.sent.iter().any(|message| {
            message["method"] == json!("Network.requestServedFromCache")
                && message["sessionId"] == json!("SID-1")
        }),
        "the initial document response must not be reported as a cache hit"
    );
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 75,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": url }
    }))
    .await;
    wait_until_cached_request_finished(&mut ctx, "cached main document network completion").await;

    let messages = ctx.take_all();
    let response = assert_cached_request_event_order(&messages);
    assert_eq!(response["params"]["response"]["fromDiskCache"], json!(true));
    assert_eq!(
        document_hits.load(Ordering::SeqCst),
        1,
        "the cached second navigation must not contact the origin"
    );

    ctx.process_async(json!({
        "id": 80,
        "method": "Network.setCacheDisabled",
        "sessionId": "SID-1",
        "params": {"cacheDisabled": true}
    }))
    .await;
    ctx.expect_result(80, json!({}), Some("SID-1"));
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 81,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": {"url": url}
    }))
    .await;
    wait_until_messages(
        &mut ctx,
        Some("SID-1"),
        "cache-disabled main document network completion",
        |messages| {
            let Some(request_id) = messages.iter().find_map(|message| {
                if message["method"] == json!("Network.requestWillBeSent")
                    && message["params"]["type"] == json!("Document")
                {
                    message["params"]["requestId"].as_str()
                } else {
                    None
                }
            }) else {
                return false;
            };
            messages.iter().any(|message| {
                message["method"] == json!("Network.loadingFinished")
                    && message["params"]["requestId"] == json!(request_id)
            })
        },
    )
    .await;
    assert_eq!(
        document_hits.load(Ordering::SeqCst),
        2,
        "cache-disabled navigation must contact the origin"
    );
    assert!(
        !ctx.sent.iter().any(|message| {
            message["method"] == json!("Network.requestServedFromCache")
                && message["sessionId"] == json!("SID-1")
        }),
        "cache-disabled navigation must not be reported as a cache hit"
    );

    ctx.process_async(json!({
        "id": 82,
        "method": "Network.setCacheDisabled",
        "sessionId": "SID-1",
        "params": {"cacheDisabled": false}
    }))
    .await;
    ctx.expect_result(82, json!({}), Some("SID-1"));
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 83,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": {"url": url}
    }))
    .await;
    wait_until_cached_request_finished(
        &mut ctx,
        "cache re-enabled main document network completion",
    )
    .await;
    assert_eq!(
        document_hits.load(Ordering::SeqCst),
        2,
        "re-enabled cache should reuse the response stored by the bypass navigation"
    );

    server.abort();
    let _ = fs::remove_dir_all(cache_dir);
}

#[tokio::test(flavor = "multi_thread")]
async fn clear_browser_cache_requires_browser_context() {
    let mut ctx = TestContext::new();
    ctx.process_async(json!({"id": 3, "method": "Network.clearBrowserCache"}))
        .await;
    ctx.expect_error(3, -31998, "BrowserContextNotLoaded");
}
#[tokio::test(flavor = "multi_thread")]
async fn clear_browser_cache_clears_response_body_and_stream_artifacts() {
    let mut ctx = TestContext::new();
    let mut bc = BrowserContext::new_with_page_for_test("BID-1", "TID-1");
    bc.record_captured_response_body("REQ-1".to_owned(), "body".to_owned(), [None]);
    bc.insert_io_stream("STREAM-1".to_owned(), b"payload".to_vec(), 0);
    ctx.conn.install_browser_context_fixture_for_test(bc);

    ctx.process_async(json!({"id": 4, "method": "Network.clearBrowserCache"}))
        .await;
    ctx.expect_result(4, json!({}), None);

    let bc = ctx.conn.browser_context.as_ref().unwrap();
    assert!(bc.captured_response_bodies_empty_for_test());
    assert!(bc.io_streams_empty_for_test());
}
#[tokio::test(flavor = "multi_thread")]
async fn clear_browser_cache_clears_configured_disk_http_cache() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock should be after epoch")
        .as_nanos();
    let cache_dir = std::env::temp_dir().join(format!(
        "moli-cdp-clear-http-cache-{}-{nonce}",
        std::process::id()
    ));
    let entry_dir = cache_dir.join("0123456789abcdef.entry");
    fs::create_dir_all(&entry_dir).expect("cache entry dir should be created");
    fs::write(entry_dir.join("body.test.bin"), b"cached")
        .expect("cache body fixture should be written");
    fs::write(cache_dir.join("owner.lock"), b"keep").expect("unrelated root file should write");

    let mut fetch_config = FetchConfig::default();
    fetch_config.set_http_cache_dir(Some(cache_dir.display().to_string()));
    let mut ctx = TestContext::from_conn(crate::conn::CdpConnection::new_with_fetch_config(
        fetch_config,
    ));
    let browser_context = ctx.conn.new_browser_context("BID-1".into());
    ctx.conn
        .install_browser_context_fixture_for_test(browser_context);

    ctx.process_async(json!({"id": 44, "method": "Network.clearBrowserCache"}))
        .await;
    ctx.expect_result(44, json!({}), None);

    assert!(!entry_dir.exists());
    assert!(cache_dir.join("owner.lock").exists());

    let _ = fs::remove_dir_all(cache_dir);
}

#[tokio::test(flavor = "multi_thread")]
async fn clear_browser_cache_uses_browser_context_http_cache_owner() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock should be after epoch")
        .as_nanos();
    let cache_dir = std::env::temp_dir().join(format!(
        "moli-cdp-clear-context-http-cache-{}-{nonce}",
        std::process::id()
    ));
    let entry_dir = cache_dir.join("0123456789abcdef.entry");
    fs::create_dir_all(&entry_dir).expect("cache entry dir should be created");
    fs::write(entry_dir.join("body.test.bin"), b"cached")
        .expect("cache body fixture should be written");

    let mut fetch_config = FetchConfig::default();
    fetch_config.set_http_cache_dir(None);
    let mut ctx = TestContext::from_conn(crate::conn::CdpConnection::new_with_fetch_config(
        fetch_config,
    ));
    let mut browser_context = BrowserContext::new("BID-1".into());
    browser_context.http_cache_root = Some(cache_dir.clone());
    ctx.conn
        .install_browser_context_fixture_for_test(browser_context);

    ctx.process_async(json!({"id": 45, "method": "Network.clearBrowserCache"}))
        .await;
    ctx.expect_result(45, json!({}), None);

    assert!(!entry_dir.exists());

    let _ = fs::remove_dir_all(cache_dir);
}

#[tokio::test(flavor = "multi_thread")]
async fn clear_browser_cache_targets_command_session_browser_context() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock should be after epoch")
        .as_nanos();
    let active_cache_dir = std::env::temp_dir().join(format!(
        "moli-cdp-active-http-cache-owner-{}-{nonce}",
        std::process::id()
    ));
    let inactive_cache_dir = std::env::temp_dir().join(format!(
        "moli-cdp-inactive-http-cache-owner-{}-{nonce}",
        std::process::id()
    ));
    let active_entry_dir = active_cache_dir.join("active.entry");
    let inactive_entry_dir = inactive_cache_dir.join("inactive.entry");
    fs::create_dir_all(&active_entry_dir).expect("active cache entry dir should be created");
    fs::create_dir_all(&inactive_entry_dir).expect("inactive cache entry dir should be created");
    fs::write(active_entry_dir.join("body.test.bin"), b"active")
        .expect("active cache body fixture should be written");
    fs::write(inactive_entry_dir.join("body.test.bin"), b"inactive")
        .expect("inactive cache body fixture should be written");

    let mut active = BrowserContext::new_with_page_for_test("BID-cache-active", "TID-cache-active");
    active.attach_active_session("SID-cache-active");
    active.http_cache_root = Some(active_cache_dir.clone());
    active.record_captured_response_body("REQ-active".to_owned(), "active".to_owned(), [None]);

    let mut inactive =
        BrowserContext::new_with_page_for_test("BID-cache-inactive", "TID-cache-inactive");
    inactive.attach_active_session("SID-cache-inactive");
    inactive.http_cache_root = Some(inactive_cache_dir.clone());
    inactive.record_captured_response_body(
        "REQ-inactive".to_owned(),
        "inactive".to_owned(),
        [None],
    );

    let mut ctx = TestContext::new();
    ctx.conn.install_browser_context_fixture_for_test(active);
    ctx.conn
        .push_inactive_browser_context_fixture_for_test(inactive);

    ctx.process_async(json!({
        "id": 46,
        "sessionId": "SID-cache-inactive",
        "method": "Network.clearBrowserCache"
    }))
    .await;
    ctx.expect_result(46, json!({}), Some("SID-cache-inactive"));

    assert!(active_entry_dir.exists());
    assert!(!inactive_entry_dir.exists());
    assert!(
        !ctx.conn
            .browser_context
            .as_ref()
            .unwrap()
            .captured_response_bodies_empty_for_test()
    );
    assert!(ctx.conn.inactive_browser_contexts[0].captured_response_bodies_empty_for_test());

    let _ = fs::remove_dir_all(active_cache_dir);
    let _ = fs::remove_dir_all(inactive_cache_dir);
}

#[test]
fn new_ephemeral_browser_context_inherits_effective_http_cache_owner() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock should be after epoch")
        .as_nanos();
    let cache_dir = std::env::temp_dir().join(format!(
        "moli-cdp-ephemeral-http-cache-owner-{}-{nonce}",
        std::process::id()
    ));
    let mut fetch_config = FetchConfig::default();
    fetch_config.set_http_cache_dir(Some(cache_dir.display().to_string()));
    fetch_config.set_http_cache_max_bytes(Some(77));

    let conn = crate::conn::CdpConnection::new_with_fetch_config(fetch_config);
    let browser_context = conn.new_ephemeral_browser_context("BID-ephemeral".to_owned());

    assert_eq!(
        browser_context.http_cache_root.as_deref(),
        Some(cache_dir.as_path())
    );
    assert_eq!(browser_context.http_cache_max_bytes, Some(77));
}

#[test]
fn new_browser_context_inherits_effective_http_cache_owner() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock should be after epoch")
        .as_nanos();
    let cache_dir = std::env::temp_dir().join(format!(
        "moli-cdp-default-http-cache-owner-{}-{nonce}",
        std::process::id()
    ));
    let mut fetch_config = FetchConfig::default();
    fetch_config.set_http_cache_dir(Some(cache_dir.display().to_string()));
    fetch_config.set_http_cache_max_bytes(Some(77));

    let conn = crate::conn::CdpConnection::new_with_fetch_config(fetch_config);
    let browser_context = conn.new_browser_context("BID-default".to_owned());

    assert_eq!(
        browser_context.http_cache_root.as_deref(),
        Some(cache_dir.as_path())
    );
    assert_eq!(browser_context.http_cache_max_bytes, Some(77));
}

#[tokio::test(flavor = "multi_thread")]
async fn clear_browser_cache_keeps_pending_response_navigation_transfer() {
    let mut ctx = TestContext::new();
    let mut bc = BrowserContext::new_with_page_for_test("BID-1", "TID-1");
    let url = Url::parse("https://example.test/document").unwrap();
    bc.register_pending_fetch_response_navigation(
        "INT-1".to_owned(),
        None,
        NavigationDispatchState {
            navigate_id: Some(1),
            owner: crate::conn::CommandOwnerScope::for_session("SID-1"),
            result_projection: crate::conn::NavigationResultProjection::Cdp(
                json!({"frameId": "TID-1", "loaderId": LOADER_ID}),
            ),
            frame_id: "TID-1".to_owned(),
            session_id: Some("SID-1".to_owned()),
            request_id: Some("REQ-1".to_owned()),
            loader_id: LOADER_ID.to_owned(),
            request_announced: true,
            requested_url: url.clone(),
            request_method: "GET".to_owned(),
            request_body: None,
            request_body_bytes: None,
            request_headers: Vec::new(),
            request_load_policy: crate::conn::NavigationRequestLoadPolicy::DocumentInitiated,
            timestamp: 0.0,
            source_document_security: Default::default(),
        },
        DocumentBodySource::BufferedRaw {
            requested_url: url.clone(),
            request_method: "GET".to_owned(),
            request_headers: Vec::new(),
            response: RawResponse::from_head_and_body(
                ResponseHead {
                    final_url: url,
                    status: 200,
                    headers: Vec::new(),
                    request_cookie_report: None,
                    cookie_set_reports: Vec::new(),
                    redirected: false,
                    redirect_chain: Vec::new(),
                    from_cache: false,
                    negotiated_http_version: None,
                },
                b"body".to_vec(),
            ),
            network_observation_journal: Default::default(),
        },
    );
    ctx.conn.install_browser_context_fixture_for_test(bc);

    ctx.process_async(json!({"id": 5, "method": "Network.clearBrowserCache"}))
        .await;
    ctx.expect_result(5, json!({}), None);

    let bc = ctx.conn.browser_context.as_ref().unwrap();
    assert!(
        bc.active_page_target()
            .fetch_owner
            .pending_fetch_response_transfer_is_pending_for_test("INT-1")
    );
}
#[tokio::test(flavor = "multi_thread")]
async fn set_cache_disabled_requires_browser_context() {
    let mut ctx = TestContext::new();
    ctx.process_async(json!({
        "id": 20,
        "method": "Network.setCacheDisabled",
        "params": { "cacheDisabled": true }
    }))
    .await;
    ctx.expect_error(20, -31998, "BrowserContextNotLoaded");
}
#[tokio::test(flavor = "multi_thread")]
async fn set_cache_disabled_rejects_invalid_params() {
    let mut ctx = TestContext::new();
    ctx.conn.browser_context = Some(BrowserContext::new_with_page_for_test("BID-1", "TID-1"));

    ctx.process_async(json!({
        "id": 21,
        "method": "Network.setCacheDisabled",
        "params": {}
    }))
    .await;
    ctx.expect_error(21, -32602, "InvalidParams");
}
#[tokio::test(flavor = "multi_thread")]
async fn set_cache_disabled_updates_browser_context_state() {
    let mut ctx = TestContext::new();
    ctx.conn.browser_context = Some(BrowserContext::new_with_page_for_test("BID-1", "TID-1"));

    ctx.process_async(json!({
        "id": 219,
        "method": "Network.enable"
    }))
    .await;
    ctx.expect_result(219, json!({}), None);

    ctx.process_async(json!({
        "id": 22,
        "method": "Network.setCacheDisabled",
        "params": { "cacheDisabled": true }
    }))
    .await;
    ctx.expect_result(22, json!({}), None);
    assert!(
        ctx.conn
            .browser_context
            .as_ref()
            .unwrap()
            .active_page_target()
            .effective_policy()
            .cache_disabled()
    );

    ctx.process_async(json!({
        "id": 23,
        "method": "Network.setCacheDisabled",
        "params": { "cacheDisabled": false }
    }))
    .await;
    ctx.expect_result(23, json!({}), None);
    assert!(
        !ctx.conn
            .browser_context
            .as_ref()
            .unwrap()
            .active_page_target()
            .effective_policy()
            .cache_disabled()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn devtools_set_cache_behavior_global_updates_existing_targets_and_default() {
    let mut ctx = TestContext::new();
    let mut bc = BrowserContext::new("BID-1".into());
    bc.set_active_target_id("TID-active".to_owned());
    bc.insert_page_target_host(PageTargetHost::new(
        "TID-background".to_owned(),
        None,
        TargetIdentityState::about_blank(),
        TargetPageSlot::empty_for_test_fixture(),
    ));
    ctx.conn.install_browser_context_fixture_for_test(bc);

    let result = ctx
        .conn
        .execute_devtools_command(crate::devtools_runtime::DevToolsCommand::SetCacheBehavior(
            crate::devtools_runtime::DevToolsSetCacheBehaviorCommand {
                context: crate::devtools_runtime::DevToolsCommandContext {
                    protocol: crate::devtools_runtime::DevToolsProtocol::WebDriverBidi,
                    session_id: Some(crate::devtools_runtime::DevToolsSessionId::from(
                        "bidi-session-1",
                    )),
                    target_id: None,
                    browser_context_id: None,
                },
                target_ids: Vec::new(),
                cache_disabled: true,
            },
        ))
        .await
        .into_parts()
        .0
        .expect("global BiDi cache behavior should succeed");
    assert_eq!(
        result,
        crate::devtools_runtime::DevToolsCommandResult::Empty
    );

    let bc = ctx.conn.browser_context.as_ref().expect("browser context");
    assert!(bc.active_page_target().effective_policy().cache_disabled());
    assert!(
        bc.background_target("TID-background")
            .filter(|target| target.has_non_default_session_state())
            .expect("background state")
            .effective_policy()
            .cache_disabled()
    );
    assert!(
        ctx.conn
            .new_browser_context("BID-future".to_owned())
            .global_cache_disabled
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn devtools_set_cache_behavior_contexts_only_updates_requested_targets() {
    let mut ctx = TestContext::new();
    let mut bc = BrowserContext::new("BID-1".into());
    bc.set_active_target_id("TID-active".to_owned());
    bc.insert_page_target_host(PageTargetHost::new(
        "TID-background".to_owned(),
        None,
        TargetIdentityState::about_blank(),
        TargetPageSlot::empty_for_test_fixture(),
    ));
    ctx.conn.install_browser_context_fixture_for_test(bc);

    ctx.conn
        .execute_devtools_command(crate::devtools_runtime::DevToolsCommand::SetCacheBehavior(
            crate::devtools_runtime::DevToolsSetCacheBehaviorCommand {
                context: crate::devtools_runtime::DevToolsCommandContext {
                    protocol: crate::devtools_runtime::DevToolsProtocol::WebDriverBidi,
                    session_id: Some(crate::devtools_runtime::DevToolsSessionId::from(
                        "bidi-session-1",
                    )),
                    target_id: None,
                    browser_context_id: None,
                },
                target_ids: vec![crate::devtools_runtime::DevToolsTargetId::from(
                    "TID-background",
                )],
                cache_disabled: true,
            },
        ))
        .await
        .into_parts()
        .0
        .expect("context-scoped BiDi cache behavior should succeed");

    let bc = ctx.conn.browser_context.as_ref().expect("browser context");
    assert!(!bc.active_page_target().effective_policy().cache_disabled());
    assert!(
        bc.background_target("TID-background")
            .filter(|target| target.has_non_default_session_state())
            .expect("background state")
            .effective_policy()
            .cache_disabled()
    );
    assert!(
        !ctx.conn
            .new_browser_context("BID-future".to_owned())
            .global_cache_disabled
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn devtools_set_cache_behavior_rejects_unknown_context() {
    let mut ctx = TestContext::new();
    ctx.conn.browser_context = Some(BrowserContext::new_with_page_for_test("BID-1", "TID-1"));

    let error = ctx
        .conn
        .execute_devtools_command(crate::devtools_runtime::DevToolsCommand::SetCacheBehavior(
            crate::devtools_runtime::DevToolsSetCacheBehaviorCommand {
                context: crate::devtools_runtime::DevToolsCommandContext {
                    protocol: crate::devtools_runtime::DevToolsProtocol::WebDriverBidi,
                    session_id: Some(crate::devtools_runtime::DevToolsSessionId::from(
                        "bidi-session-1",
                    )),
                    target_id: None,
                    browser_context_id: None,
                },
                target_ids: vec![crate::devtools_runtime::DevToolsTargetId::from(
                    "TID-missing",
                )],
                cache_disabled: true,
            },
        ))
        .await
        .into_parts()
        .0
        .expect_err("unknown context should fail");

    assert_eq!(
        error.kind,
        crate::devtools_runtime::DevToolsErrorKind::NoSuchTarget
    );
    assert_eq!(error.message, "NoSuchTarget");
}

#[tokio::test(flavor = "multi_thread")]
async fn devtools_set_cache_behavior_without_contexts_sets_future_default() {
    let mut ctx = TestContext::new();

    ctx.conn
        .execute_devtools_command(crate::devtools_runtime::DevToolsCommand::SetCacheBehavior(
            crate::devtools_runtime::DevToolsSetCacheBehaviorCommand {
                context: crate::devtools_runtime::DevToolsCommandContext {
                    protocol: crate::devtools_runtime::DevToolsProtocol::WebDriverBidi,
                    session_id: Some(crate::devtools_runtime::DevToolsSessionId::from(
                        "bidi-session-1",
                    )),
                    target_id: None,
                    browser_context_id: None,
                },
                target_ids: Vec::new(),
                cache_disabled: true,
            },
        ))
        .await
        .into_parts()
        .0
        .expect("global cache behavior should be accepted before contexts exist");

    assert!(
        ctx.conn
            .new_browser_context("BID-future".to_owned())
            .global_cache_disabled
    );
}
#[tokio::test(flavor = "multi_thread")]
async fn set_bypass_service_worker_requires_browser_context() {
    let mut ctx = TestContext::new();
    ctx.process_async(json!({
        "id": 24,
        "method": "Network.setBypassServiceWorker",
        "params": { "bypass": true }
    }))
    .await;
    ctx.expect_error(24, -31998, "BrowserContextNotLoaded");
}
#[tokio::test(flavor = "multi_thread")]
async fn set_bypass_service_worker_rejects_invalid_params() {
    let mut ctx = TestContext::new();
    ctx.conn.browser_context = Some(BrowserContext::new("BID-1".into()));
    ctx.process_async(json!({
        "id": 25,
        "method": "Network.setBypassServiceWorker",
        "params": {}
    }))
    .await;
    ctx.expect_error(25, -32602, "InvalidParams");
}
#[tokio::test(flavor = "multi_thread")]
async fn set_bypass_service_worker_updates_browser_context_state() {
    let mut ctx = TestContext::new();
    ctx.conn.browser_context = Some(BrowserContext::new_with_page_for_test("BID-1", "TID-1"));

    ctx.process_async(json!({
        "id": 239,
        "method": "Network.enable"
    }))
    .await;
    ctx.expect_result(239, json!({}), None);

    ctx.process_async(json!({
        "id": 26,
        "method": "Network.setBypassServiceWorker",
        "params": { "bypass": true }
    }))
    .await;
    ctx.expect_result(26, json!({}), None);
    assert!(
        ctx.conn
            .browser_context
            .as_ref()
            .unwrap()
            .active_page_target()
            .effective_policy()
            .bypass_service_worker()
    );

    ctx.process_async(json!({
        "id": 27,
        "method": "Network.setBypassServiceWorker",
        "params": { "bypass": false }
    }))
    .await;
    ctx.expect_result(27, json!({}), None);
    assert!(
        !ctx.conn
            .browser_context
            .as_ref()
            .unwrap()
            .active_page_target()
            .effective_policy()
            .bypass_service_worker()
    );
}
