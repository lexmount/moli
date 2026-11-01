use super::*;

const CHILD_BODY_EXPRESSION: &str =
    "document.querySelector('iframe').contentDocument?.body?.textContent ?? ''";

fn counted_html(hits: &Arc<AtomicUsize>, body: &'static str) -> axum::routing::MethodRouter {
    let hits = hits.clone();
    any(move || {
        let hits = hits.clone();
        async move {
            hits.fetch_add(1, Ordering::SeqCst);
            ([(CONTENT_TYPE.as_str(), "text/html")], body)
        }
    })
}

async fn serve_child_document_fixture(
    page: &'static str,
    child_hits: &Arc<AtomicUsize>,
) -> (std::net::SocketAddr, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let router = Router::new()
        .route(
            "/page",
            get(move || async move { ([(CONTENT_TYPE.as_str(), "text/html")], page) }),
        )
        .route(
            "/child",
            counted_html(child_hits, "<!doctype html><body>child</body>"),
        );
    let server = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    (addr, server)
}

async fn enable_fetch_patterns(ctx: &mut TestContext, id: u64, patterns: Value) {
    ctx.process_async(json!({
        "id": id,
        "method": "Fetch.enable",
        "sessionId": "SID-1",
        "params": { "patterns": patterns }
    }))
    .await;
    ctx.expect_result(id, json!({}), Some("SID-1"));
}

/// Waits for the paused child Document request and checks Chromium's
/// Network projection: one prior requestWillBeSent whose id is the loader id.
async fn wait_for_child_document_pause(
    ctx: &mut TestContext,
    url: &str,
    description: &str,
) -> (Value, String) {
    wait_until_message(ctx, "SID-1", description, |message| {
        message["method"] == json!("Fetch.requestPaused")
            && message["params"]["request"]["url"] == json!(url)
    })
    .await;
    let paused = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Fetch.requestPaused")
                && message["params"]["request"]["url"] == json!(url)
        })
        .cloned()
        .expect("child Document requestPaused event");
    assert_eq!(paused["params"]["resourceType"], json!("Document"));
    let request = network_request_announced_before_fetch_pause(ctx, &paused, Some("Document"));
    assert_eq!(request["params"]["loaderId"], paused["params"]["networkId"]);
    let network_id = paused["params"]["networkId"]
        .as_str()
        .expect("child Document network id")
        .to_owned();
    (paused, network_id)
}

fn request_will_be_sent_count(ctx: &TestContext, network_id: &str) -> usize {
    ctx.sent
        .iter()
        .filter(|message| {
            message["method"] == json!("Network.requestWillBeSent")
                && message["params"]["requestId"] == json!(network_id)
        })
        .count()
}

async fn evaluate_script(ctx: &mut TestContext, id: u64, expression: &str) {
    ctx.process_async(json!({
        "id": id,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": { "expression": expression }
    }))
    .await;
    let _ = take_response_by_id(ctx, id);
}

#[tokio::test(flavor = "multi_thread")]
async fn child_frame_document_request_pauses_until_fail_request() {
    let child_hits = Arc::new(AtomicUsize::new(0));
    let (addr, server) = serve_child_document_fixture(
        "<!doctype html><html><body><iframe></iframe></body></html>",
        &child_hits,
    )
    .await;
    let page_url = format!("http://{addr}/page");
    let child_url = format!("http://{addr}/child");
    let mut ctx = TestContext::new();
    with_loaded_http_document(&mut ctx, &page_url, "SID-1", "TID-1").await;
    let child_frame_id = child_frame_id_for_single_iframe_async(&mut ctx, 37_000).await;
    ctx.sent.clear();

    enable_fetch_patterns(&mut ctx, 37_001, json!([{ "urlPattern": "*" }])).await;
    enable_runtime_async(&mut ctx, "SID-1", 37_002).await;
    evaluate_script(
        &mut ctx,
        37_003,
        "document.querySelector('iframe').src = '/child'",
    )
    .await;

    let (paused, network_id) =
        wait_for_child_document_pause(&mut ctx, &child_url, "child frame document requestPaused")
            .await;
    assert_eq!(paused["params"]["frameId"], json!(child_frame_id));
    assert_eq!(paused["params"]["request"]["method"], json!("GET"));
    assert_eq!(child_hits.load(Ordering::SeqCst), 0);

    let request_id = paused["params"]["requestId"].as_str().unwrap().to_owned();
    ctx.process_async(json!({
        "id": 37_004,
        "method": "Fetch.failRequest",
        "sessionId": "SID-1",
        "params": { "requestId": request_id, "errorReason": "BlockedByClient" }
    }))
    .await;
    ctx.expect_result(37_004, json!({}), Some("SID-1"));

    wait_until_message(
        &mut ctx,
        "SID-1",
        "failed child frame document loadingFailed",
        |message| {
            message["method"] == json!("Network.loadingFailed")
                && message["params"]["requestId"] == json!(network_id)
        },
    )
    .await;
    assert_eq!(child_hits.load(Ordering::SeqCst), 0);

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn child_frame_form_post_pauses_with_post_data_until_continue_request() {
    let child_hits = Arc::new(AtomicUsize::new(0));
    let post_hits = Arc::new(AtomicUsize::new(0));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let router = Router::new()
        .route(
            "/page",
            get(|| async {
                (
                    [(CONTENT_TYPE.as_str(), "text/html")],
                    r#"<!doctype html><html><body>
<iframe name="sink"></iframe>
<form id="f" target="sink" method="post" action="/frame-post"><input type="hidden" name="a" value="1"></form>
</body></html>"#,
                )
            }),
        )
        .route("/child", counted_html(&child_hits, "child"))
        .route(
            "/frame-post",
            counted_html(&post_hits, "<!doctype html><body>posted</body>"),
        );
    let server = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    let page_url = format!("http://{addr}/page");
    let post_url = format!("http://{addr}/frame-post");
    let mut ctx = TestContext::new();
    with_loaded_http_document(&mut ctx, &page_url, "SID-1", "TID-1").await;
    let child_frame_id = child_frame_id_for_single_iframe_async(&mut ctx, 37_100).await;
    ctx.sent.clear();

    enable_fetch_patterns(
        &mut ctx,
        37_101,
        json!([{ "urlPattern": "*/frame-post", "resourceType": "Document" }]),
    )
    .await;
    enable_runtime_async(&mut ctx, "SID-1", 37_102).await;
    evaluate_script(&mut ctx, 37_103, "document.getElementById('f').submit()").await;

    let (paused, network_id) =
        wait_for_child_document_pause(&mut ctx, &post_url, "child frame form POST requestPaused")
            .await;
    assert_eq!(paused["params"]["frameId"], json!(child_frame_id));
    assert_eq!(paused["params"]["request"]["method"], json!("POST"));
    assert_eq!(paused["params"]["request"]["postData"], json!("a=1"));
    assert_eq!(post_hits.load(Ordering::SeqCst), 0);

    let request_id = paused["params"]["requestId"].as_str().unwrap().to_owned();
    ctx.process_async(json!({
        "id": 37_104,
        "method": "Fetch.continueRequest",
        "sessionId": "SID-1",
        "params": { "requestId": request_id }
    }))
    .await;
    ctx.expect_result(37_104, json!({}), Some("SID-1"));

    evaluate_until_value_async(
        &mut ctx,
        "SID-1",
        37_110,
        CHILD_BODY_EXPRESSION,
        &json!("posted"),
        "continued child frame form POST commits its response",
    )
    .await;
    assert_eq!(post_hits.load(Ordering::SeqCst), 1);
    assert_eq!(child_hits.load(Ordering::SeqCst), 0);
    wait_for_network_loading_finished(
        &mut ctx,
        "SID-1",
        &network_id,
        "continued child frame form POST loadingFinished",
    )
    .await;
    assert_eq!(request_will_be_sent_count(&ctx, &network_id), 1);

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn child_frame_document_fulfill_request_commits_synthetic_document() {
    let child_hits = Arc::new(AtomicUsize::new(0));
    let (addr, server) = serve_child_document_fixture(
        "<!doctype html><html><body><iframe></iframe></body></html>",
        &child_hits,
    )
    .await;
    let page_url = format!("http://{addr}/page");
    let child_url = format!("http://{addr}/child");
    let mut ctx = TestContext::new();
    with_loaded_http_document(&mut ctx, &page_url, "SID-1", "TID-1").await;
    ctx.sent.clear();

    enable_fetch_patterns(
        &mut ctx,
        37_201,
        json!([{ "urlPattern": "*/child", "resourceType": "Document" }]),
    )
    .await;
    enable_runtime_async(&mut ctx, "SID-1", 37_202).await;
    evaluate_script(
        &mut ctx,
        37_203,
        "document.querySelector('iframe').src = '/child'",
    )
    .await;

    let (paused, network_id) =
        wait_for_child_document_pause(&mut ctx, &child_url, "child frame document requestPaused")
            .await;
    let request_id = paused["params"]["requestId"].as_str().unwrap().to_owned();
    ctx.process_async(json!({
        "id": 37_204,
        "method": "Fetch.fulfillRequest",
        "sessionId": "SID-1",
        "params": {
            "requestId": request_id,
            "responseCode": 200,
            "responseHeaders": [{ "name": "content-type", "value": "text/html" }],
            "body": "PCFkb2N0eXBlIGh0bWw+PGJvZHk+ZnVsZmlsbGVkPC9ib2R5Pg=="
        }
    }))
    .await;
    ctx.expect_result(37_204, json!({}), Some("SID-1"));

    evaluate_until_value_async(
        &mut ctx,
        "SID-1",
        37_210,
        CHILD_BODY_EXPRESSION,
        &json!("fulfilled"),
        "fulfilled child frame document commits",
    )
    .await;
    assert_eq!(child_hits.load(Ordering::SeqCst), 0);
    wait_for_network_loading_finished(
        &mut ctx,
        "SID-1",
        &network_id,
        "fulfilled child frame document loadingFinished",
    )
    .await;
    assert_eq!(request_will_be_sent_count(&ctx, &network_id), 1);

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn removing_child_frame_aborts_its_paused_document_request() {
    let child_hits = Arc::new(AtomicUsize::new(0));
    let (addr, server) = serve_child_document_fixture(
        "<!doctype html><html><body><iframe></iframe></body></html>",
        &child_hits,
    )
    .await;
    let page_url = format!("http://{addr}/page");
    let child_url = format!("http://{addr}/child");
    let mut ctx = TestContext::new();
    with_loaded_http_document(&mut ctx, &page_url, "SID-1", "TID-1").await;
    ctx.sent.clear();

    enable_fetch_patterns(&mut ctx, 37_301, json!([{ "urlPattern": "*" }])).await;
    enable_runtime_async(&mut ctx, "SID-1", 37_302).await;
    evaluate_script(
        &mut ctx,
        37_303,
        "document.querySelector('iframe').src = '/child'",
    )
    .await;
    let (paused, network_id) =
        wait_for_child_document_pause(&mut ctx, &child_url, "child frame document requestPaused")
            .await;

    evaluate_script(
        &mut ctx,
        37_304,
        "document.querySelector('iframe').remove()",
    )
    .await;
    wait_until_message(
        &mut ctx,
        "SID-1",
        "removed child frame document loadingFailed",
        |message| {
            message["method"] == json!("Network.loadingFailed")
                && message["params"]["requestId"] == json!(network_id)
        },
    )
    .await;

    let request_id = paused["params"]["requestId"].as_str().unwrap().to_owned();
    ctx.process_async(json!({
        "id": 37_305,
        "method": "Fetch.continueRequest",
        "sessionId": "SID-1",
        "params": { "requestId": request_id }
    }))
    .await;
    let response = take_response_by_id(&mut ctx, 37_305);
    assert!(response.get("error").is_some(), "{response:?}");
    assert_eq!(child_hits.load(Ordering::SeqCst), 0);

    server.abort();
}
