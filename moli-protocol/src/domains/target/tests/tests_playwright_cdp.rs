use super::*;
use moli_core::testing::{ScriptRunOutcome, ScriptSkipReason};

async fn flush_until_playwright_subresources_finished(
    ctx: &mut TestContext,
    session_id: &str,
    resource_type: &str,
    expected_request_count: usize,
    description: &str,
) {
    crate::testing::wait_until_messages(ctx, Some(session_id), description, |messages| {
        let request_ids = messages
            .iter()
            .filter(|message| {
                message["method"] == json!("Network.requestWillBeSent")
                    && message["params"]["type"] == json!(resource_type)
            })
            .filter_map(|message| message["params"]["requestId"].as_str())
            .collect::<Vec<_>>();
        request_ids.len() >= expected_request_count
            && request_ids
                .iter()
                .take(expected_request_count)
                .all(|request_id| {
                    messages.iter().any(|message| {
                        message["method"] == json!("Network.loadingFinished")
                            && message["params"]["requestId"] == json!(request_id)
                    })
                })
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn playwright_new_context_about_blank_create_isolated_world_materializes_page() {
    let mut ctx = TestContext::new();
    ctx.process_async(json!({
        "id": 9100,
        "method": "Target.setAutoAttach",
        "params": {
            "autoAttach": true,
            "waitForDebuggerOnStart": true,
            "flatten": true
        }
    }))
    .await;
    take_response_by_id(&mut ctx, 9100);
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 9101,
        "method": "Target.createTarget",
        "params": { "url": "about:blank" }
    }))
    .await;
    take_response_by_id(&mut ctx, 9101);
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 9102,
        "method": "Target.createBrowserContext"
    }))
    .await;
    let browser_context_id = take_response_by_id(&mut ctx, 9102)["result"]["browserContextId"]
        .as_str()
        .expect("browserContextId")
        .to_owned();

    ctx.process_async(json!({
        "id": 9103,
        "method": "Target.createTarget",
        "params": {
            "browserContextId": browser_context_id,
            "url": "about:blank"
        }
    }))
    .await;
    let target_id = take_response_by_id(&mut ctx, 9103)["result"]["targetId"]
        .as_str()
        .expect("targetId")
        .to_owned();
    let session_id = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Target.attachedToTarget")
                && message["params"]["targetInfo"]["targetId"] == json!(target_id)
        })
        .and_then(|message| message["params"]["sessionId"].as_str())
        .expect("auto-attached session id")
        .to_owned();
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 9104,
        "method": "Runtime.enable",
        "sessionId": session_id
    }))
    .await;
    ctx.expect_result(9104, json!({}), Some(&session_id));
    let default_context_id = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Runtime.executionContextCreated")
                && message["sessionId"] == json!(session_id)
                && message["params"]["context"]["auxData"]["isDefault"] == json!(true)
                && message["params"]["context"]["auxData"]["frameId"] == json!(target_id)
        })
        .and_then(|message| message["params"]["context"]["id"].as_i64())
        .unwrap_or_else(|| {
            panic!(
                "Runtime.enable should replay the existing fresh about:blank default execution context: {:?}",
                ctx.sent
            )
        });
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 9105,
        "method": "Page.addScriptToEvaluateOnNewDocument",
        "sessionId": session_id,
        "params": {
            "source": "",
            "worldName": "__playwright_utility_world_page@new-context"
        }
    }))
    .await;
    ctx.expect_result(
        9105,
        json!({
            "identifier": "1"
        }),
        Some(&session_id),
    );
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 9106,
        "method": "Page.createIsolatedWorld",
        "sessionId": session_id,
        "params": {
            "frameId": target_id,
            "worldName": "__playwright_utility_world_page@new-context",
            "grantUniveralAccess": true
        }
    }))
    .await;
    let isolated_context_events = ctx
        .sent
        .iter()
        .filter(|message| {
            message["method"] == json!("Runtime.executionContextCreated")
                && message["sessionId"] == json!(session_id)
                && message["params"]["context"]["auxData"]["isDefault"] == json!(false)
                && message["params"]["context"]["name"]
                    == json!("__playwright_utility_world_page@new-context")
        })
        .collect::<Vec<_>>();
    assert_eq!(
        isolated_context_events.len(),
        1,
        "pre-created utility world should be reported once, not replayed again by createIsolatedWorld: {:?}",
        ctx.sent
    );
    assert!(
        isolated_context_events[0]["params"]["context"]["uniqueId"]
            .as_str()
            .is_some(),
        "about:blank createIsolatedWorld event should come from V8 native batch: {:?}",
        isolated_context_events[0]
    );
    let isolated_world = take_response_by_id(&mut ctx, 9106);
    assert!(
        isolated_world["result"]["executionContextId"]
            .as_i64()
            .is_some(),
        "fresh about:blank target in a non-current browser context should materialize before createIsolatedWorld: {isolated_world:?}"
    );

    ctx.sent.clear();
    ctx.process_async(json!({
        "id": 9107,
        "method": "Runtime.evaluate",
        "sessionId": session_id,
        "params": {
            "contextId": default_context_id,
            "expression": "document.body.innerHTML = '<input id=\"chooser\" type=\"file\" multiple>'; undefined"
        }
    }))
    .await;
    let evaluate_result = take_response_by_id(&mut ctx, 9107);
    assert!(
        evaluate_result.get("error").is_none(),
        "the replayed default context id should be usable for Runtime.evaluate: {evaluate_result:?}"
    );

    ctx.process_async(json!({
        "id": 9108,
        "method": "DOM.getDocument",
        "sessionId": session_id
    }))
    .await;
    let root_id = take_response_by_id(&mut ctx, 9108)["result"]["root"]["nodeId"]
        .as_u64()
        .expect("DOM.getDocument root nodeId");

    ctx.process_async(json!({
        "id": 9109,
        "method": "DOM.querySelector",
        "sessionId": session_id,
        "params": {
            "nodeId": root_id,
            "selector": "#chooser"
        }
    }))
    .await;
    let chooser_node_id = take_response_by_id(&mut ctx, 9109)["result"]["nodeId"]
        .as_u64()
        .expect("DOM.querySelector nodeId");
    assert!(chooser_node_id > 0);

    ctx.process_async(json!({
        "id": 9110,
        "method": "DOM.describeNode",
        "sessionId": session_id,
        "params": {
            "nodeId": chooser_node_id
        }
    }))
    .await;
    let chooser_backend_node_id =
        take_response_by_id(&mut ctx, 9110)["result"]["node"]["backendNodeId"]
            .as_u64()
            .expect("DOM.describeNode backendNodeId");

    ctx.process_async(json!({
        "id": 9111,
        "method": "DOM.resolveNode",
        "sessionId": session_id,
        "params": {
            "backendNodeId": chooser_backend_node_id,
            "executionContextId": default_context_id
        }
    }))
    .await;
    let resolved = take_response_by_id(&mut ctx, 9111);
    assert_eq!(
        resolved["result"]["object"]["subtype"],
        json!("node"),
        "DOM.resolveNode should accept the replayed default context id: {resolved:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn playwright_page_session_auto_attach_does_not_duplicate_new_page_target_attach() {
    let mut ctx = TestContext::new();
    ctx.process_async(json!({
        "id": 9120,
        "method": "Target.setAutoAttach",
        "params": {
            "autoAttach": true,
            "waitForDebuggerOnStart": true,
            "flatten": true
        }
    }))
    .await;
    ctx.expect_result(9120, json!({}), None);
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 9121,
        "method": "Target.createTarget",
        "params": { "url": "about:blank" }
    }))
    .await;
    let first_target_id = take_response_by_id(&mut ctx, 9121)["result"]["targetId"]
        .as_str()
        .expect("first target id")
        .to_owned();
    let page_session_id = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Target.attachedToTarget")
                && message["params"]["targetInfo"]["targetId"] == json!(first_target_id)
        })
        .and_then(|message| message["params"]["sessionId"].as_str())
        .map(str::to_owned)
        .unwrap_or_else(|| panic!("root autoAttach should attach first page: {:?}", ctx.sent));
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 9122,
        "method": "Target.setAutoAttach",
        "sessionId": page_session_id,
        "params": {
            "autoAttach": true,
            "waitForDebuggerOnStart": true,
            "flatten": true
        }
    }))
    .await;
    ctx.expect_result(9122, json!({}), Some(&page_session_id));
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 9123,
        "method": "Target.createBrowserContext"
    }))
    .await;
    let browser_context_id = take_response_by_id(&mut ctx, 9123)["result"]["browserContextId"]
        .as_str()
        .expect("browser context id")
        .to_owned();
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 9124,
        "method": "Target.createTarget",
        "params": {
            "browserContextId": browser_context_id,
            "url": "about:blank"
        }
    }))
    .await;
    let new_target_id = take_response_by_id(&mut ctx, 9124)["result"]["targetId"]
        .as_str()
        .expect("new target id")
        .to_owned();
    let attached_to_new_target = ctx
        .sent
        .iter()
        .filter(|message| {
            message["method"] == json!("Target.attachedToTarget")
                && message["params"]["targetInfo"]["targetId"] == json!(new_target_id)
        })
        .collect::<Vec<_>>();
    assert_eq!(
        attached_to_new_target.len(),
        1,
        "browser-created page target should only be auto-attached by the browser/root owner: {:?}",
        ctx.sent
    );
    let new_page_session_id = attached_to_new_target[0]["params"]["sessionId"]
        .as_str()
        .expect("new page session id")
        .to_owned();
    assert!(
        ctx.conn
            .auto_attached_sessions_for_owner(Some(&page_session_id))
            .is_empty(),
        "page session autoAttach owner must not own a browser-created top-level page session"
    );
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 9125,
        "method": "Target.setAutoAttach",
        "sessionId": new_page_session_id,
        "params": {
            "autoAttach": true,
            "waitForDebuggerOnStart": true,
            "flatten": true
        }
    }))
    .await;
    ctx.expect_result(9125, json!({}), Some(&new_page_session_id));
    assert!(
        ctx.sent
            .iter()
            .all(|message| message["method"] != json!("Target.attachedToTarget")),
        "page session autoAttach must not attach existing top-level page targets: {:?}",
        ctx.sent
    );
    assert!(
        ctx.conn
            .auto_attached_sessions_for_owner(Some(&new_page_session_id))
            .is_empty(),
        "new page session autoAttach owner must not own other top-level page sessions"
    );
}

async fn wait_for_playwright_child_frame_navigation(
    ctx: &mut TestContext,
    session_id: &str,
    child_frame_id: &str,
    expected_url: &str,
) {
    crate::testing::wait_until_message(
        ctx,
        Some(session_id),
        "child frame navigated before playwright-style body evaluation",
        |message| {
            message["method"] == json!("Page.frameNavigated")
                && message["params"]["frame"]["id"] == json!(child_frame_id)
                && message["params"]["frame"]["url"] == json!(expected_url)
        },
    )
    .await;
}

fn latest_playwright_child_default_context_id(
    ctx: &TestContext,
    session_id: &str,
    child_frame_id: &str,
    expected_url: &str,
) -> i64 {
    ctx.sent
        .iter()
        .rev()
        .find(|message| {
            message["sessionId"] == json!(session_id)
                && message["method"] == json!("Runtime.executionContextCreated")
                && message["params"]["context"]["auxData"]["isDefault"] == json!(true)
                && message["params"]["context"]["auxData"]["frameId"] == json!(child_frame_id)
        })
        .and_then(|message| message["params"]["context"]["id"].as_i64())
        .unwrap_or_else(|| {
            panic!(
                "expected child default Runtime.executionContextCreated for frame {child_frame_id} before evaluating {expected_url}; sent={:?}",
                ctx.sent
            )
        })
}

#[tokio::test(flavor = "multi_thread")]
async fn playwright_over_cdp_navigation_applies_extra_headers_and_referrer() {
    async fn handler(
        headers: HeaderMap,
        axum::extract::State(seen): axum::extract::State<
            Arc<Mutex<(Option<String>, Option<String>)>>,
        >,
    ) -> impl IntoResponse {
        let referer = headers
            .get(axum::http::header::REFERER)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let extra = headers
            .get("x-cdp-test")
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        *seen.lock() = (referer, extra);
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>ok</body></html>",
        )
    }

    let seen = Arc::new(Mutex::new((None, None)));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server_seen = Arc::clone(&seen);
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/page", get(handler))
                .with_state(server_seen),
        )
        .await
        .unwrap();
    });

    let mut ctx = TestContext::new();
    let session_id = create_attached_page_session_async(&mut ctx, 220, 221, 222, 2396, 2397)
        .await
        .session_id;

    ctx.process_async(json!({
        "id": 223,
        "method": "Network.setExtraHTTPHeaders",
        "sessionId": session_id,
        "params": {
            "headers": {
                "x-cdp-test": "works"
            }
        }
    }))
    .await;
    ctx.expect_result(223, json!({}), Some(&session_id));

    let url = format!("http://{addr}/page");
    ctx.process_async(json!({
        "id": 224,
        "method": "Page.navigate",
        "sessionId": session_id,
        "params": {
            "url": url,
            "referrer": "https://www.google.com/"
        }
    }))
    .await;
    let response = take_response_by_id(&mut ctx, 224);
    assert_eq!(response["sessionId"], json!(session_id));
    assert!(
        ctx.sent
            .iter()
            .all(|message| message.get("error").is_none()),
        "unexpected protocol error during navigation: {:?}",
        ctx.sent
    );

    let (referer, extra) = seen.lock().clone();
    assert_eq!(referer.as_deref(), Some("https://www.google.com/"));
    assert_eq!(extra.as_deref(), Some("works"));

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn playwright_over_cdp_child_frame_playwright_style_utility_script_uses_child_scope() {
    async fn parent() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body><iframe src=\"/child\"></iframe></body></html>",
        )
    }

    async fn child() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>child-playwright-attached</body></html>",
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/parent", get(parent))
                .route("/child", get(child)),
        )
        .await
        .unwrap();
    });

    let mut ctx = TestContext::new();
    let attached = create_attached_page_session_async(&mut ctx, 5200, 5201, 5202, 5203, 5204).await;
    let session_id = attached.session_id.clone();
    ctx.enable_page_events_for_test(Some(session_id.as_str()));
    let url = format!("http://{addr}/parent");

    ctx.process_async(json!({
        "id": 5205,
        "method": "Page.navigate",
        "sessionId": session_id,
        "params": { "url": url }
    }))
    .await;
    let navigation = take_response_by_id(&mut ctx, 5205);
    assert_eq!(navigation["sessionId"], json!(session_id));

    crate::testing::wait_until_message(
        &mut ctx,
        Some(session_id.as_str()),
        "Playwright child frame attachment after Page.navigate response",
        |message| {
            message["sessionId"] == json!(session_id)
                && message["method"] == json!("Page.frameAttached")
                && message["params"]["parentFrameId"] == json!(attached.target_id)
        },
    )
    .await;
    let child_frame_id = ctx
        .sent
        .iter()
        .find(|message| {
            message["sessionId"] == json!(session_id)
                && message["method"] == json!("Page.frameAttached")
                && message["params"]["parentFrameId"] == json!(attached.target_id)
        })
        .and_then(|message| message["params"]["frameId"].as_str())
        .map(str::to_owned)
        .expect("child frame should emit Page.frameAttached");
    wait_for_playwright_child_frame_navigation(
        &mut ctx,
        session_id.as_str(),
        &child_frame_id,
        &format!("http://{addr}/child"),
    )
    .await;
    let child_default_context_id = latest_playwright_child_default_context_id(
        &ctx,
        session_id.as_str(),
        &child_frame_id,
        &format!("http://{addr}/child"),
    );

    ctx.process_async(json!({
        "id": 5206,
        "method": "Runtime.evaluate",
        "sessionId": session_id,
        "params": {
            "contextId": child_default_context_id,
            "expression": "(() => { const module = { exports: {} }; class UtilityScript { constructor(global, isUnderTest) { this.global = global; this.isUnderTest = isUnderTest; } evaluate(isFunction, returnByValue, expression, argCount, ...argsAndHandles) { const args = argsAndHandles.slice(0, argCount); let result = this.global.eval(expression); if (isFunction === true) { result = result(...args); } else if (isFunction === false) { result = result; } else if (typeof result === 'function') { result = result(...args); } return returnByValue ? result : result; } } module.exports.UtilityScript = () => UtilityScript; return new (module.exports.UtilityScript())(globalThis, false); })()"
        }
    }))
    .await;
    let utility_response = take_response_by_id(&mut ctx, 5206);
    let object_id = utility_response["result"]["result"]["objectId"]
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| panic!("playwright-style utility object id: {utility_response:?}"));

    ctx.process_async(json!({
        "id": 5207,
        "method": "Runtime.callFunctionOn",
        "sessionId": session_id,
        "params": {
            "objectId": object_id.clone(),
            "functionDeclaration": "(utilityScript, ...args) => utilityScript.evaluate(...args)",
            "arguments": [
                { "objectId": object_id },
                {},
                { "value": true },
                { "value": "document.body.textContent.trim()" },
                { "value": 1 },
                { "value": { "v": "null" } }
            ],
            "returnByValue": true,
            "awaitPromise": true,
            "userGesture": true
        }
    }))
    .await;
    let result = take_response_by_id(&mut ctx, 5207);
    assert_eq!(
        result["result"]["result"]["value"],
        json!("child-playwright-attached")
    );

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn playwright_connect_over_cdp_auto_attach_child_frame_utility_script_uses_child_scope() {
    super::tests_patchright::patchright_8mb_stack(
        "playwright-child-frame-utility-scope",
        || async {
    async fn parent() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            concat!(
                "<!doctype html><html><body>",
                "<script>",
                "for (let i = 0; i < 13; i++) ",
                "document.body.appendChild(document.createElement('iframe'));",
                "</script>",
                "<iframe src=\"/child\"></iframe>",
                "</body></html>",
            ),
        )
    }

    async fn warmup() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><head><title>warmup</title></head><body>warmup-document</body></html>",
        )
    }

    async fn child() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>child-playwright-auto-attach</body></html>",
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/warmup", get(warmup))
                .route("/parent", get(parent))
                .route("/child", get(child)),
        )
        .await
        .unwrap();
    });

    let mut ctx = TestContext::new();

    ctx.process_async(json!({
        "id": 5300,
        "method": "Target.setAutoAttach",
        "params": {
            "autoAttach": true,
            "waitForDebuggerOnStart": true,
            "flatten": true
        }
    }))
    .await;
    ctx.expect_result(5300, json!({}), None);

    ctx.process_async(json!({
        "id": 5301,
        "method": "Browser.setDownloadBehavior",
        "params": {
            "behavior": "allowAndName",
            "downloadPath": "/tmp/moli-playwright-artifacts",
            "eventsEnabled": true
        }
    }))
    .await;
    ctx.expect_result(5301, json!({}), None);

    ctx.process_async(json!({
        "id": 5302,
        "method": "Target.getTargetInfo"
    }))
    .await;
    let browser_target_info = take_response_by_id(&mut ctx, 5302);
    assert_eq!(
        browser_target_info["result"]["targetInfo"]["type"],
        json!("browser")
    );

    ctx.process_async(json!({
        "id": 5303,
        "method": "Target.createTarget",
        "params": {
            "url": "about:blank"
        }
    }))
    .await;
    let target_created = ctx
        .sent
        .iter()
        .find(|message| message["method"] == json!("Target.targetCreated"))
        .cloned()
        .expect("Target.createTarget should emit Target.targetCreated");
    let attached = ctx
        .sent
        .iter()
        .find(|message| message["method"] == json!("Target.attachedToTarget"))
        .cloned()
        .expect("Target.createTarget under auto-attach should emit Target.attachedToTarget");
    let session_id = attached["params"]["sessionId"]
        .as_str()
        .map(str::to_owned)
        .expect("attached session id");
    let target_id = target_created["params"]["targetInfo"]["targetId"]
        .as_str()
        .map(str::to_owned)
        .expect("created target id");
    let create_target = take_response_by_id(&mut ctx, 5303);
    assert_eq!(create_target["result"]["targetId"], json!(target_id));

    ctx.process_async(json!({
        "id": 5304,
        "method": "Page.enable",
        "sessionId": session_id
    }))
    .await;
    ctx.expect_result(5304, json!({}), Some(&session_id));

    ctx.process_async(json!({
        "id": 5305,
        "method": "Page.getFrameTree",
        "sessionId": session_id
    }))
    .await;
    let frame_tree = take_response_by_id(&mut ctx, 5305);
    assert_eq!(
        frame_tree["result"]["frameTree"]["frame"]["id"],
        json!(target_id)
    );

    ctx.process_async(json!({
        "id": 5306,
        "method": "Log.enable",
        "sessionId": session_id
    }))
    .await;
    ctx.expect_result(5306, json!({}), Some(&session_id));

    ctx.process_async(json!({
        "id": 5307,
        "method": "Page.setLifecycleEventsEnabled",
        "sessionId": session_id,
        "params": { "enabled": true }
    }))
    .await;
    ctx.expect_result(5307, json!({}), Some(&session_id));

    ctx.process_async(json!({
        "id": 5308,
        "method": "Runtime.enable",
        "sessionId": session_id
    }))
    .await;
    ctx.expect_result(5308, json!({}), Some(&session_id));

    ctx.process_async(json!({
        "id": 5309,
        "method": "Page.addScriptToEvaluateOnNewDocument",
        "sessionId": session_id,
        "params": {
            "source": "",
            "worldName": "__playwright_utility_world_page@lm"
        }
    }))
    .await;
    let add_script = take_response_by_id(&mut ctx, 5309);
    assert!(add_script["result"]["identifier"].as_str().is_some());

    ctx.process_async(json!({
        "id": 53_091,
        "method": "Page.createIsolatedWorld",
        "sessionId": session_id,
        "params": {
            "frameId": target_id,
            "worldName": "__playwright_utility_world_page@lm",
            "grantUniveralAccess": true
        }
    }))
    .await;
    let isolated_world = take_response_by_id(&mut ctx, 53_091);
    assert!(
        isolated_world["result"]["executionContextId"]
            .as_i64()
            .is_some(),
        "Page.createIsolatedWorld should return an executionContextId: {:?}",
        isolated_world
    );

    ctx.process_async(json!({
        "id": 5310,
        "method": "Network.enable",
        "sessionId": session_id
    }))
    .await;
    ctx.expect_result(5310, json!({}), Some(&session_id));

    ctx.process_async(json!({
        "id": 5311,
        "method": "Target.setAutoAttach",
        "sessionId": session_id,
        "params": {
            "autoAttach": true,
            "waitForDebuggerOnStart": true,
            "flatten": true
        }
    }))
    .await;
    ctx.expect_result(5311, json!({}), Some(&session_id));

    ctx.process_async(json!({
        "id": 5312,
        "method": "Emulation.setFocusEmulationEnabled",
        "sessionId": session_id,
        "params": { "enabled": true }
    }))
    .await;
    ctx.expect_result(5312, json!({}), Some(&session_id));

    ctx.process_async(json!({
        "id": 5313,
        "method": "Emulation.setEmulatedMedia",
        "sessionId": session_id,
        "params": {
            "media": "",
            "features": [
                { "name": "prefers-color-scheme", "value": "light" },
                { "name": "prefers-reduced-motion", "value": "no-preference" },
                { "name": "forced-colors", "value": "none" },
                { "name": "prefers-contrast", "value": "no-preference" }
            ]
        }
    }))
    .await;
    ctx.expect_result(5313, json!({}), Some(&session_id));

    ctx.process_async(json!({
        "id": 5314,
        "method": "Runtime.runIfWaitingForDebugger",
        "sessionId": session_id
    }))
    .await;
    ctx.expect_result(5314, json!({}), Some(&session_id));

    let warmup_url = format!("http://{addr}/warmup");
    ctx.process_async(json!({
        "id": 53_141,
        "method": "Page.navigate",
        "sessionId": session_id,
        "params": { "url": warmup_url }
    }))
    .await;
    let warmup_navigation = take_response_by_id(&mut ctx, 53_141);
    assert_eq!(warmup_navigation["sessionId"], json!(session_id));
    assert!(
        ctx.sent
            .iter()
            .all(|message| message.get("error").is_none()),
        "unexpected protocol error during warmup navigation: {:?}",
        ctx.sent
    );
    let warmup_utility_context_id = ctx
        .sent
        .iter()
        .rev()
        .find(|message| {
            message["sessionId"] == json!(session_id)
                && message["method"] == json!("Runtime.executionContextCreated")
                && message["params"]["context"]["auxData"]["isDefault"] == json!(false)
                && message["params"]["context"]["auxData"]["frameId"] == json!(target_id)
                && message["params"]["context"]["name"]
                    == json!("__playwright_utility_world_page@lm")
        })
        .and_then(|message| message["params"]["context"]["id"].as_i64())
        .expect("warmup navigation should publish its Playwright utility context");
    ctx.process_async(json!({
        "id": 53_142,
        "method": "Runtime.evaluate",
        "sessionId": session_id,
        "params": {
            "contextId": warmup_utility_context_id,
            "expression": "({ title: document.title })"
        }
    }))
    .await;
    let warmup_utility_evaluate = take_response_by_id(&mut ctx, 53_142);
    let warmup_utility_object_id = warmup_utility_evaluate["result"]["result"]["objectId"]
        .as_str()
        .unwrap_or_else(|| {
            panic!(
                "warmup utility evaluation should retain a remote object: {warmup_utility_evaluate:?}"
            )
        })
        .to_owned();

    let replacement_output_start = ctx.sent.len();
    let url = format!("http://{addr}/parent");
    ctx.process_async(json!({
        "id": 5315,
        "method": "Page.navigate",
        "sessionId": session_id,
        "params": { "url": url }
    }))
    .await;
    let navigation = take_response_by_id(&mut ctx, 5315);
    assert_eq!(navigation["sessionId"], json!(session_id));
    assert!(
        ctx.sent
            .iter()
            .all(|message| message.get("error").is_none()),
        "unexpected protocol error during connect-over-cdp child navigation: {:?}",
        ctx.sent
    );
    crate::testing::wait_until_messages(
        &mut ctx,
        Some(session_id.as_str()),
        "replacement document DOMContentLoaded",
        |messages| {
            messages.iter().any(|message| {
                message["method"] == json!("Page.lifecycleEvent")
                    && message["params"]["frameId"] == json!(target_id)
                    && message["params"]["name"] == json!("DOMContentLoaded")
            })
        },
    )
    .await;

    let replacement_root_utility_context_ids = ctx
        .sent
        .iter()
        .skip(replacement_output_start)
        .filter(|message| {
            message["sessionId"] == json!(session_id)
                && message["method"] == json!("Runtime.executionContextCreated")
                && message["params"]["context"]["auxData"]["isDefault"] == json!(false)
                && message["params"]["context"]["auxData"]["frameId"] == json!(target_id)
                && message["params"]["context"]["name"]
                    == json!("__playwright_utility_world_page@lm")
        })
        .filter_map(|message| message["params"]["context"]["id"].as_i64())
        .collect::<Vec<_>>();
    let root_utility_context_id = replacement_root_utility_context_ids
        .first()
        .copied()
        .unwrap_or_else(|| {
            panic!(
                "navigation should publish the root Playwright utility context: {:?}",
                ctx.sent
            )
        });
    assert_eq!(
        replacement_root_utility_context_ids.len(),
        1,
        "one committed document must publish exactly one root Playwright utility context: {replacement_root_utility_context_ids:?}"
    );
    let root_utility_created_before_evaluate = ctx
        .sent
        .iter()
        .filter(|message| {
            message["sessionId"] == json!(session_id)
                && message["method"] == json!("Runtime.executionContextCreated")
                && message["params"]["context"]["auxData"]["isDefault"] == json!(false)
                && message["params"]["context"]["auxData"]["frameId"] == json!(target_id)
                && message["params"]["context"]["name"]
                    == json!("__playwright_utility_world_page@lm")
        })
        .count();

    ctx.process_async(json!({
        "id": 53_151,
        "method": "Runtime.evaluate",
        "sessionId": session_id,
        "params": {
            "contextId": root_utility_context_id,
            "expression": "document.documentElement.outerHTML",
            "returnByValue": true
        }
    }))
    .await;
    let root_utility_evaluate = take_response_by_id(&mut ctx, 53_151);
    assert!(
        root_utility_evaluate.get("error").is_none(),
        "the published root utility context must be immediately usable: {root_utility_evaluate:?}"
    );
    assert!(
        root_utility_evaluate["result"]["result"]["value"]
            .as_str()
            .is_some_and(|html| html.contains("<iframe src=\"/child\"></iframe>")),
        "root utility evaluation should observe the committed document: {root_utility_evaluate:?}"
    );
    let root_utility_created_after_evaluate = ctx
        .sent
        .iter()
        .filter(|message| {
            message["sessionId"] == json!(session_id)
                && message["method"] == json!("Runtime.executionContextCreated")
                && message["params"]["context"]["auxData"]["isDefault"] == json!(false)
                && message["params"]["context"]["auxData"]["frameId"] == json!(target_id)
                && message["params"]["context"]["name"]
                    == json!("__playwright_utility_world_page@lm")
        })
        .count();
    assert_eq!(
        root_utility_created_after_evaluate,
        root_utility_created_before_evaluate,
        "the first command must not repair a previously published context by emitting a replacement"
    );

    ctx.process_async(json!({
        "id": 53_152,
        "method": "Runtime.callFunctionOn",
        "sessionId": session_id,
        "params": {
            "objectId": warmup_utility_object_id,
            "functionDeclaration": "function() { globalThis.__lm_stale_warmup_object_ran = true; return this.title; }",
            "returnByValue": true
        }
    }))
    .await;
    let stale_warmup_object_call = take_response_by_id(&mut ctx, 53_152);
    assert_eq!(
        stale_warmup_object_call["error"]["code"],
        json!(-32000),
        "the previous document's remote object must fail closed: {stale_warmup_object_call:?}"
    );
    assert_eq!(
        stale_warmup_object_call["error"]["message"],
        json!("Cannot find context with specified id")
    );

    ctx.process_async(json!({
        "id": 53_153,
        "method": "Runtime.evaluate",
        "sessionId": session_id,
        "params": {
            "contextId": root_utility_context_id,
            "expression": "globalThis.__lm_stale_warmup_object_ran === true",
            "returnByValue": true
        }
    }))
    .await;
    let replacement_after_stale_object = take_response_by_id(&mut ctx, 53_153);
    assert_eq!(
        replacement_after_stale_object["result"]["result"]["value"],
        json!(false),
        "stale remote object dispatch must not mutate the replacement utility realm"
    );

    let child_url = format!("http://{addr}/child");
    crate::testing::wait_until_messages(
        &mut ctx,
        Some(session_id.as_str()),
        "network child-frame Document request",
        |messages| {
            messages.iter().any(|message| {
                message["sessionId"] == json!(session_id)
                    && message["method"] == json!("Network.requestWillBeSent")
                    && message["params"]["type"] == json!("Document")
                    && message["params"]["request"]["url"] == json!(child_url)
            })
        },
    )
    .await;
    let child_frame_id = ctx
        .sent
        .iter()
        .find(|message| {
            message["sessionId"] == json!(session_id)
                && message["method"] == json!("Network.requestWillBeSent")
                && message["params"]["type"] == json!("Document")
                && message["params"]["request"]["url"] == json!(child_url)
        })
        .and_then(|message| message["params"]["frameId"].as_str())
        .map(str::to_owned)
        .expect("network child frame should emit a Document request");
    wait_for_playwright_child_frame_navigation(
        &mut ctx,
        session_id.as_str(),
        &child_frame_id,
        &child_url,
    )
    .await;
    let child_default_context_id = latest_playwright_child_default_context_id(
        &ctx,
        session_id.as_str(),
        &child_frame_id,
        &child_url,
    );

    ctx.process_async(json!({
        "id": 5316,
        "method": "Runtime.evaluate",
        "sessionId": session_id,
        "params": {
            "contextId": child_default_context_id,
            "expression": "(() => { const module = { exports: {} }; class UtilityScript { constructor(global, isUnderTest) { this.global = global; this.isUnderTest = isUnderTest; } evaluate(isFunction, returnByValue, expression, argCount, ...argsAndHandles) { const args = argsAndHandles.slice(0, argCount); let result = this.global.eval(expression); if (isFunction === true) { result = result(...args); } else if (isFunction === false) { result = result; } else if (typeof result === 'function') { result = result(...args); } return returnByValue ? result : result; } } module.exports.UtilityScript = () => UtilityScript; return new (module.exports.UtilityScript())(globalThis, false); })()"
        }
    }))
    .await;
    let utility_response = take_response_by_id(&mut ctx, 5316);
    let object_id = utility_response["result"]["result"]["objectId"]
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| {
            panic!("playwright-style utility object id in auto-attach flow: {utility_response:?}")
        });

    ctx.process_async(json!({
        "id": 5317,
        "method": "Runtime.callFunctionOn",
        "sessionId": session_id,
        "params": {
            "objectId": object_id.clone(),
            "functionDeclaration": "(utilityScript, ...args) => utilityScript.evaluate(...args)",
            "arguments": [
                { "objectId": object_id },
                {},
                { "value": true },
                { "value": "document.body.textContent.trim()" },
                { "value": 1 },
                { "value": { "v": "null" } }
            ],
            "returnByValue": true,
            "awaitPromise": true,
            "userGesture": true
        }
    }))
    .await;
    let result = take_response_by_id(&mut ctx, 5317);
    assert_eq!(
        result["result"]["result"]["value"],
        json!("child-playwright-auto-attach"),
        "unexpected child evaluation payload; sent={:?}",
        ctx.sent
    );

    server.abort();
        },
    )
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn playwright_connect_over_cdp_second_page_window_query_keeps_background_target_parked() {
    let mut ctx = TestContext::new();
    load_bc_with_titled_page_async(
        &mut ctx,
        "BID-PW-WINDOW",
        "TID-PW-FIRST",
        "<title>first</title><div id='first'>first page</div>",
    )
    .await;
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .attach_active_session("SID-first");

    ctx.process_async(json!({
        "id": 53191,
        "method": "Target.setAutoAttach",
        "params": {
            "autoAttach": true,
            "waitForDebuggerOnStart": true,
            "flatten": true
        }
    }))
    .await;
    ctx.expect_result(53191, json!({}), None);

    ctx.process_async(json!({
        "id": 53192,
        "method": "Target.createTarget",
        "params": {
            "background": true,
            "browserContextId": "BID-PW-WINDOW",
            "url": "about:blank"
        }
    }))
    .await;
    let created = ctx.take_one();
    assert_eq!(created["method"], "Target.targetCreated");
    let second_target_id = created["params"]["targetInfo"]["targetId"]
        .as_str()
        .expect("second target id")
        .to_owned();
    let attached = ctx.take_one();
    assert_eq!(attached["method"], "Target.attachedToTarget");
    let second_session_id = attached["params"]["sessionId"]
        .as_str()
        .expect("second target session id")
        .to_owned();
    ctx.expect_result(53192, json!({ "targetId": second_target_id }), None);

    ctx.process_async(json!({
        "id": 53193,
        "method": "Browser.getWindowForTarget",
        "sessionId": second_session_id
    }))
    .await;
    let response = take_response_by_id(&mut ctx, 53193);
    assert!(
        response["result"]["windowId"].as_u64().is_some(),
        "Browser.getWindowForTarget should succeed for the second auto-attached page: {:?}",
        response
    );
    assert_eq!(response["result"]["bounds"]["windowState"], json!("normal"));

    let bc = ctx
        .conn
        .browser_context
        .as_ref()
        .expect("active browser context");
    assert_eq!(bc.active_target_id(), Some("TID-PW-FIRST"));
    assert_eq!(bc.active_session_id(), Some("SID-first"));
    assert!(
        bc.background_target(&second_target_id).is_some(),
        "second page should remain parked after Browser.getWindowForTarget"
    );
    assert!(
        bc.background_target("TID-PW-FIRST").is_none(),
        "first page should remain active after second-page Browser.getWindowForTarget"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn playwright_over_cdp_child_frame_is_visible_in_frame_tree_after_frame_attachment() {
    async fn parent() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body><iframe src=\"/child\"></iframe></body></html>",
        )
    }

    async fn child() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>child-frame-tree-visible</body></html>",
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/parent", get(parent))
                .route("/child", get(child)),
        )
        .await
        .unwrap();
    });

    let mut ctx = TestContext::new();
    let attached = create_attached_page_session_async(&mut ctx, 5318, 5319, 5320, 5321, 5322).await;
    let session_id = attached.session_id.clone();
    let target_id = attached.target_id.clone();
    ctx.enable_page_events_for_test(Some(session_id.as_str()));
    let url = format!("http://{addr}/parent");

    ctx.process_async(json!({
        "id": 5323,
        "method": "Page.navigate",
        "sessionId": session_id,
        "params": { "url": url }
    }))
    .await;
    let navigation = take_response_by_id(&mut ctx, 5323);
    assert_eq!(navigation["sessionId"], json!(attached.session_id));

    crate::testing::wait_until_message(
        &mut ctx,
        Some(session_id.as_str()),
        "Playwright child frame attachment after Page.navigate response",
        |message| {
            message["sessionId"] == json!(session_id)
                && message["method"] == json!("Page.frameAttached")
                && message["params"]["parentFrameId"] == json!(target_id)
        },
    )
    .await;
    let child_frame_id = ctx
        .sent
        .iter()
        .find(|message| {
            message["sessionId"] == json!(attached.session_id)
                && message["method"] == json!("Page.frameAttached")
                && message["params"]["parentFrameId"] == json!(target_id)
        })
        .and_then(|message| message["params"]["frameId"].as_str())
        .map(str::to_owned)
        .expect("child frame should emit Page.frameAttached before frame tree query");
    let child_attached_index = ctx
        .sent
        .iter()
        .position(|message| {
            message["sessionId"] == json!(attached.session_id)
                && message["method"] == json!("Page.frameAttached")
                && message["params"]["frameId"] == json!(child_frame_id)
        })
        .expect("child frame attached index");
    crate::testing::wait_until_message(
        &mut ctx,
        Some(session_id.as_str()),
        "main load after child frame attachment",
        |message| {
            message["sessionId"] == json!(session_id)
                && message["method"] == json!("Page.loadEventFired")
        },
    )
    .await;
    let main_load_event_index = ctx
        .sent
        .iter()
        .position(|message| {
            message["sessionId"] == json!(attached.session_id)
                && message["method"] == json!("Page.loadEventFired")
        })
        .expect("main load event index");
    let child_url = format!("http://{addr}/child");
    let child_navigated_index = ctx
        .sent
        .iter()
        .position(|message| {
            message["sessionId"] == json!(attached.session_id)
                && message["method"] == json!("Page.frameNavigated")
                && message["params"]["frame"]["id"] == json!(child_frame_id)
                && message["params"]["frame"]["url"] == json!(child_url.as_str())
        })
        .unwrap_or_else(|| {
            panic!(
                "child final Page.frameNavigated before main load: {:?}",
                ctx.sent
            )
        });
    assert!(
        child_attached_index < main_load_event_index,
        "child Page.frameAttached should be emitted before main Page.loadEventFired; sent={:?}",
        ctx.sent
    );
    assert!(
        child_navigated_index < main_load_event_index,
        "child final Page.frameNavigated should be emitted before main Page.loadEventFired; sent={:?}",
        ctx.sent
    );

    ctx.process_async(json!({
        "id": 5324,
        "method": "Page.getFrameTree",
        "sessionId": attached.session_id
    }))
    .await;
    let frame_tree = take_response_by_id(&mut ctx, 5324);
    let child_frames = frame_tree["result"]["frameTree"]["childFrames"]
        .as_array()
        .expect("frame tree childFrames array");
    assert_eq!(
        child_frames.len(),
        1,
        "child frame should already be visible in frame tree immediately after Page.navigate response"
    );
    assert_eq!(child_frames[0]["frame"]["id"], json!(child_frame_id));
    assert_eq!(child_frames[0]["frame"]["url"], json!(child_url.as_str()));

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn playwright_over_cdp_navigation_redirect_exposes_final_response_and_body() {
    async fn redirect_handler(
        axum::extract::State(final_url): axum::extract::State<String>,
    ) -> impl IntoResponse {
        Redirect::temporary(&final_url)
    }

    async fn final_handler() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body><main>redirected</main></body></html>",
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let start_url = format!("http://{addr}/start");
    let final_url = format!("http://{addr}/final");
    let server_final_url = final_url.clone();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/start", get(redirect_handler))
                .route("/final", get(final_handler))
                .with_state(server_final_url),
        )
        .await
        .unwrap();
    });

    let mut ctx = TestContext::new();
    let session_id = create_attached_page_session_async(&mut ctx, 2240, 2241, 2242, 2243, 2398)
        .await
        .session_id;

    ctx.process_async(json!({
        "id": 2244,
        "method": "Page.navigate",
        "sessionId": session_id,
        "params": { "url": start_url }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 2244);
    assert!(
        ctx.sent
            .iter()
            .all(|message| message.get("error").is_none()),
        "unexpected protocol error during redirect navigation: {:?}",
        ctx.sent
    );
    let expected_request_id = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Network.requestWillBeSent")
                && message["params"]["request"]["url"] == json!(start_url)
        })
        .and_then(|message| message["params"]["requestId"].as_str())
        .expect("redirect start request id")
        .to_owned();
    crate::testing::wait_until_messages(
        &mut ctx,
        Some(session_id.as_str()),
        "redirected main-document network completion",
        |messages| {
            messages.iter().any(|message| {
                message["method"] == json!("Network.loadingFinished")
                    && message["params"]["requestId"] == json!(expected_request_id)
            })
        },
    )
    .await;

    let emitted = ctx.take_all();
    let first_request = emitted
        .iter()
        .find(|message| {
            message["method"] == json!("Network.requestWillBeSent")
                && message["params"]["request"]["url"] == json!(start_url)
        })
        .expect("redirect start request should be emitted");
    let request_id = first_request["params"]["requestId"]
        .as_str()
        .expect("request id")
        .to_owned();
    assert_eq!(request_id, expected_request_id);

    let redirected_request = emitted
        .iter()
        .find(|message| {
            message["method"] == json!("Network.requestWillBeSent")
                && message["params"]["requestId"] == json!(request_id)
                && message["params"]["request"]["url"] == json!(final_url)
        })
        .expect("redirect target request should be emitted");
    assert_eq!(
        redirected_request["params"]["redirectResponse"]["url"],
        json!(start_url)
    );
    assert_eq!(
        redirected_request["params"]["redirectResponse"]["status"],
        json!(307)
    );

    let final_response = emitted
        .iter()
        .find(|message| {
            message["method"] == json!("Network.responseReceived")
                && message["params"]["requestId"] == json!(request_id)
        })
        .expect("final response should be emitted");
    assert_eq!(
        final_response["params"]["response"]["url"],
        json!(final_url)
    );
    assert_eq!(final_response["params"]["response"]["status"], json!(200));

    let frame_navigated = emitted
        .iter()
        .find(|message| message["method"] == json!("Page.frameNavigated"))
        .expect("frameNavigated should be emitted for committed redirect document");
    assert_eq!(
        frame_navigated["params"]["frame"]["url"],
        json!(final_url),
        "Playwright updates page.url() from Page.frameNavigated, so redirects must commit the final URL"
    );

    assert!(emitted.iter().any(|message| {
        message["method"] == json!("Network.loadingFinished")
            && message["params"]["requestId"] == json!(request_id)
    }));

    ctx.process_async(json!({
        "id": 2245,
        "method": "Network.getResponseBody",
        "sessionId": session_id,
        "params": { "requestId": request_id }
    }))
    .await;
    ctx.expect_result(
        2245,
        json!({
            "body": "<!doctype html><html><body><main>redirected</main></body></html>",
            "base64Encoded": false
        }),
        Some(&session_id),
    );

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn playwright_over_cdp_navigation_multi_hop_redirect_preserves_redirect_chain() {
    async fn start_handler(
        axum::extract::State((middle_url, _final_url)): axum::extract::State<(String, String)>,
    ) -> impl IntoResponse {
        Redirect::temporary(&middle_url)
    }

    async fn middle_handler(
        axum::extract::State((_middle_url, final_url)): axum::extract::State<(String, String)>,
    ) -> impl IntoResponse {
        Redirect::temporary(&final_url)
    }

    async fn final_handler() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body><main>redirect chain</main></body></html>",
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let start_url = format!("http://{addr}/start");
    let middle_url = format!("http://{addr}/middle");
    let final_url = format!("http://{addr}/final");
    let server_state = (middle_url.clone(), final_url.clone());
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/start", get(start_handler))
                .route("/middle", get(middle_handler))
                .route("/final", get(final_handler))
                .with_state(server_state),
        )
        .await
        .unwrap();
    });

    let mut ctx = TestContext::new();
    let session_id = create_attached_page_session_async(&mut ctx, 2246, 2247, 2248, 2249, 2399)
        .await
        .session_id;

    ctx.process_async(json!({
        "id": 2250,
        "method": "Page.navigate",
        "sessionId": session_id,
        "params": { "url": start_url }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 2250);
    assert!(
        ctx.sent
            .iter()
            .all(|message| message.get("error").is_none()),
        "unexpected protocol error during multi-hop redirect navigation: {:?}",
        ctx.sent
    );
    let expected_request_id = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Network.requestWillBeSent")
                && message["params"]["request"]["url"] == json!(start_url)
        })
        .and_then(|message| message["params"]["requestId"].as_str())
        .expect("multi-hop redirect start request id")
        .to_owned();
    crate::testing::wait_until_messages(
        &mut ctx,
        Some(session_id.as_str()),
        "multi-hop redirect main-document network completion",
        |messages| {
            messages.iter().any(|message| {
                message["method"] == json!("Network.loadingFinished")
                    && message["params"]["requestId"] == json!(expected_request_id)
            })
        },
    )
    .await;

    let emitted = ctx.take_all();
    let first_request = emitted
        .iter()
        .find(|message| {
            message["method"] == json!("Network.requestWillBeSent")
                && message["params"]["request"]["url"] == json!(start_url)
        })
        .expect("redirect start request should be emitted");
    let request_id = first_request["params"]["requestId"]
        .as_str()
        .expect("request id")
        .to_owned();
    assert_eq!(request_id, expected_request_id);

    let middle_request = emitted
        .iter()
        .find(|message| {
            message["method"] == json!("Network.requestWillBeSent")
                && message["params"]["requestId"] == json!(request_id)
                && message["params"]["request"]["url"] == json!(middle_url)
        })
        .expect("middle redirect request should be emitted");
    assert_eq!(
        middle_request["params"]["redirectResponse"]["url"],
        json!(start_url)
    );
    assert_eq!(
        middle_request["params"]["redirectResponse"]["status"],
        json!(307)
    );
    assert_eq!(
        middle_request["params"]["redirectResponse"]["headers"]["location"],
        json!(middle_url)
    );

    let final_request = emitted
        .iter()
        .find(|message| {
            message["method"] == json!("Network.requestWillBeSent")
                && message["params"]["requestId"] == json!(request_id)
                && message["params"]["request"]["url"] == json!(final_url)
        })
        .expect("final redirect request should be emitted");
    assert_eq!(
        final_request["params"]["redirectResponse"]["url"],
        json!(middle_url)
    );
    assert_eq!(
        final_request["params"]["redirectResponse"]["status"],
        json!(307)
    );
    assert_eq!(
        final_request["params"]["redirectResponse"]["headers"]["location"],
        json!(final_url)
    );

    let final_response = emitted
        .iter()
        .find(|message| {
            message["method"] == json!("Network.responseReceived")
                && message["params"]["requestId"] == json!(request_id)
        })
        .expect("final response should be emitted");
    assert_eq!(
        final_response["params"]["response"]["url"],
        json!(final_url)
    );
    assert_eq!(final_response["params"]["response"]["status"], json!(200));

    assert!(emitted.iter().any(|message| {
        message["method"] == json!("Network.loadingFinished")
            && message["params"]["requestId"] == json!(request_id)
    }));

    ctx.process_async(json!({
        "id": 2251,
        "method": "Network.getResponseBody",
        "sessionId": session_id,
        "params": { "requestId": request_id }
    }))
    .await;
    ctx.expect_result(
        2251,
        json!({
            "body": "<!doctype html><html><body><main>redirect chain</main></body></html>",
            "base64Encoded": false
        }),
        Some(&session_id),
    );

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn playwright_over_cdp_navigation_multi_hop_redirect_preserves_history_headers() {
    let seen = Arc::new(Mutex::new(
        Vec::<(String, Option<String>, Option<String>)>::new(),
    ));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let start_url = format!("http://{addr}/start");
    let middle_url = format!("http://{addr}/middle");
    let final_url = format!("http://{addr}/final");

    let seen_start = Arc::clone(&seen);
    let seen_middle = Arc::clone(&seen);
    let seen_final = Arc::clone(&seen);
    let middle_url_for_start = middle_url.clone();
    let final_url_for_middle = final_url.clone();
    let server = tokio::spawn(async move {
        axum::serve(
                listener,
                Router::new()
                    .route(
                        "/start",
                        get(move |headers: HeaderMap| {
                            let seen = Arc::clone(&seen_start);
                            let middle_url = middle_url_for_start.clone();
                            async move {
                                let referer = headers
                                    .get(axum::http::header::REFERER)
                                    .and_then(|value| value.to_str().ok())
                                    .map(str::to_owned);
                                let extra = headers
                                    .get("x-cdp-test")
                                    .and_then(|value| value.to_str().ok())
                                    .map(str::to_owned);
                                seen.lock()
                                    .push(("/start".to_owned(), referer, extra));
                                Redirect::temporary(&middle_url)
                            }
                        }),
                    )
                    .route(
                        "/middle",
                        get(move |headers: HeaderMap| {
                            let seen = Arc::clone(&seen_middle);
                            let final_url = final_url_for_middle.clone();
                            async move {
                                let referer = headers
                                    .get(axum::http::header::REFERER)
                                    .and_then(|value| value.to_str().ok())
                                    .map(str::to_owned);
                                let extra = headers
                                    .get("x-cdp-test")
                                    .and_then(|value| value.to_str().ok())
                                    .map(str::to_owned);
                                seen.lock()
                                    .push(("/middle".to_owned(), referer, extra));
                                Redirect::temporary(&final_url)
                            }
                        }),
                    )
                    .route(
                        "/final",
                        get(move |headers: HeaderMap| {
                            let seen = Arc::clone(&seen_final);
                            async move {
                                let referer = headers
                                    .get(axum::http::header::REFERER)
                                    .and_then(|value| value.to_str().ok())
                                    .map(str::to_owned);
                                let extra = headers
                                    .get("x-cdp-test")
                                    .and_then(|value| value.to_str().ok())
                                    .map(str::to_owned);
                                seen.lock()
                                    .push(("/final".to_owned(), referer, extra));
                                (
                                    [(CONTENT_TYPE.as_str(), "text/html")],
                                    "<!doctype html><html><body><main>redirect history headers</main></body></html>",
                                )
                            }
                        }),
                    ),
            )
            .await
            .unwrap();
    });

    let mut ctx = TestContext::new();
    let session_id = create_attached_page_session_async(&mut ctx, 2272, 2273, 2274, 2275, 2400)
        .await
        .session_id;

    ctx.process_async(json!({
        "id": 2276,
        "method": "Network.setExtraHTTPHeaders",
        "sessionId": session_id,
        "params": {
            "headers": {
                "x-cdp-test": "history"
            }
        }
    }))
    .await;
    ctx.expect_result(2276, json!({}), Some(&session_id));

    ctx.process_async(json!({
        "id": 2277,
        "method": "Page.navigate",
        "sessionId": session_id,
        "params": {
            "url": start_url,
            "referrer": "https://www.google.com/"
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 2277);
    assert!(
        ctx.sent
            .iter()
            .all(|message| message.get("error").is_none()),
        "unexpected protocol error during redirect header navigation: {:?}",
        ctx.sent
    );

    let emitted = ctx.take_all();
    let first_request = emitted
        .iter()
        .find(|message| {
            message["method"] == json!("Network.requestWillBeSent")
                && message["params"]["request"]["url"] == json!(start_url)
        })
        .expect("redirect start request should be emitted");
    let request_id = first_request["params"]["requestId"]
        .as_str()
        .expect("request id")
        .to_owned();
    assert_eq!(
        first_request["params"]["request"]["headers"]["Referer"],
        json!("https://www.google.com/")
    );
    assert_eq!(
        first_request["params"]["request"]["headers"]["x-cdp-test"],
        json!("history")
    );

    let middle_request = emitted
        .iter()
        .find(|message| {
            message["method"] == json!("Network.requestWillBeSent")
                && message["params"]["requestId"] == json!(request_id)
                && message["params"]["request"]["url"] == json!(middle_url)
        })
        .expect("middle redirect request should be emitted");
    assert_eq!(
        middle_request["params"]["redirectResponse"]["headers"]["location"],
        json!(middle_url)
    );
    assert_eq!(
        middle_request["params"]["request"]["headers"]["Referer"],
        json!("https://www.google.com/")
    );
    assert_eq!(
        middle_request["params"]["request"]["headers"]["x-cdp-test"],
        json!("history")
    );

    let final_request = emitted
        .iter()
        .find(|message| {
            message["method"] == json!("Network.requestWillBeSent")
                && message["params"]["requestId"] == json!(request_id)
                && message["params"]["request"]["url"] == json!(final_url)
        })
        .expect("final redirect request should be emitted");
    assert_eq!(
        final_request["params"]["redirectResponse"]["headers"]["location"],
        json!(final_url)
    );
    assert_eq!(
        final_request["params"]["request"]["headers"]["Referer"],
        json!("https://www.google.com/")
    );
    assert_eq!(
        final_request["params"]["request"]["headers"]["x-cdp-test"],
        json!("history")
    );

    let final_response = emitted
        .iter()
        .find(|message| {
            message["method"] == json!("Network.responseReceived")
                && message["params"]["requestId"] == json!(request_id)
        })
        .expect("final response should be emitted");
    assert_eq!(
        final_response["params"]["response"]["headers"]["content-type"],
        json!("text/html")
    );

    ctx.process_async(json!({
        "id": 2278,
        "method": "Network.getResponseBody",
        "sessionId": session_id,
        "params": { "requestId": request_id }
    }))
    .await;
    ctx.expect_result(
            2278,
            json!({
                "body": "<!doctype html><html><body><main>redirect history headers</main></body></html>",
                "base64Encoded": false
            }),
            Some(&session_id),
        );

    let seen = seen.lock().clone();
    assert_eq!(
        seen,
        vec![
            (
                "/start".to_owned(),
                Some("https://www.google.com/".to_owned()),
                Some("history".to_owned()),
            ),
            (
                "/middle".to_owned(),
                Some("https://www.google.com/".to_owned()),
                Some("history".to_owned()),
            ),
            (
                "/final".to_owned(),
                Some("https://www.google.com/".to_owned()),
                Some("history".to_owned()),
            ),
        ]
    );

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn playwright_over_cdp_redirected_html_content_matches_final_document() {
    async fn start_handler(
        axum::extract::State(final_url): axum::extract::State<String>,
    ) -> impl IntoResponse {
        Redirect::temporary(&final_url)
    }

    async fn final_handler() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html; charset=utf-8")],
            "<!doctype html><html><head><title>Doc</title></head><body><main>redirected html content</main></body></html>",
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let start_url = format!("http://{addr}/start");
    let final_url = format!("http://{addr}/final");
    let server_final_url = final_url.clone();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/start", get(start_handler))
                .route("/final", get(final_handler))
                .with_state(server_final_url),
        )
        .await
        .unwrap();
    });

    let mut ctx = TestContext::new();
    let session_id = create_attached_page_session_async(&mut ctx, 2279, 2280, 2281, 2282, 2401)
        .await
        .session_id;

    ctx.process_async(json!({
        "id": 2284,
        "method": "Page.navigate",
        "sessionId": session_id,
        "params": { "url": start_url }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 2284);
    assert!(
        ctx.sent
            .iter()
            .all(|message| message.get("error").is_none()),
        "unexpected protocol error during redirected html navigation: {:?}",
        ctx.sent
    );

    let emitted = ctx.take_all();
    let first_request = emitted
        .iter()
        .find(|message| {
            message["method"] == json!("Network.requestWillBeSent")
                && message["params"]["request"]["url"] == json!(start_url)
        })
        .expect("redirect start request should be emitted");
    let request_id = first_request["params"]["requestId"]
        .as_str()
        .expect("request id")
        .to_owned();

    let final_response = emitted
        .iter()
        .find(|message| {
            message["method"] == json!("Network.responseReceived")
                && message["params"]["requestId"] == json!(request_id)
        })
        .expect("final response should be emitted");
    assert_eq!(
        final_response["params"]["response"]["url"],
        json!(final_url)
    );
    assert_eq!(
        final_response["params"]["response"]["headers"]["content-type"],
        json!("text/html; charset=utf-8")
    );

    ctx.process_async(json!({
        "id": 2285,
        "method": "Runtime.evaluate",
        "sessionId": session_id,
        "params": { "expression": "document.documentElement.outerHTML" }
    }))
    .await;
    let outer_html = take_response_by_id(&mut ctx, 2285)["result"]["result"]["value"]
        .as_str()
        .expect("outer html")
        .to_owned();
    assert_eq!(
        outer_html,
        "<html><head><title>Doc</title></head><body><main>redirected html content</main></body></html>"
    );

    ctx.process_async(json!({
        "id": 2286,
        "method": "Network.getResponseBody",
        "sessionId": session_id,
        "params": { "requestId": request_id }
    }))
    .await;
    let body = take_response_by_id(&mut ctx, 2286)["result"]["body"]
        .as_str()
        .expect("response body")
        .to_owned();
    assert_eq!(
        body,
        "<!doctype html><html><head><title>Doc</title></head><body><main>redirected html content</main></body></html>"
    );
    assert!(body.ends_with(&outer_html));

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn playwright_over_cdp_response_factory_surfaces_redirect_history_html_and_context_cookies() {
    async fn start_handler() -> impl IntoResponse {
        (
            [
                ("location", "/final"),
                ("set-cookie", "rid=1; Path=/"),
                (CONTENT_TYPE.as_str(), "text/plain"),
            ],
            StatusCode::TEMPORARY_REDIRECT,
        )
    }

    async fn final_handler(
        headers: HeaderMap,
        axum::extract::State(seen_cookie): axum::extract::State<Arc<Mutex<Option<String>>>>,
    ) -> impl IntoResponse {
        let cookie = headers
            .get(axum::http::header::COOKIE)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        *seen_cookie.lock() = cookie;
        (
            [
                (CONTENT_TYPE.as_str(), "text/html; charset=utf-8"),
                ("set-cookie", "pid=2; Path=/"),
            ],
            "<!doctype html><html><head><title>Doc</title></head><body><main>response-factory payload</main></body></html>",
        )
    }

    let seen_cookie = Arc::new(Mutex::new(None));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let start_url = format!("http://{addr}/start");
    let final_url = format!("http://{addr}/final");
    let server_seen_cookie = Arc::clone(&seen_cookie);
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/start", get(start_handler))
                .route("/final", get(final_handler))
                .with_state(server_seen_cookie),
        )
        .await
        .unwrap();
    });

    let mut ctx = TestContext::new();
    let attached =
        create_attached_page_session_async(&mut ctx, 22860, 22861, 22862, 22863, 22864).await;
    let browser_context_id = attached.browser_context_id;
    let session_id = attached.session_id;

    ctx.process_async(json!({
        "id": 22865,
        "method": "Page.navigate",
        "sessionId": session_id,
        "params": { "url": start_url }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 22865);
    assert!(
        ctx.sent
            .iter()
            .all(|message| message.get("error").is_none()),
        "unexpected protocol error during response-factory smoke navigation: {:?}",
        ctx.sent
    );

    let emitted = ctx.take_all();
    let first_request = emitted
        .iter()
        .find(|message| {
            message["method"] == json!("Network.requestWillBeSent")
                && message["params"]["request"]["url"] == json!(start_url)
        })
        .expect("redirect start request should be emitted");
    let request_id = first_request["params"]["requestId"]
        .as_str()
        .expect("request id")
        .to_owned();

    let redirected_request = emitted
        .iter()
        .find(|message| {
            message["method"] == json!("Network.requestWillBeSent")
                && message["params"]["requestId"] == json!(request_id)
                && message["params"]["request"]["url"] == json!(final_url)
        })
        .expect("redirect target request should be emitted");
    assert_eq!(
        redirected_request["params"]["redirectResponse"]["headers"]["location"],
        json!("/final")
    );
    assert_eq!(
        redirected_request["params"]["redirectResponse"]["headers"]["set-cookie"],
        json!("rid=1; Path=/")
    );

    let final_response = emitted
        .iter()
        .find(|message| {
            message["method"] == json!("Network.responseReceived")
                && message["params"]["requestId"] == json!(request_id)
        })
        .expect("final response should be emitted");
    assert_eq!(
        final_response["params"]["response"]["url"],
        json!(final_url)
    );
    assert_eq!(
        final_response["params"]["response"]["headers"]["content-type"],
        json!("text/html; charset=utf-8")
    );

    ctx.process_async(json!({
        "id": 22866,
        "method": "Runtime.evaluate",
        "sessionId": session_id,
        "params": { "expression": "document.documentElement.outerHTML" }
    }))
    .await;
    let outer_html = take_response_by_id(&mut ctx, 22866)["result"]["result"]["value"]
        .as_str()
        .expect("outer html")
        .to_owned();
    assert_eq!(
        outer_html,
        "<html><head><title>Doc</title></head><body><main>response-factory payload</main></body></html>"
    );

    ctx.process_async(json!({
        "id": 22867,
        "method": "Network.getResponseBody",
        "sessionId": session_id,
        "params": { "requestId": request_id }
    }))
    .await;
    let body = take_response_by_id(&mut ctx, 22867)["result"]["body"]
        .as_str()
        .expect("response body")
        .to_owned();
    assert_eq!(
        body,
        "<!doctype html><html><head><title>Doc</title></head><body><main>response-factory payload</main></body></html>"
    );
    assert!(body.ends_with(&outer_html));

    ctx.process_async(json!({
        "id": 22868,
        "method": "Storage.getCookies",
        "params": { "browserContextId": browser_context_id }
    }))
    .await;
    let cookies = take_response_by_id(&mut ctx, 22868);
    let mut stored_cookies = cookies["result"]["cookies"]
        .as_array()
        .expect("cookies array")
        .iter()
        .map(|cookie| {
            (
                cookie["name"].as_str().expect("cookie name").to_owned(),
                cookie["value"].as_str().expect("cookie value").to_owned(),
            )
        })
        .collect::<Vec<_>>();
    stored_cookies.sort();
    assert_eq!(
        stored_cookies,
        vec![
            ("pid".to_owned(), "2".to_owned()),
            ("rid".to_owned(), "1".to_owned()),
        ]
    );

    assert_eq!(seen_cookie.lock().as_deref(), Some("rid=1"));

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn playwright_over_cdp_response_factory_surfaces_history_html_cookies_and_captured_xhr() {
    async fn start_handler() -> impl IntoResponse {
        (
            [
                ("location", "/final"),
                ("set-cookie", "rid=1; Path=/"),
                (CONTENT_TYPE.as_str(), "text/plain"),
            ],
            StatusCode::TEMPORARY_REDIRECT,
        )
    }

    async fn final_handler(
        headers: HeaderMap,
        axum::extract::State(seen): axum::extract::State<Arc<Mutex<Vec<(String, Option<String>)>>>>,
    ) -> impl IntoResponse {
        let cookie = headers
            .get(axum::http::header::COOKIE)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        seen.lock().push(("/final".to_owned(), cookie));
        (
            [
                (CONTENT_TYPE.as_str(), "text/html; charset=utf-8"),
                ("set-cookie", "pid=2; Path=/"),
            ],
            "<!doctype html><html><head><title>Doc</title></head><body><main>response-factory payload</main></body></html>",
        )
    }

    async fn capture_fetch_handler(
        headers: HeaderMap,
        axum::extract::State(seen): axum::extract::State<Arc<Mutex<Vec<(String, Option<String>)>>>>,
    ) -> impl IntoResponse {
        let cookie = headers
            .get(axum::http::header::COOKIE)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        seen.lock().push(("/capture-fetch".to_owned(), cookie));
        (
            [
                (CONTENT_TYPE.as_str(), "application/json"),
                ("x-target-kind", "fetch"),
            ],
            "{\"kind\":\"capture-fetch\"}",
        )
    }

    async fn capture_xhr_handler(
        headers: HeaderMap,
        axum::extract::State(seen): axum::extract::State<Arc<Mutex<Vec<(String, Option<String>)>>>>,
    ) -> impl IntoResponse {
        let cookie = headers
            .get(axum::http::header::COOKIE)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        seen.lock().push(("/capture-xhr".to_owned(), cookie));
        (
            [
                (CONTENT_TYPE.as_str(), "text/plain"),
                ("x-target-kind", "xhr"),
            ],
            "capture xhr body",
        )
    }

    let seen = Arc::new(Mutex::new(Vec::<(String, Option<String>)>::new()));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let start_url = format!("http://{addr}/start");
    let final_url = format!("http://{addr}/final");
    let capture_fetch_url = format!("http://{addr}/capture-fetch");
    let capture_xhr_url = format!("http://{addr}/capture-xhr");
    let server_seen = Arc::clone(&seen);
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/start", get(start_handler))
                .route("/final", get(final_handler))
                .route("/capture-fetch", get(capture_fetch_handler))
                .route("/capture-xhr", get(capture_xhr_handler))
                .with_state(server_seen),
        )
        .await
        .unwrap();
    });

    let mut ctx = TestContext::new();
    let attached =
        create_attached_page_session_async(&mut ctx, 22869, 22870, 22871, 22872, 22873).await;
    let browser_context_id = attached.browser_context_id;
    let session_id = attached.session_id;

    ctx.process_async(json!({
        "id": 22874,
        "method": "Page.navigate",
        "sessionId": session_id,
        "params": { "url": start_url }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 22874);
    assert!(
        ctx.sent
            .iter()
            .all(|message| message.get("error").is_none()),
        "unexpected protocol error during response-factory capture smoke navigation: {:?}",
        ctx.sent
    );

    let emitted = ctx.take_all();
    let first_request = emitted
        .iter()
        .find(|message| {
            message["method"] == json!("Network.requestWillBeSent")
                && message["params"]["request"]["url"] == json!(start_url)
        })
        .expect("redirect start request should be emitted");
    let document_request_id = first_request["params"]["requestId"]
        .as_str()
        .expect("request id")
        .to_owned();

    let redirected_request = emitted
        .iter()
        .find(|message| {
            message["method"] == json!("Network.requestWillBeSent")
                && message["params"]["requestId"] == json!(document_request_id)
                && message["params"]["request"]["url"] == json!(final_url)
        })
        .expect("redirect target request should be emitted");
    assert_eq!(
        redirected_request["params"]["redirectResponse"]["headers"]["location"],
        json!("/final")
    );
    assert_eq!(
        redirected_request["params"]["redirectResponse"]["headers"]["set-cookie"],
        json!("rid=1; Path=/")
    );

    let final_response = emitted
        .iter()
        .find(|message| {
            message["method"] == json!("Network.responseReceived")
                && message["params"]["requestId"] == json!(document_request_id)
        })
        .expect("final response should be emitted");
    assert_eq!(
        final_response["params"]["response"]["url"],
        json!(final_url)
    );
    assert_eq!(
        final_response["params"]["response"]["headers"]["content-type"],
        json!("text/html; charset=utf-8")
    );

    ctx.process_async(json!({
        "id": 22875,
        "method": "Runtime.evaluate",
        "sessionId": session_id,
        "params": { "expression": "document.documentElement.outerHTML" }
    }))
    .await;
    let outer_html = take_response_by_id(&mut ctx, 22875)["result"]["result"]["value"]
        .as_str()
        .expect("outer html")
        .to_owned();
    assert_eq!(
        outer_html,
        "<html><head><title>Doc</title></head><body><main>response-factory payload</main></body></html>"
    );

    ctx.process_async(json!({
            "id": 22876,
            "method": "Runtime.evaluate",
            "sessionId": session_id,
            "params": {
                "expression": "Promise.all([fetch('/capture-fetch').then(r => r.text()), new Promise(resolve => { const xhr = new XMLHttpRequest(); xhr.open('GET', '/capture-xhr'); xhr.onload = () => resolve(xhr.responseText); xhr.send(); })])"
            }
    })).await;
    let evaluation = take_response_by_id(&mut ctx, 22876);
    assert_eq!(evaluation["id"], json!(22876));
    flush_until_playwright_subresources_finished(
        &mut ctx,
        &session_id,
        "Fetch",
        1,
        "playwright response factory fetch completion",
    )
    .await;
    flush_until_playwright_subresources_finished(
        &mut ctx,
        &session_id,
        "XHR",
        1,
        "playwright response factory xhr completion",
    )
    .await;

    let subresource_responses = ctx
        .sent
        .iter()
        .filter(|message| {
            message["method"] == json!("Network.responseReceived")
                && matches!(
                    message["params"]["type"].as_str(),
                    Some("Fetch") | Some("XHR")
                )
        })
        .cloned()
        .collect::<Vec<_>>();
    assert_eq!(subresource_responses.len(), 2, "{subresource_responses:?}");

    let fetch_request_id = subresource_responses
        .iter()
        .find(|message| {
            message["params"]["type"] == json!("Fetch")
                && message["params"]["response"]["url"] == json!(capture_fetch_url)
        })
        .and_then(|message| message["params"]["requestId"].as_str())
        .expect("fetch request id")
        .to_owned();
    let xhr_request_id = subresource_responses
        .iter()
        .find(|message| {
            message["params"]["type"] == json!("XHR")
                && message["params"]["response"]["url"] == json!(capture_xhr_url)
        })
        .and_then(|message| message["params"]["requestId"].as_str())
        .expect("xhr request id")
        .to_owned();

    ctx.process_async(json!({
        "id": 22877,
        "method": "Network.getResponseBody",
        "sessionId": session_id,
        "params": { "requestId": document_request_id }
    }))
    .await;
    let body = take_response_by_id(&mut ctx, 22877)["result"]["body"]
        .as_str()
        .expect("response body")
        .to_owned();
    assert_eq!(
        body,
        "<!doctype html><html><head><title>Doc</title></head><body><main>response-factory payload</main></body></html>"
    );
    assert!(body.ends_with(&outer_html));

    ctx.process_async(json!({
        "id": 22878,
        "method": "Network.getResponseBody",
        "sessionId": session_id,
        "params": { "requestId": fetch_request_id }
    }))
    .await;
    ctx.expect_result(
        22878,
        json!({
            "body": "{\"kind\":\"capture-fetch\"}",
            "base64Encoded": false
        }),
        Some(&session_id),
    );

    ctx.process_async(json!({
        "id": 22879,
        "method": "Network.getResponseBody",
        "sessionId": session_id,
        "params": { "requestId": xhr_request_id }
    }))
    .await;
    ctx.expect_result(
        22879,
        json!({
            "body": "capture xhr body",
            "base64Encoded": false
        }),
        Some(&session_id),
    );

    ctx.process_async(json!({
        "id": 22880,
        "method": "Storage.getCookies",
        "params": { "browserContextId": browser_context_id }
    }))
    .await;
    let cookies = take_response_by_id(&mut ctx, 22880);
    let mut stored_cookies = cookies["result"]["cookies"]
        .as_array()
        .expect("cookies array")
        .iter()
        .map(|cookie| {
            (
                cookie["name"].as_str().expect("cookie name").to_owned(),
                cookie["value"].as_str().expect("cookie value").to_owned(),
            )
        })
        .collect::<Vec<_>>();
    stored_cookies.sort();
    assert_eq!(
        stored_cookies,
        vec![
            ("pid".to_owned(), "2".to_owned()),
            ("rid".to_owned(), "1".to_owned()),
        ]
    );

    let seen_requests = seen.lock().clone();
    assert_eq!(
        seen_requests.first(),
        Some(&("/final".to_owned(), Some("rid=1".to_owned())))
    );
    let mut subresource_seen = seen_requests[1..].to_vec();
    subresource_seen.sort();
    assert_eq!(
        subresource_seen,
        vec![
            ("/capture-fetch".to_owned(), Some("rid=1; pid=2".to_owned())),
            ("/capture-xhr".to_owned(), Some("rid=1; pid=2".to_owned())),
        ]
    );

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn playwright_over_cdp_response_factory_surfaces_document_and_captured_xhr_headers() {
    async fn start_handler(headers: HeaderMap) -> impl IntoResponse {
        let _extra = headers
            .get("x-cdp-test")
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default();
        let _referer = headers
            .get(axum::http::header::REFERER)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default();
        (
            [
                ("location", "/final"),
                ("x-doc-hop", "start"),
                (CONTENT_TYPE.as_str(), "text/plain"),
            ],
            StatusCode::TEMPORARY_REDIRECT,
        )
    }

    async fn final_handler(
        headers: HeaderMap,
        axum::extract::State(seen): axum::extract::State<
            Arc<Mutex<Vec<(String, Option<String>, Option<String>)>>>,
        >,
    ) -> impl IntoResponse {
        let extra = headers
            .get("x-cdp-test")
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let referer = headers
            .get(axum::http::header::REFERER)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        seen.lock().push(("/final".to_owned(), extra, referer));
        (
            [
                (CONTENT_TYPE.as_str(), "text/html; charset=utf-8"),
                ("x-doc-response", "final"),
            ],
            "<!doctype html><html><body><main>header surface</main></body></html>",
        )
    }

    async fn capture_fetch_handler(
        headers: HeaderMap,
        axum::extract::State(seen): axum::extract::State<
            Arc<Mutex<Vec<(String, Option<String>, Option<String>)>>>,
        >,
    ) -> impl IntoResponse {
        let extra = headers
            .get("x-cdp-test")
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let referer = headers
            .get(axum::http::header::REFERER)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        seen.lock()
            .push(("/capture-fetch".to_owned(), extra, referer));
        (
            [
                (CONTENT_TYPE.as_str(), "application/json"),
                ("x-target-kind", "fetch"),
                ("x-subresource-response", "fetch"),
            ],
            "{\"kind\":\"capture-fetch\"}",
        )
    }

    async fn capture_xhr_handler(
        headers: HeaderMap,
        axum::extract::State(seen): axum::extract::State<
            Arc<Mutex<Vec<(String, Option<String>, Option<String>)>>>,
        >,
    ) -> impl IntoResponse {
        let extra = headers
            .get("x-cdp-test")
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let referer = headers
            .get(axum::http::header::REFERER)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        seen.lock()
            .push(("/capture-xhr".to_owned(), extra, referer));
        (
            [
                (CONTENT_TYPE.as_str(), "text/plain"),
                ("x-target-kind", "xhr"),
                ("x-subresource-response", "xhr"),
            ],
            "capture xhr body",
        )
    }

    let seen = Arc::new(Mutex::new(
        Vec::<(String, Option<String>, Option<String>)>::new(),
    ));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let start_url = format!("http://{addr}/start");
    let final_url = format!("http://{addr}/final");
    let capture_fetch_url = format!("http://{addr}/capture-fetch");
    let capture_xhr_url = format!("http://{addr}/capture-xhr");
    let server_seen = Arc::clone(&seen);
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/start", get(start_handler))
                .route("/final", get(final_handler))
                .route("/capture-fetch", get(capture_fetch_handler))
                .route("/capture-xhr", get(capture_xhr_handler))
                .with_state(server_seen),
        )
        .await
        .unwrap();
    });

    let mut ctx = TestContext::new();
    let attached =
        create_attached_page_session_async(&mut ctx, 22881, 22882, 22883, 22884, 22885).await;
    let session_id = attached.session_id;

    ctx.process_async(json!({
        "id": 22886,
        "method": "Network.setExtraHTTPHeaders",
        "sessionId": session_id,
        "params": {
            "headers": {
                "x-cdp-test": "response-factory"
            }
        }
    }))
    .await;
    ctx.expect_result(22886, json!({}), Some(&session_id));

    ctx.process_async(json!({
        "id": 22887,
        "method": "Page.navigate",
        "sessionId": session_id,
        "params": {
            "url": start_url,
            "referrer": "https://www.google.com/"
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 22887);
    assert!(
        ctx.sent
            .iter()
            .all(|message| message.get("error").is_none()),
        "unexpected protocol error during response-factory header smoke navigation: {:?}",
        ctx.sent
    );

    let emitted = ctx.take_all();
    let first_request = emitted
        .iter()
        .find(|message| {
            message["method"] == json!("Network.requestWillBeSent")
                && message["params"]["request"]["url"] == json!(start_url)
        })
        .expect("redirect start request should be emitted");
    let document_request_id = first_request["params"]["requestId"]
        .as_str()
        .expect("document request id")
        .to_owned();
    assert_eq!(
        first_request["params"]["request"]["headers"]["Referer"],
        json!("https://www.google.com/")
    );
    assert_eq!(
        first_request["params"]["request"]["headers"]["x-cdp-test"],
        json!("response-factory")
    );

    let redirected_request = emitted
        .iter()
        .find(|message| {
            message["method"] == json!("Network.requestWillBeSent")
                && message["params"]["requestId"] == json!(document_request_id)
                && message["params"]["request"]["url"] == json!(final_url)
        })
        .expect("redirect target request should be emitted");
    assert_eq!(
        redirected_request["params"]["redirectResponse"]["headers"]["location"],
        json!("/final")
    );
    assert_eq!(
        redirected_request["params"]["redirectResponse"]["headers"]["x-doc-hop"],
        json!("start")
    );
    assert_eq!(
        redirected_request["params"]["request"]["headers"]["Referer"],
        json!("https://www.google.com/")
    );
    assert_eq!(
        redirected_request["params"]["request"]["headers"]["x-cdp-test"],
        json!("response-factory")
    );

    let final_response = emitted
        .iter()
        .find(|message| {
            message["method"] == json!("Network.responseReceived")
                && message["params"]["requestId"] == json!(document_request_id)
        })
        .expect("final response should be emitted");
    assert_eq!(
        final_response["params"]["response"]["url"],
        json!(final_url)
    );
    assert_eq!(
        final_response["params"]["response"]["headers"]["content-type"],
        json!("text/html; charset=utf-8")
    );
    assert_eq!(
        final_response["params"]["response"]["headers"]["x-doc-response"],
        json!("final")
    );

    ctx.process_async(json!({
            "id": 22888,
            "method": "Runtime.evaluate",
            "sessionId": session_id,
            "params": {
                "expression": "Promise.all([fetch('/capture-fetch').then(r => r.text()), new Promise(resolve => { const xhr = new XMLHttpRequest(); xhr.open('GET', '/capture-xhr'); xhr.onload = () => resolve(xhr.responseText); xhr.send(); })])"
            }
    })).await;
    let evaluation = take_response_by_id(&mut ctx, 22888);
    assert_eq!(evaluation["id"], json!(22888));
    flush_until_playwright_subresources_finished(
        &mut ctx,
        &session_id,
        "Fetch",
        1,
        "playwright response factory fetch completion",
    )
    .await;
    flush_until_playwright_subresources_finished(
        &mut ctx,
        &session_id,
        "XHR",
        1,
        "playwright response factory xhr completion",
    )
    .await;

    let fetch_request = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Network.requestWillBeSent")
                && message["params"]["type"] == json!("Fetch")
                && message["params"]["request"]["url"] == json!(capture_fetch_url)
        })
        .cloned()
        .expect("fetch request event");
    assert_eq!(
        fetch_request["params"]["request"]["headers"]["x-cdp-test"],
        json!("response-factory")
    );
    let fetch_request_id = fetch_request["params"]["requestId"]
        .as_str()
        .expect("fetch request id")
        .to_owned();

    let xhr_request = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Network.requestWillBeSent")
                && message["params"]["type"] == json!("XHR")
                && message["params"]["request"]["url"] == json!(capture_xhr_url)
        })
        .cloned()
        .expect("xhr request event");
    assert_eq!(
        xhr_request["params"]["request"]["headers"]["x-cdp-test"],
        json!("response-factory")
    );
    let xhr_request_id = xhr_request["params"]["requestId"]
        .as_str()
        .expect("xhr request id")
        .to_owned();

    let fetch_response = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Network.responseReceived")
                && message["params"]["requestId"] == json!(fetch_request_id)
        })
        .cloned()
        .expect("fetch response event");
    assert_eq!(fetch_response["params"]["type"], json!("Fetch"));
    assert_eq!(
        fetch_response["params"]["response"]["headers"]["x-target-kind"],
        json!("fetch")
    );
    assert_eq!(
        fetch_response["params"]["response"]["headers"]["x-subresource-response"],
        json!("fetch")
    );

    let xhr_response = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Network.responseReceived")
                && message["params"]["requestId"] == json!(xhr_request_id)
        })
        .cloned()
        .expect("xhr response event");
    assert_eq!(xhr_response["params"]["type"], json!("XHR"));
    assert_eq!(
        xhr_response["params"]["response"]["headers"]["x-target-kind"],
        json!("xhr")
    );
    assert_eq!(
        xhr_response["params"]["response"]["headers"]["x-subresource-response"],
        json!("xhr")
    );

    let seen = seen.lock().clone();
    assert_eq!(
        seen.first(),
        Some(&(
            "/final".to_owned(),
            Some("response-factory".to_owned()),
            Some("https://www.google.com/".to_owned()),
        ))
    );
    let mut trailing = seen.into_iter().skip(1).collect::<Vec<_>>();
    trailing.sort();
    assert_eq!(
        trailing,
        vec![
            (
                "/capture-fetch".to_owned(),
                Some("response-factory".to_owned()),
                Some(final_url.clone()),
            ),
            (
                "/capture-xhr".to_owned(),
                Some("response-factory".to_owned()),
                Some(final_url.clone()),
            ),
        ]
    );

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn playwright_over_cdp_response_factory_surfaces_persist_across_second_page_in_same_context()
{
    async fn first_start() -> impl IntoResponse {
        (
            [
                ("location", "/first-final"),
                ("set-cookie", "rid=1; Path=/"),
                (CONTENT_TYPE.as_str(), "text/plain"),
            ],
            StatusCode::TEMPORARY_REDIRECT,
        )
    }

    async fn first_final(
        headers: HeaderMap,
        axum::extract::State(seen): axum::extract::State<
            Arc<Mutex<Vec<(String, Option<String>, Option<String>)>>>,
        >,
    ) -> impl IntoResponse {
        let cookie = headers
            .get(axum::http::header::COOKIE)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let extra = headers
            .get("x-cdp-test")
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        seen.lock().push(("/first-final".to_owned(), extra, cookie));
        (
            [
                (CONTENT_TYPE.as_str(), "text/html; charset=utf-8"),
                ("set-cookie", "pid=2; Path=/"),
            ],
            "<!doctype html><html><body><main>first page</main></body></html>",
        )
    }

    async fn first_fetch(
        headers: HeaderMap,
        axum::extract::State(seen): axum::extract::State<
            Arc<Mutex<Vec<(String, Option<String>, Option<String>)>>>,
        >,
    ) -> impl IntoResponse {
        let cookie = headers
            .get(axum::http::header::COOKIE)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let extra = headers
            .get("x-cdp-test")
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        seen.lock().push(("/first-fetch".to_owned(), extra, cookie));
        (
            [
                (CONTENT_TYPE.as_str(), "application/json"),
                ("x-target-kind", "fetch"),
            ],
            "{\"kind\":\"first-fetch\"}",
        )
    }

    async fn first_xhr(
        headers: HeaderMap,
        axum::extract::State(seen): axum::extract::State<
            Arc<Mutex<Vec<(String, Option<String>, Option<String>)>>>,
        >,
    ) -> impl IntoResponse {
        let cookie = headers
            .get(axum::http::header::COOKIE)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let extra = headers
            .get("x-cdp-test")
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        seen.lock().push(("/first-xhr".to_owned(), extra, cookie));
        (
            [
                (CONTENT_TYPE.as_str(), "text/plain"),
                ("x-target-kind", "xhr"),
            ],
            "first xhr body",
        )
    }

    async fn second_start(
        headers: HeaderMap,
        axum::extract::State(seen): axum::extract::State<
            Arc<Mutex<Vec<(String, Option<String>, Option<String>)>>>,
        >,
    ) -> impl IntoResponse {
        let cookie = headers
            .get(axum::http::header::COOKIE)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let extra = headers
            .get("x-cdp-test")
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        seen.lock()
            .push(("/second-start".to_owned(), extra, cookie));
        (
            [
                ("location", "/second-final"),
                (CONTENT_TYPE.as_str(), "text/plain"),
            ],
            StatusCode::TEMPORARY_REDIRECT,
        )
    }

    async fn second_final(
        headers: HeaderMap,
        axum::extract::State(seen): axum::extract::State<
            Arc<Mutex<Vec<(String, Option<String>, Option<String>)>>>,
        >,
    ) -> impl IntoResponse {
        let cookie = headers
            .get(axum::http::header::COOKIE)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let extra = headers
            .get("x-cdp-test")
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        seen.lock()
            .push(("/second-final".to_owned(), extra, cookie));
        (
            [
                (CONTENT_TYPE.as_str(), "text/html; charset=utf-8"),
                ("set-cookie", "sid2=3; Path=/"),
            ],
            "<!doctype html><html><body><main>second page</main></body></html>",
        )
    }

    async fn second_fetch(
        headers: HeaderMap,
        axum::extract::State(seen): axum::extract::State<
            Arc<Mutex<Vec<(String, Option<String>, Option<String>)>>>,
        >,
    ) -> impl IntoResponse {
        let cookie = headers
            .get(axum::http::header::COOKIE)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let extra = headers
            .get("x-cdp-test")
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        seen.lock()
            .push(("/second-fetch".to_owned(), extra, cookie));
        (
            [
                (CONTENT_TYPE.as_str(), "application/json"),
                ("x-target-kind", "fetch"),
            ],
            "{\"kind\":\"second-fetch\"}",
        )
    }

    async fn second_xhr(
        headers: HeaderMap,
        axum::extract::State(seen): axum::extract::State<
            Arc<Mutex<Vec<(String, Option<String>, Option<String>)>>>,
        >,
    ) -> impl IntoResponse {
        let cookie = headers
            .get(axum::http::header::COOKIE)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let extra = headers
            .get("x-cdp-test")
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        seen.lock().push(("/second-xhr".to_owned(), extra, cookie));
        (
            [
                (CONTENT_TYPE.as_str(), "text/plain"),
                ("x-target-kind", "xhr"),
            ],
            "second xhr body",
        )
    }

    let seen = Arc::new(Mutex::new(
        Vec::<(String, Option<String>, Option<String>)>::new(),
    ));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let first_start_url = format!("http://{addr}/first-start");
    let first_fetch_url = format!("http://{addr}/first-fetch");
    let first_xhr_url = format!("http://{addr}/first-xhr");
    let second_start_url = format!("http://{addr}/second-start");
    let second_fetch_url = format!("http://{addr}/second-fetch");
    let second_xhr_url = format!("http://{addr}/second-xhr");
    let server_seen = Arc::clone(&seen);
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/first-start", get(first_start))
                .route("/first-final", get(first_final))
                .route("/first-fetch", get(first_fetch))
                .route("/first-xhr", get(first_xhr))
                .route("/second-start", get(second_start))
                .route("/second-final", get(second_final))
                .route("/second-fetch", get(second_fetch))
                .route("/second-xhr", get(second_xhr))
                .with_state(server_seen),
        )
        .await
        .unwrap();
    });

    let mut ctx = TestContext::new();

    let first_attached =
        create_attached_page_session_async(&mut ctx, 22890, 22891, 22892, 22893, 22894).await;
    let browser_context_id = first_attached.browser_context_id.clone();
    let first_target_id = first_attached.target_id.clone();
    let first_session_id = first_attached.session_id.clone();

    ctx.process_async(json!({
        "id": 22895,
        "method": "Network.setExtraHTTPHeaders",
        "sessionId": first_session_id,
        "params": {
            "headers": {
                "x-cdp-test": "page-one"
            }
        }
    }))
    .await;
    ctx.expect_result(22895, json!({}), Some(&first_session_id));

    ctx.process_async(json!({
        "id": 22896,
        "method": "Page.navigate",
        "sessionId": first_session_id,
        "params": {
            "url": first_start_url,
            "referrer": "https://www.google.com/"
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 22896);
    assert!(
        ctx.sent
            .iter()
            .all(|message| message.get("error").is_none()),
        "unexpected protocol error during first page navigation: {:?}",
        ctx.sent
    );
    ctx.take_all();

    ctx.process_async(json!({
            "id": 22897,
            "method": "Runtime.evaluate",
            "sessionId": first_session_id,
            "params": {
                "expression": "Promise.all([fetch('/first-fetch').then(r => r.text()), new Promise(resolve => { const xhr = new XMLHttpRequest(); xhr.open('GET', '/first-xhr'); xhr.onload = () => resolve(xhr.responseText); xhr.send(); })])"
            }
    })).await;
    let first_eval = take_response_by_id(&mut ctx, 22897);
    assert_eq!(first_eval["id"], json!(22897));
    flush_until_playwright_subresources_finished(
        &mut ctx,
        &first_session_id,
        "Fetch",
        1,
        "playwright first page fetch completion",
    )
    .await;
    flush_until_playwright_subresources_finished(
        &mut ctx,
        &first_session_id,
        "XHR",
        1,
        "playwright first page xhr completion",
    )
    .await;

    let first_fetch_request_id = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Network.responseReceived")
                && message["params"]["type"] == json!("Fetch")
                && message["params"]["response"]["url"] == json!(first_fetch_url)
        })
        .and_then(|message| message["params"]["requestId"].as_str())
        .expect("first fetch request id")
        .to_owned();
    let first_xhr_request_id = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Network.responseReceived")
                && message["params"]["type"] == json!("XHR")
                && message["params"]["response"]["url"] == json!(first_xhr_url)
        })
        .and_then(|message| message["params"]["requestId"].as_str())
        .expect("first xhr request id")
        .to_owned();

    ctx.process_async(json!({
        "id": 22898,
        "method": "Network.getResponseBody",
        "sessionId": first_session_id,
        "params": { "requestId": first_fetch_request_id }
    }))
    .await;
    ctx.expect_result(
        22898,
        json!({
            "body": "{\"kind\":\"first-fetch\"}",
            "base64Encoded": false
        }),
        Some(&first_session_id),
    );

    ctx.process_async(json!({
        "id": 22899,
        "method": "Network.getResponseBody",
        "sessionId": first_session_id,
        "params": { "requestId": first_xhr_request_id }
    }))
    .await;
    ctx.expect_result(
        22899,
        json!({
            "body": "first xhr body",
            "base64Encoded": false
        }),
        Some(&first_session_id),
    );

    ctx.process_async(json!({
        "id": 22900,
        "method": "Target.closeTarget",
        "params": { "targetId": first_target_id }
    }))
    .await;
    ctx.expect_result(22900, json!({ "success": true }), None);
    ctx.take_all();

    let second_attached = attach_page_session_in_existing_context_async(
        &mut ctx,
        &browser_context_id,
        22901,
        22902,
        22903,
        22904,
    )
    .await;
    let second_target_id = second_attached.target_id.clone();
    assert_ne!(second_target_id, first_target_id);
    let second_session_id = second_attached.session_id.clone();

    ctx.process_async(json!({
        "id": 22905,
        "method": "Network.setExtraHTTPHeaders",
        "sessionId": second_session_id,
        "params": {
            "headers": {
                "x-cdp-test": "page-two"
            }
        }
    }))
    .await;
    ctx.expect_result(22905, json!({}), Some(&second_session_id));

    ctx.process_async(json!({
        "id": 22906,
        "method": "Page.navigate",
        "sessionId": second_session_id,
        "params": {
            "url": second_start_url,
            "referrer": "https://www.google.com/"
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 22906);
    assert!(
        ctx.sent
            .iter()
            .all(|message| message.get("error").is_none()),
        "unexpected protocol error during second page navigation: {:?}",
        ctx.sent
    );
    ctx.take_all();

    ctx.process_async(json!({
            "id": 22907,
            "method": "Runtime.evaluate",
            "sessionId": second_session_id,
            "params": {
                "expression": "Promise.all([fetch('/second-fetch').then(r => r.text()), new Promise(resolve => { const xhr = new XMLHttpRequest(); xhr.open('GET', '/second-xhr'); xhr.onload = () => resolve(xhr.responseText); xhr.send(); })])"
            }
    })).await;
    let second_eval = take_response_by_id(&mut ctx, 22907);
    assert_eq!(second_eval["id"], json!(22907));
    flush_until_playwright_subresources_finished(
        &mut ctx,
        &second_session_id,
        "Fetch",
        1,
        "playwright second page fetch completion",
    )
    .await;
    flush_until_playwright_subresources_finished(
        &mut ctx,
        &second_session_id,
        "XHR",
        1,
        "playwright second page xhr completion",
    )
    .await;

    let second_fetch_request_id = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Network.responseReceived")
                && message["params"]["type"] == json!("Fetch")
                && message["params"]["response"]["url"] == json!(second_fetch_url)
        })
        .and_then(|message| message["params"]["requestId"].as_str())
        .expect("second fetch request id")
        .to_owned();
    let second_xhr_request_id = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Network.responseReceived")
                && message["params"]["type"] == json!("XHR")
                && message["params"]["response"]["url"] == json!(second_xhr_url)
        })
        .and_then(|message| message["params"]["requestId"].as_str())
        .expect("second xhr request id")
        .to_owned();

    ctx.process_async(json!({
        "id": 22908,
        "method": "Network.getResponseBody",
        "sessionId": second_session_id,
        "params": { "requestId": second_fetch_request_id }
    }))
    .await;
    ctx.expect_result(
        22908,
        json!({
            "body": "{\"kind\":\"second-fetch\"}",
            "base64Encoded": false
        }),
        Some(&second_session_id),
    );

    ctx.process_async(json!({
        "id": 22909,
        "method": "Network.getResponseBody",
        "sessionId": second_session_id,
        "params": { "requestId": second_xhr_request_id }
    }))
    .await;
    ctx.expect_result(
        22909,
        json!({
            "body": "second xhr body",
            "base64Encoded": false
        }),
        Some(&second_session_id),
    );

    ctx.process_async(json!({
        "id": 22910,
        "method": "Storage.getCookies",
        "params": { "browserContextId": browser_context_id }
    }))
    .await;
    let cookies = take_response_by_id(&mut ctx, 22910);
    let mut stored_cookies = cookies["result"]["cookies"]
        .as_array()
        .expect("cookies array")
        .iter()
        .map(|cookie| {
            (
                cookie["name"].as_str().expect("cookie name").to_owned(),
                cookie["value"].as_str().expect("cookie value").to_owned(),
            )
        })
        .collect::<Vec<_>>();
    stored_cookies.sort();
    assert_eq!(
        stored_cookies,
        vec![
            ("pid".to_owned(), "2".to_owned()),
            ("rid".to_owned(), "1".to_owned()),
            ("sid2".to_owned(), "3".to_owned()),
        ]
    );

    let seen = seen.lock().clone();
    let first_final_seen = seen
        .iter()
        .find(|(path, _, _)| path == "/first-final")
        .expect("first final request");
    assert_eq!(first_final_seen.1.as_deref(), Some("page-one"));
    assert_eq!(first_final_seen.2.as_deref(), Some("rid=1"));

    let first_fetch_seen = seen
        .iter()
        .find(|(path, _, _)| path == "/first-fetch")
        .expect("first fetch request");
    assert_eq!(first_fetch_seen.1.as_deref(), Some("page-one"));
    assert!(
        first_fetch_seen
            .2
            .as_deref()
            .is_some_and(|cookie| cookie.contains("rid=1") && cookie.contains("pid=2"))
    );

    let first_xhr_seen = seen
        .iter()
        .find(|(path, _, _)| path == "/first-xhr")
        .expect("first xhr request");
    assert_eq!(first_xhr_seen.1.as_deref(), Some("page-one"));
    assert!(
        first_xhr_seen
            .2
            .as_deref()
            .is_some_and(|cookie| cookie.contains("rid=1") && cookie.contains("pid=2"))
    );

    let second_start_seen = seen
        .iter()
        .find(|(path, _, _)| path == "/second-start")
        .expect("second start request");
    assert_eq!(second_start_seen.1.as_deref(), Some("page-two"));
    assert!(
        second_start_seen
            .2
            .as_deref()
            .is_some_and(|cookie| cookie.contains("rid=1") && cookie.contains("pid=2"))
    );

    let second_final_seen = seen
        .iter()
        .find(|(path, _, _)| path == "/second-final")
        .expect("second final request");
    assert_eq!(second_final_seen.1.as_deref(), Some("page-two"));
    assert!(
        second_final_seen
            .2
            .as_deref()
            .is_some_and(|cookie| cookie.contains("rid=1") && cookie.contains("pid=2"))
    );

    let second_fetch_seen = seen
        .iter()
        .find(|(path, _, _)| path == "/second-fetch")
        .expect("second fetch request");
    assert_eq!(second_fetch_seen.1.as_deref(), Some("page-two"));
    assert!(second_fetch_seen.2.as_deref().is_some_and(|cookie| {
        cookie.contains("rid=1") && cookie.contains("pid=2") && cookie.contains("sid2=3")
    }));

    let second_xhr_seen = seen
        .iter()
        .find(|(path, _, _)| path == "/second-xhr")
        .expect("second xhr request");
    assert_eq!(second_xhr_seen.1.as_deref(), Some("page-two"));
    assert!(second_xhr_seen.2.as_deref().is_some_and(|cookie| {
        cookie.contains("rid=1") && cookie.contains("pid=2") && cookie.contains("sid2=3")
    }));

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn playwright_over_cdp_runtime_fetch_exposes_subresource_response_and_body() {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>ok</body></html>",
        )
    }

    async fn api() -> impl IntoResponse {
        (
            [
                (CONTENT_TYPE.as_str(), "text/plain"),
                ("x-target-fetch", "ok"),
            ],
            "target fetch body",
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let page_url = format!("http://{addr}/page");
    let api_url = format!("http://{addr}/api");
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/page", get(page))
                .route("/api", get(api)),
        )
        .await
        .unwrap();
    });

    let mut ctx = TestContext::new();
    let session_id = create_attached_page_session_async(&mut ctx, 2246, 2247, 2248, 2249, 2250)
        .await
        .session_id;

    ctx.process_async(json!({
        "id": 2251,
        "method": "Page.navigate",
        "sessionId": session_id,
        "params": { "url": page_url }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 2251);
    assert!(
        ctx.sent
            .iter()
            .all(|message| message.get("error").is_none()),
        "unexpected protocol error during page navigation: {:?}",
        ctx.sent
    );
    ctx.take_all();

    ctx.process_async(json!({
        "id": 2252,
        "method": "Runtime.evaluate",
        "sessionId": session_id,
        "params": { "expression": "fetch('/api').then(r => r.text())" }
    }))
    .await;
    let evaluation = take_response_by_id(&mut ctx, 2252);
    assert_eq!(evaluation["id"], json!(2252));
    flush_until_playwright_subresources_finished(
        &mut ctx,
        &session_id,
        "Fetch",
        1,
        "playwright runtime fetch network completion",
    )
    .await;

    let request = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Network.requestWillBeSent")
                && message["params"]["type"] == json!("Fetch")
        })
        .cloned()
        .expect("runtime fetch request event");
    assert_eq!(request["sessionId"], json!(session_id));
    assert_eq!(request["params"]["request"]["url"], json!(api_url));
    let request_id = request["params"]["requestId"]
        .as_str()
        .expect("runtime fetch request id")
        .to_owned();

    let response_event = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Network.responseReceived")
                && message["params"]["requestId"] == json!(request_id)
        })
        .cloned()
        .expect("runtime fetch response event");
    assert_eq!(response_event["params"]["type"], json!("Fetch"));
    assert_eq!(
        response_event["params"]["response"]["headers"]["x-target-fetch"],
        json!("ok")
    );

    assert!(ctx.sent.iter().any(|message| {
        message["method"] == json!("Network.loadingFinished")
            && message["params"]["requestId"] == json!(request_id)
    }));

    ctx.process_async(json!({
        "id": 2253,
        "method": "Network.getResponseBody",
        "sessionId": session_id,
        "params": { "requestId": request_id }
    }))
    .await;
    ctx.expect_result(
        2253,
        json!({
            "body": "target fetch body",
            "base64Encoded": false
        }),
        Some(&session_id),
    );

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn playwright_over_cdp_runtime_fetch_captures_multiple_subresource_responses() {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>ok</body></html>",
        )
    }

    async fn api_one() -> impl IntoResponse {
        (
            [
                (CONTENT_TYPE.as_str(), "text/plain"),
                ("x-target-fetch", "one"),
            ],
            "target fetch one",
        )
    }

    async fn api_two() -> impl IntoResponse {
        (
            [
                (CONTENT_TYPE.as_str(), "text/plain"),
                ("x-target-fetch", "two"),
            ],
            "target fetch two",
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let page_url = format!("http://{addr}/page");
    let api_one_url = format!("http://{addr}/api-one");
    let api_two_url = format!("http://{addr}/api-two");
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/page", get(page))
                .route("/api-one", get(api_one))
                .route("/api-two", get(api_two)),
        )
        .await
        .unwrap();
    });

    let mut ctx = TestContext::new();
    let session_id = create_attached_page_session_async(&mut ctx, 2254, 2255, 2256, 2257, 2258)
        .await
        .session_id;

    ctx.process_async(json!({
        "id": 2259,
        "method": "Page.navigate",
        "sessionId": session_id,
        "params": { "url": page_url }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 2259);
    assert!(
        ctx.sent
            .iter()
            .all(|message| message.get("error").is_none()),
        "unexpected protocol error during page navigation: {:?}",
        ctx.sent
    );
    ctx.take_all();

    ctx.process_async(json!({
            "id": 2260,
            "method": "Runtime.evaluate",
            "sessionId": session_id,
            "params": {
                "expression": "Promise.all([fetch('/api-one').then(r => r.text()), fetch('/api-two').then(r => r.text())])"
            }
        })).await;
    let evaluation = take_response_by_id(&mut ctx, 2260);
    assert_eq!(evaluation["id"], json!(2260));
    flush_until_playwright_subresources_finished(
        &mut ctx,
        &session_id,
        "Fetch",
        2,
        "playwright runtime fetch network completions",
    )
    .await;

    let fetch_requests = ctx
        .sent
        .iter()
        .filter(|message| {
            message["method"] == json!("Network.requestWillBeSent")
                && message["params"]["type"] == json!("Fetch")
        })
        .cloned()
        .collect::<Vec<_>>();
    assert_eq!(fetch_requests.len(), 2, "{fetch_requests:?}");

    let first_request = fetch_requests
        .iter()
        .find(|message| message["params"]["request"]["url"] == json!(api_one_url))
        .expect("api-one request should be emitted");
    let second_request = fetch_requests
        .iter()
        .find(|message| message["params"]["request"]["url"] == json!(api_two_url))
        .expect("api-two request should be emitted");
    let first_request_id = first_request["params"]["requestId"]
        .as_str()
        .expect("first request id")
        .to_owned();
    let second_request_id = second_request["params"]["requestId"]
        .as_str()
        .expect("second request id")
        .to_owned();
    assert_ne!(first_request_id, second_request_id);

    let first_response = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Network.responseReceived")
                && message["params"]["requestId"] == json!(first_request_id)
        })
        .cloned()
        .expect("api-one response event");
    let second_response = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Network.responseReceived")
                && message["params"]["requestId"] == json!(second_request_id)
        })
        .cloned()
        .expect("api-two response event");
    assert_eq!(
        first_response["params"]["response"]["headers"]["x-target-fetch"],
        json!("one")
    );
    assert_eq!(
        second_response["params"]["response"]["headers"]["x-target-fetch"],
        json!("two")
    );

    assert!(ctx.sent.iter().any(|message| {
        message["method"] == json!("Network.loadingFinished")
            && message["params"]["requestId"] == json!(first_request_id)
    }));
    assert!(ctx.sent.iter().any(|message| {
        message["method"] == json!("Network.loadingFinished")
            && message["params"]["requestId"] == json!(second_request_id)
    }));

    ctx.process_async(json!({
        "id": 2261,
        "method": "Network.getResponseBody",
        "sessionId": session_id,
        "params": { "requestId": first_request_id }
    }))
    .await;
    ctx.expect_result(
        2261,
        json!({
            "body": "target fetch one",
            "base64Encoded": false
        }),
        Some(&session_id),
    );

    ctx.process_async(json!({
        "id": 2262,
        "method": "Network.getResponseBody",
        "sessionId": session_id,
        "params": { "requestId": second_request_id }
    }))
    .await;
    ctx.expect_result(
        2262,
        json!({
            "body": "target fetch two",
            "base64Encoded": false
        }),
        Some(&session_id),
    );

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn playwright_over_cdp_runtime_xhr_captures_multiple_subresource_responses() {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>ok</body></html>",
        )
    }

    async fn api_one() -> impl IntoResponse {
        (
            [
                (CONTENT_TYPE.as_str(), "text/plain"),
                ("x-target-xhr", "one"),
            ],
            "target xhr one",
        )
    }

    async fn api_two() -> impl IntoResponse {
        (
            [
                (CONTENT_TYPE.as_str(), "text/plain"),
                ("x-target-xhr", "two"),
            ],
            "target xhr two",
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let page_url = format!("http://{addr}/page");
    let api_one_url = format!("http://{addr}/api-one");
    let api_two_url = format!("http://{addr}/api-two");
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/page", get(page))
                .route("/api-one", get(api_one))
                .route("/api-two", get(api_two)),
        )
        .await
        .unwrap();
    });

    let mut ctx = TestContext::new();
    let session_id = create_attached_page_session_async(&mut ctx, 2263, 2264, 2265, 2266, 2267)
        .await
        .session_id;

    ctx.process_async(json!({
        "id": 2268,
        "method": "Page.navigate",
        "sessionId": session_id,
        "params": { "url": page_url }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 2268);
    assert!(
        ctx.sent
            .iter()
            .all(|message| message.get("error").is_none()),
        "unexpected protocol error during page navigation: {:?}",
        ctx.sent
    );
    ctx.take_all();

    ctx.process_async(json!({
            "id": 2269,
            "method": "Runtime.evaluate",
            "sessionId": session_id,
            "params": {
                "expression": "Promise.all(['/api-one', '/api-two'].map(path => new Promise(resolve => { const xhr = new XMLHttpRequest(); xhr.open('GET', path); xhr.onload = () => resolve(xhr.responseText); xhr.send(); })))"
            }
        })).await;
    let evaluation = take_response_by_id(&mut ctx, 2269);
    assert_eq!(evaluation["id"], json!(2269));
    flush_until_playwright_subresources_finished(
        &mut ctx,
        &session_id,
        "XHR",
        2,
        "playwright runtime xhr network completions",
    )
    .await;

    let xhr_requests = ctx
        .sent
        .iter()
        .filter(|message| {
            message["method"] == json!("Network.requestWillBeSent")
                && message["params"]["type"] == json!("XHR")
        })
        .cloned()
        .collect::<Vec<_>>();
    assert_eq!(xhr_requests.len(), 2, "{xhr_requests:?}");

    let first_request = xhr_requests
        .iter()
        .find(|message| message["params"]["request"]["url"] == json!(api_one_url))
        .expect("api-one xhr request should be emitted");
    let second_request = xhr_requests
        .iter()
        .find(|message| message["params"]["request"]["url"] == json!(api_two_url))
        .expect("api-two xhr request should be emitted");
    let first_request_id = first_request["params"]["requestId"]
        .as_str()
        .expect("first xhr request id")
        .to_owned();
    let second_request_id = second_request["params"]["requestId"]
        .as_str()
        .expect("second xhr request id")
        .to_owned();
    assert_ne!(first_request_id, second_request_id);

    let first_response = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Network.responseReceived")
                && message["params"]["requestId"] == json!(first_request_id)
        })
        .cloned()
        .expect("api-one xhr response event");
    let second_response = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Network.responseReceived")
                && message["params"]["requestId"] == json!(second_request_id)
        })
        .cloned()
        .expect("api-two xhr response event");
    assert_eq!(first_response["params"]["type"], json!("XHR"));
    assert_eq!(second_response["params"]["type"], json!("XHR"));
    assert_eq!(
        first_response["params"]["response"]["headers"]["x-target-xhr"],
        json!("one")
    );
    assert_eq!(
        second_response["params"]["response"]["headers"]["x-target-xhr"],
        json!("two")
    );

    assert!(ctx.sent.iter().any(|message| {
        message["method"] == json!("Network.loadingFinished")
            && message["params"]["requestId"] == json!(first_request_id)
    }));
    assert!(ctx.sent.iter().any(|message| {
        message["method"] == json!("Network.loadingFinished")
            && message["params"]["requestId"] == json!(second_request_id)
    }));

    ctx.process_async(json!({
        "id": 2270,
        "method": "Network.getResponseBody",
        "sessionId": session_id,
        "params": { "requestId": first_request_id }
    }))
    .await;
    ctx.expect_result(
        2270,
        json!({
            "body": "target xhr one",
            "base64Encoded": false
        }),
        Some(&session_id),
    );

    ctx.process_async(json!({
        "id": 2271,
        "method": "Network.getResponseBody",
        "sessionId": session_id,
        "params": { "requestId": second_request_id }
    }))
    .await;
    ctx.expect_result(
        2271,
        json!({
            "body": "target xhr two",
            "base64Encoded": false
        }),
        Some(&session_id),
    );

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn playwright_over_cdp_runtime_mixed_fetch_and_xhr_capture_subresource_responses() {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>ok</body></html>",
        )
    }

    async fn fetch_api() -> impl IntoResponse {
        (
            [
                (CONTENT_TYPE.as_str(), "application/json"),
                ("x-target-kind", "fetch"),
            ],
            "{\"kind\":\"fetch\"}",
        )
    }

    async fn xhr_api() -> impl IntoResponse {
        (
            [
                (CONTENT_TYPE.as_str(), "text/plain"),
                ("x-target-kind", "xhr"),
            ],
            "xhr body",
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let page_url = format!("http://{addr}/page");
    let fetch_url = format!("http://{addr}/fetch-api");
    let xhr_url = format!("http://{addr}/xhr-api");
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/page", get(page))
                .route("/fetch-api", get(fetch_api))
                .route("/xhr-api", get(xhr_api)),
        )
        .await
        .unwrap();
    });

    let mut ctx = TestContext::new();
    let session_id = create_attached_page_session_async(&mut ctx, 2287, 2288, 2289, 2290, 2291)
        .await
        .session_id;

    ctx.process_async(json!({
        "id": 2292,
        "method": "Page.navigate",
        "sessionId": session_id,
        "params": { "url": page_url }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 2292);
    assert!(
        ctx.sent
            .iter()
            .all(|message| message.get("error").is_none()),
        "unexpected protocol error during page navigation: {:?}",
        ctx.sent
    );
    ctx.take_all();

    ctx.process_async(json!({
            "id": 2293,
            "method": "Runtime.evaluate",
            "sessionId": session_id,
            "params": {
                "expression": "Promise.all([fetch('/fetch-api').then(r => r.text()), new Promise(resolve => { const xhr = new XMLHttpRequest(); xhr.open('GET', '/xhr-api'); xhr.onload = () => resolve(xhr.responseText); xhr.send(); })])"
            }
        })).await;
    let evaluation = take_response_by_id(&mut ctx, 2293);
    assert_eq!(evaluation["id"], json!(2293));
    flush_until_playwright_subresources_finished(
        &mut ctx,
        &session_id,
        "Fetch",
        1,
        "playwright mixed runtime fetch network completion",
    )
    .await;
    flush_until_playwright_subresources_finished(
        &mut ctx,
        &session_id,
        "XHR",
        1,
        "playwright mixed runtime xhr network completion",
    )
    .await;

    let fetch_request = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Network.requestWillBeSent")
                && message["params"]["type"] == json!("Fetch")
                && message["params"]["request"]["url"] == json!(fetch_url)
        })
        .cloned()
        .expect("fetch request should be emitted");
    let xhr_request = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Network.requestWillBeSent")
                && message["params"]["type"] == json!("XHR")
                && message["params"]["request"]["url"] == json!(xhr_url)
        })
        .cloned()
        .expect("xhr request should be emitted");
    let fetch_request_id = fetch_request["params"]["requestId"]
        .as_str()
        .expect("fetch request id")
        .to_owned();
    let xhr_request_id = xhr_request["params"]["requestId"]
        .as_str()
        .expect("xhr request id")
        .to_owned();
    assert_ne!(fetch_request_id, xhr_request_id);

    let fetch_response = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Network.responseReceived")
                && message["params"]["requestId"] == json!(fetch_request_id)
        })
        .cloned()
        .expect("fetch response should be emitted");
    let xhr_response = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Network.responseReceived")
                && message["params"]["requestId"] == json!(xhr_request_id)
        })
        .cloned()
        .expect("xhr response should be emitted");
    assert_eq!(fetch_response["params"]["type"], json!("Fetch"));
    assert_eq!(xhr_response["params"]["type"], json!("XHR"));
    assert_eq!(
        fetch_response["params"]["response"]["headers"]["x-target-kind"],
        json!("fetch")
    );
    assert_eq!(
        xhr_response["params"]["response"]["headers"]["x-target-kind"],
        json!("xhr")
    );

    assert!(ctx.sent.iter().any(|message| {
        message["method"] == json!("Network.loadingFinished")
            && message["params"]["requestId"] == json!(fetch_request_id)
    }));
    assert!(ctx.sent.iter().any(|message| {
        message["method"] == json!("Network.loadingFinished")
            && message["params"]["requestId"] == json!(xhr_request_id)
    }));

    ctx.process_async(json!({
        "id": 2294,
        "method": "Network.getResponseBody",
        "sessionId": session_id,
        "params": { "requestId": fetch_request_id }
    }))
    .await;
    ctx.expect_result(
        2294,
        json!({
            "body": "{\"kind\":\"fetch\"}",
            "base64Encoded": false
        }),
        Some(&session_id),
    );

    ctx.process_async(json!({
        "id": 2295,
        "method": "Network.getResponseBody",
        "sessionId": session_id,
        "params": { "requestId": xhr_request_id }
    }))
    .await;
    ctx.expect_result(
        2295,
        json!({
            "body": "xhr body",
            "base64Encoded": false
        }),
        Some(&session_id),
    );

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn playwright_over_cdp_capture_xhr_filter_surface_distinguishes_target_responses() {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>ok</body></html>",
        )
    }

    async fn capture_fetch() -> impl IntoResponse {
        (
            [
                (CONTENT_TYPE.as_str(), "application/json"),
                ("x-target-kind", "capture-fetch"),
            ],
            "{\"kind\":\"capture-fetch\"}",
        )
    }

    async fn ignore_fetch() -> impl IntoResponse {
        (
            [
                (CONTENT_TYPE.as_str(), "application/json"),
                ("x-target-kind", "ignore-fetch"),
            ],
            "{\"kind\":\"ignore-fetch\"}",
        )
    }

    async fn capture_xhr() -> impl IntoResponse {
        (
            [
                (CONTENT_TYPE.as_str(), "text/plain"),
                ("x-target-kind", "capture-xhr"),
            ],
            "capture xhr body",
        )
    }

    async fn ignore_xhr() -> impl IntoResponse {
        (
            [
                (CONTENT_TYPE.as_str(), "text/plain"),
                ("x-target-kind", "ignore-xhr"),
            ],
            "ignore xhr body",
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let page_url = format!("http://{addr}/page");
    let capture_fetch_url = format!("http://{addr}/capture-fetch");
    let ignore_fetch_url = format!("http://{addr}/ignore-fetch");
    let capture_xhr_url = format!("http://{addr}/capture-xhr");
    let ignore_xhr_url = format!("http://{addr}/ignore-xhr");
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/page", get(page))
                .route("/capture-fetch", get(capture_fetch))
                .route("/ignore-fetch", get(ignore_fetch))
                .route("/capture-xhr", get(capture_xhr))
                .route("/ignore-xhr", get(ignore_xhr)),
        )
        .await
        .unwrap();
    });

    let mut ctx = TestContext::new();
    let session_id = create_attached_page_session_async(&mut ctx, 2296, 2297, 2298, 2299, 2300)
        .await
        .session_id;

    ctx.process_async(json!({
        "id": 2301,
        "method": "Page.navigate",
        "sessionId": session_id,
        "params": { "url": page_url }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 2301);
    assert!(
        ctx.sent
            .iter()
            .all(|message| message.get("error").is_none()),
        "unexpected protocol error during page navigation: {:?}",
        ctx.sent
    );
    assert!(ctx.sent.iter().any(|message| {
        message["method"] == json!("Network.responseReceived")
            && message["params"]["type"] == json!("Document")
            && message["params"]["response"]["url"] == json!(page_url)
    }));
    ctx.take_all();

    ctx.process_async(json!({
            "id": 2302,
            "method": "Runtime.evaluate",
            "sessionId": session_id,
            "params": {
                "expression": "Promise.all([fetch('/capture-fetch').then(r => r.text()), fetch('/ignore-fetch').then(r => r.text()), new Promise(resolve => { const xhr = new XMLHttpRequest(); xhr.open('GET', '/capture-xhr'); xhr.onload = () => resolve(xhr.responseText); xhr.send(); }), new Promise(resolve => { const xhr = new XMLHttpRequest(); xhr.open('GET', '/ignore-xhr'); xhr.onload = () => resolve(xhr.responseText); xhr.send(); })])"
            }
    })).await;
    let evaluation = take_response_by_id(&mut ctx, 2302);
    assert_eq!(evaluation["id"], json!(2302));
    flush_until_playwright_subresources_finished(
        &mut ctx,
        &session_id,
        "Fetch",
        2,
        "playwright capture fetch completions",
    )
    .await;
    flush_until_playwright_subresources_finished(
        &mut ctx,
        &session_id,
        "XHR",
        2,
        "playwright capture xhr completions",
    )
    .await;

    let subresource_responses = ctx
        .sent
        .iter()
        .filter(|message| {
            message["method"] == json!("Network.responseReceived")
                && matches!(
                    message["params"]["type"].as_str(),
                    Some("Fetch") | Some("XHR")
                )
        })
        .cloned()
        .collect::<Vec<_>>();
    assert_eq!(subresource_responses.len(), 4, "{subresource_responses:?}");

    let matched = subresource_responses
        .iter()
        .filter(|message| {
            let url = message["params"]["response"]["url"]
                .as_str()
                .unwrap_or_default();
            url.contains("capture")
        })
        .cloned()
        .collect::<Vec<_>>();
    assert_eq!(matched.len(), 2, "{matched:?}");
    assert!(matched.iter().any(|message| {
        message["params"]["type"] == json!("Fetch")
            && message["params"]["response"]["url"] == json!(capture_fetch_url)
            && message["params"]["response"]["headers"]["x-target-kind"] == json!("capture-fetch")
    }));
    assert!(matched.iter().any(|message| {
        message["params"]["type"] == json!("XHR")
            && message["params"]["response"]["url"] == json!(capture_xhr_url)
            && message["params"]["response"]["headers"]["x-target-kind"] == json!("capture-xhr")
    }));
    assert!(!matched.iter().any(|message| {
        message["params"]["response"]["url"] == json!(ignore_fetch_url)
            || message["params"]["response"]["url"] == json!(ignore_xhr_url)
            || message["params"]["response"]["url"] == json!(page_url)
    }));

    let capture_fetch_request_id = matched
        .iter()
        .find(|message| message["params"]["response"]["url"] == json!(capture_fetch_url))
        .and_then(|message| message["params"]["requestId"].as_str())
        .expect("capture fetch request id")
        .to_owned();
    let capture_xhr_request_id = matched
        .iter()
        .find(|message| message["params"]["response"]["url"] == json!(capture_xhr_url))
        .and_then(|message| message["params"]["requestId"].as_str())
        .expect("capture xhr request id")
        .to_owned();

    ctx.process_async(json!({
        "id": 2303,
        "method": "Network.getResponseBody",
        "sessionId": session_id,
        "params": { "requestId": capture_fetch_request_id }
    }))
    .await;
    ctx.expect_result(
        2303,
        json!({
            "body": "{\"kind\":\"capture-fetch\"}",
            "base64Encoded": false
        }),
        Some(&session_id),
    );

    ctx.process_async(json!({
        "id": 2304,
        "method": "Network.getResponseBody",
        "sessionId": session_id,
        "params": { "requestId": capture_xhr_request_id }
    }))
    .await;
    ctx.expect_result(
        2304,
        json!({
            "body": "capture xhr body",
            "base64Encoded": false
        }),
        Some(&session_id),
    );

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn playwright_over_cdp_redirect_and_capture_xhr_flows_do_not_interfere() {
    async fn redirect_handler(
        axum::extract::State(page_url): axum::extract::State<String>,
    ) -> impl IntoResponse {
        Redirect::temporary(&page_url)
    }

    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>redirected page</body></html>",
        )
    }

    async fn capture_fetch() -> impl IntoResponse {
        (
            [
                (CONTENT_TYPE.as_str(), "application/json"),
                ("x-target-kind", "capture-fetch"),
            ],
            "{\"kind\":\"capture-fetch\"}",
        )
    }

    async fn capture_xhr() -> impl IntoResponse {
        (
            [
                (CONTENT_TYPE.as_str(), "text/plain"),
                ("x-target-kind", "capture-xhr"),
            ],
            "capture xhr body",
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let start_url = format!("http://{addr}/start");
    let page_url = format!("http://{addr}/page");
    let capture_fetch_url = format!("http://{addr}/capture-fetch");
    let capture_xhr_url = format!("http://{addr}/capture-xhr");
    let server_page_url = page_url.clone();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/start", get(redirect_handler))
                .route("/page", get(page))
                .route("/capture-fetch", get(capture_fetch))
                .route("/capture-xhr", get(capture_xhr))
                .with_state(server_page_url),
        )
        .await
        .unwrap();
    });

    let mut ctx = TestContext::new();
    let session_id = create_attached_page_session_async(&mut ctx, 2305, 2306, 2307, 2308, 2309)
        .await
        .session_id;

    ctx.process_async(json!({
        "id": 2310,
        "method": "Page.navigate",
        "sessionId": session_id,
        "params": { "url": start_url }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 2310);
    assert!(
        ctx.sent
            .iter()
            .all(|message| message.get("error").is_none()),
        "unexpected protocol error during redirect navigation: {:?}",
        ctx.sent
    );

    let emitted = ctx.take_all();
    let document_request = emitted
        .iter()
        .find(|message| {
            message["method"] == json!("Network.requestWillBeSent")
                && message["params"]["request"]["url"] == json!(start_url)
        })
        .expect("redirect start request should be emitted");
    let document_request_id = document_request["params"]["requestId"]
        .as_str()
        .expect("document request id")
        .to_owned();

    let redirected_request = emitted
        .iter()
        .find(|message| {
            message["method"] == json!("Network.requestWillBeSent")
                && message["params"]["requestId"] == json!(document_request_id)
                && message["params"]["request"]["url"] == json!(page_url)
        })
        .expect("redirect target request should be emitted");
    assert_eq!(
        redirected_request["params"]["redirectResponse"]["url"],
        json!(start_url)
    );
    assert_eq!(
        redirected_request["params"]["redirectResponse"]["status"],
        json!(307)
    );

    let final_document_response = emitted
        .iter()
        .find(|message| {
            message["method"] == json!("Network.responseReceived")
                && message["params"]["requestId"] == json!(document_request_id)
        })
        .cloned()
        .expect("final document response should be emitted");
    assert_eq!(final_document_response["params"]["type"], json!("Document"));
    assert_eq!(
        final_document_response["params"]["response"]["url"],
        json!(page_url)
    );

    ctx.process_async(json!({
            "id": 2311,
            "method": "Runtime.evaluate",
            "sessionId": session_id,
            "params": {
                "expression": "Promise.all([fetch('/capture-fetch').then(r => r.text()), new Promise(resolve => { const xhr = new XMLHttpRequest(); xhr.open('GET', '/capture-xhr'); xhr.onload = () => resolve(xhr.responseText); xhr.send(); })])"
            }
    })).await;
    let evaluation = take_response_by_id(&mut ctx, 2311);
    assert_eq!(evaluation["id"], json!(2311));
    flush_until_playwright_subresources_finished(
        &mut ctx,
        &session_id,
        "Fetch",
        1,
        "playwright redirected capture fetch completion",
    )
    .await;
    flush_until_playwright_subresources_finished(
        &mut ctx,
        &session_id,
        "XHR",
        1,
        "playwright redirected capture xhr completion",
    )
    .await;

    let subresource_responses = ctx
        .sent
        .iter()
        .filter(|message| {
            message["method"] == json!("Network.responseReceived")
                && matches!(
                    message["params"]["type"].as_str(),
                    Some("Fetch") | Some("XHR")
                )
        })
        .cloned()
        .collect::<Vec<_>>();
    assert_eq!(subresource_responses.len(), 2, "{subresource_responses:?}");
    assert!(
        subresource_responses
            .iter()
            .all(|message| { message["params"]["requestId"] != json!(document_request_id) })
    );

    let fetch_response = subresource_responses
        .iter()
        .find(|message| message["params"]["type"] == json!("Fetch"))
        .expect("fetch response should be emitted");
    let xhr_response = subresource_responses
        .iter()
        .find(|message| message["params"]["type"] == json!("XHR"))
        .expect("xhr response should be emitted");
    assert_eq!(
        fetch_response["params"]["response"]["url"],
        json!(capture_fetch_url)
    );
    assert_eq!(
        xhr_response["params"]["response"]["url"],
        json!(capture_xhr_url)
    );

    let fetch_request_id = fetch_response["params"]["requestId"]
        .as_str()
        .expect("fetch request id")
        .to_owned();
    let xhr_request_id = xhr_response["params"]["requestId"]
        .as_str()
        .expect("xhr request id")
        .to_owned();

    ctx.process_async(json!({
        "id": 2312,
        "method": "Network.getResponseBody",
        "sessionId": session_id,
        "params": { "requestId": document_request_id }
    }))
    .await;
    ctx.expect_result(
        2312,
        json!({
            "body": "<!doctype html><html><body>redirected page</body></html>",
            "base64Encoded": false
        }),
        Some(&session_id),
    );

    ctx.process_async(json!({
        "id": 2313,
        "method": "Network.getResponseBody",
        "sessionId": session_id,
        "params": { "requestId": fetch_request_id }
    }))
    .await;
    ctx.expect_result(
        2313,
        json!({
            "body": "{\"kind\":\"capture-fetch\"}",
            "base64Encoded": false
        }),
        Some(&session_id),
    );

    ctx.process_async(json!({
        "id": 2314,
        "method": "Network.getResponseBody",
        "sessionId": session_id,
        "params": { "requestId": xhr_request_id }
    }))
    .await;
    ctx.expect_result(
        2314,
        json!({
            "body": "capture xhr body",
            "base64Encoded": false
        }),
        Some(&session_id),
    );

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn playwright_over_cdp_context_cookie_flows_through_redirect_and_subresources() {
    async fn redirect_handler(
        headers: HeaderMap,
        axum::extract::State(state): axum::extract::State<(
            String,
            Arc<Mutex<Vec<(String, Option<String>)>>>,
        )>,
    ) -> impl IntoResponse {
        let (page_url, seen) = state;
        let cookie = headers
            .get(axum::http::header::COOKIE)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        seen.lock().push(("/start".to_owned(), cookie));
        Redirect::temporary(&page_url)
    }

    async fn page_handler(
        headers: HeaderMap,
        axum::extract::State(state): axum::extract::State<(
            String,
            Arc<Mutex<Vec<(String, Option<String>)>>>,
        )>,
    ) -> impl IntoResponse {
        let (_, seen) = state;
        let cookie = headers
            .get(axum::http::header::COOKIE)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        seen.lock().push(("/page".to_owned(), cookie));
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>redirected page</body></html>",
        )
    }

    async fn capture_fetch_handler(
        headers: HeaderMap,
        axum::extract::State(state): axum::extract::State<(
            String,
            Arc<Mutex<Vec<(String, Option<String>)>>>,
        )>,
    ) -> impl IntoResponse {
        let (_, seen) = state;
        let cookie = headers
            .get(axum::http::header::COOKIE)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        seen.lock().push(("/capture-fetch".to_owned(), cookie));
        (
            [
                (CONTENT_TYPE.as_str(), "application/json"),
                ("x-target-kind", "capture-fetch"),
            ],
            "{\"kind\":\"capture-fetch\"}",
        )
    }

    async fn capture_xhr_handler(
        headers: HeaderMap,
        axum::extract::State(state): axum::extract::State<(
            String,
            Arc<Mutex<Vec<(String, Option<String>)>>>,
        )>,
    ) -> impl IntoResponse {
        let (_, seen) = state;
        let cookie = headers
            .get(axum::http::header::COOKIE)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        seen.lock().push(("/capture-xhr".to_owned(), cookie));
        (
            [
                (CONTENT_TYPE.as_str(), "text/plain"),
                ("x-target-kind", "capture-xhr"),
            ],
            "capture xhr body",
        )
    }

    let seen = Arc::new(Mutex::new(Vec::<(String, Option<String>)>::new()));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let start_url = format!("http://{addr}/start");
    let page_url = format!("http://{addr}/page");
    let capture_fetch_url = format!("http://{addr}/capture-fetch");
    let capture_xhr_url = format!("http://{addr}/capture-xhr");
    let server_state = (page_url.clone(), Arc::clone(&seen));
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/start", get(redirect_handler))
                .route("/page", get(page_handler))
                .route("/capture-fetch", get(capture_fetch_handler))
                .route("/capture-xhr", get(capture_xhr_handler))
                .with_state(server_state),
        )
        .await
        .unwrap();
    });

    let mut ctx = TestContext::new();
    let attached = create_attached_page_session_async(&mut ctx, 2315, 2317, 2318, 2319, 2320).await;
    let browser_context_id = attached.browser_context_id;
    let session_id = attached.session_id;

    ctx.process_async(json!({
        "id": 2316,
        "method": "Storage.setCookies",
        "params": {
            "browserContextId": browser_context_id,
            "cookies": [{
                "name": "sid",
                "value": "seeded",
                "url": page_url
            }]
        }
    }))
    .await;
    ctx.expect_result(
        2316,
        json!({
            "success": true,
            "cookieReports": [{
                "status": { "kind": "Accepted", "storeAction": "Inserted" },
                "rejectionReasons": [],
                "effectiveSameSite": "NoRestriction",
                "warningReasons": []
            }]
        }),
        None,
    );
    ctx.process_async(json!({
        "id": 2321,
        "method": "Page.navigate",
        "sessionId": session_id,
        "params": { "url": start_url }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 2321);
    assert!(
        ctx.sent
            .iter()
            .all(|message| message.get("error").is_none()),
        "unexpected protocol error during redirect navigation: {:?}",
        ctx.sent
    );

    let emitted = ctx.take_all();
    let document_request = emitted
        .iter()
        .find(|message| {
            message["method"] == json!("Network.requestWillBeSent")
                && message["params"]["request"]["url"] == json!(start_url)
        })
        .expect("redirect start request should be emitted");
    let document_request_id = document_request["params"]["requestId"]
        .as_str()
        .expect("document request id")
        .to_owned();

    let redirected_request = emitted
        .iter()
        .find(|message| {
            message["method"] == json!("Network.requestWillBeSent")
                && message["params"]["requestId"] == json!(document_request_id)
                && message["params"]["request"]["url"] == json!(page_url)
        })
        .expect("redirect target request should be emitted");
    assert_eq!(
        redirected_request["params"]["redirectResponse"]["url"],
        json!(start_url)
    );

    ctx.process_async(json!({
            "id": 2322,
            "method": "Runtime.evaluate",
            "sessionId": session_id,
            "params": {
                "expression": "Promise.all([fetch('/capture-fetch').then(r => r.text()), new Promise(resolve => { const xhr = new XMLHttpRequest(); xhr.open('GET', '/capture-xhr'); xhr.onload = () => resolve(xhr.responseText); xhr.send(); })])"
            }
    })).await;
    let evaluation = take_response_by_id(&mut ctx, 2322);
    assert_eq!(evaluation["id"], json!(2322));
    flush_until_playwright_subresources_finished(
        &mut ctx,
        &session_id,
        "Fetch",
        1,
        "playwright cookie flow fetch completion",
    )
    .await;
    flush_until_playwright_subresources_finished(
        &mut ctx,
        &session_id,
        "XHR",
        1,
        "playwright cookie flow xhr completion",
    )
    .await;

    let subresource_responses = ctx
        .sent
        .iter()
        .filter(|message| {
            message["method"] == json!("Network.responseReceived")
                && matches!(
                    message["params"]["type"].as_str(),
                    Some("Fetch") | Some("XHR")
                )
        })
        .cloned()
        .collect::<Vec<_>>();
    assert_eq!(subresource_responses.len(), 2, "{subresource_responses:?}");

    let fetch_request_id = subresource_responses
        .iter()
        .find(|message| {
            message["params"]["type"] == json!("Fetch")
                && message["params"]["response"]["url"] == json!(capture_fetch_url)
        })
        .and_then(|message| message["params"]["requestId"].as_str())
        .expect("fetch request id")
        .to_owned();
    let xhr_request_id = subresource_responses
        .iter()
        .find(|message| {
            message["params"]["type"] == json!("XHR")
                && message["params"]["response"]["url"] == json!(capture_xhr_url)
        })
        .and_then(|message| message["params"]["requestId"].as_str())
        .expect("xhr request id")
        .to_owned();

    ctx.process_async(json!({
        "id": 2323,
        "method": "Network.getResponseBody",
        "sessionId": session_id,
        "params": { "requestId": document_request_id }
    }))
    .await;
    ctx.expect_result(
        2323,
        json!({
            "body": "<!doctype html><html><body>redirected page</body></html>",
            "base64Encoded": false
        }),
        Some(&session_id),
    );

    ctx.process_async(json!({
        "id": 2324,
        "method": "Network.getResponseBody",
        "sessionId": session_id,
        "params": { "requestId": fetch_request_id }
    }))
    .await;
    ctx.expect_result(
        2324,
        json!({
            "body": "{\"kind\":\"capture-fetch\"}",
            "base64Encoded": false
        }),
        Some(&session_id),
    );

    ctx.process_async(json!({
        "id": 2325,
        "method": "Network.getResponseBody",
        "sessionId": session_id,
        "params": { "requestId": xhr_request_id }
    }))
    .await;
    ctx.expect_result(
        2325,
        json!({
            "body": "capture xhr body",
            "base64Encoded": false
        }),
        Some(&session_id),
    );

    let seen = seen.lock().clone();
    assert_eq!(
        seen.get(..2),
        Some(
            [
                ("/start".to_owned(), Some("sid=seeded".to_owned())),
                ("/page".to_owned(), Some("sid=seeded".to_owned())),
            ]
            .as_slice()
        )
    );
    let mut subresources = seen[2..].to_vec();
    subresources.sort();
    assert_eq!(
        subresources,
        vec![
            ("/capture-fetch".to_owned(), Some("sid=seeded".to_owned())),
            ("/capture-xhr".to_owned(), Some("sid=seeded".to_owned())),
        ]
    );

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn playwright_over_cdp_response_set_cookies_surface_in_context_after_subresources() {
    async fn redirect_handler() -> impl IntoResponse {
        Redirect::temporary("/page")
    }

    async fn page_handler(
        headers: HeaderMap,
        axum::extract::State(seen): axum::extract::State<Arc<Mutex<Vec<(String, Option<String>)>>>>,
    ) -> impl IntoResponse {
        let cookie = headers
            .get(axum::http::header::COOKIE)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        seen.lock().push(("/page".to_owned(), cookie));
        (
            [
                (CONTENT_TYPE.as_str(), "text/html"),
                ("set-cookie", "sid=page; Path=/"),
            ],
            "<!doctype html><html><body>page cookie</body></html>",
        )
    }

    async fn capture_fetch_handler(
        headers: HeaderMap,
        axum::extract::State(seen): axum::extract::State<Arc<Mutex<Vec<(String, Option<String>)>>>>,
    ) -> impl IntoResponse {
        let cookie = headers
            .get(axum::http::header::COOKIE)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        seen.lock().push(("/capture-fetch".to_owned(), cookie));
        (
            [
                (CONTENT_TYPE.as_str(), "application/json"),
                ("set-cookie", "fetchid=1; Path=/"),
            ],
            "{\"kind\":\"capture-fetch\"}",
        )
    }

    async fn capture_xhr_handler(
        headers: HeaderMap,
        axum::extract::State(seen): axum::extract::State<Arc<Mutex<Vec<(String, Option<String>)>>>>,
    ) -> impl IntoResponse {
        let cookie = headers
            .get(axum::http::header::COOKIE)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        seen.lock().push(("/capture-xhr".to_owned(), cookie));
        (
            [
                (CONTENT_TYPE.as_str(), "text/plain"),
                ("set-cookie", "xhrid=1; Path=/"),
            ],
            "capture xhr body",
        )
    }

    let seen = Arc::new(Mutex::new(Vec::<(String, Option<String>)>::new()));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let start_url = format!("http://{addr}/start");
    let server_seen = Arc::clone(&seen);
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/start", get(redirect_handler))
                .route("/page", get(page_handler))
                .route("/capture-fetch", get(capture_fetch_handler))
                .route("/capture-xhr", get(capture_xhr_handler))
                .with_state(server_seen),
        )
        .await
        .unwrap();
    });

    let mut ctx = TestContext::new();
    let attached = create_attached_page_session_async(&mut ctx, 2326, 2327, 2328, 2329, 2330).await;
    let browser_context_id = attached.browser_context_id;
    let session_id = attached.session_id;

    ctx.process_async(json!({
        "id": 2331,
        "method": "Page.navigate",
        "sessionId": session_id,
        "params": { "url": start_url }
    }))
    .await;
    let response = take_response_by_id(&mut ctx, 2331);
    assert_eq!(response["sessionId"], json!(session_id));
    assert!(
        ctx.sent
            .iter()
            .all(|message| message.get("error").is_none()),
        "unexpected protocol error during navigation: {:?}",
        ctx.sent
    );
    ctx.take_all();

    ctx.process_async(json!({
            "id": 2332,
            "method": "Runtime.evaluate",
            "sessionId": session_id,
            "params": {
                "expression": "fetch('/capture-fetch').then(r => r.text()).then(() => new Promise(resolve => { const xhr = new XMLHttpRequest(); xhr.open('GET', '/capture-xhr'); xhr.onload = () => resolve(xhr.responseText); xhr.send(); }))"
            }
    })).await;
    let evaluation = take_response_by_id(&mut ctx, 2332);
    assert_eq!(evaluation["id"], json!(2332));
    flush_until_playwright_subresources_finished(
        &mut ctx,
        &session_id,
        "Fetch",
        1,
        "playwright set-cookie fetch completion",
    )
    .await;
    flush_until_playwright_subresources_finished(
        &mut ctx,
        &session_id,
        "XHR",
        1,
        "playwright set-cookie xhr completion",
    )
    .await;

    ctx.process_async(json!({
        "id": 2333,
        "method": "Storage.getCookies",
        "params": { "browserContextId": browser_context_id }
    }))
    .await;
    let cookies_response = take_response_by_id(&mut ctx, 2333);
    let mut stored_cookies = cookies_response["result"]["cookies"]
        .as_array()
        .expect("cookies array")
        .iter()
        .map(|cookie| {
            (
                cookie["name"].as_str().expect("cookie name").to_owned(),
                cookie["value"].as_str().expect("cookie value").to_owned(),
            )
        })
        .collect::<Vec<_>>();
    stored_cookies.sort();
    assert_eq!(
        stored_cookies,
        vec![
            ("fetchid".to_owned(), "1".to_owned()),
            ("sid".to_owned(), "page".to_owned()),
            ("xhrid".to_owned(), "1".to_owned()),
        ]
    );

    let seen_requests = seen.lock().clone();
    assert_eq!(seen_requests.len(), 3, "{seen_requests:?}");
    assert_eq!(
        seen_requests
            .iter()
            .find(|(path, _)| path == "/page")
            .expect("page request should be seen")
            .1,
        None
    );
    assert_eq!(
        seen_requests
            .iter()
            .find(|(path, _)| path == "/capture-fetch")
            .expect("fetch request should be seen")
            .1
            .as_deref(),
        Some("sid=page")
    );
    assert_eq!(
        seen_requests
            .iter()
            .find(|(path, _)| path == "/capture-xhr")
            .expect("xhr request should be seen")
            .1
            .as_deref(),
        Some("sid=page; fetchid=1")
    );

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn playwright_over_cdp_redirect_set_cookie_populates_history_and_context_surface() {
    async fn redirect_handler() -> impl IntoResponse {
        (
            [
                ("location", "/page"),
                ("set-cookie", "rid=1; Path=/"),
                (CONTENT_TYPE.as_str(), "text/plain"),
            ],
            StatusCode::TEMPORARY_REDIRECT,
        )
    }

    async fn page_handler(
        headers: HeaderMap,
        axum::extract::State(seen): axum::extract::State<Arc<Mutex<Vec<(String, Option<String>)>>>>,
    ) -> impl IntoResponse {
        let cookie = headers
            .get(axum::http::header::COOKIE)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        seen.lock().push(("/page".to_owned(), cookie));
        (
            [
                (CONTENT_TYPE.as_str(), "text/html"),
                ("set-cookie", "pid=2; Path=/"),
            ],
            "<!doctype html><html><body>redirected page</body></html>",
        )
    }

    async fn capture_handler(
        headers: HeaderMap,
        axum::extract::State(seen): axum::extract::State<Arc<Mutex<Vec<(String, Option<String>)>>>>,
    ) -> impl IntoResponse {
        let cookie = headers
            .get(axum::http::header::COOKIE)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        seen.lock().push(("/capture".to_owned(), cookie));
        (
            [(CONTENT_TYPE.as_str(), "application/json")],
            "{\"kind\":\"capture\"}",
        )
    }

    let seen = Arc::new(Mutex::new(Vec::<(String, Option<String>)>::new()));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let start_url = format!("http://{addr}/start");
    let page_url = format!("http://{addr}/page");
    let capture_url = format!("http://{addr}/capture");
    let server_seen = Arc::clone(&seen);
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/start", get(redirect_handler))
                .route("/page", get(page_handler))
                .route("/capture", get(capture_handler))
                .with_state(server_seen),
        )
        .await
        .unwrap();
    });

    let mut ctx = TestContext::new();
    let attached = create_attached_page_session_async(&mut ctx, 2334, 2335, 2336, 2337, 2338).await;
    let browser_context_id = attached.browser_context_id;
    let session_id = attached.session_id;

    ctx.process_async(json!({
        "id": 2339,
        "method": "Page.navigate",
        "sessionId": session_id,
        "params": { "url": start_url }
    }))
    .await;
    let response = take_response_by_id(&mut ctx, 2339);
    assert_eq!(response["sessionId"], json!(session_id));
    assert!(
        ctx.sent
            .iter()
            .all(|message| message.get("error").is_none()),
        "unexpected protocol error during redirect navigation: {:?}",
        ctx.sent
    );

    let emitted = ctx.take_all();
    let document_request = emitted
        .iter()
        .find(|message| {
            message["method"] == json!("Network.requestWillBeSent")
                && message["params"]["request"]["url"] == json!(start_url)
        })
        .expect("redirect start request should be emitted");
    let document_request_id = document_request["params"]["requestId"]
        .as_str()
        .expect("document request id")
        .to_owned();

    let redirected_request = emitted
        .iter()
        .find(|message| {
            message["method"] == json!("Network.requestWillBeSent")
                && message["params"]["requestId"] == json!(document_request_id)
                && message["params"]["request"]["url"] == json!(page_url)
        })
        .expect("redirect target request should be emitted");
    assert_eq!(
        redirected_request["params"]["redirectResponse"]["headers"]["location"],
        json!("/page")
    );
    assert_eq!(
        redirected_request["params"]["redirectResponse"]["headers"]["set-cookie"],
        json!("rid=1; Path=/")
    );

    ctx.process_async(json!({
        "id": 2340,
        "method": "Runtime.evaluate",
        "sessionId": session_id,
        "params": {
            "expression": "fetch('/capture').then(r => r.text())"
        }
    }))
    .await;
    let evaluation = take_response_by_id(&mut ctx, 2340);
    assert_eq!(evaluation["id"], json!(2340));
    flush_until_playwright_subresources_finished(
        &mut ctx,
        &session_id,
        "Fetch",
        1,
        "playwright redirect capture fetch completion",
    )
    .await;

    let capture_request_id = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Network.responseReceived")
                && message["params"]["response"]["url"] == json!(capture_url)
        })
        .and_then(|message| message["params"]["requestId"].as_str())
        .expect("capture request id")
        .to_owned();

    ctx.process_async(json!({
        "id": 2341,
        "method": "Network.getResponseBody",
        "sessionId": session_id,
        "params": { "requestId": capture_request_id }
    }))
    .await;
    ctx.expect_result(
        2341,
        json!({
            "body": "{\"kind\":\"capture\"}",
            "base64Encoded": false
        }),
        Some(&session_id),
    );

    ctx.process_async(json!({
        "id": 2342,
        "method": "Storage.getCookies",
        "params": { "browserContextId": browser_context_id }
    }))
    .await;
    let cookies_response = take_response_by_id(&mut ctx, 2342);
    let mut stored_cookies = cookies_response["result"]["cookies"]
        .as_array()
        .expect("cookies array")
        .iter()
        .map(|cookie| {
            (
                cookie["name"].as_str().expect("cookie name").to_owned(),
                cookie["value"].as_str().expect("cookie value").to_owned(),
            )
        })
        .collect::<Vec<_>>();
    stored_cookies.sort();
    assert_eq!(
        stored_cookies,
        vec![
            ("pid".to_owned(), "2".to_owned()),
            ("rid".to_owned(), "1".to_owned()),
        ]
    );

    let seen_requests = seen.lock().clone();
    assert_eq!(
        seen_requests,
        vec![
            ("/page".to_owned(), Some("rid=1".to_owned())),
            ("/capture".to_owned(), Some("rid=1; pid=2".to_owned())),
        ]
    );

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn playwright_over_cdp_inactive_context_seeded_cookies_apply_to_navigation() {
    async fn handler(
        headers: HeaderMap,
        axum::extract::State(seen_cookie): axum::extract::State<Arc<Mutex<Option<String>>>>,
    ) -> impl IntoResponse {
        let cookie = headers
            .get(axum::http::header::COOKIE)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        *seen_cookie.lock() = cookie;
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>cookie</body></html>",
        )
    }

    let seen_cookie = Arc::new(Mutex::new(None));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server_seen_cookie = Arc::clone(&seen_cookie);
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/page", get(handler))
                .with_state(server_seen_cookie),
        )
        .await
        .unwrap();
    });

    let page_url = format!("http://{addr}/page");
    let mut ctx = TestContext::new();

    ctx.process_async(json!({
        "id": 225,
        "method": "Target.createBrowserContext"
    }))
    .await;
    let active_browser_context_id = ctx
        .conn
        .browser_context
        .as_ref()
        .expect("active browser context should exist")
        .id
        .clone();
    ctx.expect_result(
        225,
        json!({ "browserContextId": active_browser_context_id }),
        None,
    );

    ctx.process_async(json!({
        "id": 226,
        "method": "Target.createBrowserContext"
    }))
    .await;
    let inactive_browser_context_id =
        take_response_by_id(&mut ctx, 226)["result"]["browserContextId"]
            .as_str()
            .expect("inactive browser context id")
            .to_owned();
    assert_ne!(inactive_browser_context_id, active_browser_context_id);

    ctx.process_async(json!({
        "id": 227,
        "method": "Storage.setCookies",
        "params": {
            "browserContextId": inactive_browser_context_id,
            "cookies": [{
                "name": "sid",
                "value": "seeded",
                "url": page_url
            }]
        }
    }))
    .await;
    ctx.expect_result(
        227,
        json!({
            "success": true,
            "cookieReports": [{
                "status": { "kind": "Accepted", "storeAction": "Inserted" },
                "rejectionReasons": [],
                "effectiveSameSite": "NoRestriction",
                "warningReasons": []
            }]
        }),
        None,
    );

    let attached = attach_page_session_in_existing_context_async(
        &mut ctx,
        &inactive_browser_context_id,
        228,
        229,
        230,
        231,
    )
    .await;
    let session_id = attached.session_id;

    ctx.process_async(json!({
        "id": 2290,
        "method": "Storage.getCookies",
        "params": { "browserContextId": inactive_browser_context_id }
    }))
    .await;
    let cookies = take_response_by_id(&mut ctx, 2290);
    let stored_cookie = cookies["result"]["cookies"][0].clone();
    assert_eq!(stored_cookie["name"], json!("sid"));
    assert_eq!(stored_cookie["value"], json!("seeded"));
    assert_eq!(stored_cookie["domain"], json!("127.0.0.1"));
    assert_eq!(stored_cookie["path"], json!("/"));
    assert_eq!(stored_cookie["secure"], json!(false));

    let active_cookie_report = {
        let page_url = url::Url::parse(&page_url).expect("page url should parse");
        let cookie_store = ctx
            .conn
            .browser_context
            .as_ref()
            .expect("active browser context")
            .cookie_store_for_test()
            .clone();
        let mut cookie_store = cookie_store.lock();
        cookie_store.cookie_access_report_for_request(
            &page_url,
            NetworkCookieRequestContext::top_level_navigation("GET")
                .with_site_for_cookies_url(&page_url, &page_url)
                .with_top_frame_origin_url(&page_url, &page_url),
        )
    };
    assert!(
        active_cookie_report.excluded_cookies.is_empty(),
        "{active_cookie_report:?}"
    );
    assert_eq!(active_cookie_report.included_cookies.len(), 1);
    assert_eq!(active_cookie_report.included_cookies[0].cookie.name, "sid");

    ctx.process_async(json!({
        "id": 2291,
        "method": "Page.navigate",
        "sessionId": session_id,
        "params": { "url": page_url }
    }))
    .await;
    let response = take_response_by_id(&mut ctx, 2291);
    assert_eq!(response["sessionId"], json!(session_id));
    assert!(
        ctx.sent
            .iter()
            .all(|message| message.get("error").is_none()),
        "unexpected protocol error during navigation: {:?}",
        ctx.sent
    );

    assert_eq!(seen_cookie.lock().as_deref(), Some("sid=seeded"));
    assert_eq!(
        ctx.conn.browser_context.as_ref().map(|bc| bc.id.as_str()),
        Some(inactive_browser_context_id.as_str())
    );

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn playwright_over_cdp_context_persists_response_cookies_across_navigations() {
    async fn seed_handler() -> impl IntoResponse {
        (
            [
                (CONTENT_TYPE.as_str(), "text/html"),
                ("set-cookie", "sid=server; Path=/"),
            ],
            "<!doctype html><html><body>seeded</body></html>",
        )
    }

    async fn check_handler(
        headers: HeaderMap,
        axum::extract::State(seen_cookie): axum::extract::State<Arc<Mutex<Option<String>>>>,
    ) -> impl IntoResponse {
        let cookie = headers
            .get(axum::http::header::COOKIE)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        *seen_cookie.lock() = cookie;
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>checked</body></html>",
        )
    }

    let seen_cookie = Arc::new(Mutex::new(None));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let seed_url = format!("http://{addr}/seed");
    let check_url = format!("http://{addr}/check");
    let server_seen_cookie = Arc::clone(&seen_cookie);
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/seed", get(seed_handler))
                .route("/check", get(check_handler))
                .with_state(server_seen_cookie),
        )
        .await
        .unwrap();
    });

    let mut ctx = TestContext::new();
    let attached = create_attached_page_session_async(&mut ctx, 2300, 2301, 2302, 2394, 2395).await;
    let browser_context_id = attached.browser_context_id;
    let session_id = attached.session_id;

    ctx.process_async(json!({
        "id": 2303,
        "method": "Page.navigate",
        "sessionId": session_id,
        "params": { "url": seed_url }
    }))
    .await;
    let seed_response = take_response_by_id(&mut ctx, 2303);
    assert_eq!(seed_response["sessionId"], json!(session_id));
    assert!(
        ctx.sent
            .iter()
            .all(|message| message.get("error").is_none()),
        "unexpected protocol error during seed navigation: {:?}",
        ctx.sent
    );
    ctx.take_all();

    ctx.process_async(json!({
        "id": 2304,
        "method": "Storage.getCookies",
        "params": { "browserContextId": browser_context_id }
    }))
    .await;
    let cookies = take_response_by_id(&mut ctx, 2304);
    let stored_cookie = cookies["result"]["cookies"][0].clone();
    assert_eq!(stored_cookie["name"], json!("sid"));
    assert_eq!(stored_cookie["value"], json!("server"));
    assert_eq!(stored_cookie["domain"], json!("127.0.0.1"));

    ctx.process_async(json!({
        "id": 2306,
        "method": "Page.navigate",
        "sessionId": session_id,
        "params": { "url": check_url }
    }))
    .await;
    let check_response = take_response_by_id(&mut ctx, 2306);
    assert_eq!(check_response["sessionId"], json!(session_id));
    assert!(
        ctx.sent
            .iter()
            .all(|message| message.get("error").is_none()),
        "unexpected protocol error during check navigation: {:?}",
        ctx.sent
    );

    assert_eq!(seen_cookie.lock().as_deref(), Some("sid=server"));

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn playwright_over_cdp_target_document_start_script_does_not_leak_to_new_target() {
    async fn handler() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>ok</body></html>",
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let url = format!("http://{addr}/page");
    let server = tokio::spawn(async move {
        axum::serve(listener, Router::new().route("/page", get(handler)))
            .await
            .unwrap();
    });

    let mut ctx = TestContext::new();
    let first_attached =
        create_attached_page_session_async(&mut ctx, 2310, 2311, 2312, 2390, 2313).await;
    let browser_context_id = first_attached.browser_context_id.clone();
    let first_target_id = first_attached.target_id.clone();
    let first_session_id = first_attached.session_id.clone();

    ctx.process_async(json!({
        "id": 2314,
        "method": "Page.addScriptToEvaluateOnNewDocument",
        "sessionId": first_session_id,
        "params": {
            "source": "globalThis.__lm_page_seed = (globalThis.__lm_page_seed || 0) + 1;"
        }
    }))
    .await;
    let preload = take_response_by_id(&mut ctx, 2314);
    assert!(preload["result"]["identifier"].is_string());

    ctx.process_async(json!({
        "id": 2315,
        "method": "Page.navigate",
        "sessionId": first_session_id,
        "params": { "url": url }
    }))
    .await;
    let first_navigation = take_response_by_id(&mut ctx, 2315);
    assert_eq!(first_navigation["sessionId"], json!(first_session_id));
    assert!(
        ctx.sent
            .iter()
            .all(|message| message.get("error").is_none()),
        "unexpected protocol error during first navigation: {:?}",
        ctx.sent
    );
    ctx.take_all();

    ctx.process_async(json!({
        "id": 2316,
        "method": "Runtime.evaluate",
        "sessionId": first_session_id,
        "params": {
            "expression": "globalThis.__lm_page_seed"
        }
    }))
    .await;
    let first_evaluation = take_response_by_id(&mut ctx, 2316);
    assert_eq!(first_evaluation["result"]["result"]["value"], json!(1));

    ctx.process_async(json!({
        "id": 2317,
        "method": "Target.closeTarget",
        "params": { "targetId": first_target_id }
    }))
    .await;
    ctx.expect_result(2317, json!({ "success": true }), None);
    ctx.take_all();

    let second_attached = attach_page_session_in_existing_context_async(
        &mut ctx,
        &browser_context_id,
        2318,
        2319,
        2391,
        2320,
    )
    .await;
    let second_target_id = second_attached.target_id.clone();
    assert_ne!(second_target_id, first_target_id);
    let second_session_id = second_attached.session_id.clone();

    ctx.process_async(json!({
        "id": 2321,
        "method": "Page.navigate",
        "sessionId": second_session_id,
        "params": { "url": url }
    }))
    .await;
    let second_navigation = take_response_by_id(&mut ctx, 2321);
    assert_eq!(second_navigation["sessionId"], json!(second_session_id));
    assert!(
        ctx.sent
            .iter()
            .all(|message| message.get("error").is_none()),
        "unexpected protocol error during second navigation: {:?}",
        ctx.sent
    );
    ctx.take_all();

    ctx.process_async(json!({
        "id": 2322,
        "method": "Runtime.evaluate",
        "sessionId": second_session_id,
        "params": {
            "expression": "globalThis.__lm_page_seed"
        }
    }))
    .await;
    let second_evaluation = take_response_by_id(&mut ctx, 2322);
    assert_eq!(
        second_evaluation["result"]["result"]["type"],
        json!("undefined")
    );

    let active = ctx
        .conn
        .browser_context
        .as_ref()
        .expect("active browser context");
    assert_eq!(active.id, browser_context_id);
    assert_eq!(active.active_target_id(), Some(second_target_id.as_str()));
    assert_eq!(active.active_session_id(), Some(second_session_id.as_str()));
    assert_eq!(
        active
            .active_page_state()
            .active_target
            .owner_state
            .document_start_scripts
            .len(),
        0
    );

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn playwright_over_cdp_context_profile_surfaces_permissions_tls_and_metrics() {
    async fn handler() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>ok</body></html>",
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let origin = format!("http://{addr}");
    let url = format!("{origin}/page");
    let server = tokio::spawn(async move {
        axum::serve(listener, Router::new().route("/page", get(handler)))
            .await
            .unwrap();
    });

    let mut ctx = TestContext::new();
    let attached = create_attached_page_session_async(&mut ctx, 230, 231, 232, 2392, 233).await;
    let browser_context_id = attached.browser_context_id;
    let target_id = attached.target_id;
    let session_id = attached.session_id;

    ctx.process_async(json!({
        "id": 234,
        "method": "Security.setIgnoreCertificateErrors",
        "sessionId": session_id,
        "params": { "ignore": true }
    }))
    .await;
    ctx.expect_result(234, json!({}), Some(&session_id));
    assert!(!ctx.conn.tls_verify_host());
    ctx.conn
        .ensure_resource_request_client()
        .expect("loader should exist after tls override");
    assert!(!ctx.conn.tls_verify_host());

    ctx.process_async(json!({
        "id": 235,
        "method": "Emulation.setDeviceMetricsOverride",
        "sessionId": session_id,
        "params": {
            "width": 1280,
            "height": 720,
            "deviceScaleFactor": 2,
            "screenWidth": 1440,
            "screenHeight": 900,
            "mobile": false
        }
    }))
    .await;
    ctx.expect_result(235, json!({}), Some(&session_id));

    ctx.process_async(json!({
        "id": 236,
        "method": "Page.navigate",
        "sessionId": session_id,
        "params": { "url": url }
    }))
    .await;
    let response = take_response_by_id(&mut ctx, 236);
    assert_eq!(response["sessionId"], json!(session_id));
    assert!(
        ctx.sent
            .iter()
            .all(|message| message.get("error").is_none()),
        "unexpected protocol error during profile navigation: {:?}",
        ctx.sent
    );
    ctx.take_all();

    ctx.process_async(json!({
        "id": 237,
        "method": "Browser.setPermission",
        "params": {
            "permission": { "name": "geolocation" },
            "setting": "denied",
            "origin": origin,
            "browserContextId": browser_context_id
        }
    }))
    .await;
    ctx.expect_result(237, json!({}), None);
    assert_eq!(
        current_permission_state_async(&mut ctx, &session_id, "geolocation").await,
        "denied"
    );

    ctx.process_async(json!({
        "id": 238,
        "method": "Page.getLayoutMetrics",
        "sessionId": session_id
    }))
    .await;
    let metrics = take_response_by_id(&mut ctx, 238);
    assert_eq!(
        metrics["result"]["layoutViewport"]["clientWidth"],
        json!(1280)
    );
    assert_eq!(
        metrics["result"]["layoutViewport"]["clientHeight"],
        json!(720)
    );
    assert_eq!(metrics["result"]["visualViewport"]["scale"], json!(2.0));

    ctx.process_async(json!({
            "id": 239,
            "method": "Runtime.evaluate",
            "sessionId": session_id,
            "params": {
                "expression": "JSON.stringify({ innerWidth: window.innerWidth, innerHeight: window.innerHeight, dpr: window.devicePixelRatio, screenWidth: screen.width, screenHeight: screen.height })"
            }
        })).await;
    let surface = take_response_by_id(&mut ctx, 239);
    let payload = surface["result"]["result"]["value"]
        .as_str()
        .expect("stringified surface payload");
    let payload: serde_json::Value =
        serde_json::from_str(payload).expect("surface payload should be valid json");
    assert_eq!(payload["innerWidth"], 1280);
    assert_eq!(payload["innerHeight"], 720);
    assert_eq!(payload["dpr"], 2.0);
    assert_eq!(payload["screenWidth"], 1440);
    assert_eq!(payload["screenHeight"], 900);

    let active = ctx
        .conn
        .browser_context
        .as_ref()
        .expect("active browser context");
    assert_eq!(active.id, browser_context_id);
    assert_eq!(active.active_target_id(), Some(target_id.as_str()));
    assert_eq!(active.active_session_id(), Some(session_id.as_str()));
    assert_eq!(
        active.active_page_state().tls_verify_host_override,
        Some(false)
    );
    assert_eq!(
        active
            .active_page_state()
            .emulated_device_metrics
            .as_ref()
            .map(|metrics| (metrics.width, metrics.height, metrics.device_scale_factor)),
        Some((1280, 720, 2.0))
    );

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn cdp_device_metrics_live_override_updates_css_media_queries() {
    let mut ctx = TestContext::new();
    let session_id =
        create_attached_page_session_async(&mut ctx, 56100, 56101, 56102, 56103, 56104)
            .await
            .session_id;

    ctx.process_async(json!({
        "id": 56105,
        "method": "Page.navigate",
        "sessionId": session_id,
        "params": {
            "url": "data:text/html,<!doctype html><html><head></head><body><div id=target>target</div></body></html>"
        }
    }))
    .await;
    let navigation = take_response_by_id(&mut ctx, 56105);
    assert_eq!(navigation["sessionId"], json!(session_id));
    assert!(
        ctx.sent
            .iter()
            .all(|message| message.get("error").is_none()),
        "unexpected protocol error during live metrics navigation: {:?}",
        ctx.sent
    );
    ctx.take_all();

    ctx.process_async(json!({
        "id": 56106,
        "method": "Emulation.setDeviceMetricsOverride",
        "sessionId": session_id,
        "params": {
            "width": 800,
            "height": 600,
            "deviceScaleFactor": 1,
            "screenWidth": 1920,
            "screenHeight": 1080,
            "mobile": false
        }
    }))
    .await;
    ctx.expect_result(56106, json!({}), Some(&session_id));

    ctx.process_async(json!({
        "id": 56107,
        "method": "Runtime.evaluate",
        "sessionId": session_id,
        "params": {
            "expression": r#"
JSON.stringify((() => {
  const style = document.createElement('style');
  style.textContent = `
    @media (width: 800px) and (device-width: 1920px) {
      #target { color: rgb(1, 2, 3); }
    }
    @media (width: 800px) and (device-width: 800px) {
      #target { color: rgb(4, 5, 6); }
    }
  `;
  document.head.appendChild(style);
  const target = document.getElementById('target');
  return {
    innerWidth: window.innerWidth,
    innerHeight: window.innerHeight,
    screenWidth: screen.width,
    screenHeight: screen.height,
    width: matchMedia('(width: 800px)').matches,
    deviceWidth: matchMedia('(device-width: 1920px)').matches,
    wrongDeviceWidth: matchMedia('(device-width: 800px)').matches,
    combined: matchMedia('(width: 800px) and (device-width: 1920px)').matches,
    wrongCombined: matchMedia('(width: 800px) and (device-width: 800px)').matches,
    mediaRules: Array.from(style.sheet.cssRules).map(rule => rule.matches).join('|'),
    color: getComputedStyle(target).color
  };
})())
"#
        }
    }))
    .await;
    let evaluation = take_response_by_id(&mut ctx, 56107);
    let payload = evaluation["result"]["result"]["value"]
        .as_str()
        .expect("device metrics media payload");
    let payload: serde_json::Value =
        serde_json::from_str(payload).expect("device metrics media payload should be JSON");

    assert_eq!(payload["innerWidth"], json!(800));
    assert_eq!(payload["innerHeight"], json!(600));
    assert_eq!(payload["screenWidth"], json!(1920));
    assert_eq!(payload["screenHeight"], json!(1080));
    assert_eq!(payload["width"], json!(true));
    assert_eq!(payload["deviceWidth"], json!(true));
    assert_eq!(payload["wrongDeviceWidth"], json!(false));
    assert_eq!(payload["combined"], json!(true));
    assert_eq!(payload["wrongCombined"], json!(false));
    assert_eq!(payload["mediaRules"], json!("true|false"));
    assert_eq!(payload["color"], json!("rgb(1, 2, 3)"));
}

#[tokio::test(flavor = "multi_thread")]
async fn playwright_over_cdp_script_execution_disabled_blocks_page_scripts_but_not_runtime_eval() {
    async fn handler() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body><script>document.body.dataset.inlineRan='yes'; globalThis.__inlineRan = true;</script>ok</body></html>",
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let url = format!("http://{addr}/page");
    let server = tokio::spawn(async move {
        axum::serve(listener, Router::new().route("/page", get(handler)))
            .await
            .unwrap();
    });

    let mut ctx = TestContext::new();
    let attached = create_attached_page_session_async(&mut ctx, 240, 241, 242, 2393, 243).await;
    let session_id = attached.session_id;

    ctx.process_async(json!({
        "id": 244,
        "method": "Page.addScriptToEvaluateOnNewDocument",
        "sessionId": session_id,
        "params": {
            "source": "globalThis.__preloadRan = true;"
        }
    }))
    .await;
    let preload = take_response_by_id(&mut ctx, 244);
    assert!(preload["result"]["identifier"].is_string());

    ctx.process_async(json!({
        "id": 245,
        "method": "Emulation.setScriptExecutionDisabled",
        "sessionId": session_id,
        "params": { "value": true }
    }))
    .await;
    ctx.expect_result(245, json!({}), Some(&session_id));

    ctx.process_async(json!({
        "id": 246,
        "method": "Page.navigate",
        "sessionId": session_id,
        "params": { "url": url }
    }))
    .await;
    let response = take_response_by_id(&mut ctx, 246);
    assert_eq!(response["sessionId"], json!(session_id));
    let loader_id = response["result"]["loaderId"]
        .as_str()
        .expect("script-disabled navigation loader id")
        .to_owned();
    assert!(
        ctx.sent
            .iter()
            .all(|message| message.get("error").is_none()),
        "unexpected protocol error during script-disabled navigation: {:?}",
        ctx.sent
    );
    ctx.take_all();
    crate::testing::wait_until_renderer_document_load(
        &mut ctx,
        Some(session_id.as_str()),
        &attached.target_id,
        &loader_id,
    )
    .await;
    ctx.conn
        .browser_context
        .as_mut()
        .and_then(|browser_context| {
            browser_context
                .active_page_state_mut()
                .active_target
                .runtime_slot
                .loaded_page_mut()
        })
        .expect("script-disabled loaded page")
        .refresh_script_execution_report_async()
        .await
        .expect("script-disabled report refresh");

    let active = ctx
        .conn
        .browser_context
        .as_ref()
        .expect("active browser context");
    assert!(active.active_page_state().script_execution_disabled);
    let script_runs = active
        .active_page_state()
        .active_target
        .runtime_slot
        .loaded_page()
        .expect("loaded page should exist")
        .script_execution()
        .runs();
    assert!(
        script_runs.iter().any(|run| matches!(
            run.outcome(),
            ScriptRunOutcome::Skipped(ScriptSkipReason::ScriptExecutionDisabled)
        )),
        "expected at least one skipped page script when script execution is disabled"
    );

    ctx.process_async(json!({
            "id": 247,
            "method": "Runtime.evaluate",
            "sessionId": session_id,
            "params": {
                "expression": "JSON.stringify({ preloadRan: !!globalThis.__preloadRan, inlineRan: !!globalThis.__inlineRan, dataset: document.body.dataset.inlineRan || null, runtimeEvalStillWorks: 1 + 1 })"
            }
        })).await;
    let evaluation = take_response_by_id(&mut ctx, 247);
    let payload = evaluation["result"]["result"]["value"]
        .as_str()
        .expect("script-disabled evaluation payload should be a string");
    let payload: serde_json::Value = serde_json::from_str(payload)
        .expect("script-disabled evaluation payload should be valid json");
    assert_eq!(payload["preloadRan"], false);
    assert_eq!(payload["inlineRan"], false);
    assert_eq!(payload["dataset"], serde_json::Value::Null);
    assert_eq!(payload["runtimeEvalStillWorks"], 2);

    server.abort();
}
