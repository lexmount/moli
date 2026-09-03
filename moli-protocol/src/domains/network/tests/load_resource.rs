use super::*;
use serde_json::Value;
use std::sync::atomic::{AtomicUsize, Ordering};

const SESSION_ID: &str = "SID-1";
const TARGET_ID: &str = "TID-1";

async fn install_loaded_page(ctx: &mut TestContext, page_url: &str) {
    let mut browser_context = ctx.conn.new_browser_context("BID-1".to_owned());
    browser_context.set_active_target_id(TARGET_ID);
    browser_context.attach_active_session(SESSION_ID);
    ctx.conn
        .install_browser_context_fixture_for_test(browser_context);
    ctx.install_navigation_fixture_for_session_owner(page_url, Some(SESSION_ID))
        .await;
    ctx.sent.clear();
}

async fn load_resource(
    ctx: &mut TestContext,
    id: u64,
    url: &str,
    disable_cache: bool,
    include_credentials: bool,
) -> Value {
    ctx.process_async(json!({
        "id": id,
        "method": "Network.loadNetworkResource",
        "sessionId": SESSION_ID,
        "params": {
            "frameId": TARGET_ID,
            "url": url,
            "options": {
                "disableCache": disable_cache,
                "includeCredentials": include_credentials
            }
        }
    }))
    .await;
    ctx.take_response_by_id(id)
}

async fn read_stream(ctx: &mut TestContext, id: u64, stream: &str) -> String {
    ctx.process_async(json!({
        "id": id,
        "method": "IO.read",
        "sessionId": SESSION_ID,
        "params": { "handle": stream }
    }))
    .await;
    let response = ctx.take_response_by_id(id);
    assert_eq!(response["result"]["base64Encoded"], json!(false));
    response["result"]["data"]
        .as_str()
        .expect("IO.read text body")
        .to_owned()
}

async fn close_stream(ctx: &mut TestContext, id: u64, stream: &str) {
    ctx.process_async(json!({
        "id": id,
        "method": "IO.close",
        "sessionId": SESSION_ID,
        "params": { "handle": stream }
    }))
    .await;
    ctx.expect_result(id, json!({}), Some(SESSION_ID));
}

fn resource_stream(response: &Value) -> &str {
    response["result"]["resource"]["stream"]
        .as_str()
        .unwrap_or_else(|| panic!("loadNetworkResource stream: {response}"))
}

#[tokio::test(flavor = "multi_thread")]
async fn load_network_resource_returns_io_stream_without_page_network_events() {
    async fn resource(State(hits): State<Arc<AtomicUsize>>) -> impl IntoResponse {
        hits.fetch_add(1, Ordering::SeqCst);
        (
            [
                (CONTENT_TYPE.as_str(), "text/plain"),
                ("x-moli-resource", "present"),
            ],
            "REAL-SERVER-BODY",
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let hits = Arc::new(AtomicUsize::new(0));
    let server_hits = Arc::clone(&hits);
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/page", get(plain_page))
                .route("/resource", get(resource))
                .with_state(server_hits),
        )
        .await
        .unwrap();
    });

    let mut ctx = TestContext::new();
    install_loaded_page(&mut ctx, &format!("http://{addr}/page")).await;
    ctx.process_async(json!({
        "id": 81_000,
        "method": "Network.enable",
        "sessionId": SESSION_ID
    }))
    .await;
    ctx.expect_result(81_000, json!({}), Some(SESSION_ID));

    let resource_url = format!("http://{addr}/resource");
    let response = load_resource(&mut ctx, 81_001, &resource_url, false, false).await;
    assert_eq!(response["sessionId"], json!(SESSION_ID));
    assert_eq!(response["result"]["resource"]["success"], json!(true));
    assert_eq!(response["result"]["resource"]["httpStatusCode"], json!(200));
    assert_eq!(
        response["result"]["resource"]["headers"]["x-moli-resource"],
        json!("present")
    );
    let stream = resource_stream(&response).to_owned();
    assert_eq!(hits.load(Ordering::SeqCst), 1);
    assert!(
        ctx.sent.iter().all(|message| {
            !message["method"]
                .as_str()
                .is_some_and(|method| method.starts_with("Network."))
        }),
        "browser-side DevTools loads must not enter the page Network event stream: {:?}",
        ctx.sent
    );

    assert_eq!(
        read_stream(&mut ctx, 81_002, &stream).await,
        "REAL-SERVER-BODY"
    );
    close_stream(&mut ctx, 81_003, &stream).await;
    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn load_network_resource_matches_chromium_validation_order() {
    let mut ctx = TestContext::new();
    install_loaded_page(&mut ctx, "data:text/html,<body>resource owner</body>").await;

    ctx.process_async(json!({
        "id": 81_010,
        "method": "Network.loadNetworkResource",
        "sessionId": SESSION_ID,
        "params": {
            "url": "not a URL",
            "options": { "disableCache": false, "includeCredentials": false }
        }
    }))
    .await;
    ctx.expect_error(81_010, -32602, "The url must be valid");

    ctx.process_async(json!({
        "id": 81_011,
        "method": "Network.loadNetworkResource",
        "sessionId": SESSION_ID,
        "params": {
            "url": "file:///tmp/resource",
            "options": { "disableCache": false, "includeCredentials": false }
        }
    }))
    .await;
    ctx.expect_error(81_011, -32602, "Unsupported URL scheme");

    ctx.process_async(json!({
        "id": 81_012,
        "method": "Network.loadNetworkResource",
        "sessionId": SESSION_ID,
        "params": {
            "url": "https://example.test/resource",
            "options": { "disableCache": false, "includeCredentials": false }
        }
    }))
    .await;
    ctx.expect_error(
        81_012,
        -32602,
        "Parameter frameId must be provided for frame targets",
    );

    ctx.process_async(json!({
        "id": 81_013,
        "method": "Network.loadNetworkResource",
        "sessionId": SESSION_ID,
        "params": {
            "frameId": "UNKNOWN-FRAME",
            "url": "ftp://example.test/resource",
            "options": { "disableCache": false, "includeCredentials": false }
        }
    }))
    .await;
    ctx.expect_error(81_013, -32602, "Frame not found");

    ctx.process_async(json!({
        "id": 81_014,
        "method": "Network.loadNetworkResource",
        "sessionId": SESSION_ID,
        "params": {
            "frameId": TARGET_ID,
            "url": "ftp://example.test/resource",
            "options": { "disableCache": false, "includeCredentials": false }
        }
    }))
    .await;
    ctx.expect_error(81_014, -32602, "Unsupported URL scheme");
}

#[tokio::test(flavor = "multi_thread")]
async fn load_network_resource_enforces_document_connect_src_before_fetch() {
    async fn csp_page() -> impl IntoResponse {
        (
            [
                (CONTENT_TYPE.as_str(), "text/html"),
                ("content-security-policy", "connect-src 'none'"),
            ],
            "<!doctype html><body>csp owner</body>",
        )
    }

    async fn blocked_resource(State(hits): State<Arc<AtomicUsize>>) -> impl IntoResponse {
        hits.fetch_add(1, Ordering::SeqCst);
        "must not be fetched"
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let hits = Arc::new(AtomicUsize::new(0));
    let server_hits = Arc::clone(&hits);
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/page", get(csp_page))
                .route("/blocked", get(blocked_resource))
                .with_state(server_hits),
        )
        .await
        .unwrap();
    });

    let mut ctx = TestContext::new();
    install_loaded_page(&mut ctx, &format!("http://{addr}/page")).await;
    let response = load_resource(
        &mut ctx,
        81_020,
        &format!("http://{addr}/blocked"),
        false,
        false,
    )
    .await;
    assert_eq!(response["error"]["code"], json!(-32000));
    assert_eq!(response["error"]["message"], json!("CSP violation"));
    assert_eq!(hits.load(Ordering::SeqCst), 0);
    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn load_network_resource_reports_http_failure_without_a_stream() {
    async fn missing() -> impl IntoResponse {
        (
            StatusCode::NOT_FOUND,
            [("x-resource-status", "missing")],
            "not found",
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/page", get(plain_page))
                .route("/missing", get(missing)),
        )
        .await
        .unwrap();
    });

    let mut ctx = TestContext::new();
    install_loaded_page(&mut ctx, &format!("http://{addr}/page")).await;
    let response = load_resource(
        &mut ctx,
        81_030,
        &format!("http://{addr}/missing"),
        false,
        false,
    )
    .await;
    let resource = &response["result"]["resource"];
    assert_eq!(resource["success"], json!(false));
    assert_eq!(resource["netError"], json!(-379));
    assert_eq!(
        resource["netErrorName"],
        json!("net::ERR_HTTP_RESPONSE_CODE_FAILURE")
    );
    assert_eq!(resource["httpStatusCode"], json!(404));
    assert_eq!(resource["headers"]["x-resource-status"], json!("missing"));
    assert!(resource.get("stream").is_none());
    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn load_network_resource_include_credentials_controls_cross_origin_cookies() {
    async fn cookie_echo(headers: HeaderMap) -> impl IntoResponse {
        let cookie = headers
            .get(axum::http::header::COOKIE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("no-cookie")
            .to_owned();
        ([("cache-control", "no-store")], cookie)
    }

    let page_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let page_addr = page_listener.local_addr().unwrap();
    let page_server = tokio::spawn(async move {
        axum::serve(page_listener, Router::new().route("/page", get(plain_page)))
            .await
            .unwrap();
    });
    let resource_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let resource_addr = resource_listener.local_addr().unwrap();
    let resource_server = tokio::spawn(async move {
        axum::serve(
            resource_listener,
            Router::new().route("/cookie", get(cookie_echo)),
        )
        .await
        .unwrap();
    });

    let resource_url = format!("http://{resource_addr}/cookie");
    let mut ctx = TestContext::new();
    install_loaded_page(&mut ctx, &format!("http://{page_addr}/page")).await;
    ctx.process_async(json!({
        "id": 81_040,
        "method": "Network.setCookie",
        "sessionId": SESSION_ID,
        "params": {
            "name": "resource_auth",
            "value": "secret",
            "url": resource_url
        }
    }))
    .await;
    assert_eq!(
        ctx.take_response_by_id(81_040)["result"]["success"],
        json!(true)
    );

    let without_credentials = load_resource(&mut ctx, 81_041, &resource_url, true, false).await;
    let stream = resource_stream(&without_credentials).to_owned();
    assert_eq!(read_stream(&mut ctx, 81_042, &stream).await, "no-cookie");
    close_stream(&mut ctx, 81_043, &stream).await;

    let with_credentials = load_resource(&mut ctx, 81_044, &resource_url, true, true).await;
    let stream = resource_stream(&with_credentials).to_owned();
    assert_eq!(
        read_stream(&mut ctx, 81_045, &stream).await,
        "resource_auth=secret"
    );
    close_stream(&mut ctx, 81_046, &stream).await;

    page_server.abort();
    resource_server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn load_network_resource_disable_cache_bypasses_and_replaces_fresh_entry() {
    async fn cacheable_resource(State(hits): State<Arc<AtomicUsize>>) -> impl IntoResponse {
        let version = hits.fetch_add(1, Ordering::SeqCst) + 1;
        (
            [("cache-control", "public, max-age=3600")],
            format!("resource-v{version}"),
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let hits = Arc::new(AtomicUsize::new(0));
    let server_hits = Arc::clone(&hits);
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/page", get(plain_page))
                .route("/cacheable", get(cacheable_resource))
                .with_state(server_hits),
        )
        .await
        .unwrap();
    });

    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock should be after epoch")
        .as_nanos();
    let cache_dir = std::env::temp_dir().join(format!(
        "moli-cdp-load-resource-cache-{}-{nonce}",
        std::process::id()
    ));
    let mut fetch_config = FetchConfig::default();
    fetch_config.set_http_cache_dir(Some(cache_dir.display().to_string()));
    let mut ctx = TestContext::from_conn(CdpConnection::new_with_fetch_config(fetch_config));
    install_loaded_page(&mut ctx, &format!("http://{addr}/page")).await;
    let resource_url = format!("http://{addr}/cacheable");

    let first = load_resource(&mut ctx, 81_050, &resource_url, false, false).await;
    let stream = resource_stream(&first).to_owned();
    assert_eq!(read_stream(&mut ctx, 81_051, &stream).await, "resource-v1");
    close_stream(&mut ctx, 81_052, &stream).await;

    let cached = load_resource(&mut ctx, 81_053, &resource_url, false, false).await;
    let stream = resource_stream(&cached).to_owned();
    assert_eq!(read_stream(&mut ctx, 81_054, &stream).await, "resource-v1");
    close_stream(&mut ctx, 81_055, &stream).await;
    assert_eq!(hits.load(Ordering::SeqCst), 1);

    let bypassed = load_resource(&mut ctx, 81_056, &resource_url, true, false).await;
    let stream = resource_stream(&bypassed).to_owned();
    assert_eq!(read_stream(&mut ctx, 81_057, &stream).await, "resource-v2");
    close_stream(&mut ctx, 81_058, &stream).await;

    let replaced = load_resource(&mut ctx, 81_059, &resource_url, false, false).await;
    let stream = resource_stream(&replaced).to_owned();
    assert_eq!(read_stream(&mut ctx, 81_060, &stream).await, "resource-v2");
    close_stream(&mut ctx, 81_061, &stream).await;
    assert_eq!(hits.load(Ordering::SeqCst), 2);

    server.abort();
    let _ = fs::remove_dir_all(cache_dir);
}
