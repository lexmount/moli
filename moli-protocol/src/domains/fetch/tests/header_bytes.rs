use super::*;
use axum::extract::{OriginalUri, State};
use parking_lot::Mutex;

struct CapturedRequest {
    path: String,
    authenticated: bool,
    headers: Vec<Vec<u8>>,
}
type CapturedHeaders = Arc<Mutex<Vec<CapturedRequest>>>;

async fn capture_headers(
    State(captured): State<CapturedHeaders>,
    OriginalUri(uri): OriginalUri,
    headers: HeaderMap,
) -> axum::response::Response {
    let authenticated = headers.contains_key("authorization");
    captured.lock().push(CapturedRequest {
        path: uri.path().to_owned(),
        authenticated,
        headers: headers
            .get_all("x-raw")
            .iter()
            .map(|value| value.as_bytes().to_vec())
            .collect(),
    });
    if uri.path() == "/start" {
        axum::response::Redirect::temporary("/protected").into_response()
    } else if !authenticated {
        (
            StatusCode::UNAUTHORIZED,
            [(WWW_AUTHENTICATE.as_str(), "Basic realm=\"bytes\"")],
            "authenticate",
        )
            .into_response()
    } else {
        "ok".into_response()
    }
}

async fn check_intercepted_header_bytes(script: &str, override_headers: bool) {
    let captured = CapturedHeaders::default();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = Router::new()
        .route(
            "/page",
            get(|| async {
                (
                    [(CONTENT_TYPE.as_str(), "text/html")],
                    "<!doctype html><body>ready",
                )
            }),
        )
        .route("/start", get(capture_headers))
        .route("/protected", get(capture_headers))
        .with_state(captured.clone());
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let mut ctx = TestContext::new();
    with_loaded_http_document(&mut ctx, &format!("http://{addr}/page"), "SID-1", "TID-1").await;
    enable_runtime_async(&mut ctx, "SID-1", 70100).await;
    ctx.process_async(json!({
        "id": 70101, "sessionId": "SID-1", "method": "Fetch.enable",
        "params": {"handleAuthRequests": true, "patterns": [{"urlPattern": "*/start"}]}
    }))
    .await;
    ctx.expect_result(70101, json!({}), Some("SID-1"));
    ctx.process_async(json!({
        "id": 70102, "sessionId": "SID-1", "method": "Runtime.evaluate",
        "params": {"expression": script}
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 70102);
    wait_until_message(&mut ctx, "SID-1", "request pause", |message| {
        message["method"] == "Fetch.requestPaused"
    })
    .await;
    let paused = ctx.take_first_matching("Fetch.requestPaused", |message| {
        message["method"] == "Fetch.requestPaused"
    });
    let request_id = paused["params"]["requestId"].as_str().unwrap().to_owned();
    let mut params = json!({"requestId": request_id});
    if override_headers {
        params["headers"] = json!([
            {"name": "x-raw", "value": "é"},
            {"name": "X-Raw", "value": "ÿ"}
        ]);
    }
    ctx.process_async(json!({
        "id": 70103, "sessionId": "SID-1", "method": "Fetch.continueRequest", "params": params
    }))
    .await;
    ctx.expect_result(70103, json!({}), Some("SID-1"));
    wait_until_message(&mut ctx, "SID-1", "auth pause after redirect", |message| {
        message["method"] == "Fetch.authRequired"
    })
    .await;
    let auth = ctx.take_first_matching("Fetch.authRequired", |message| {
        message["method"] == "Fetch.authRequired"
    });
    ctx.process_async(json!({
        "id": 70104, "sessionId": "SID-1", "method": "Fetch.continueWithAuth",
        "params": {"requestId": auth["params"]["requestId"], "authChallengeResponse": {
            "response": "ProvideCredentials", "username": "user", "password": "pass"
        }}
    }))
    .await;
    ctx.expect_result(70104, json!({}), Some("SID-1"));
    evaluate_until_value_async(
        &mut ctx,
        "SID-1",
        70105,
        "globalThis.__headerBytesResult",
        &json!("ok"),
        "request completion",
    )
    .await;
    let expected = if override_headers {
        vec![vec![0xc3, 0xa9], vec![0xc3, 0xbf]]
    } else {
        vec![vec![0xe9, 0xff]]
    };
    let captured = captured.lock();
    assert!(captured.iter().any(|request| request.path == "/start"));
    assert!(
        captured
            .iter()
            .any(|request| request.path == "/protected" && !request.authenticated)
    );
    assert!(
        captured
            .iter()
            .any(|request| request.path == "/protected" && request.authenticated)
    );
    for request in captured.iter() {
        assert_eq!(
            request.headers, expected,
            "{}, authenticated={}",
            request.path, request.authenticated
        );
    }
    server.abort();
}

const FETCH: &str = r#"globalThis.__headerBytesResult = 'pending';
fetch('/start', {headers: {'x-raw': '\u00e9\u00ff'}}).then(r => r.text()).then(value => {globalThis.__headerBytesResult = value});"#;
const XHR: &str = r#"globalThis.__headerBytesResult = 'pending';
var xhr = new XMLHttpRequest(); xhr.open('GET', '/start'); xhr.setRequestHeader('x-raw', '\u00e9\u00ff');
xhr.onload = () => {globalThis.__headerBytesResult = xhr.responseText}; xhr.send();"#;
const WORKER: &str = r#"globalThis.__headerBytesResult = 'pending';
var worker = new Worker(URL.createObjectURL(new Blob([
    "self.onmessage = event => fetch(event.data, {headers: {'x-raw': '\\u00e9\\u00ff'}}).then(r => r.text()).then(value => postMessage(value))"
], {type: 'text/javascript'})));
worker.onmessage = event => {globalThis.__headerBytesResult = event.data};
worker.postMessage(new URL('/start', location.href).href);"#;

#[tokio::test(flavor = "multi_thread")]
async fn fetch_header_bytes_survive_continue_redirect_and_auth() {
    check_intercepted_header_bytes(FETCH, false).await;
}
#[tokio::test(flavor = "multi_thread")]
async fn xhr_header_bytes_survive_continue_redirect_and_auth() {
    check_intercepted_header_bytes(XHR, false).await;
}
#[tokio::test(flavor = "multi_thread")]
async fn cdp_fetch_header_overrides_use_utf8_and_preserve_duplicates() {
    check_intercepted_header_bytes(FETCH, true).await;
}
#[tokio::test(flavor = "multi_thread")]
async fn cdp_xhr_header_overrides_use_utf8_and_preserve_duplicates() {
    check_intercepted_header_bytes(XHR, true).await;
}
#[tokio::test(flavor = "multi_thread")]
async fn worker_header_bytes_survive_continue_redirect_and_auth() {
    check_intercepted_header_bytes(WORKER, false).await;
}

async fn check_response_header_bytes(continue_response: bool, binary: bool) {
    use base64::{Engine as _, engine::general_purpose::STANDARD};
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let app = Router::new()
            .route(
                "/page",
                get(|| async {
                    (
                        [(CONTENT_TYPE.as_str(), "text/html")],
                        "<!doctype html><body>ready",
                    )
                }),
            )
            .route("/api", get(|| async { "original" }));
        axum::serve(listener, app).await.unwrap();
    });
    let mut ctx = TestContext::new();
    with_loaded_http_document(&mut ctx, &format!("http://{addr}/page"), "SID-1", "TID-1").await;
    enable_runtime_async(&mut ctx, "SID-1", 70200).await;
    ctx.process_async(json!({
        "id": 70201, "sessionId": "SID-1", "method": "Fetch.enable",
        "params": {"patterns": [{"urlPattern": "*/api", "requestStage": if continue_response {"Response"} else {"Request"}}]}
    })).await;
    ctx.expect_result(70201, json!({}), Some("SID-1"));
    ctx.process_async(json!({
        "id": 70202, "sessionId": "SID-1", "method": "Runtime.evaluate",
        "params": {"expression": "globalThis.__responseHeaderBytes = null; fetch('/api').then(r => { globalThis.__responseHeaderBytes = Array.from(r.headers.get('x-raw'), ch => ch.charCodeAt(0)); });"}
    })).await;
    let _ = take_response_by_id(&mut ctx, 70202);
    wait_until_message(
        &mut ctx,
        "SID-1",
        "response header interception",
        |message| message["method"] == "Fetch.requestPaused",
    )
    .await;
    let paused = ctx.take_first_matching("Fetch.requestPaused", |message| {
        message["method"] == "Fetch.requestPaused"
    });
    let mut params = json!({"requestId": paused["params"]["requestId"], "responseCode": 200});
    if binary {
        params["binaryResponseHeaders"] =
            json!(STANDARD.encode(b"content-type: text/plain\0x-raw: \xe9\xff\0x-raw: \xc3\xa9\0"));
    } else {
        params["responseHeaders"] = json!([
            {"name": "content-type", "value": "text/plain"},
            {"name": "x-raw", "value": "éÿ"},
            {"name": "x-raw", "value": "é"}
        ]);
    }
    if !continue_response {
        params["body"] = json!(STANDARD.encode("fulfilled"));
    }
    ctx.process_async(json!({
        "id": 70203, "sessionId": "SID-1",
        "method": if continue_response {"Fetch.continueResponse"} else {"Fetch.fulfillRequest"}, "params": params
    })).await;
    ctx.expect_result(70203, json!({}), Some("SID-1"));
    let expected = if binary {
        json!([0xe9, 0xff, 44, 32, 0xc3, 0xa9])
    } else {
        json!([0xc3, 0xa9, 0xc3, 0xbf, 44, 32, 0xc3, 0xa9])
    };
    evaluate_until_value_async(
        &mut ctx,
        "SID-1",
        70204,
        "JSON.stringify(globalThis.__responseHeaderBytes)",
        &json!(expected.to_string()),
        "raw response header bytes in Fetch Headers",
    )
    .await;
    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn fulfilled_binary_response_header_bytes_reach_fetch_headers() {
    check_response_header_bytes(false, true).await;
}
#[tokio::test(flavor = "multi_thread")]
async fn continued_binary_response_header_bytes_reach_fetch_headers() {
    check_response_header_bytes(true, true).await;
}
#[tokio::test(flavor = "multi_thread")]
async fn fulfilled_text_response_header_bytes_are_utf8() {
    check_response_header_bytes(false, false).await;
}
