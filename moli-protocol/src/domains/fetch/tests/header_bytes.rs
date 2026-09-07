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

async fn check_intercepted_header_bytes(script: &str, override_headers: bool, request_path: &str) {
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
    ctx.process_async(json!({"id": 70107, "sessionId": "SID-1", "method": "Network.enable"}))
        .await;
    ctx.expect_result(70107, json!({}), Some("SID-1"));
    ctx.process_async(json!({
        "id": 70101, "sessionId": "SID-1", "method": "Fetch.enable",
        "params": {"handleAuthRequests": true, "patterns": [{"urlPattern": format!("*{request_path}")}]}
    }))
    .await;
    ctx.expect_result(70101, json!({}), Some("SID-1"));
    ctx.process_async(json!({
        "id": 70102, "sessionId": "SID-1", "method": "Runtime.evaluate",
        "params": {"expression": script.replace("/start", request_path)}
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
    let network_id = paused["params"]["networkId"].clone();
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
    if script == XHR {
        evaluate_until_value_async(
            &mut ctx,
            "SID-1",
            70106,
            "globalThis.__headerBytesFinalPath",
            &json!("/protected"),
            "authenticated response URL",
        )
        .await;
    } else {
        evaluate_until_value_async(
            &mut ctx,
            "SID-1",
            70106,
            "globalThis.__headerBytesRedirected",
            &json!(request_path == "/start"),
            "redirect history survives authentication",
        )
        .await;
    }
    let request_urls = ctx
        .sent
        .iter()
        .filter(|message| {
            message["method"] == "Network.requestWillBeSent"
                && message["params"]["requestId"] == network_id
        })
        .map(|message| message["params"]["request"]["url"].clone())
        .collect::<Vec<_>>();
    let expected_urls = if request_path == "/start" {
        vec![
            json!(format!("http://{addr}/start")),
            json!(format!("http://{addr}/protected")),
        ]
    } else {
        vec![json!(format!("http://{addr}/protected"))]
    };
    assert_eq!(
        request_urls, expected_urls,
        "auth retries preserve Network request identity"
    );
    let captured = captured.lock();
    assert_eq!(captured.len(), if request_path == "/start" { 3 } else { 2 });
    assert!(captured.iter().any(|request| request.path == request_path));
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
        let expected = if override_headers && request.path == request_path {
            vec![vec![0xc3, 0xbf]]
        } else {
            vec![vec![0xe9, 0xff]]
        };
        assert_eq!(
            request.headers, expected,
            "{}, authenticated={}",
            request.path, request.authenticated
        );
    }
    server.abort();
}

const FETCH: &str = r#"globalThis.__headerBytesResult = 'pending';
fetch('/start', {headers: {'x-raw': '\u00e9\u00ff'}}).then(r => {globalThis.__headerBytesRedirected = r.redirected; return r.text()}).then(value => {globalThis.__headerBytesResult = value});"#;
const XHR: &str = r#"globalThis.__headerBytesResult = 'pending';
var xhr = new XMLHttpRequest(); xhr.open('GET', '/start'); xhr.setRequestHeader('x-raw', '\u00e9\u00ff');
xhr.onload = () => {globalThis.__headerBytesFinalPath = new URL(xhr.responseURL).pathname; globalThis.__headerBytesResult = xhr.responseText}; xhr.send();"#;
const WORKER: &str = r#"globalThis.__headerBytesResult = 'pending';
var worker = new Worker(URL.createObjectURL(new Blob([
    "self.onmessage = event => fetch(event.data, {headers: {'x-raw': '\\u00e9\\u00ff'}}).then(async r => postMessage({value: await r.text(), redirected: r.redirected}))"
], {type: 'text/javascript'})));
worker.onmessage = event => {globalThis.__headerBytesResult = event.data.value; globalThis.__headerBytesRedirected = event.data.redirected};
worker.postMessage(new URL('/start', location.href).href);"#;

#[tokio::test(flavor = "multi_thread")]
async fn fetch_header_bytes_survive_continue_redirect_and_auth() {
    check_intercepted_header_bytes(FETCH, false, "/start").await;
}
#[tokio::test(flavor = "multi_thread")]
async fn xhr_header_bytes_survive_continue_redirect_and_auth() {
    check_intercepted_header_bytes(XHR, false, "/start").await;
}
#[tokio::test(flavor = "multi_thread")]
async fn cdp_fetch_header_overrides_use_utf8_last_value_and_expire_on_redirect() {
    check_intercepted_header_bytes(FETCH, true, "/start").await;
}
#[tokio::test(flavor = "multi_thread")]
async fn cdp_xhr_header_overrides_use_utf8_last_value_and_expire_on_redirect() {
    check_intercepted_header_bytes(XHR, true, "/start").await;
}
#[tokio::test(flavor = "multi_thread")]
async fn worker_header_bytes_survive_continue_redirect_and_auth() {
    check_intercepted_header_bytes(WORKER, false, "/start").await;
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

#[tokio::test(flavor = "multi_thread")]
async fn cdp_worker_header_overrides_expire_on_redirect_before_auth_retry() {
    check_intercepted_header_bytes(WORKER, true, "/start").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn cdp_fetch_header_override_survives_same_hop_auth_retry() {
    check_intercepted_header_bytes(FETCH, true, "/protected").await;
}
#[tokio::test(flavor = "multi_thread")]
async fn cdp_xhr_header_override_survives_same_hop_auth_retry() {
    check_intercepted_header_bytes(XHR, true, "/protected").await;
}
#[tokio::test(flavor = "multi_thread")]
async fn cdp_worker_header_override_survives_same_hop_auth_retry() {
    check_intercepted_header_bytes(WORKER, true, "/protected").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn navigation_header_override_expires_on_redirect_before_auth_retry() {
    let captured = CapturedHeaders::default();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = Router::new()
        .route("/start", get(capture_headers))
        .route("/protected", get(capture_headers))
        .with_state(captured.clone());
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let mut ctx = TestContext::new();
    ctx.conn
        .install_browser_context_fixture_for_test(attached_browser_context(&ctx.conn));
    ctx.enable_page_events_for_test(Some("SID-1"));
    ctx.conn
        .set_global_extra_headers(moli_fetch::RequestHeaders::from_bytes(vec![(
            "x-raw".into(),
            vec![0xe9, 0xff],
        )]));
    ctx.process_async(json!({"id": 70300, "sessionId": "SID-1", "method": "Fetch.enable", "params": {"handleAuthRequests": true}})).await;
    ctx.expect_result(70300, json!({}), Some("SID-1"));
    ctx.process_async(json!({"id": 70301, "sessionId": "SID-1", "method": "Page.navigate", "params": {"url": format!("http://{addr}/start")}})).await;
    let paused = take_main_document_request_pause(&mut ctx).await;
    ctx.process_async(
        json!({"id": 70302, "sessionId": "SID-1", "method": "Fetch.continueRequest", "params": {
            "requestId": paused["params"]["requestId"], "headers": [{"name": "x-raw", "value": "ÿ"}]
        }}),
    )
    .await;
    ctx.expect_result(70302, json!({}), Some("SID-1"));
    wait_until_message(&mut ctx, "SID-1", "redirected navigation auth", |message| {
        message["method"] == "Fetch.authRequired"
    })
    .await;
    let auth = ctx.take_first_matching("auth", |message| message["method"] == "Fetch.authRequired");
    ctx.process_async(json!({"id": 70303, "sessionId": "SID-1", "method": "Fetch.continueWithAuth", "params": {
        "requestId": auth["params"]["requestId"], "authChallengeResponse": {"response": "ProvideCredentials", "username": "user", "password": "pass"}
    }})).await;
    ctx.expect_result(70303, json!({}), Some("SID-1"));
    wait_until_frame_stopped_loading(&mut ctx, "TID-1").await;
    let captured = captured.lock();
    assert_eq!(captured.len(), 3);
    assert_eq!(captured[0].path, "/start");
    assert_eq!(captured[0].headers, vec![vec![0xc3, 0xbf]]);
    for request in &captured[1..] {
        assert_eq!(request.path, "/protected");
        assert_eq!(request.headers, vec![vec![0xe9, 0xff]]);
    }
    assert!(!captured[1].authenticated);
    assert!(captured[2].authenticated);
    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn bidi_extra_header_bytes_reach_fetch_xhr_worker_and_navigation() {
    use crate::devtools_runtime::{
        DevToolsCommand, DevToolsCommandContext, DevToolsCommandResult, DevToolsProtocol,
        DevToolsSessionId, DevToolsSetExtraHeadersCommand, DevToolsTargetId,
    };
    async fn echo_headers(headers: HeaderMap) -> String {
        ["x-extra", "x-utf8"]
            .into_iter()
            .map(|name| {
                headers
                    .get(name)
                    .expect("extra header")
                    .as_bytes()
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("/")
    }
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
        .route("/echo", get(echo_headers));
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let mut ctx = TestContext::new();
    with_loaded_http_document(&mut ctx, &format!("http://{addr}/page"), "SID-1", "TID-1").await;
    enable_runtime_async(&mut ctx, "SID-1", 70400).await;
    let outcome = ctx
        .conn
        .execute_devtools_command(DevToolsCommand::SetExtraHeaders(
            DevToolsSetExtraHeadersCommand {
                context: DevToolsCommandContext {
                    protocol: DevToolsProtocol::WebDriverBidi,
                    session_id: Some(DevToolsSessionId::from("bidi-session-1")),
                    target_id: None,
                    browser_context_id: None,
                },
                target_ids: vec![DevToolsTargetId::from("TID-1")],
                browser_context_ids: Vec::new(),
                headers: moli_fetch::RequestHeaders::from_bytes(vec![
                    ("x-extra".into(), vec![0xe9, 0xff]),
                    ("x-utf8".into(), vec![0xc3, 0xa9]),
                ]),
            },
        ))
        .await;
    assert!(matches!(
        outcome.into_complete_parts().0,
        Ok(DevToolsCommandResult::Empty)
    ));
    for script in [FETCH, XHR, WORKER] {
        ctx.process_async(json!({"id": 70401, "sessionId": "SID-1", "method": "Runtime.evaluate", "params": {"expression": script.replace("/start", "/echo")}})).await;
        let _ = take_response_by_id(&mut ctx, 70401);
        evaluate_until_value_async(
            &mut ctx,
            "SID-1",
            70402,
            "globalThis.__headerBytesResult",
            &json!("e9ff/c3a9"),
            "extra headers on the wire",
        )
        .await;
    }
    ctx.enable_page_events_for_test(Some("SID-1"));
    ctx.process_async(json!({"id": 70403, "sessionId": "SID-1", "method": "Page.navigate", "params": {"url": format!("http://{addr}/echo")}})).await;
    wait_until_frame_stopped_loading(&mut ctx, "TID-1").await;
    evaluate_until_value_async(
        &mut ctx,
        "SID-1",
        70404,
        "document.body.textContent",
        &json!("e9ff/c3a9"),
        "navigation extra headers on the wire",
    )
    .await;
    server.abort();
}
