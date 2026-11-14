use super::*;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

async fn exact_pattern_fixture(
    intercept_target: bool,
) -> (
    TestContext,
    String,
    Arc<AtomicUsize>,
    tokio::task::JoinHandle<()>,
) {
    use axum::{Router, routing::get};
    let requests = Arc::new(AtomicUsize::new(0));
    let count = requests.clone();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route(
                    "/index.html",
                    get(|| async { "<!doctype html><body>redirect patterns</body>" }),
                )
                .route(
                    "/exact-target",
                    get(move || {
                        count.fetch_add(1, Ordering::SeqCst);
                        async { "network-target" }
                    }),
                ),
        )
        .await
        .unwrap();
    });
    let mut ctx = TestContext::new();
    with_loaded_http_document(&mut ctx, &format!("{origin}/index.html"), "SID-1", "TID-1").await;
    enable_runtime_async(&mut ctx, "SID-1", 81_000).await;
    command(&mut ctx, 81_001, "Network.enable", json!({})).await;
    let mut patterns =
        vec![json!({"urlPattern":format!("{origin}/exact-source"),"requestStage":"Request"})];
    if intercept_target {
        patterns
            .push(json!({"urlPattern":format!("{origin}/exact-target"),"requestStage":"Request"}));
    }
    command(
        &mut ctx,
        81_002,
        "Fetch.enable",
        json!({"patterns":patterns}),
    )
    .await;
    (ctx, origin, requests, server)
}

#[tokio::test(flavor = "multi_thread")]
async fn fulfilled_redirect_target_matching_an_exact_pattern_pauses_before_transport() {
    let (mut ctx, origin, requests, server) = exact_pattern_fixture(true).await;
    let source = format!("{origin}/exact-source");
    let target = format!("{origin}/exact-target");
    command(&mut ctx,81_010,"Runtime.evaluate",json!({"expression":"globalThis.exactRedirect=fetch('/exact-source').then(r=>r.text()); 'started'"})).await;
    let first = wait_for_request_paused(&mut ctx, &source, "exact source request").await;
    command(&mut ctx,81_011,"Fetch.fulfillRequest",json!({"requestId":first["params"]["requestId"],"responseCode":302,"responseHeaders":[{"name":"Location","value":"/exact-target"}]})).await;
    let next = wait_for_request_paused(&mut ctx, &target, "exact redirect target request").await;
    assert_eq!(
        requests.load(Ordering::SeqCst),
        0,
        "target must not reach the backend before interception"
    );
    assert!(
        first["params"]["requestId"]
            .as_str()
            .is_some_and(|id| !id.is_empty())
    );
    assert!(
        first["params"]["networkId"]
            .as_str()
            .is_some_and(|id| !id.is_empty())
    );
    assert_ne!(next["params"]["requestId"], first["params"]["requestId"]);
    assert_eq!(next["params"]["networkId"], first["params"]["networkId"]);
    assert_eq!(
        next["params"]["redirectedRequestId"],
        first["params"]["requestId"]
    );
    command(&mut ctx,81_012,"Fetch.fulfillRequest",json!({"requestId":next["params"]["requestId"],"responseCode":200,"body":"ZXhhY3QtdGFyZ2V0"})).await;
    let result = command(
        &mut ctx,
        81_013,
        "Runtime.evaluate",
        json!({"expression":"exactRedirect","awaitPromise":true,"returnByValue":true}),
    )
    .await;
    assert_eq!(result["result"]["result"]["value"], "exact-target");
    assert_eq!(
        requests.load(Ordering::SeqCst),
        0,
        "fulfilled target must stay off the network"
    );
    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn fulfilled_redirect_target_without_a_matching_pattern_continues_once() {
    let (mut ctx, origin, requests, server) = exact_pattern_fixture(false).await;
    command(&mut ctx,82_010,"Runtime.evaluate",json!({"expression":"globalThis.exactRedirect=fetch('/exact-source').then(r=>r.text()); 'started'"})).await;
    let first = wait_for_request_paused(
        &mut ctx,
        &format!("{origin}/exact-source"),
        "exact source request",
    )
    .await;
    command(&mut ctx,82_011,"Fetch.fulfillRequest",json!({"requestId":first["params"]["requestId"],"responseCode":302,"responseHeaders":[{"name":"Location","value":"/exact-target"}]})).await;
    let result = command(
        &mut ctx,
        82_012,
        "Runtime.evaluate",
        json!({"expression":"exactRedirect","awaitPromise":true,"returnByValue":true}),
    )
    .await;
    assert_eq!(result["result"]["result"]["value"], "network-target");
    assert_eq!(
        requests.load(Ordering::SeqCst),
        1,
        "unmatched target must be dispatched exactly once"
    );
    assert!(
        !ctx.sent
            .iter()
            .any(|message| message["method"] == "Fetch.requestPaused"
                && message["params"]["request"]["url"] == format!("{origin}/exact-target"))
    );
    server.abort();
}
