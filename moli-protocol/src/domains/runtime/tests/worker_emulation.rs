use super::*;
use axum::{Json, http::HeaderMap};
use serde_json::Value;

async fn evaluate(ctx: &mut TestContext, session: &str, expression: &str) -> Value {
    ctx.process_and_wait_for_response_async(json!({
        "id": 99200, "sessionId": session, "method": "Runtime.evaluate",
        "params": {"expression": expression, "awaitPromise": true, "returnByValue": true}
    }))
    .await;
    let response = take_response_by_id(ctx, 99200);
    assert!(response.get("error").is_none(), "{response}");
    assert!(
        response["result"].get("exceptionDetails").is_none(),
        "{response}"
    );
    response["result"]["result"]["value"].clone()
}

async fn set_ua(ctx: &mut TestContext, session: &str, method: &str, params: Value) {
    ctx.process_and_wait_for_response_async(json!({
        "id": 99201, "sessionId": session, "method": method, "params": params
    }))
    .await;
    ctx.expect_result(99201, json!({}), Some(session));
}

const SNAPSHOT: &str = r#"(() => {
    globalThis.heldNavigator ??= navigator;
    globalThis.heldUaData ??= navigator.userAgentData;
    return {
        ua: heldNavigator.userAgent, app: heldNavigator.appVersion,
        platform: heldNavigator.platform, language: heldNavigator.language,
        languages: Array.from(heldNavigator.languages),
        metadata: navigator.userAgentData.toJSON(), held: heldUaData.toJSON()
    };
})()"#;
const HEADERS: &str = "fetch('/headers', {cache:'no-store'}).then(response => response.json())";

async fn worker_user_agent_override(target_type: &str, create: &str) {
    use crate::conn::{CdpConnection, CdpInitialStoragePartition};
    use moli_core::{LayoutPolicy, OptionalResourceFetchMask, runtime::NavigationRuntimeConfig};
    use moli_fetch::FetchConfig;

    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><body>Worker UA</body>",
        )
    }
    async fn worker() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/javascript")],
            r#"
            onconnect = event => event.ports[0].postMessage('ready');
            if (typeof registration !== 'undefined') {
                addEventListener('install', event => event.waitUntil(skipWaiting()));
                addEventListener('activate', event => event.waitUntil(clients.claim()));
            }
        "#,
        )
    }
    async fn headers(headers: HeaderMap) -> Json<std::collections::BTreeMap<String, String>> {
        Json(
            headers
                .iter()
                .map(|(key, value)| (key.to_string(), value.to_str().unwrap().to_owned()))
                .collect(),
        )
    }
    async fn websocket(
        headers: HeaderMap,
        upgrade: axum::extract::ws::WebSocketUpgrade,
    ) -> impl IntoResponse {
        let ua = headers["user-agent"].to_str().unwrap().to_owned();
        upgrade.on_upgrade(move |mut socket| async move {
            socket
                .send(axum::extract::ws::Message::Text(ua.into()))
                .await
                .unwrap();
            while let Some(Ok(message)) = socket.recv().await {
                if matches!(message, axum::extract::ws::Message::Close(_)) {
                    break;
                }
            }
        })
    }
    async fn imported(headers: HeaderMap) -> impl IntoResponse {
        (
            [
                ("content-type", "text/javascript"),
                ("cache-control", "max-age=3600"),
                ("vary", "User-Agent"),
            ],
            format!(
                "globalThis.importedUA = {};",
                serde_json::to_string(headers["user-agent"].to_str().unwrap()).unwrap()
            ),
        )
    }
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/page", get(page))
                .route("/worker.js", get(worker))
                .route("/headers", get(headers))
                .route("/socket", get(websocket))
                .route(
                    "/redirect",
                    get(|| async { axum::response::Redirect::temporary("/headers") }),
                )
                .route("/imported.js", get(imported)),
        )
        .await
        .unwrap();
    });
    let mut fetch = FetchConfig::default();
    fetch.set_languages(vec!["en-US".to_owned()]).unwrap();
    let runtime_config = NavigationRuntimeConfig::new(
        fetch,
        OptionalResourceFetchMask::NONE,
        true,
        LayoutPolicy::Lazy,
    );
    let mut ctx = TestContext::from_conn(
        CdpConnection::new_with_initial_storage_partition_and_runtime_config(
            CdpInitialStoragePartition::memory(),
            runtime_config,
        ),
    );
    with_loaded_http_document_async(
        &mut ctx,
        &format!("http://{addr}/page"),
        "SID-page",
        "TID-page",
    )
    .await;
    ctx.process_async(json!({"id":99202,"method":"Target.setAutoAttach","params":{"autoAttach":true,"waitForDebuggerOnStart":false}})).await;
    ctx.expect_result(99202, json!({}), None);
    if target_type == "worker" {
        ctx.process_async(json!({"id":99208,"sessionId":"SID-page","method":"Target.setAutoAttach","params":{"autoAttach":true,"waitForDebuggerOnStart":false,"flatten":true}})).await;
        ctx.expect_result(99208, json!({}), Some("SID-page"));
    }
    evaluate(&mut ctx, "SID-page", create).await;
    wait_until_message(&mut ctx, None, "Worker UA target", |message| {
        message["method"] == "Target.attachedToTarget"
            && message["params"]["targetInfo"]["type"] == target_type
    })
    .await;
    let attached = ctx.take_first_matching("Worker UA target", |message| {
        message["method"] == "Target.attachedToTarget"
            && message["params"]["targetInfo"]["type"] == target_type
    });
    let session = attached["params"]["sessionId"].as_str().unwrap().to_owned();
    let target = attached["params"]["targetInfo"]["targetId"]
        .as_str()
        .unwrap()
        .to_owned();
    let baseline = evaluate(&mut ctx, &session, SNAPSHOT).await;
    let baseline_headers = evaluate(&mut ctx, &session, HEADERS).await;
    let page_baseline = evaluate(&mut ctx, "SID-page", SNAPSHOT).await;
    let page_headers = evaluate(&mut ctx, "SID-page", HEADERS).await;

    assert_eq!(baseline["languages"], json!(["en-US"]));
    assert_eq!(baseline_headers["accept-language"], "en-US,en;q=0.9");
    set_ua(
        &mut ctx,
        &session,
        "Emulation.setUserAgentOverride",
        json!({"userAgent":"", "acceptLanguage":"en-US,en"}),
    )
    .await;
    let expanded = evaluate(&mut ctx, &session, SNAPSHOT).await;
    assert_eq!(expanded["languages"], json!(["en-US", "en"]));
    assert_eq!(
        evaluate(&mut ctx, &session, HEADERS).await,
        baseline_headers
    );
    set_ua(
        &mut ctx,
        &session,
        "Emulation.setUserAgentOverride",
        json!({"userAgent":""}),
    )
    .await;
    assert_eq!(evaluate(&mut ctx, &session, SNAPSHOT).await, baseline);

    // Seed both the renderer and HTTP caches before changing the Worker UA.
    let import = "importScripts('/imported.js'); importedUA";
    if target_type != "service_worker" {
        assert_eq!(
            evaluate(&mut ctx, &session, import).await,
            baseline_headers["user-agent"]
        );
    }

    set_ua(&mut ctx, &session, "Emulation.setUserAgentOverride", json!({
        "userAgent":"Mozilla/5.0 WorkerAgent", "acceptLanguage":"fr-FR,fr", "platform":"ignored-in-worker",
        "userAgentMetadata": {"brands":[{"brand":"Worker","version":"7"}], "platform":"WorkerOS", "platformVersion":"2", "architecture":"arm", "model":"M", "mobile":true}
    })).await;
    let mut expected = baseline.clone();
    expected["language"] = json!("fr-FR");
    expected["languages"] = json!(["fr-FR", "fr"]);
    expected["metadata"] =
        json!({"brands":[{"brand":"Worker","version":"7"}],"platform":"WorkerOS","mobile":true});
    assert_eq!(evaluate(&mut ctx, &session, SNAPSHOT).await, expected);
    let actual = evaluate(&mut ctx, &session, HEADERS).await;
    assert_eq!(actual["user-agent"], "Mozilla/5.0 WorkerAgent");
    assert_eq!(actual["accept-language"], "fr-FR,fr;q=0.9");
    assert!(actual["sec-ch-ua"].as_str().unwrap().contains("Worker"));
    assert_eq!(evaluate(&mut ctx, &session, "fetch('/redirect').then(response => response.json()).then(headers => headers['user-agent'])").await, "Mozilla/5.0 WorkerAgent");
    assert_eq!(evaluate(&mut ctx, &session, "new Promise((resolve, reject) => { const socket = new WebSocket('ws://' + location.host + '/socket'); socket.onmessage = event => { socket.close(); resolve(event.data); }; socket.onerror = () => reject(new Error('websocket failed')); })").await, "Mozilla/5.0 WorkerAgent");
    if target_type != "service_worker" {
        assert_eq!(evaluate(&mut ctx, &session, "new Promise((resolve, reject) => { const xhr = new XMLHttpRequest(); xhr.open('GET','/headers'); xhr.onload = () => resolve(JSON.parse(xhr.responseText)['user-agent']); xhr.onerror = () => reject(new Error('xhr failed')); xhr.send(); })").await, "Mozilla/5.0 WorkerAgent");
    }

    if target_type != "service_worker" {
        assert_eq!(
            evaluate(&mut ctx, &session, import).await,
            "Mozilla/5.0 WorkerAgent"
        );
    }
    assert_eq!(evaluate(&mut ctx, &session, "navigator.userAgentData.getHighEntropyValues(['architecture','platformVersion']).then(x => [x.architecture,x.platformVersion])").await, json!(["arm", "2"]));
    assert_eq!(
        evaluate(&mut ctx, "SID-page", SNAPSHOT).await,
        page_baseline
    );
    assert_eq!(evaluate(&mut ctx, "SID-page", HEADERS).await, page_headers);

    // An override without metadata restores the Worker's creation metadata;
    // its UA string, appVersion and platform also remain creation-time values.
    set_ua(
        &mut ctx,
        &session,
        "Network.setUserAgentOverride",
        json!({"userAgent":"WorkerAlias", "acceptLanguage":"de-DE,de"}),
    )
    .await;
    expected = baseline.clone();
    expected["language"] = json!("de-DE");
    expected["languages"] = json!(["de-DE", "de"]);
    assert_eq!(evaluate(&mut ctx, &session, SNAPSHOT).await, expected);
    assert_eq!(
        evaluate(&mut ctx, &session, HEADERS).await["user-agent"],
        "WorkerAlias"
    );
    if target_type != "service_worker" {
        assert_eq!(evaluate(&mut ctx, &session, import).await, "WorkerAlias");
    }
    ctx.process_and_wait_for_response_async(json!({"id":99203,"sessionId":session,"method":"Emulation.setUserAgentOverride","params":{"userAgent":"bad\nagent"}})).await;
    ctx.expect_error(99203, -32602, "Invalid characters found in userAgent");
    assert_eq!(evaluate(&mut ctx, &session, SNAPSHOT).await, expected);
    assert_eq!(
        evaluate(&mut ctx, &session, HEADERS).await["user-agent"],
        "WorkerAlias"
    );

    set_ua(
        &mut ctx,
        &session,
        "Emulation.setUserAgentOverride",
        json!({"userAgent":""}),
    )
    .await;
    assert_eq!(evaluate(&mut ctx, &session, SNAPSHOT).await, baseline);
    assert_eq!(
        evaluate(&mut ctx, &session, HEADERS).await,
        baseline_headers
    );
    // Sessions contribute UA/metadata and language independently. Updating the
    // earlier agent cannot overtake a later agent; clearing reveals its value.
    ctx.process_async(json!({"id":99206,"method":"Target.attachToTarget","params":{"targetId":target,"flatten":true}})).await;
    let second = take_response_by_id(&mut ctx, 99206)["result"]["sessionId"]
        .as_str()
        .unwrap()
        .to_owned();
    set_ua(
        &mut ctx,
        &session,
        "Emulation.setUserAgentOverride",
        json!({"userAgent":"First", "acceptLanguage":"fr-FR"}),
    )
    .await;
    set_ua(
        &mut ctx,
        &second,
        "Network.setUserAgentOverride",
        json!({"userAgent":"Second"}),
    )
    .await;
    set_ua(
        &mut ctx,
        &session,
        "Emulation.setUserAgentOverride",
        json!({"userAgent":"FirstChanged", "acceptLanguage":"it-IT"}),
    )
    .await;
    let headers = evaluate(&mut ctx, &session, HEADERS).await;
    assert_eq!(headers["user-agent"], "Second");
    assert_eq!(headers["accept-language"], "it-IT");
    set_ua(
        &mut ctx,
        &second,
        "Emulation.setUserAgentOverride",
        json!({"userAgent":""}),
    )
    .await;
    assert_eq!(
        evaluate(&mut ctx, &second, HEADERS).await["user-agent"],
        "FirstChanged"
    );
    ctx.process_async(
        json!({"id":99207,"method":"Target.detachFromTarget","params":{"sessionId":second}}),
    )
    .await;
    ctx.expect_result(99207, json!({}), None);

    set_ua(
        &mut ctx,
        &session,
        "Network.setUserAgentOverride",
        json!({"userAgent":"DetachAgent"}),
    )
    .await;
    ctx.process_async(
        json!({"id":99204,"method":"Target.detachFromTarget","params":{"sessionId":session}}),
    )
    .await;
    ctx.expect_result(99204, json!({}), None);
    ctx.process_async(json!({"id":99205,"method":"Target.attachToTarget","params":{"targetId":target,"flatten":true}})).await;
    let observer = take_response_by_id(&mut ctx, 99205)["result"]["sessionId"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(evaluate(&mut ctx, &observer, SNAPSHOT).await, baseline);
    assert_eq!(
        evaluate(&mut ctx, &observer, HEADERS).await,
        baseline_headers
    );
    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn dedicated_worker_user_agent_override_updates_requests_and_restores_on_detach() {
    worker_user_agent_override(
        "worker",
        "globalThis.uaWorker = new Worker('/worker.js'); 'started'",
    )
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn shared_worker_user_agent_override_updates_requests_and_restores_on_detach() {
    worker_user_agent_override("shared_worker", "globalThis.uaWorker = new SharedWorker('/worker.js', 'ua'); uaWorker.port.start(); 'started'").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn service_worker_user_agent_override_updates_requests_and_restores_on_detach() {
    worker_user_agent_override("service_worker", "navigator.serviceWorker.register('/worker.js').then(() => navigator.serviceWorker.ready).then(() => 'ready')").await;
}
