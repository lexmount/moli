use super::*;
use base64::Engine as _;

const CSS: &str = "body{background-color:rgb(4,5,6)}";
const FULFILLED_CSS: &str = "body{background-color:rgb(1,2,3)}";

async fn stylesheet_server() -> (String, Arc<AtomicUsize>, tokio::task::JoinHandle<()>) {
    let hits = Arc::new(AtomicUsize::new(0));
    let captured = hits.clone();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let app = Router::new()
            .route(
                "/page",
                get(|| async {
                    (
                        [(CONTENT_TYPE.as_str(), "text/html")],
                        "<!doctype html><body>styles",
                    )
                }),
            )
            .route(
                "/parser",
                get(|| async {
                    (
                        [(CONTENT_TYPE.as_str(), "text/html")],
                        "<!doctype html><link rel=stylesheet href=/start.css><body>parser",
                    )
                }),
            )
            .route(
                "/start.css",
                get(|| async { axum::response::Redirect::temporary("/actual.css") }),
            )
            .route(
                "/actual.css",
                get(|| async {
                    (
                        [(CONTENT_TYPE.as_str(), "text/css")],
                        "@import url('/nested.css');body{background-color:rgb(4,5,6)}",
                    )
                }),
            )
            .route(
                "/nested.css",
                get(|| async { ([(CONTENT_TYPE.as_str(), "text/css")], "body{color:red}") }),
            )
            .route(
                "/cors.css",
                get(|| async { ([(CONTENT_TYPE.as_str(), "text/css")], CSS) }),
            )
            .route(
                "/cors-allow.css",
                get(|| async {
                    (
                        [
                            (CONTENT_TYPE.as_str(), "text/css"),
                            ("access-control-allow-origin", "*"),
                        ],
                        CSS,
                    )
                }),
            )
            .route(
                "/same.css",
                get(move || {
                    let captured = captured.clone();
                    async move {
                        captured.fetch_add(1, Ordering::SeqCst);
                        ([(CONTENT_TYPE.as_str(), "text/css")], CSS)
                    }
                }),
            );
        axum::serve(listener, app).await.unwrap();
    });
    (format!("http://{addr}"), hits, server)
}

async fn command(ctx: &mut TestContext, id: &mut u64, method: &str, params: Value) {
    let current = *id;
    *id += 1;
    ctx.process_async(json!({"id":current,"sessionId":"SID-1","method":method,"params":params}))
        .await;
    ctx.expect_result(current, json!({}), Some("SID-1"));
}

async fn evaluate(ctx: &mut TestContext, id: &mut u64, expression: &str) -> Value {
    let current = *id;
    *id += 1;
    ctx.process_async(json!({"id":current,"sessionId":"SID-1","method":"Runtime.evaluate","params":{"expression":expression}})).await;
    let response = take_response_by_id(ctx, current);
    assert!(
        response["result"].get("exceptionDetails").is_none(),
        "{response}"
    );
    response["result"]["result"]["value"].clone()
}

async fn append_link(
    ctx: &mut TestContext,
    id: &mut u64,
    name: &str,
    url: &str,
    preload: bool,
    cross_origin: Option<&str>,
) {
    evaluate(
        ctx,
        id,
        &format!(
            r#"(() => {{
        globalThis.__cssDone ??= {{}}; globalThis.__cssLinks ??= {{}};
        const name = {name}; const link = document.createElement('link');
        __cssDone[name] = null; __cssLinks[name] = link;
        link.rel = {rel}; if ({preload}) link.as = 'style';
        if ({cross_origin} !== null) link.crossOrigin = {cross_origin};
        link.href = {url};
        link.onload = () => __cssDone[name] = 'load';
        link.onerror = () => __cssDone[name] = 'error';
        document.head.appendChild(link);
    }})()"#,
            name = json!(name),
            rel = json!(if preload { "preload" } else { "stylesheet" }),
            cross_origin = json!(cross_origin),
            url = json!(url),
        ),
    )
    .await;
}

async fn expect_value(ctx: &mut TestContext, id: &mut u64, expression: &str, expected: Value) {
    evaluate_until_value_async(ctx, "SID-1", *id, expression, &expected, expression).await;
    *id += 100;
}

async fn pause_for(ctx: &mut TestContext, url: &str) -> Value {
    wait_until_message(ctx, "SID-1", "stylesheet interception", |m| {
        m["method"] == "Fetch.requestPaused" && m["params"]["request"]["url"] == url
    })
    .await;
    let paused = ctx.take_first_matching("stylesheet pause", |m| {
        m["method"] == "Fetch.requestPaused" && m["params"]["request"]["url"] == url
    });
    assert_eq!(paused["params"]["resourceType"], "Stylesheet");
    paused
}

async fn fulfill(ctx: &mut TestContext, id: &mut u64, pause: &Value) {
    command(
        ctx,
        id,
        "Fetch.fulfillRequest",
        json!({
            "requestId":pause["params"]["requestId"], "responseCode":200,
            "responseHeaders":[{"name":"Content-Type","value":"text/css"}],
            "body":base64::engine::general_purpose::STANDARD.encode(FULFILLED_CSS),
        }),
    )
    .await;
}

async fn setup(ctx: &mut TestContext, base: &str, id: &mut u64, stage: &str) {
    with_loaded_http_document(ctx, &format!("{base}/page"), "SID-1", "TID-1").await;
    ctx.with_fixture(|conn| {
        conn.runtime_session_owner_slot_mut(Some("SID-1"))
            .expect("stylesheet test target")
            .disable_primary_network_events();
    });
    enable_runtime_async(ctx, "SID-1", *id).await;
    *id += 1;
    command(ctx, id, "Network.enable", json!({})).await;
    command(
        ctx,
        id,
        "Network.setCacheDisabled",
        json!({"cacheDisabled":true}),
    )
    .await;
    command(
        ctx,
        id,
        "Fetch.enable",
        json!({"patterns":[{"resourceType":"Stylesheet","requestStage":stage}]}),
    )
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn stylesheet_disabled_cache_reintercepts_a_successful_same_url_for_a_new_owner() {
    let (base, hits, server) = stylesheet_server().await;
    for disable_after_first in [false, true] {
        let mut ctx = TestContext::new();
        let mut id = 81000;
        setup(&mut ctx, &base, &mut id, "Request").await;
        if disable_after_first {
            command(
                &mut ctx,
                &mut id,
                "Network.setCacheDisabled",
                json!({"cacheDisabled":false}),
            )
            .await;
        }
        let url = format!("{base}/same.css");
        append_link(&mut ctx, &mut id, "first", &url, false, None).await;
        let first = pause_for(&mut ctx, &url).await;
        fulfill(&mut ctx, &mut id, &first).await;
        expect_value(&mut ctx, &mut id, "__cssDone.first", json!("load")).await;
        expect_value(
            &mut ctx,
            &mut id,
            "getComputedStyle(document.body).backgroundColor",
            json!("rgb(1, 2, 3)"),
        )
        .await;
        evaluate(&mut ctx, &mut id, "__cssLinks.first.remove()").await;
        command(
            &mut ctx,
            &mut id,
            "Network.setCacheDisabled",
            json!({"cacheDisabled":true}),
        )
        .await;
        append_link(&mut ctx, &mut id, "second", &url, false, None).await;
        let second = pause_for(&mut ctx, &url).await;
        assert_ne!(first["params"]["requestId"], second["params"]["requestId"]);
        assert_ne!(first["params"]["networkId"], second["params"]["networkId"]);
        command(
            &mut ctx,
            &mut id,
            "Fetch.failRequest",
            json!({"requestId":second["params"]["requestId"],"errorReason":"Aborted"}),
        )
        .await;
        expect_value(&mut ctx, &mut id, "__cssDone.second", json!("error")).await;
        expect_value(
            &mut ctx,
            &mut id,
            "getComputedStyle(document.body).backgroundColor",
            json!("rgba(0, 0, 0, 0)"),
        )
        .await;
        assert_eq!(
            hits.load(Ordering::SeqCst),
            0,
            "fulfilled and aborted requests never reach the origin"
        );
    }
    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn stylesheet_disabled_cache_preserves_pending_sharing_and_preload_consumption() {
    let (base, hits, server) = stylesheet_server().await;
    for (preload, completed_preload) in [(false, false), (true, false), (true, true)] {
        let mut ctx = TestContext::new();
        let mut id = 82000;
        setup(&mut ctx, &base, &mut id, "Request").await;
        let url = format!("{base}/same.css");
        append_link(&mut ctx, &mut id, "first", &url, preload, None).await;
        let first = pause_for(&mut ctx, &url).await;
        if completed_preload {
            fulfill(&mut ctx, &mut id, &first).await;
            expect_value(&mut ctx, &mut id, "__cssDone.first", json!("load")).await;
        }
        append_link(&mut ctx, &mut id, "second", &url, false, None).await;
        if !completed_preload {
            fulfill(&mut ctx, &mut id, &first).await;
        }
        expect_value(&mut ctx, &mut id, "__cssDone.first", json!("load")).await;
        expect_value(&mut ctx, &mut id, "__cssDone.second", json!("load")).await;
        assert!(
            !ctx.sent
                .iter()
                .any(|m| m["method"] == "Fetch.requestPaused"
                    && m["params"]["request"]["url"] == url),
            "a pending transfer or unused preload must supply both consumers"
        );
        expect_value(
            &mut ctx,
            &mut id,
            "getComputedStyle(document.body).backgroundColor",
            json!("rgb(1, 2, 3)"),
        )
        .await;
        append_link(&mut ctx, &mut id, "third", &url, false, None).await;
        let third = pause_for(&mut ctx, &url).await;
        command(
            &mut ctx,
            &mut id,
            "Fetch.failRequest",
            json!({"requestId":third["params"]["requestId"],"errorReason":"Aborted"}),
        )
        .await;
        expect_value(&mut ctx, &mut id, "__cssDone.third", json!("error")).await;
        expect_value(
            &mut ctx,
            &mut id,
            "getComputedStyle(document.body).backgroundColor",
            json!("rgb(1, 2, 3)"),
        )
        .await;
        assert_eq!(hits.load(Ordering::SeqCst), 0);
    }
    server.abort();
}

fn assert_initiator(ctx: &TestContext, paused: &Value, url: &str, expected: &str) {
    let event = ctx
        .sent
        .iter()
        .find(|m| {
            m["method"] == "Network.requestWillBeSent"
                && m["params"]["requestId"] == paused["params"]["networkId"]
                && m["params"]["request"]["url"] == url
        })
        .expect("Network request must precede its Fetch pause");
    assert_eq!(event["params"]["type"], "Stylesheet");
    assert_eq!(event["params"]["initiator"]["type"], expected, "{event}");
}

#[tokio::test(flavor = "multi_thread")]
async fn stylesheet_interception_preserves_parser_import_redirect_and_script_initiators() {
    let (base, _, server) = stylesheet_server().await;
    let mut ctx = TestContext::new();
    let mut id = 83000;
    setup(&mut ctx, &base, &mut id, "Request").await;
    ctx.process_async(json!({"id":id,"sessionId":"SID-1","method":"Page.navigate","params":{"url":format!("{base}/parser")}})).await;
    let navigation = take_response_by_id(&mut ctx, id);
    assert!(navigation.get("error").is_none(), "{navigation}");
    id += 1;
    for path in ["/start.css", "/actual.css", "/nested.css"] {
        let url = format!("{base}{path}");
        let paused = pause_for(&mut ctx, &url).await;
        assert_initiator(&ctx, &paused, &url, "parser");
        command(
            &mut ctx,
            &mut id,
            "Fetch.continueRequest",
            json!({"requestId":paused["params"]["requestId"]}),
        )
        .await;
    }
    expect_value(
        &mut ctx,
        &mut id,
        "getComputedStyle(document.body).backgroundColor",
        json!("rgb(4, 5, 6)"),
    )
    .await;
    let url = format!("{base}/same.css");
    append_link(&mut ctx, &mut id, "dynamic", &url, false, None).await;
    let paused = pause_for(&mut ctx, &url).await;
    assert_initiator(&ctx, &paused, &url, "script");
    command(
        &mut ctx,
        &mut id,
        "Fetch.continueRequest",
        json!({"requestId":paused["params"]["requestId"]}),
    )
    .await;
    expect_value(&mut ctx, &mut id, "__cssDone.dynamic", json!("load")).await;
    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn intercepted_stylesheet_redirect_reports_extra_info_for_both_hops() {
    let (base, _, server) = stylesheet_server().await;
    let mut ctx = TestContext::new();
    let mut id = 83500;
    setup(&mut ctx, &base, &mut id, "Request").await;

    let start = format!("{base}/start.css");
    let destination = format!("{base}/actual.css");
    append_link(&mut ctx, &mut id, "redirected", &start, false, None).await;
    let first = pause_for(&mut ctx, &start).await;
    let request_id = first["params"]["networkId"].as_str().unwrap().to_owned();
    command(
        &mut ctx,
        &mut id,
        "Fetch.continueRequest",
        json!({"requestId":first["params"]["requestId"]}),
    )
    .await;
    let second = pause_for(&mut ctx, &destination).await;
    assert_eq!(second["params"]["networkId"], request_id);
    command(
        &mut ctx,
        &mut id,
        "Fetch.continueRequest",
        json!({"requestId":second["params"]["requestId"]}),
    )
    .await;
    wait_until_message(
        &mut ctx,
        "SID-1",
        "stylesheet network completion",
        |message| {
            message["method"] == "Network.loadingFinished"
                && message["params"]["requestId"] == request_id
        },
    )
    .await;

    let mut request_events = Vec::new();
    let mut request_extra_count = 0;
    let mut response_extra_statuses = Vec::new();
    for message in ctx
        .sent
        .iter()
        .filter(|message| message["params"]["requestId"] == request_id)
    {
        match message["method"].as_str() {
            Some("Network.requestWillBeSent") => request_events.push(&message["params"]),
            Some("Network.requestWillBeSentExtraInfo") => request_extra_count += 1,
            Some("Network.responseReceivedExtraInfo") => {
                response_extra_statuses.push(message["params"]["statusCode"].as_u64().unwrap());
            }
            _ => {}
        }
    }
    assert_eq!(request_events.len(), 2);
    assert_eq!(request_events[0]["request"]["url"], start);
    assert_eq!(request_events[1]["request"]["url"], destination);
    assert_eq!(request_events[1]["redirectHasExtraInfo"], true);
    assert_eq!(request_events[1]["redirectResponse"]["url"], start);
    assert_eq!(request_extra_count, 2);
    assert_eq!(response_extra_statuses, [307, 200]);
    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn stylesheet_response_interception_preserves_cors_and_cssom_access() {
    let (base, _, page_server) = stylesheet_server().await;
    let (cross, _, css_server) = stylesheet_server().await;
    for (cross_origin, path, stage, done, cssom) in [
        (
            None,
            "/cors.css",
            "Response",
            "load",
            json!("SecurityError"),
        ),
        (
            Some("anonymous"),
            "/cors-allow.css",
            "Response",
            "load",
            json!(1),
        ),
        (
            Some("anonymous"),
            "/cors.css",
            "Request",
            "error",
            json!("SecurityError"),
        ),
    ] {
        let mut ctx = TestContext::new();
        let mut id = 84000;
        setup(&mut ctx, &base, &mut id, stage).await;
        let url = format!("{cross}{path}");
        append_link(&mut ctx, &mut id, "cors", &url, false, cross_origin).await;
        let paused = pause_for(&mut ctx, &url).await;
        assert_initiator(&ctx, &paused, &url, "script");
        if stage == "Response" {
            assert_eq!(paused["params"]["responseStatusCode"], 200);
            let body_id = id;
            id += 1;
            ctx.process_async(json!({"id":body_id,"sessionId":"SID-1","method":"Fetch.getResponseBody","params":{"requestId":paused["params"]["requestId"]}})).await;
            let response = take_response_by_id(&mut ctx, body_id);
            let body = response["result"]["body"].as_str().expect("response body");
            let bytes = if response["result"]["base64Encoded"] == true {
                base64::engine::general_purpose::STANDARD
                    .decode(body)
                    .unwrap()
            } else {
                body.as_bytes().to_vec()
            };
            assert_eq!(bytes, CSS.as_bytes());
            command(
                &mut ctx,
                &mut id,
                "Fetch.continueResponse",
                json!({"requestId":paused["params"]["requestId"]}),
            )
            .await;
        } else {
            command(
                &mut ctx,
                &mut id,
                "Fetch.continueRequest",
                json!({"requestId":paused["params"]["requestId"]}),
            )
            .await;
        }
        expect_value(&mut ctx, &mut id, "__cssDone.cors", json!(done)).await;
        expect_value(&mut ctx, &mut id, "(() => {const sheet=__cssLinks.cors.sheet;if(!sheet)return 'no-sheet';try{return sheet.cssRules.length}catch(e){return e.name}})()", cssom).await;
        expect_value(
            &mut ctx,
            &mut id,
            "getComputedStyle(document.body).backgroundColor",
            json!(if done == "load" {
                "rgb(4, 5, 6)"
            } else {
                "rgba(0, 0, 0, 0)"
            }),
        )
        .await;
        let network_id = &paused["params"]["networkId"];
        assert_eq!(
            ctx.sent
                .iter()
                .filter(|m| m["method"] == "Network.requestWillBeSent"
                    && m["params"]["requestId"] == *network_id)
                .count(),
            1,
            "{cross_origin:?} {path} {stage}; events={:?}",
            ctx.sent
                .iter()
                .filter(|m| m["params"]["requestId"] == *network_id)
                .collect::<Vec<_>>()
        );
        assert_eq!(
            ctx.sent
                .iter()
                .filter(|m| (m["method"] == "Network.loadingFinished"
                    || m["method"] == "Network.loadingFailed")
                    && m["params"]["requestId"] == *network_id)
                .count(),
            1
        );
    }
    page_server.abort();
    css_server.abort();
}
