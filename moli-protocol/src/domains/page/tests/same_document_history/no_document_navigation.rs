use super::*;

async fn no_document_page(status: u16, attachment: bool) -> SameDocumentPage {
    let routes = axum::Router::new().route(
        "/no-document",
        axum::routing::get(move |uri: axum::http::Uri| async move {
            use axum::response::IntoResponse;
            if uri.query() == Some("open") {
                return axum::response::Response::builder()
                    .header("content-type", "text/html")
                    .body(axum::body::Body::from_stream(
                        futures_util::stream::pending::<Result<Vec<u8>, std::io::Error>>(),
                    ))
                    .unwrap();
            }
            let body = if status == 200 {
                "<!doctype html><script>globalThis.unexpectedResponseScript = true</script>"
            } else {
                ""
            };
            let mut response = (
                axum::http::StatusCode::from_u16(status).unwrap(),
                [(axum::http::header::CACHE_CONTROL, "no-store")],
                body,
            )
                .into_response();
            response.headers_mut().insert(
                axum::http::header::CONTENT_TYPE,
                axum::http::HeaderValue::from_static("text/html"),
            );
            if attachment {
                response.headers_mut().insert(
                    axum::http::header::CONTENT_DISPOSITION,
                    axum::http::HeaderValue::from_static("attachment; filename=ignored.html"),
                );
            }
            response
        }),
    );
    let mut page = SameDocumentPage::with_routes(routes).await;
    page.ctx.enable_background_navigation_scheduler_for_test();
    page.command("Runtime.enable", json!({})).await;
    page.command("Network.enable", json!({})).await;
    page.evaluate(
        r#"(async () => {
            const child = document.createElement('iframe');
            child.srcdoc = '<!doctype html><body>retained child';
            const loaded = new Promise(resolve => child.onload = resolve);
            document.body.append(child);
            await loaded;
            const saved = {document, child: child.contentWindow.document,
                entry: navigation.currentEntry, url: location.href, length: history.length};
            const events = globalThis.noDocumentLifecycleEvents = [];
            for (const type of ['pagehide', 'unload', 'load']) {
                addEventListener(type, () => events.push(type));
            }
            globalThis.noDocumentSnapshot = () => ({
                document: document === saved.document,
                child: child.contentWindow.document === saved.child,
                entry: navigation.currentEntry === saved.entry,
                url: location.href === saved.url,
                history: history.length === saved.length,
            });
        })()"#,
    )
    .await;
    page
}

async fn arm_probe(page: &mut SameDocumentPage) -> (serde_json::Value, serde_json::Value) {
    let tree = page.command("Page.getFrameTree", json!({})).await;
    let history = page.command("Page.getNavigationHistory", json!({})).await;
    page.ctx.sent.clear();
    page.ctx
        .process_async(json!({
            "id": 9001,
            "sessionId": SESSION,
            "method": "Runtime.evaluate",
            "params": {
                "expression": "new Promise(resolve => { globalThis.resolveNoDocument = resolve; })",
                "awaitPromise": true,
                "returnByValue": true,
            },
        }))
        .await;
    (tree, history)
}

fn assert_aborted_result(result: &serde_json::Value) {
    assert_eq!(result["errorText"], "net::ERR_ABORTED", "{result}");
    assert_eq!(result["isDownload"], false, "{result}");
    assert!(result["loaderId"].is_string(), "{result}");
}

async fn assert_document_retained(
    page: &mut SameDocumentPage,
    url: &str,
    status: u16,
    before: (serde_json::Value, serde_json::Value),
    network_enabled: bool,
) {
    wait_until_frame_stopped_loading(&mut page.ctx, FRAME).await;
    assert!(!page.ctx.sent.iter().any(|event| event["id"] == 9001));
    assert!(
        !page.ctx.sent.iter().any(|event| matches!(
            event["method"].as_str(),
            Some(
                "Runtime.executionContextDestroyed"
                    | "Runtime.executionContextsCleared"
                    | "Page.frameNavigated"
                    | "Browser.downloadWillBegin"
            )
        )),
        "unexpected document replacement or download: {:?}",
        page.ctx.sent
    );
    if network_enabled {
        let responses = page
            .ctx
            .sent
            .iter()
            .enumerate()
            .filter(|(_, event)| {
                event["method"] == "Network.responseReceived"
                    && event["params"]["response"]["url"] == url
            })
            .collect::<Vec<_>>();
        assert_eq!(responses.len(), 1, "{url}: {:?}", page.ctx.sent);
        let (response_index, response) = responses[0];
        assert_eq!(response["params"]["response"]["status"], status);
        let request_id = response["params"]["requestId"].clone();
        let terminals = page
            .ctx
            .sent
            .iter()
            .enumerate()
            .filter(|(_, event)| {
                event["params"]["requestId"] == request_id
                    && matches!(
                        event["method"].as_str(),
                        Some("Network.loadingFailed" | "Network.loadingFinished")
                    )
            })
            .collect::<Vec<_>>();
        assert_eq!(terminals.len(), 1, "{terminals:?}");
        assert!(terminals[0].0 > response_index);
        assert_eq!(terminals[0].1["method"], "Network.loadingFailed");
        assert_eq!(terminals[0].1["params"]["errorText"], "net::ERR_ABORTED");
        assert_eq!(terminals[0].1["params"]["canceled"], true);
    }
    assert_eq!(page.command("Page.getFrameTree", json!({})).await, before.0);
    assert_eq!(
        page.command("Page.getNavigationHistory", json!({})).await,
        before.1
    );
    page.evaluate("resolveNoDocument(noDocumentSnapshot()); undefined")
        .await;
    wait_until_message(
        &mut page.ctx,
        SESSION,
        "retained pending evaluation",
        |event| event["id"] == 9001,
    )
    .await;
    let pending = take_response_by_id(&mut page.ctx, 9001);
    assert!(pending["error"].is_null(), "{pending}");
    assert_eq!(
        pending["result"]["result"]["value"],
        json!({
            "document": true, "child": true, "entry": true, "url": true, "history": true,
        })
    );
    assert_eq!(
        page.evaluate("typeof unexpectedResponseScript").await,
        "undefined"
    );
    assert_eq!(page.evaluate("noDocumentLifecycleEvents").await, json!([]));
}

async fn assert_next_navigation(page: &mut SameDocumentPage) {
    let next_url = format!("{}?next", page.base_url);
    page.evaluate(
        r#"(() => {
        const events = [];
        for (const type of ['pagehide', 'unload']) {
            addEventListener(type, () => {
                events.push(type);
                sessionStorage.setItem('navigationUnloadEvents', JSON.stringify(events));
                console.log('retained-document-' + type);
            });
        }
    })()"#,
    )
    .await;
    page.ctx.sent.clear();
    let next = page
        .command("Page.navigate", json!({"url": next_url}))
        .await;
    assert!(next["errorText"].is_null(), "{next}");
    wait_until_frame_stopped_loading(&mut page.ctx, FRAME).await;
    assert_eq!(page.evaluate("location.href").await, next_url);
    assert_eq!(
        page.evaluate("typeof noDocumentSnapshot").await,
        "undefined"
    );
    assert_eq!(
        page.evaluate("JSON.parse(sessionStorage.getItem('navigationUnloadEvents'))")
            .await,
        json!(["pagehide", "unload"])
    );
    let unload_messages = page
        .ctx
        .sent
        .iter()
        .enumerate()
        .filter_map(|(index, event)| {
            (event["method"] == "Runtime.consoleAPICalled")
                .then(|| event["params"]["args"][0]["value"].as_str())
                .flatten()
                .filter(|value| value.starts_with("retained-document-"))
                .map(|value| (index, value))
        })
        .collect::<Vec<_>>();
    assert_eq!(
        unload_messages
            .iter()
            .map(|(_, value)| *value)
            .collect::<Vec<_>>(),
        ["retained-document-pagehide", "retained-document-unload"]
    );
    let replacement = page
        .ctx
        .sent
        .iter()
        .position(|event| event["method"] == "Page.frameNavigated")
        .expect("replacement document should commit");
    assert!(
        unload_messages
            .iter()
            .all(|(index, _)| *index < replacement),
        "{:?}",
        page.ctx.sent
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn no_document_navigation_keeps_the_document_and_pending_evaluation() {
    tokio::task::LocalSet::new()
        .run_until(async {
            for (status, attachment) in [(204, false), (205, false), (204, true), (205, true)] {
                let mut page = no_document_page(status, attachment).await;
                let before = arm_probe(&mut page).await;
                let url = page.base_url.replace("/history.html", "/no-document");
                let result = page.command("Page.navigate", json!({"url": url})).await;
                assert_aborted_result(&result);
                assert_document_retained(&mut page, &url, status, before, true).await;
                assert_eq!(page.evaluate("noDocumentLifecycleEvents").await, json!([]));
                assert_next_navigation(&mut page).await;
            }
        })
        .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn no_document_navigation_preserves_renderer_requests_and_navigation_api() {
    tokio::task::LocalSet::new().run_until(async {
        for status in [204, 205] {
            for mode in ["location", "navigation"] {
                for network_enabled in [false, true] {
                    let mut page = no_document_page(status, false).await;
                    if !network_enabled {
                        page.command("Network.disable", json!({})).await;
                    }
                    let before = arm_probe(&mut page).await;
                    let url = page.base_url.replace("/history.html", "/no-document");
                    page.evaluate(&format!(r#"(() => {{
                        globalThis.navigationOutcomes = {{committed:'pending', finished:'pending', events:[]}};
                        navigation.addEventListener('navigatesuccess', () => navigationOutcomes.events.push('success'));
                        navigation.addEventListener('navigateerror', () => navigationOutcomes.events.push('error'));
                        if ({mode:?} === 'location') {{ location.href = {url:?}; }} else {{
                            const result = navigation.navigate({url:?});
                            for (const name of ['committed', 'finished']) {{
                                result[name].then(() => {{navigationOutcomes[name] = 'fulfilled'}},
                                    error => {{navigationOutcomes[name] = error.name}});
                            }}
                        }}
                    }})()"#)).await;
                    assert_document_retained(&mut page, &url, status, before, network_enabled).await;
                    assert_eq!(page.evaluate("navigationOutcomes").await, json!({
                        "committed":"pending", "finished":"pending", "events":[],
                    }));
                    assert_eq!(page.evaluate("navigation.transition").await, serde_json::Value::Null);
                    if mode == "navigation" {
                        page.evaluate("navigation.navigate('#after').finished.then(() => true)").await;
                        assert_eq!(page.evaluate("navigationOutcomes").await, json!({
                            "committed":"AbortError", "finished":"AbortError", "events":["error", "success"],
                        }));
                    }
                    assert_next_navigation(&mut page).await;
                }
            }
        }
    }).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn no_document_navigation_handles_fetch_interception() {
    tokio::task::LocalSet::new()
        .run_until(async {
            for status in [204, 205] {
                for mode in [
                    "request-fulfill",
                    "response-fulfill",
                    "continue",
                    "override",
                    "captured",
                    "open-stream",
                ] {
                    let source_status = if mode == "continue" { status } else { 200 };
                    let mut page = no_document_page(source_status, false).await;
                    let request_stage = if mode == "request-fulfill" {
                        "Request"
                    } else {
                        "Response"
                    };
                    page.command(
                        "Fetch.enable",
                        json!({"patterns":[{
                            "urlPattern":"*/no-document*", "requestStage":request_stage,
                        }]}),
                    )
                    .await;
                    let before = arm_probe(&mut page).await;
                    let mut url = page.base_url.replace("/history.html", "/no-document");
                    if mode == "open-stream" {
                        url.push_str("?open");
                    }
                    page.ctx.process_async(json!({
                    "id":9002, "method":"Page.navigate", "sessionId":SESSION, "params":{"url":url},
                })).await;
                    wait_until_message(
                        &mut page.ctx,
                        SESSION,
                        "no-document Fetch pause",
                        |event| event["method"] == "Fetch.requestPaused",
                    )
                    .await;
                    let paused = page
                        .ctx
                        .sent
                        .iter()
                        .find(|event| event["method"] == "Fetch.requestPaused")
                        .unwrap();
                    let request_id = paused["params"]["requestId"].clone();
                    if mode == "captured" {
                        let body = page
                            .command("Fetch.getResponseBody", json!({"requestId":request_id}))
                            .await;
                        assert!(
                            body["body"]
                                .as_str()
                                .is_some_and(|body| body.contains("unexpectedResponseScript")),
                            "{body}"
                        );
                    }
                    let (method, params) = match mode {
                        "request-fulfill" | "response-fulfill" => (
                            "Fetch.fulfillRequest",
                            json!({
                                "requestId":request_id, "responseCode":status, "body":"",
                                "responseHeaders":[{"name":"Content-Type", "value":"text/html"}],
                            }),
                        ),
                        "continue" => ("Fetch.continueResponse", json!({"requestId":request_id})),
                        _ => (
                            "Fetch.continueResponse",
                            json!({
                                "requestId":request_id, "responseCode":status,
                                "responseHeaders":[{"name":"Content-Type", "value":"text/html"}],
                            }),
                        ),
                    };
                    tokio::time::timeout(
                        std::time::Duration::from_secs(5),
                        page.command(method, params),
                    )
                    .await
                    .expect("ignoring a response must not wait for its body EOF");
                    wait_until_message(
                        &mut page.ctx,
                        SESSION,
                        "ignored Page.navigate response",
                        |event| event["id"] == 9002,
                    )
                    .await;
                    let result = take_response_by_id(&mut page.ctx, 9002);
                    assert!(result["error"].is_null(), "{status}/{mode}: {result}");
                    assert_aborted_result(&result["result"]);
                    assert_document_retained(&mut page, &url, status, before, true).await;
                    assert_eq!(page.evaluate("noDocumentLifecycleEvents").await, json!([]));
                    page.command("Fetch.disable", json!({})).await;
                    assert_next_navigation(&mut page).await;
                }
            }
        })
        .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn no_document_navigation_preserves_protocol_result_semantics() {
    use crate::automation::{
        AutomationCommand, AutomationContext, AutomationResult, DevToolsNavigateCommand,
        DevToolsNavigationWait, DevToolsSessionId, DevToolsTargetId, FrontendProtocol,
    };
    tokio::task::LocalSet::new()
        .run_until(async {
            for status in [204, 205] {
                for protocol in [
                    FrontendProtocol::Cdp,
                    FrontendProtocol::WebDriverClassic,
                    FrontendProtocol::WebDriverBidi,
                ] {
                    let mut page = no_document_page(status, false).await;
                    page.evaluate(
                        r#"(() => {
                        globalThis.protocolBeforeUnload = [];
                        addEventListener('beforeunload', () => protocolBeforeUnload.push('root'));
                        document.querySelector('iframe').contentWindow.addEventListener(
                            'beforeunload', () => protocolBeforeUnload.push('child'));
                    })()"#,
                    )
                    .await;
                    let tree = page.command("Page.getFrameTree", json!({})).await;
                    let url = page.base_url.replace("/history.html", "/no-document");
                    let outcome = tokio::time::timeout(
                        std::time::Duration::from_secs(5),
                        page.ctx
                            .conn
                            .execute_automation_command(AutomationCommand::Navigate(
                                DevToolsNavigateCommand {
                                    context: AutomationContext {
                                        protocol,
                                        session_id: Some(DevToolsSessionId::from(SESSION)),
                                        target_id: Some(DevToolsTargetId::from(FRAME)),
                                        browser_context_id: None,
                                    },
                                    url,
                                    referrer: None,
                                    wait: DevToolsNavigationWait::Load,
                                },
                            )),
                    )
                    .await
                    .expect("ignored response must settle the protocol navigation");
                    let (result, _, _, renderer_predecessor) = outcome.into_complete_parts();
                    page.ctx
                        .route_direct_command_renderer_predecessor_for_test(
                            renderer_predecessor
                                .expect("beforeunload must publish its renderer turn"),
                        )
                        .await;
                    let AutomationResult::Navigate(result) = result.unwrap() else {
                        panic!("expected navigation result");
                    };
                    assert_eq!(
                        result.error_text.as_deref(),
                        (protocol == FrontendProtocol::Cdp).then_some("net::ERR_ABORTED")
                    );
                    assert_eq!(
                        page.evaluate("protocolBeforeUnload").await,
                        json!(["root", "child"])
                    );
                    assert_eq!(page.command("Page.getFrameTree", json!({})).await, tree);
                    assert_eq!(
                        page.evaluate("noDocumentSnapshot()").await,
                        json!({
                            "document":true,"child":true,"entry":true,"url":true,"history":true,
                        })
                    );
                }
            }
        })
        .await;
}
