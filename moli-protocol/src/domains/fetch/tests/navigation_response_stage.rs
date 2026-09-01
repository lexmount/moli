use super::*;
use base64::Engine as _;

#[tokio::test(flavor = "multi_thread")]
async fn response_stage_pause_happens_before_navigation_body_eof() {
    let (tail_tx, tail_rx) = tokio::sync::oneshot::channel::<()>();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut buf = [0u8; 2048];
        let _ = stream.read(&mut buf).await.unwrap();
        let head = "<!doctype html><html><body><main id=content>";
        let tail = "streamed tail</main></body></html>";
        let response_head = "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n";
        stream.write_all(response_head.as_bytes()).await.unwrap();
        stream
            .write_all(format!("{:X}\r\n{head}\r\n", head.len()).as_bytes())
            .await
            .unwrap();
        let _ = tail_rx.await;
        stream
            .write_all(format!("{:X}\r\n{tail}\r\n0\r\n\r\n", tail.len()).as_bytes())
            .await
            .unwrap();
        let _ = stream.shutdown().await;
    });

    let mut ctx = TestContext::new();
    let mut bc = attached_browser_context();
    bc.active_page_state_mut()
        .runtime_slot
        .enable_primary_network_events();
    ctx.conn.browser_context = Some(bc);
    let url = format!("http://{addr}/page");

    ctx.process_async(json!({
        "id": 360,
        "method": "Fetch.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(360, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 361,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": url }
    }))
    .await;

    let paused = take_main_document_request_pause(&mut ctx).await;
    let request_id = paused["params"]["requestId"].as_str().unwrap().to_owned();

    tokio::time::timeout(
        std::time::Duration::from_millis(300),
        ctx.process_async(json!({
            "id": 362,
            "method": "Fetch.continueRequest",
            "sessionId": "SID-1",
            "params": { "requestId": request_id, "interceptResponse": true }
        })),
    )
    .await
    .expect("response-stage pause should not wait for body EOF");
    ctx.expect_result(362, json!({}), Some("SID-1"));
    let response_paused = take_main_document_response_pause(&mut ctx);
    assert_eq!(response_paused["method"], "Fetch.requestPaused");
    assert_eq!(response_paused["params"]["responseStatusCode"], 200);
    let response_request_id = response_paused["params"]["requestId"]
        .as_str()
        .expect("response-stage request id")
        .to_owned();
    let prepared_agent = ctx
        .conn
        .browser_context
        .as_ref()
        .and_then(|bc| {
            bc.active_page_state()
                .fetch_owner
                .pending_fetch_response_prepared_renderer_agent_for_test(&response_request_id)
        })
        .expect("final response head should reserve a renderer agent before continueResponse");
    assert!(
        ctx.conn
            .browser_context
            .as_ref()
            .and_then(|bc| bc.loaded_page())
            .is_none(),
        "page should not be committed before Fetch.continueResponse"
    );

    tail_tx.send(()).unwrap();
    ctx.process_async(json!({
        "id": 363,
        "method": "Fetch.continueResponse",
        "sessionId": "SID-1",
        "params": { "requestId": response_request_id }
    }))
    .await;
    ctx.expect_result(363, json!({}), Some("SID-1"));
    ctx.expect_result(
        361,
        json!({ "frameId": "TID-1", "loaderId": LOADER_ID }),
        Some("SID-1"),
    );

    {
        let page = ctx
            .conn
            .browser_context
            .as_ref()
            .and_then(|bc| bc.loaded_page())
            .expect("loaded page");
        assert_eq!(
            page.renderer_devtools_agent_token(),
            prepared_agent,
            "continueResponse must commit the exact agent reserved at the response head"
        );
    }
    assert!(
        loaded_page_html_for_test(&mut ctx)
            .await
            .contains("streamed tail")
    );

    server.await.unwrap();
}

async fn assert_empty_http_error_response_stage(ctx: &mut TestContext, url: &str) {
    ctx.process_async(json!({
        "id": 36_480,
        "method": "Fetch.enable",
        "sessionId": "SID-1",
        "params": {
            "patterns": [{
                "urlPattern": "*",
                "requestStage": "Response",
                "resourceType": "Document"
            }]
        }
    }))
    .await;
    ctx.expect_result(36_480, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 36_481,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": url }
    }))
    .await;
    let paused = take_main_document_request_pause(ctx).await;
    assert_eq!(paused["params"]["responseStatusCode"], json!(429));
    let request_id = paused["params"]["requestId"]
        .as_str()
        .expect("HTTP error response-stage request id")
        .to_owned();
    let network_id = paused["params"]["networkId"]
        .as_str()
        .expect("HTTP error response-stage network id")
        .to_owned();
    assert!(
        ctx.conn
            .browser_context
            .as_ref()
            .and_then(|bc| {
                bc.active_page_state()
                    .fetch_owner
                    .pending_fetch_response_prepared_renderer_agent_for_test(&request_id)
            })
            .is_none(),
        "an empty HTTP error must be classified from its body after continueResponse"
    );

    ctx.process_async(json!({
        "id": 36_482,
        "method": "Fetch.continueResponse",
        "sessionId": "SID-1",
        "params": { "requestId": request_id }
    }))
    .await;
    ctx.expect_result(36_482, json!({}), Some("SID-1"));
    wait_until_message(
        ctx,
        "SID-1",
        "empty HTTP error navigation result",
        |message| message["id"] == json!(36_481),
    )
    .await;
    let navigate = take_response_by_id(ctx, 36_481);
    assert_eq!(navigate["result"]["frameId"], json!("TID-1"));
    assert_eq!(navigate["result"]["isDownload"], json!(false));
    assert_eq!(
        navigate["result"]["errorText"],
        json!("net::ERR_HTTP_RESPONSE_CODE_FAILURE")
    );
    wait_until_message(
        ctx,
        "SID-1",
        "Fetch-continued empty HTTP error Document stop loading",
        |message| message["method"] == json!("Page.frameStoppedLoading"),
    )
    .await;

    let response_index = ctx
        .sent
        .iter()
        .position(|message| {
            message["method"] == json!("Network.responseReceived")
                && message["params"]["requestId"] == json!(network_id)
                && message["params"]["response"]["status"] == json!(429)
        })
        .unwrap_or_else(|| panic!("missing original HTTP 429 response: {:?}", ctx.sent));
    let failure_index = ctx
        .sent
        .iter()
        .position(|message| {
            message["method"] == json!("Network.loadingFailed")
                && message["params"]["requestId"] == json!(network_id)
                && message["params"]["errorText"] == json!("net::ERR_HTTP_RESPONSE_CODE_FAILURE")
        })
        .unwrap_or_else(|| panic!("missing HTTP response-code failure: {:?}", ctx.sent));
    assert!(response_index < failure_index);

    let page = ctx
        .conn
        .browser_context
        .as_ref()
        .and_then(|bc| bc.loaded_page())
        .expect("browser-owned HTTP error Document should commit");
    assert_eq!(page.final_url().as_str(), NETWORK_ERROR_PAGE_URL);
    assert_eq!(
        ctx.conn
            .browser_context
            .as_ref()
            .expect("browser context should remain installed")
            .target_url(),
        url
    );
    assert!(
        loaded_page_html_for_test(ctx)
            .await
            .contains("HTTP ERROR 429")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn empty_http_error_response_stage_commits_browser_error_document_after_continue() {
    async fn empty_rate_limit() -> impl IntoResponse {
        (
            StatusCode::TOO_MANY_REQUESTS,
            [
                (CONTENT_TYPE.as_str(), "text/html; charset=utf-8"),
                (axum::http::header::CONTENT_LENGTH.as_str(), "0"),
            ],
            "",
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new().route("/empty-429", get(empty_rate_limit)),
        )
        .await
        .unwrap();
    });

    let mut ctx = TestContext::new();
    let mut bc = attached_browser_context();
    bc.active_page_state_mut()
        .runtime_slot
        .enable_primary_network_events();
    ctx.conn.browser_context = Some(bc);
    ctx.enable_page_events_for_test(Some("SID-1"));
    ctx.enable_background_navigation_scheduler_for_test();
    let url = format!("http://{addr}/empty-429");

    tokio::task::LocalSet::new()
        .run_until(assert_empty_http_error_response_stage(&mut ctx, &url))
        .await;

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn response_stage_commit_uses_configuration_added_while_paused_before_author_script() {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            r#"<!doctype html><script>
globalThis.__commitOrdering = JSON.stringify([
  globalThis.__latePreload,
  typeof lateBinding,
  localStorage.getItem("__lateWorld")
]);
lateBinding("author-script");
</script>"#,
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, Router::new().route("/page", get(page)))
            .await
            .unwrap();
    });

    let mut ctx = TestContext::new();
    let mut bc = attached_browser_context();
    bc.active_page_state_mut()
        .runtime_slot
        .enable_primary_network_events();
    ctx.conn.browser_context = Some(bc);
    let initial_page = ctx
        .conn
        .load_page_via_runtime_async("data:text/html,<title>initial</title>")
        .await
        .expect("initial document should load with its target owner installed");
    let _ = ctx
        .conn
        .browser_context
        .as_mut()
        .expect("browser context should remain installed")
        .active_page_state_mut()
        .runtime_slot
        .replace_loaded_page(Some(initial_page));

    ctx.process_async(json!({
        "id": 36_500,
        "method": "Runtime.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(36_500, json!({}), Some("SID-1"));
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 36_501,
        "method": "Fetch.enable",
        "sessionId": "SID-1",
        "params": {
            "patterns": [{
                "urlPattern": "*",
                "requestStage": "Response",
                "resourceType": "Document"
            }]
        }
    }))
    .await;
    ctx.expect_result(36_501, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 36_502,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": format!("http://{addr}/page") }
    }))
    .await;
    let paused = take_main_document_request_pause(&mut ctx).await;
    let request_id = paused["params"]["requestId"]
        .as_str()
        .expect("response-stage request id")
        .to_owned();

    ctx.process_async(json!({
        "id": 36_503,
        "method": "Page.addScriptToEvaluateOnNewDocument",
        "sessionId": "SID-1",
        "params": {
            "source": "globalThis.__latePreload = 'ready';"
        }
    }))
    .await;
    assert_eq!(
        take_response_by_id(&mut ctx, 36_503)["result"]["identifier"],
        json!("1")
    );

    ctx.process_async(json!({
        "id": 36_504,
        "method": "Page.addScriptToEvaluateOnNewDocument",
        "sessionId": "SID-1",
        "params": {
            "source": "globalThis.__lateWorld = 'ready'; localStorage.setItem('__lateWorld', 'ready'); lateBinding('named-preload');",
            "worldName": "late-world"
        }
    }))
    .await;
    assert_eq!(
        take_response_by_id(&mut ctx, 36_504)["result"]["identifier"],
        json!("2")
    );

    ctx.process_async(json!({
        "id": 36_505,
        "method": "Runtime.addBinding",
        "sessionId": "SID-1",
        "params": { "name": "lateBinding" }
    }))
    .await;
    ctx.expect_result(36_505, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 36_506,
        "method": "Fetch.continueResponse",
        "sessionId": "SID-1",
        "params": { "requestId": request_id }
    }))
    .await;
    ctx.expect_result(36_506, json!({}), Some("SID-1"));
    let navigate = take_response_by_id(&mut ctx, 36_502);
    assert_eq!(navigate["result"]["frameId"], json!("TID-1"));

    ctx.process_async(json!({
        "id": 36_507,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": "globalThis.__commitOrdering",
            "returnByValue": true
        }
    }))
    .await;
    assert_eq!(
        take_response_by_id(&mut ctx, 36_507)["result"]["result"]["value"],
        json!(r#"["ready","function","ready"]"#)
    );

    let named_context_id = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Runtime.executionContextCreated")
                && message["params"]["context"]["name"] == json!("late-world")
        })
        .and_then(|message| message["params"]["context"]["id"].as_i64())
        .expect("commit should publish the named-world execution context");
    ctx.process_async(json!({
        "id": 36_508,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "contextId": named_context_id,
            "expression": "globalThis.__lateWorld",
            "returnByValue": true
        }
    }))
    .await;
    assert_eq!(
        take_response_by_id(&mut ctx, 36_508)["result"]["result"]["value"],
        json!("ready")
    );
    let binding_payloads = ctx
        .sent
        .iter()
        .filter(|message| {
            message["method"] == json!("Runtime.bindingCalled")
                && message["params"]["name"] == json!("lateBinding")
        })
        .map(|message| message["params"]["payload"].clone())
        .collect::<Vec<_>>();
    assert_eq!(
        binding_payloads,
        vec![json!("named-preload"), json!("author-script")],
        "the named-world preload must run before the first parser author script"
    );

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn response_stage_xml_commit_uses_live_configuration_before_first_author_script() {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "application/xhtml+xml")],
            concat!(
                "<html xmlns='http://www.w3.org/1999/xhtml'><head><script>",
                "globalThis.__xmlCommitOrdering = JSON.stringify([",
                "globalThis.__xmlLatePreload, typeof xmlLateBinding,",
                "localStorage.getItem('__xmlLateWorld')]);",
                "xmlLateBinding('author-script');",
                "</script></head><body><main id='xml-root'>xml</main></body></html>",
            ),
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, Router::new().route("/page.xhtml", get(page)))
            .await
            .unwrap();
    });

    let mut ctx = TestContext::new();
    let mut bc = attached_browser_context();
    bc.active_page_state_mut()
        .runtime_slot
        .enable_primary_network_events();
    ctx.conn.browser_context = Some(bc);
    let initial_page = ctx
        .conn
        .load_page_via_runtime_async("data:text/html,<title>initial</title>")
        .await
        .expect("initial document should load with its target owner installed");
    let initial_agent = initial_page.renderer_devtools_agent_token();
    let _ = ctx
        .conn
        .browser_context
        .as_mut()
        .expect("browser context should remain installed")
        .active_page_state_mut()
        .runtime_slot
        .replace_loaded_page(Some(initial_page));

    ctx.process_async(json!({
        "id": 36_550,
        "method": "Runtime.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(36_550, json!({}), Some("SID-1"));
    ctx.process_async(json!({
        "id": 36_551,
        "method": "Fetch.enable",
        "sessionId": "SID-1",
        "params": {
            "patterns": [{
                "urlPattern": "*",
                "requestStage": "Response",
                "resourceType": "Document"
            }]
        }
    }))
    .await;
    ctx.expect_result(36_551, json!({}), Some("SID-1"));
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 36_552,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": format!("http://{addr}/page.xhtml") }
    }))
    .await;
    let paused = take_main_document_request_pause(&mut ctx).await;
    assert_eq!(paused["params"]["responseStatusCode"], json!(200));
    assert_eq!(
        paused["params"]["responseHeaders"]
            .as_array()
            .and_then(|headers| headers.iter().find(|header| {
                header["name"]
                    .as_str()
                    .is_some_and(|name| name.eq_ignore_ascii_case("content-type"))
            }))
            .and_then(|header| header["value"].as_str()),
        Some("application/xhtml+xml")
    );
    let request_id = paused["params"]["requestId"]
        .as_str()
        .expect("XML response-stage request id")
        .to_owned();
    assert_eq!(
        ctx.conn
            .browser_context
            .as_ref()
            .and_then(|bc| bc.loaded_page())
            .map(|page| page.renderer_devtools_agent_token()),
        Some(initial_agent),
        "the old document must remain installed while the XML response is paused"
    );

    ctx.process_async(json!({
        "id": 36_553,
        "method": "Page.addScriptToEvaluateOnNewDocument",
        "sessionId": "SID-1",
        "params": { "source": "globalThis.__xmlLatePreload = 'ready';" }
    }))
    .await;
    assert_eq!(
        take_response_by_id(&mut ctx, 36_553)["result"]["identifier"],
        json!("1")
    );
    ctx.process_async(json!({
        "id": 36_554,
        "method": "Page.addScriptToEvaluateOnNewDocument",
        "sessionId": "SID-1",
        "params": {
            "source": "globalThis.__xmlLateWorld = 'ready'; localStorage.setItem('__xmlLateWorld', 'ready'); xmlLateBinding('named-preload');",
            "worldName": "xml-late-world"
        }
    }))
    .await;
    assert_eq!(
        take_response_by_id(&mut ctx, 36_554)["result"]["identifier"],
        json!("2")
    );
    ctx.process_async(json!({
        "id": 36_555,
        "method": "Runtime.addBinding",
        "sessionId": "SID-1",
        "params": { "name": "xmlLateBinding" }
    }))
    .await;
    ctx.expect_result(36_555, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 36_556,
        "method": "Fetch.continueResponse",
        "sessionId": "SID-1",
        "params": { "requestId": request_id }
    }))
    .await;
    ctx.expect_result(36_556, json!({}), Some("SID-1"));
    let navigate = take_response_by_id(&mut ctx, 36_552);
    assert_eq!(navigate["result"]["frameId"], json!("TID-1"));

    let committed_agent = ctx
        .conn
        .browser_context
        .as_ref()
        .and_then(|bc| bc.loaded_page())
        .expect("XML document should commit")
        .renderer_devtools_agent_token();
    assert_ne!(
        committed_agent, initial_agent,
        "cross-document XML commit must install its prepared renderer agent"
    );
    ctx.process_async(json!({
        "id": 36_557,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": "JSON.stringify([globalThis.__xmlCommitOrdering, document.contentType, document.documentElement.namespaceURI, document.getElementById('xml-root').textContent])",
            "returnByValue": true
        }
    }))
    .await;
    assert_eq!(
        take_response_by_id(&mut ctx, 36_557)["result"]["result"]["value"],
        json!(
            r#"["[\"ready\",\"function\",\"ready\"]","application/xhtml+xml","http://www.w3.org/1999/xhtml","xml"]"#
        )
    );

    let named_context_id = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Runtime.executionContextCreated")
                && message["params"]["context"]["name"] == json!("xml-late-world")
        })
        .and_then(|message| message["params"]["context"]["id"].as_i64())
        .expect("XML commit should publish the late named-world context");
    ctx.process_async(json!({
        "id": 36_558,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "contextId": named_context_id,
            "expression": "globalThis.__xmlLateWorld",
            "returnByValue": true
        }
    }))
    .await;
    assert_eq!(
        take_response_by_id(&mut ctx, 36_558)["result"]["result"]["value"],
        json!("ready")
    );
    let binding_payloads = ctx
        .sent
        .iter()
        .filter(|message| {
            message["method"] == json!("Runtime.bindingCalled")
                && message["params"]["name"] == json!("xmlLateBinding")
        })
        .map(|message| message["params"]["payload"].clone())
        .collect::<Vec<_>>();
    assert_eq!(
        binding_payloads,
        vec![json!("named-preload"), json!("author-script")],
        "the XML named-world preload must run before the first parser author script"
    );

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn fulfill_request_commit_uses_configuration_added_while_paused_before_author_script() {
    let mut ctx = TestContext::new();
    ctx.conn.browser_context = Some(attached_browser_context());
    let initial_page = ctx
        .conn
        .load_page_via_runtime_async("data:text/html,<title>initial</title>")
        .await
        .expect("initial document should load with its target owner installed");
    let _ = ctx
        .conn
        .browser_context
        .as_mut()
        .expect("browser context should remain installed")
        .active_page_state_mut()
        .runtime_slot
        .replace_loaded_page(Some(initial_page));

    ctx.process_async(json!({
        "id": 36_600,
        "method": "Runtime.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(36_600, json!({}), Some("SID-1"));
    ctx.process_async(json!({
        "id": 36_601,
        "method": "Fetch.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(36_601, json!({}), Some("SID-1"));
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 36_602,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": "http://example.test/fulfilled-ordering" }
    }))
    .await;
    let paused = take_main_document_request_pause(&mut ctx).await;
    let request_id = paused["params"]["requestId"]
        .as_str()
        .expect("request-stage request id")
        .to_owned();

    ctx.process_async(json!({
        "id": 36_603,
        "method": "Page.addScriptToEvaluateOnNewDocument",
        "sessionId": "SID-1",
        "params": { "source": "globalThis.__fulfillPreload = 'ready';" }
    }))
    .await;
    assert_eq!(
        take_response_by_id(&mut ctx, 36_603)["result"]["identifier"],
        json!("1")
    );
    ctx.process_async(json!({
        "id": 36_604,
        "method": "Page.addScriptToEvaluateOnNewDocument",
        "sessionId": "SID-1",
        "params": {
            "source": "globalThis.__fulfillNamed = 'ready'; fulfillBinding('named-preload');",
            "worldName": "fulfill-world"
        }
    }))
    .await;
    assert_eq!(
        take_response_by_id(&mut ctx, 36_604)["result"]["identifier"],
        json!("2")
    );
    ctx.process_async(json!({
        "id": 36_605,
        "method": "Runtime.addBinding",
        "sessionId": "SID-1",
        "params": { "name": "fulfillBinding" }
    }))
    .await;
    ctx.expect_result(36_605, json!({}), Some("SID-1"));

    let body = base64::engine::general_purpose::STANDARD.encode(
        concat!(
            "<!doctype html><script>",
            "globalThis.__fulfillCommitOrdering=JSON.stringify([globalThis.__fulfillPreload,typeof fulfillBinding]);",
            "fulfillBinding('author-script');",
            "</script>"
        )
        .as_bytes(),
    );
    ctx.process_async(json!({
        "id": 36_606,
        "method": "Fetch.fulfillRequest",
        "sessionId": "SID-1",
        "params": {
            "requestId": request_id,
            "responseCode": 200,
            "responseHeaders": [{ "name": "content-type", "value": "text/html" }],
            "body": body
        }
    }))
    .await;
    ctx.expect_result(36_606, json!({}), Some("SID-1"));
    assert_eq!(
        take_response_by_id(&mut ctx, 36_602)["result"]["frameId"],
        json!("TID-1")
    );

    ctx.process_async(json!({
        "id": 36_607,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": "globalThis.__fulfillCommitOrdering",
            "returnByValue": true
        }
    }))
    .await;
    assert_eq!(
        take_response_by_id(&mut ctx, 36_607)["result"]["result"]["value"],
        json!(r#"["ready","function"]"#)
    );

    let named_context_id = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Runtime.executionContextCreated")
                && message["params"]["context"]["name"] == json!("fulfill-world")
        })
        .and_then(|message| message["params"]["context"]["id"].as_i64())
        .expect("fulfillRequest commit should publish the named-world execution context");
    ctx.process_async(json!({
        "id": 36_608,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "contextId": named_context_id,
            "expression": "globalThis.__fulfillNamed",
            "returnByValue": true
        }
    }))
    .await;
    assert_eq!(
        take_response_by_id(&mut ctx, 36_608)["result"]["result"]["value"],
        json!("ready")
    );

    let binding_payloads = ctx
        .sent
        .iter()
        .filter(|message| {
            message["method"] == json!("Runtime.bindingCalled")
                && message["params"]["name"] == json!("fulfillBinding")
        })
        .map(|message| message["params"]["payload"].clone())
        .collect::<Vec<_>>();
    assert_eq!(
        binding_payloads,
        vec![json!("named-preload"), json!("author-script")],
        "fulfillRequest named-world preload must run before its first author script"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn interleaved_response_heads_only_commit_the_current_prepared_document() {
    async fn first() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body><main>first</main></body></html>",
        )
    }

    async fn second() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body><main>second</main></body></html>",
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/first", get(first))
                .route("/second", get(second)),
        )
        .await
        .unwrap();
    });

    let mut ctx = TestContext::new();
    let mut bc = attached_browser_context();
    bc.active_page_state_mut()
        .runtime_slot
        .enable_primary_network_events();
    ctx.conn.browser_context = Some(bc);

    ctx.process_async(json!({
        "id": 364,
        "method": "Fetch.enable",
        "sessionId": "SID-1",
        "params": {
            "patterns": [{
                "urlPattern": "*",
                "requestStage": "Response",
                "resourceType": "Document"
            }]
        }
    }))
    .await;
    ctx.expect_result(364, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 365,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": format!("http://{addr}/first") }
    }))
    .await;
    let first_pause = take_main_document_request_pause(&mut ctx).await;
    let first_request_id = first_pause["params"]["requestId"]
        .as_str()
        .expect("first response-stage request id")
        .to_owned();
    let first_agent = ctx
        .conn
        .browser_context
        .as_ref()
        .and_then(|bc| {
            bc.active_page_state()
                .fetch_owner
                .pending_fetch_response_prepared_renderer_agent_for_test(&first_request_id)
        })
        .expect("first response head should reserve a renderer agent");

    ctx.process_async(json!({
        "id": 366,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": format!("http://{addr}/second") }
    }))
    .await;
    let second_pause = take_main_document_request_pause(&mut ctx).await;
    let second_request_id = second_pause["params"]["requestId"]
        .as_str()
        .expect("second response-stage request id")
        .to_owned();
    let second_agent = ctx
        .conn
        .browser_context
        .as_ref()
        .and_then(|bc| {
            bc.active_page_state()
                .fetch_owner
                .pending_fetch_response_prepared_renderer_agent_for_test(&second_request_id)
        })
        .expect("second response head should reserve a renderer agent");
    assert_ne!(first_agent, second_agent);

    let attachment_before_continue = ctx.conn.browser_context.as_ref().and_then(|bc| {
        bc.active_page_state()
            .runtime_slot
            .current_renderer_attachment()
    });
    ctx.process_async(json!({
        "id": 367,
        "method": "Fetch.continueResponse",
        "sessionId": "SID-1",
        "params": { "requestId": first_request_id }
    }))
    .await;
    ctx.expect_result(367, json!({}), Some("SID-1"));
    let superseded = take_response_by_id(&mut ctx, 365);
    assert_eq!(superseded["error"]["code"], -32000);
    assert_eq!(
        superseded["error"]["message"],
        "renderer channel navigation was superseded by a newer navigation"
    );
    assert_eq!(
        ctx.conn.browser_context.as_ref().and_then(|bc| bc
            .active_page_state()
            .runtime_slot
            .current_renderer_attachment()),
        attachment_before_continue,
        "a superseded response head must not switch the renderer channel"
    );

    ctx.process_async(json!({
        "id": 368,
        "method": "Fetch.continueResponse",
        "sessionId": "SID-1",
        "params": { "requestId": second_request_id }
    }))
    .await;
    ctx.expect_result(368, json!({}), Some("SID-1"));
    let current_navigation = take_response_by_id(&mut ctx, 366);
    assert_eq!(current_navigation["result"]["frameId"], "TID-1");
    assert_eq!(
        current_navigation["result"]["loaderId"],
        second_pause["params"]["networkId"]
    );

    let page = ctx
        .conn
        .browser_context
        .as_ref()
        .and_then(|bc| bc.loaded_page())
        .expect("current navigation should commit a page");
    assert_eq!(page.renderer_devtools_agent_token(), second_agent);
    assert_ne!(page.renderer_devtools_agent_token(), first_agent);
    let html = page
        .serialize_html_async()
        .await
        .expect("current page should serialize HTML");
    assert!(html.contains("second"));
    assert_eq!(
        ctx.sent
            .iter()
            .filter(|message| message["method"] == json!("Page.frameNavigated"))
            .count(),
        1,
        "interleaved response heads must publish exactly one document commit"
    );

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn response_stage_continue_request_rejects_data_url_override_without_consuming_pause() {
    let mut ctx = TestContext::new();
    let mut bc = attached_browser_context();
    bc.active_page_state_mut()
        .runtime_slot
        .enable_primary_network_events();
    ctx.conn.browser_context = Some(bc);
    let data_url = "data:text/html,<html><body><main>data response stage</main></body></html>";

    ctx.process_async(json!({
        "id": 390,
        "method": "Fetch.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(390, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 391,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": "http://example.test/placeholder" }
    }))
    .await;

    let paused = take_main_document_request_pause(&mut ctx).await;
    let request_id = paused["params"]["requestId"]
        .as_str()
        .expect("fetch request id")
        .to_owned();

    ctx.process_async(json!({
        "id": 392,
        "method": "Fetch.continueRequest",
        "sessionId": "SID-1",
        "params": {
            "requestId": request_id,
            "url": data_url,
            "interceptResponse": true
        }
    }))
    .await;
    ctx.expect_error(392, -32602, "InvalidParams");

    let bc = ctx
        .conn
        .browser_context
        .as_ref()
        .expect("browser context should remain active");
    assert!(
        bc.active_page_state()
            .fetch_owner
            .has_pending_fetch_request_id_for_test(&request_id)
    );
    assert!(
        bc.active_page_state()
            .fetch_owner
            .has_pending_fetch_navigation_for_test(&request_id)
    );
    assert!(
        !bc.has_loaded_page(),
        "rejected URL override must not build the page or consume the pause"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn response_stage_continue_request_rejects_file_url_override_without_consuming_pause() {
    let mut ctx = TestContext::new();
    let mut bc = attached_browser_context();
    bc.active_page_state_mut()
        .runtime_slot
        .enable_primary_network_events();
    ctx.conn.browser_context = Some(bc);
    ctx.process_async(json!({
        "id": 395,
        "method": "Fetch.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(395, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 396,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": "http://example.test/placeholder" }
    }))
    .await;

    let paused = take_main_document_request_pause(&mut ctx).await;
    let request_id = paused["params"]["requestId"]
        .as_str()
        .expect("fetch request id")
        .to_owned();

    ctx.process_async(json!({
        "id": 397,
        "method": "Fetch.continueRequest",
        "sessionId": "SID-1",
        "params": {
            "requestId": request_id,
            "url": "file:///etc/passwd",
            "interceptResponse": true
        }
    }))
    .await;
    ctx.expect_error(397, -32602, "InvalidParams");

    let bc = ctx
        .conn
        .browser_context
        .as_ref()
        .expect("browser context should remain active");
    assert!(
        bc.active_page_state()
            .fetch_owner
            .has_pending_fetch_request_id_for_test(&request_id)
    );
    assert!(
        bc.active_page_state()
            .fetch_owner
            .has_pending_fetch_navigation_for_test(&request_id)
    );
    assert!(
        !bc.has_loaded_page(),
        "rejected URL override must not build the page or consume the pause"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn response_stage_continue_response_streams_network_events_through_background_sink() {
    async fn handler() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body><main>response-stage background</main></body></html>",
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, Router::new().route("/page", get(handler)))
            .await
            .unwrap();
    });

    let mut ctx = TestContext::new();
    let mut bc = attached_browser_context();
    bc.active_page_state_mut()
        .runtime_slot
        .enable_primary_network_events();
    ctx.conn.browser_context = Some(bc);
    let (background_tx, mut background_rx) = tokio::sync::mpsc::unbounded_channel();
    ctx.conn.set_background_event_sender(background_tx);
    let url = format!("http://{addr}/page");

    ctx.process_async(json!({
        "id": 364,
        "method": "Fetch.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(364, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 365,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": url }
    }))
    .await;

    let mut early_navigation_events = Vec::new();
    for expected_method in ["Page.frameStartedNavigating", "Page.frameStartedLoading"] {
        let event = ctx.take_one();
        assert_eq!(event["method"], expected_method);
        early_navigation_events.push(event);
    }
    assert_eq!(
        early_navigation_events[0]["params"]["loaderId"],
        json!(LOADER_ID)
    );

    let paused = loop {
        let message = ctx.take_one();
        match message["method"].as_str() {
            Some("Fetch.requestPaused") => break message,
            Some("Network.requestWillBeSent") | Some("Network.requestWillBeSentExtraInfo") => {}
            other => {
                panic!("expected main-document Fetch.requestPaused, got {other:?}: {message:?}")
            }
        }
    };
    let request_id = paused["params"]["requestId"].as_str().unwrap().to_owned();

    ctx.process_async(json!({
        "id": 366,
        "method": "Fetch.continueRequest",
        "sessionId": "SID-1",
        "params": { "requestId": request_id, "interceptResponse": true }
    }))
    .await;
    ctx.expect_result(366, json!({}), Some("SID-1"));
    take_main_document_response_pause(&mut ctx);

    ctx.process_async(json!({
        "id": 367,
        "method": "Fetch.continueResponse",
        "sessionId": "SID-1",
        "params": { "requestId": "INT-1" }
    }))
    .await;
    ctx.expect_result(367, json!({}), Some("SID-1"));
    ctx.expect_result(
        365,
        json!({ "frameId": "TID-1", "loaderId": LOADER_ID }),
        Some("SID-1"),
    );

    let mut background_events = Vec::new();
    while let Ok(event) = background_rx.try_recv() {
        background_events.push(event.into_protocol_message());
    }
    assert!(
        background_events
            .iter()
            .any(|event| event["method"] == json!("Network.responseReceived")
                && event["params"]["requestId"] == LOADER_ID),
        "responseReceived should be emitted through the streaming background sink: {background_events:?}"
    );
    assert!(
        background_events
            .iter()
            .any(|event| event["method"] == json!("Network.loadingFinished")
                && event["params"]["requestId"] == LOADER_ID),
        "loadingFinished should be emitted through the streaming background sink: {background_events:?}"
    );
    assert!(
        ctx.sent.iter().all(|event| {
            event["method"] != json!("Network.responseReceived")
                && event["method"] != json!("Network.loadingFinished")
        }),
        "completion-time output should not duplicate streamed main-document Network events: {:?}",
        ctx.sent
    );

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn continue_response_can_override_status_and_headers() {
    async fn handler() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body><main>response-stage</main></body></html>",
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, Router::new().route("/page", get(handler)))
            .await
            .unwrap();
    });

    let mut ctx = TestContext::new();
    let mut bc = attached_browser_context();
    bc.active_page_state_mut()
        .runtime_slot
        .enable_primary_network_events();
    ctx.conn.browser_context = Some(bc);
    let url = format!("http://{addr}/page");

    ctx.process_async(json!({
        "id": 350,
        "method": "Fetch.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(350, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 351,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": url }
    }))
    .await;

    let paused = take_main_document_request_pause(&mut ctx).await;
    let request_id = paused["params"]["requestId"].as_str().unwrap().to_owned();

    ctx.process_async(json!({
        "id": 352,
        "method": "Fetch.continueRequest",
        "sessionId": "SID-1",
        "params": { "requestId": request_id, "interceptResponse": true }
    }))
    .await;
    ctx.expect_result(352, json!({}), Some("SID-1"));
    let request_extra_info = ctx.take_one();
    assert_eq!(
        request_extra_info["method"],
        "Network.requestWillBeSentExtraInfo"
    );
    assert_eq!(request_extra_info["params"]["requestId"], LOADER_ID);
    let response_extra_info = ctx.take_one();
    assert_eq!(
        response_extra_info["method"],
        "Network.responseReceivedExtraInfo"
    );
    assert_eq!(response_extra_info["params"]["requestId"], LOADER_ID);
    assert_eq!(response_extra_info["params"]["statusCode"], 200);
    assert!(
        response_extra_info["params"]["headers"]
            .get("x-override")
            .is_none()
    );
    assert_eq!(ctx.take_one()["method"], "Fetch.requestPaused");

    ctx.process_async(json!({
        "id": 353,
        "method": "Fetch.continueResponse",
        "sessionId": "SID-1",
        "params": {
            "requestId": "INT-1",
            "responseCode": 201,
            "responseHeaders": [
                { "name": "content-type", "value": "text/html" },
                { "name": "x-override", "value": "yes" }
            ],
            "responsePhrase": "Created"
        }
    }))
    .await;
    ctx.expect_result(353, json!({}), Some("SID-1"));
    ctx.expect_result(
        351,
        json!({ "frameId": "TID-1", "loaderId": LOADER_ID }),
        Some("SID-1"),
    );
    let response = ctx.take_one();
    assert_eq!(response["method"], "Network.responseReceived");
    assert_eq!(response["params"]["response"]["status"], 201);
    assert_eq!(response["params"]["hasExtraInfo"], true);
    assert_eq!(
        response["params"]["response"]["headers"]["x-override"],
        "yes"
    );

    let page = ctx
        .conn
        .browser_context
        .as_ref()
        .and_then(|bc| bc.loaded_page())
        .expect("loaded page");
    assert_eq!(page.status(), 201);
    assert!(
        page.headers()
            .iter()
            .any(|(name, value)| name == "x-override" && value == "yes")
    );

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn continue_response_header_override_keeps_streaming_parser_body() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let script_requested = Arc::new(AtomicBool::new(false));
    let release_tail = Arc::new(tokio::sync::Notify::new());
    let server_script_requested = Arc::clone(&script_requested);
    let server_release_tail = Arc::clone(&release_tail);
    let server = tokio::spawn(async move {
        for _ in 0..2 {
            let Ok((mut stream, _)) = listener.accept().await else {
                return;
            };
            let script_requested = Arc::clone(&server_script_requested);
            let release_tail = Arc::clone(&server_release_tail);
            tokio::spawn(async move {
                let mut request = Vec::new();
                let mut buf = [0_u8; 1024];
                loop {
                    let Ok(read) = stream.read(&mut buf).await else {
                        return;
                    };
                    if read == 0 {
                        return;
                    }
                    request.extend_from_slice(&buf[..read]);
                    if request.windows(4).any(|window| window == b"\r\n\r\n") {
                        break;
                    }
                }
                let request = String::from_utf8_lossy(&request);
                if request.starts_with("GET /gate.js ") {
                    script_requested.store(true, Ordering::SeqCst);
                    release_tail.notify_waiters();
                    let body = b"window.__gateScriptRan = true;";
                    let response = format!(
                        concat!(
                            "HTTP/1.1 200 OK\r\n",
                            "Content-Type: application/javascript\r\n",
                            "Content-Length: {}\r\n",
                            "\r\n"
                        ),
                        body.len()
                    );
                    let _ = stream.write_all(response.as_bytes()).await;
                    let _ = stream.write_all(body).await;
                    return;
                }

                let response = concat!(
                    "HTTP/1.1 200 OK\r\n",
                    "Content-Type: text/html; charset=utf-8\r\n",
                    "Transfer-Encoding: chunked\r\n",
                    "\r\n"
                );
                let first = b"<!doctype html><script src=\"/gate.js\"></script><main id=\"tail\">";
                let _ = stream.write_all(response.as_bytes()).await;
                let _ = stream
                    .write_all(format!("{:x}\r\n", first.len()).as_bytes())
                    .await;
                let _ = stream.write_all(first).await;
                let _ = stream.write_all(b"\r\n").await;
                if tokio::time::timeout(std::time::Duration::from_secs(2), release_tail.notified())
                    .await
                    .is_err()
                {
                    return;
                }
                let tail = b"done</main>";
                let _ = stream
                    .write_all(format!("{:x}\r\n", tail.len()).as_bytes())
                    .await;
                let _ = stream.write_all(tail).await;
                let _ = stream.write_all(b"\r\n0\r\n\r\n").await;
            });
        }
    });

    let mut ctx = TestContext::new();
    ctx.conn.browser_context = Some(attached_browser_context());
    let url = format!("http://{addr}/page");

    ctx.process_async(json!({
        "id": 364,
        "method": "Fetch.enable",
        "sessionId": "SID-1",
        "params": {
            "patterns": [{ "urlPattern": "*", "requestStage": "Request", "resourceType": "Document" }]
        }
    }))
    .await;
    ctx.expect_result(364, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 365,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": url }
    }))
    .await;

    let paused = take_main_document_request_pause(&mut ctx).await;
    let request_id = paused["params"]["requestId"].as_str().unwrap().to_owned();

    ctx.process_async(json!({
        "id": 366,
        "method": "Fetch.continueRequest",
        "sessionId": "SID-1",
        "params": { "requestId": request_id, "interceptResponse": true }
    }))
    .await;
    ctx.expect_result(366, json!({}), Some("SID-1"));
    assert_eq!(ctx.take_one()["method"], "Fetch.requestPaused");

    tokio::time::timeout(
        std::time::Duration::from_secs(4),
        ctx.process_async(json!({
            "id": 367,
            "method": "Fetch.continueResponse",
            "sessionId": "SID-1",
            "params": {
                "requestId": "INT-1",
                "responseHeaders": [
                    { "name": "content-type", "value": "text/html; charset=utf-8" },
                    { "name": "x-override", "value": "streaming" }
                ]
            }
        })),
    )
    .await
    .expect("continueResponse should keep streaming parser body and not wait for body EOF first");
    ctx.expect_result(367, json!({}), Some("SID-1"));
    ctx.expect_result(
        365,
        json!({ "frameId": "TID-1", "loaderId": LOADER_ID }),
        Some("SID-1"),
    );

    let page = ctx
        .conn
        .browser_context
        .as_ref()
        .and_then(|bc| bc.loaded_page())
        .expect("loaded page");
    assert!(
        page.headers()
            .iter()
            .any(|(name, value)| name == "x-override" && value == "streaming")
    );
    assert!(
        script_requested.load(Ordering::SeqCst),
        "parser should request the external script before the main body EOF"
    );
    assert!(
        loaded_page_html_for_test(&mut ctx)
            .await
            .contains("id=\"tail\"")
    );

    server.await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn response_stage_navigation_request_paused_includes_synthesized_cookie_header() {
    async fn handler() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body><main>response-stage</main></body></html>",
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, Router::new().route("/page", get(handler)))
            .await
            .unwrap();
    });

    let mut ctx = TestContext::new();
    let bc = attached_browser_context();
    let url = format!("http://{addr}/page");
    {
        let mut jar = bc.cookie_store_for_test().lock();
        jar.store_response_headers(
            &Url::parse(&url).unwrap(),
            &[("set-cookie".to_owned(), "sid=nav; Path=/page".to_owned())],
        );
    }
    ctx.conn.browser_context = Some(bc);

    ctx.process_async(json!({
        "id": 3_530,
        "method": "Network.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(3_530, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 3_531,
        "method": "Fetch.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(3_531, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 3_532,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": url }
    }))
    .await;

    let paused = take_main_document_request_pause(&mut ctx).await;
    let request_id = paused["params"]["requestId"].as_str().unwrap().to_owned();

    ctx.process_async(json!({
        "id": 3_533,
        "method": "Fetch.continueRequest",
        "sessionId": "SID-1",
        "params": { "requestId": request_id, "interceptResponse": true }
    }))
    .await;
    ctx.expect_result(3_533, json!({}), Some("SID-1"));

    let mut before_response_pause = Vec::new();
    let response_paused = loop {
        let message = ctx.take_one();
        if message["method"] == json!("Fetch.requestPaused") {
            break message;
        }
        before_response_pause.push(message);
    };
    let request_extra_info = before_response_pause
        .iter()
        .find(|message| message["method"] == json!("Network.requestWillBeSentExtraInfo"))
        .expect("request ExtraInfo should precede the response-stage pause");
    assert_eq!(
        request_extra_info["params"]["associatedCookies"][0]["cookie"]["name"],
        "sid"
    );
    assert_eq!(
        request_extra_info["params"]["associatedCookies"][0]["cookie"]["value"],
        "nav"
    );
    assert_eq!(response_paused["method"], "Fetch.requestPaused");
    assert_eq!(response_paused["params"]["responseStatusCode"], 200);
    assert_eq!(
        response_paused["params"]["request"]["headers"]["Cookie"],
        "sid=nav"
    );

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn continue_response_with_binary_response_headers_overrides_headers() {
    async fn handler() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body><main>response-stage</main></body></html>",
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, Router::new().route("/page", get(handler)))
            .await
            .unwrap();
    });

    let mut ctx = TestContext::new();
    ctx.conn.browser_context = Some(attached_browser_context());
    let url = format!("http://{addr}/page");

    ctx.process_async(json!({
        "id": 354,
        "method": "Fetch.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(354, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 355,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": url }
    }))
    .await;

    let paused = take_main_document_request_pause(&mut ctx).await;
    let request_id = paused["params"]["requestId"].as_str().unwrap().to_owned();

    ctx.process_async(json!({
        "id": 356,
        "method": "Fetch.continueRequest",
        "sessionId": "SID-1",
        "params": { "requestId": request_id, "interceptResponse": true }
    }))
    .await;
    ctx.expect_result(356, json!({}), Some("SID-1"));
    assert_eq!(ctx.take_one()["method"], "Fetch.requestPaused");

    ctx.process_async(json!({
        "id": 357,
        "method": "Fetch.continueResponse",
        "sessionId": "SID-1",
        "params": {
            "requestId": "INT-1",
            "responseCode": 201,
            "binaryResponseHeaders": "eC1iaW46IHllcwB4LXR3bzogMgA="
        }
    }))
    .await;
    ctx.expect_result(357, json!({}), Some("SID-1"));
    ctx.expect_result(
        355,
        json!({ "frameId": "TID-1", "loaderId": LOADER_ID }),
        Some("SID-1"),
    );

    let page = ctx
        .conn
        .browser_context
        .as_ref()
        .and_then(|bc| bc.loaded_page())
        .expect("loaded page");
    assert_eq!(page.status(), 201);
    assert!(
        page.headers()
            .iter()
            .any(|(name, value)| name == "x-bin" && value == "yes")
    );
    assert!(
        page.headers()
            .iter()
            .any(|(name, value)| name == "x-two" && value == "2")
    );

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn fail_request_at_response_stage_aborts_navigation() {
    async fn handler() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body><main>response-stage</main></body></html>",
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, Router::new().route("/page", get(handler)))
            .await
            .unwrap();
    });

    let mut ctx = TestContext::new();
    let mut bc = attached_browser_context();
    bc.active_page_state_mut()
        .runtime_slot
        .enable_primary_network_events();
    ctx.conn.browser_context = Some(bc);
    let url = format!("http://{addr}/page");

    ctx.process_async(json!({
        "id": 325,
        "method": "Fetch.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(325, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 326,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": url }
    }))
    .await;

    let paused = take_main_document_request_pause(&mut ctx).await;
    let request_id = paused["params"]["requestId"].as_str().unwrap().to_owned();

    ctx.process_async(json!({
        "id": 327,
        "method": "Fetch.continueRequest",
        "sessionId": "SID-1",
        "params": { "requestId": request_id, "interceptResponse": true }
    }))
    .await;
    ctx.expect_result(327, json!({}), Some("SID-1"));
    let response_paused = take_main_document_response_pause(&mut ctx);
    assert_eq!(response_paused["method"], "Fetch.requestPaused");
    let response_request_id = response_paused["params"]["requestId"]
        .as_str()
        .expect("response-stage request id")
        .to_owned();
    let attachment_before_cancel = ctx.conn.browser_context.as_ref().and_then(|bc| {
        bc.active_page_state()
            .runtime_slot
            .current_renderer_attachment()
    });
    assert!(
        ctx.conn
            .browser_context
            .as_ref()
            .and_then(|bc| {
                bc.active_page_state()
                    .fetch_owner
                    .pending_fetch_response_prepared_renderer_agent_for_test(&response_request_id)
            })
            .is_some(),
        "response head should own a prepared candidate before cancellation"
    );

    ctx.process_async(json!({
        "id": 328,
        "method": "Fetch.failRequest",
        "sessionId": "SID-1",
        "params": { "requestId": response_request_id, "errorReason": "Aborted" }
    }))
    .await;
    ctx.expect_result(328, json!({}), Some("SID-1"));
    let failed = ctx.take_one();
    assert_eq!(failed["method"], "Network.loadingFailed");
    assert_eq!(failed["params"]["requestId"], LOADER_ID);
    ctx.expect_error(326, -32000, "Aborted");
    let browser_context = ctx.conn.browser_context.as_ref().expect("browser context");
    assert!(browser_context.loaded_page().is_none());
    assert_eq!(
        browser_context
            .active_page_state()
            .runtime_slot
            .current_renderer_attachment(),
        attachment_before_cancel,
        "canceling a response-stage candidate must not switch the renderer channel"
    );

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn fulfill_request_at_response_stage_replaces_the_network_candidate_once() {
    async fn handler() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body><main>network-body</main></body></html>",
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, Router::new().route("/page", get(handler)))
            .await
            .unwrap();
    });

    let mut ctx = TestContext::new();
    let mut bc = attached_browser_context();
    bc.active_page_state_mut()
        .runtime_slot
        .enable_primary_network_events();
    ctx.conn.browser_context = Some(bc);

    ctx.process_async(json!({
        "id": 369,
        "method": "Fetch.enable",
        "sessionId": "SID-1",
        "params": {
            "patterns": [{
                "urlPattern": "*",
                "requestStage": "Response",
                "resourceType": "Document"
            }]
        }
    }))
    .await;
    ctx.expect_result(369, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 370,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": format!("http://{addr}/page") }
    }))
    .await;
    let paused = take_main_document_request_pause(&mut ctx).await;
    let request_id = paused["params"]["requestId"]
        .as_str()
        .expect("response-stage request id")
        .to_owned();
    let network_agent = ctx
        .conn
        .browser_context
        .as_ref()
        .and_then(|bc| {
            bc.active_page_state()
                .fetch_owner
                .pending_fetch_response_prepared_renderer_agent_for_test(&request_id)
        })
        .expect("network response head should reserve a renderer agent");

    ctx.process_async(json!({
        "id": 371,
        "method": "Fetch.fulfillRequest",
        "sessionId": "SID-1",
        "params": {
            "requestId": request_id,
            "responseCode": 200,
            "responseHeaders": [{ "name": "content-type", "value": "text/html" }],
            "body": "PCFkb2N0eXBlIGh0bWw+PGh0bWw+PGJvZHk+PG1haW4+c3ludGhldGljLWJvZHk8L21haW4+PC9ib2R5PjwvaHRtbD4="
        }
    }))
    .await;
    ctx.expect_result(371, json!({}), Some("SID-1"));
    ctx.expect_result(
        370,
        json!({ "frameId": "TID-1", "loaderId": LOADER_ID }),
        Some("SID-1"),
    );

    let page = ctx
        .conn
        .browser_context
        .as_ref()
        .and_then(|bc| bc.loaded_page())
        .expect("synthetic response should commit a page");
    assert_ne!(
        page.renderer_devtools_agent_token(),
        network_agent,
        "fulfillRequest must discard the prepared network response candidate"
    );
    let html = page
        .serialize_html_async()
        .await
        .expect("synthetic response page should serialize HTML");
    assert!(html.contains("synthetic-body"));
    assert!(!html.contains("network-body"));
    assert_eq!(
        ctx.sent
            .iter()
            .filter(|message| message["method"] == json!("Page.frameNavigated"))
            .count(),
        1,
        "fulfillRequest should publish exactly one document commit"
    );

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn take_response_body_as_stream_at_response_stage_returns_stream_and_keeps_pause_state() {
    let (tail_tx, tail_rx) = tokio::sync::oneshot::channel::<()>();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut buf = [0u8; 2048];
        let _ = stream.read(&mut buf).await.unwrap();
        let head = "<!doctype html><html";
        let tail = "><body><main>response-stage</main></body></html>";
        let response_head = "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n";
        stream.write_all(response_head.as_bytes()).await.unwrap();
        stream
            .write_all(format!("{:X}\r\n{head}\r\n", head.len()).as_bytes())
            .await
            .unwrap();
        let _ = tail_rx.await;
        stream
            .write_all(format!("{:X}\r\n{tail}\r\n0\r\n\r\n", tail.len()).as_bytes())
            .await
            .unwrap();
        let _ = stream.shutdown().await;
    });

    let mut ctx = TestContext::new();
    ctx.conn.browser_context = Some(attached_browser_context());
    let url = format!("http://{addr}/page");

    ctx.process_async(json!({
        "id": 329,
        "method": "Fetch.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(329, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 330,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": url }
    }))
    .await;

    let paused = take_main_document_request_pause(&mut ctx).await;
    let request_id = paused["params"]["requestId"].as_str().unwrap().to_owned();

    ctx.process_async(json!({
        "id": 331,
        "method": "Fetch.continueRequest",
        "sessionId": "SID-1",
        "params": { "requestId": request_id, "interceptResponse": true }
    }))
    .await;
    ctx.expect_result(331, json!({}), Some("SID-1"));
    assert_eq!(ctx.take_one()["method"], "Fetch.requestPaused");

    ctx.process_async(json!({
        "id": 332,
        "method": "Fetch.takeResponseBodyAsStream",
        "sessionId": "SID-1",
        "params": { "requestId": "INT-1" }
    }))
    .await;
    ctx.expect_result(
        332,
        json!({ "stream": "BID-1:TID-1:STREAM-1" }),
        Some("SID-1"),
    );

    tokio::time::timeout(
        std::time::Duration::from_millis(300),
        ctx.process_async(json!({
            "id": 333,
            "method": "IO.read",
            "params": { "handle": "BID-1:TID-1:STREAM-1", "size": 20 }
        })),
    )
    .await
    .expect("IO.read should return the available response body chunk before body EOF");
    ctx.expect_result(
        333,
        json!({
            "base64Encoded": false,
            "data": "<!doctype html><html",
            "eof": false
        }),
        None,
    );

    tail_tx.send(()).unwrap();
    ctx.process_async(json!({
        "id": 334,
        "method": "IO.read",
        "params": { "handle": "BID-1:TID-1:STREAM-1" }
    }))
    .await;
    ctx.expect_result(
        334,
        json!({
            "base64Encoded": false,
            "data": "><body><main>response-stage</main></body></html>",
            "eof": true
        }),
        None,
    );

    ctx.process_async(json!({
        "id": 335,
        "method": "Fetch.continueResponse",
        "sessionId": "SID-1",
        "params": { "requestId": "INT-1" }
    }))
    .await;
    ctx.expect_result(335, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 336,
        "method": "IO.close",
        "params": { "handle": "BID-1:TID-1:STREAM-1" }
    }))
    .await;
    ctx.expect_result(336, json!({}), None);

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn continue_response_rejects_while_response_body_stream_is_active() {
    let (tail_tx, tail_rx) = tokio::sync::oneshot::channel::<()>();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut buf = [0u8; 2048];
        let _ = stream.read(&mut buf).await.unwrap();
        let head = "<!doctype html><html";
        let tail = "><body><main>body-stream-active</main></body></html>";
        let response_head = "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n";
        stream.write_all(response_head.as_bytes()).await.unwrap();
        stream
            .write_all(format!("{:X}\r\n{head}\r\n", head.len()).as_bytes())
            .await
            .unwrap();
        let _ = tail_rx.await;
        stream
            .write_all(format!("{:X}\r\n{tail}\r\n0\r\n\r\n", tail.len()).as_bytes())
            .await
            .unwrap();
        let _ = stream.shutdown().await;
    });

    let mut ctx = TestContext::new();
    ctx.conn.browser_context = Some(attached_browser_context());
    let url = format!("http://{addr}/page");

    ctx.process_async(json!({
        "id": 337,
        "method": "Fetch.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(337, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 338,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": url }
    }))
    .await;

    let paused = take_main_document_request_pause(&mut ctx).await;
    let request_id = paused["params"]["requestId"].as_str().unwrap().to_owned();

    ctx.process_async(json!({
        "id": 339,
        "method": "Fetch.continueRequest",
        "sessionId": "SID-1",
        "params": { "requestId": request_id, "interceptResponse": true }
    }))
    .await;
    ctx.expect_result(339, json!({}), Some("SID-1"));
    assert_eq!(ctx.take_one()["method"], "Fetch.requestPaused");

    ctx.process_async(json!({
        "id": 340,
        "method": "Fetch.takeResponseBodyAsStream",
        "sessionId": "SID-1",
        "params": { "requestId": "INT-1" }
    }))
    .await;
    ctx.expect_result(
        340,
        json!({ "stream": "BID-1:TID-1:STREAM-1" }),
        Some("SID-1"),
    );

    ctx.process_async(json!({
        "id": 341,
        "method": "IO.read",
        "params": { "handle": "BID-1:TID-1:STREAM-1", "size": 20 }
    }))
    .await;
    ctx.expect_result(
        341,
        json!({
            "base64Encoded": false,
            "data": "<!doctype html><html",
            "eof": false
        }),
        None,
    );

    ctx.process_async(json!({
        "id": 342,
        "method": "Fetch.continueResponse",
        "sessionId": "SID-1",
        "params": { "requestId": "INT-1" }
    }))
    .await;
    ctx.expect_error(342, -32000, "ResponseBodyStreamActive");

    tail_tx.send(()).unwrap();
    ctx.process_async(json!({
        "id": 343,
        "method": "IO.read",
        "params": { "handle": "BID-1:TID-1:STREAM-1" }
    }))
    .await;
    ctx.expect_result(
        343,
        json!({
            "base64Encoded": false,
            "data": "><body><main>body-stream-active</main></body></html>",
            "eof": true
        }),
        None,
    );

    ctx.process_async(json!({
        "id": 344,
        "method": "Fetch.continueResponse",
        "sessionId": "SID-1",
        "params": { "requestId": "INT-1" }
    }))
    .await;
    ctx.expect_result(344, json!({}), Some("SID-1"));

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn io_close_cancels_active_response_body_stream_and_request_id() {
    let (_tail_tx, tail_rx) = tokio::sync::oneshot::channel::<()>();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut buf = [0u8; 2048];
        let _ = stream.read(&mut buf).await.unwrap();
        let head = "<!doctype html><html";
        let tail = "><body><main>should-not-load</main></body></html>";
        let response_head = "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n";
        stream.write_all(response_head.as_bytes()).await.unwrap();
        stream
            .write_all(format!("{:X}\r\n{head}\r\n", head.len()).as_bytes())
            .await
            .unwrap();
        let _ = tail_rx.await;
        let _ = stream
            .write_all(format!("{:X}\r\n{tail}\r\n0\r\n\r\n", tail.len()).as_bytes())
            .await;
    });

    let mut ctx = TestContext::new();
    ctx.conn.browser_context = Some(attached_browser_context());
    let url = format!("http://{addr}/page");

    ctx.process_async(json!({
        "id": 345,
        "method": "Fetch.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(345, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 346,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": url }
    }))
    .await;

    let paused = take_main_document_request_pause(&mut ctx).await;
    let request_id = paused["params"]["requestId"].as_str().unwrap().to_owned();

    ctx.process_async(json!({
        "id": 347,
        "method": "Fetch.continueRequest",
        "sessionId": "SID-1",
        "params": { "requestId": request_id, "interceptResponse": true }
    }))
    .await;
    ctx.expect_result(347, json!({}), Some("SID-1"));
    assert_eq!(ctx.take_one()["method"], "Fetch.requestPaused");

    ctx.process_async(json!({
        "id": 348,
        "method": "Fetch.takeResponseBodyAsStream",
        "sessionId": "SID-1",
        "params": { "requestId": "INT-1" }
    }))
    .await;
    ctx.expect_result(
        348,
        json!({ "stream": "BID-1:TID-1:STREAM-1" }),
        Some("SID-1"),
    );

    ctx.process_async(json!({
        "id": 349,
        "method": "IO.close",
        "params": { "handle": "BID-1:TID-1:STREAM-1" }
    }))
    .await;
    ctx.expect_result(349, json!({}), None);

    ctx.process_async(json!({
        "id": 350,
        "method": "Fetch.continueResponse",
        "sessionId": "SID-1",
        "params": { "requestId": "INT-1" }
    }))
    .await;
    ctx.expect_error(350, -32000, "RequestNotFound");

    ctx.process_async(json!({
        "id": 351,
        "method": "IO.read",
        "params": { "handle": "BID-1:TID-1:STREAM-1" }
    }))
    .await;
    ctx.expect_error(351, -32000, "StreamHandleNotFound");

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn get_response_body_does_not_consume_active_response_body_stream() {
    let (tail_tx, tail_rx) = tokio::sync::oneshot::channel::<()>();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut buf = [0u8; 2048];
        let _ = stream.read(&mut buf).await.unwrap();
        let head = "<!doctype html><html";
        let tail = "><body><main>body-stream-still-active</main></body></html>";
        let response_head = "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n";
        stream.write_all(response_head.as_bytes()).await.unwrap();
        stream
            .write_all(format!("{:X}\r\n{head}\r\n", head.len()).as_bytes())
            .await
            .unwrap();
        let _ = tail_rx.await;
        stream
            .write_all(format!("{:X}\r\n{tail}\r\n0\r\n\r\n", tail.len()).as_bytes())
            .await
            .unwrap();
        let _ = stream.shutdown().await;
    });

    let mut ctx = TestContext::new();
    ctx.conn.browser_context = Some(attached_browser_context());
    let url = format!("http://{addr}/page");

    ctx.process_async(json!({
        "id": 361,
        "method": "Fetch.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(361, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 362,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": url }
    }))
    .await;

    let paused = take_main_document_request_pause(&mut ctx).await;
    let request_id = paused["params"]["requestId"].as_str().unwrap().to_owned();

    ctx.process_async(json!({
        "id": 363,
        "method": "Fetch.continueRequest",
        "sessionId": "SID-1",
        "params": { "requestId": request_id, "interceptResponse": true }
    }))
    .await;
    ctx.expect_result(363, json!({}), Some("SID-1"));
    assert_eq!(ctx.take_one()["method"], "Fetch.requestPaused");

    ctx.process_async(json!({
        "id": 364,
        "method": "Fetch.takeResponseBodyAsStream",
        "sessionId": "SID-1",
        "params": { "requestId": "INT-1" }
    }))
    .await;
    ctx.expect_result(
        364,
        json!({ "stream": "BID-1:TID-1:STREAM-1" }),
        Some("SID-1"),
    );
    assert_eq!(
        ctx.conn
            .browser_context
            .as_ref()
            .unwrap()
            .active_page_state()
            .fetch_owner
            .active_fetch_response_body_stream_request_id_for_test("BID-1:TID-1:STREAM-1"),
        Some("INT-1")
    );

    ctx.process_async(json!({
        "id": 365,
        "method": "IO.read",
        "params": { "handle": "BID-1:TID-1:STREAM-1", "size": 20 }
    }))
    .await;
    ctx.expect_result(
        365,
        json!({
            "base64Encoded": false,
            "data": "<!doctype html><html",
            "eof": false
        }),
        None,
    );
    assert_eq!(
        ctx.conn
            .browser_context
            .as_ref()
            .unwrap()
            .active_page_state()
            .fetch_owner
            .active_fetch_response_body_stream_request_id_for_test("BID-1:TID-1:STREAM-1"),
        Some("INT-1")
    );

    ctx.process_async(json!({
        "id": 366,
        "method": "Fetch.getResponseBody",
        "sessionId": "SID-1",
        "params": { "requestId": "INT-1" }
    }))
    .await;
    ctx.expect_error(366, -32000, "RequestNotFound");
    assert_eq!(
        ctx.conn
            .browser_context
            .as_ref()
            .unwrap()
            .active_page_state()
            .fetch_owner
            .active_fetch_response_body_stream_request_id_for_test("BID-1:TID-1:STREAM-1"),
        Some("INT-1")
    );

    tail_tx.send(()).unwrap();
    ctx.process_async(json!({
        "id": 367,
        "method": "IO.read",
        "params": { "handle": "BID-1:TID-1:STREAM-1" }
    }))
    .await;
    ctx.expect_result(
        367,
        json!({
            "base64Encoded": false,
            "data": "><body><main>body-stream-still-active</main></body></html>",
            "eof": true
        }),
        None,
    );
    assert!(
        ctx.conn
            .browser_context
            .as_ref()
            .unwrap()
            .active_page_state()
            .fetch_owner
            .active_fetch_response_body_stream_request_id_for_test("BID-1:TID-1:STREAM-1")
            .is_none()
    );

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn fulfill_request_cancels_active_response_body_stream_and_uses_synthetic_response() {
    let (_tail_tx, tail_rx) = tokio::sync::oneshot::channel::<()>();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut buf = [0u8; 2048];
        let _ = stream.read(&mut buf).await.unwrap();
        let head = "<!doctype html><html";
        let tail = "><body><main>should-not-load</main></body></html>";
        let response_head = "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n";
        stream.write_all(response_head.as_bytes()).await.unwrap();
        stream
            .write_all(format!("{:X}\r\n{head}\r\n", head.len()).as_bytes())
            .await
            .unwrap();
        let _ = tail_rx.await;
        let _ = stream
            .write_all(format!("{:X}\r\n{tail}\r\n0\r\n\r\n", tail.len()).as_bytes())
            .await;
    });

    let mut ctx = TestContext::new();
    ctx.conn.browser_context = Some(attached_browser_context());
    let url = format!("http://{addr}/page");

    ctx.process_async(json!({
        "id": 345,
        "method": "Fetch.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(345, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 346,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": url }
    }))
    .await;

    let paused = take_main_document_request_pause(&mut ctx).await;
    let request_id = paused["params"]["requestId"].as_str().unwrap().to_owned();

    ctx.process_async(json!({
        "id": 347,
        "method": "Fetch.continueRequest",
        "sessionId": "SID-1",
        "params": { "requestId": request_id, "interceptResponse": true }
    }))
    .await;
    ctx.expect_result(347, json!({}), Some("SID-1"));
    assert_eq!(ctx.take_one()["method"], "Fetch.requestPaused");

    ctx.process_async(json!({
        "id": 348,
        "method": "Fetch.takeResponseBodyAsStream",
        "sessionId": "SID-1",
        "params": { "requestId": "INT-1" }
    }))
    .await;
    ctx.expect_result(
        348,
        json!({ "stream": "BID-1:TID-1:STREAM-1" }),
        Some("SID-1"),
    );

    ctx.process_async(json!({
        "id": 349,
        "method": "Fetch.fulfillRequest",
        "sessionId": "SID-1",
        "params": {
            "requestId": "INT-1",
            "responseCode": 200,
            "responseHeaders": [{ "name": "content-type", "value": "text/html" }],
            "body": "PCFkb2N0eXBlIGh0bWw+PGh0bWw+PGJvZHk+PG1haW4+aW8tZnVsZmlsbGVkPC9tYWluPjwvYm9keT48L2h0bWw+"
        }
    }))
    .await;
    ctx.expect_result(349, json!({}), Some("SID-1"));
    ctx.expect_result(
        346,
        json!({ "frameId": "TID-1", "loaderId": LOADER_ID }),
        Some("SID-1"),
    );

    let html = loaded_page_html_for_test(&mut ctx).await;
    assert!(html.contains("io-fulfilled"));
    assert!(!html.contains("should-not-load"));

    ctx.process_async(json!({
        "id": 350,
        "method": "IO.read",
        "params": { "handle": "BID-1:TID-1:STREAM-1" }
    }))
    .await;
    ctx.expect_error(350, -32000, "StreamHandleNotFound");

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn fail_request_cancels_active_response_body_stream_and_fails_navigation() {
    let (_tail_tx, tail_rx) = tokio::sync::oneshot::channel::<()>();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut buf = [0u8; 2048];
        let _ = stream.read(&mut buf).await.unwrap();
        let head = "<!doctype html><html";
        let tail = "><body><main>should-not-load</main></body></html>";
        let response_head = "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n";
        stream.write_all(response_head.as_bytes()).await.unwrap();
        stream
            .write_all(format!("{:X}\r\n{head}\r\n", head.len()).as_bytes())
            .await
            .unwrap();
        let _ = tail_rx.await;
        let _ = stream
            .write_all(format!("{:X}\r\n{tail}\r\n0\r\n\r\n", tail.len()).as_bytes())
            .await;
    });

    let mut ctx = TestContext::new();
    ctx.conn.browser_context = Some(attached_browser_context());
    let url = format!("http://{addr}/page");

    ctx.process_async(json!({
        "id": 351,
        "method": "Fetch.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(351, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 352,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": url }
    }))
    .await;

    let paused = take_main_document_request_pause(&mut ctx).await;
    let request_id = paused["params"]["requestId"].as_str().unwrap().to_owned();

    ctx.process_async(json!({
        "id": 353,
        "method": "Fetch.continueRequest",
        "sessionId": "SID-1",
        "params": { "requestId": request_id, "interceptResponse": true }
    }))
    .await;
    ctx.expect_result(353, json!({}), Some("SID-1"));
    assert_eq!(ctx.take_one()["method"], "Fetch.requestPaused");

    ctx.process_async(json!({
        "id": 354,
        "method": "Fetch.takeResponseBodyAsStream",
        "sessionId": "SID-1",
        "params": { "requestId": "INT-1" }
    }))
    .await;
    ctx.expect_result(
        354,
        json!({ "stream": "BID-1:TID-1:STREAM-1" }),
        Some("SID-1"),
    );

    ctx.process_async(json!({
        "id": 355,
        "method": "Fetch.failRequest",
        "sessionId": "SID-1",
        "params": { "requestId": "INT-1", "errorReason": "Aborted" }
    }))
    .await;
    ctx.expect_result(355, json!({}), Some("SID-1"));
    ctx.expect_error(352, -32000, "Aborted");

    ctx.process_async(json!({
        "id": 356,
        "method": "IO.read",
        "params": { "handle": "BID-1:TID-1:STREAM-1" }
    }))
    .await;
    ctx.expect_error(356, -32000, "StreamHandleNotFound");

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn clear_browser_cache_cancels_active_response_body_stream_without_stale_request() {
    let (_tail_tx, tail_rx) = tokio::sync::oneshot::channel::<()>();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut buf = [0u8; 2048];
        let _ = stream.read(&mut buf).await.unwrap();
        let head = "<!doctype html><html";
        let tail = "><body><main>should-not-load</main></body></html>";
        let response_head = "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n";
        stream.write_all(response_head.as_bytes()).await.unwrap();
        stream
            .write_all(format!("{:X}\r\n{head}\r\n", head.len()).as_bytes())
            .await
            .unwrap();
        let _ = tail_rx.await;
        let _ = stream
            .write_all(format!("{:X}\r\n{tail}\r\n0\r\n\r\n", tail.len()).as_bytes())
            .await;
    });

    let mut ctx = TestContext::new();
    ctx.conn.browser_context = Some(attached_browser_context());
    let url = format!("http://{addr}/page");

    ctx.process_async(json!({
        "id": 357,
        "method": "Fetch.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(357, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 358,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": url }
    }))
    .await;

    let paused = take_main_document_request_pause(&mut ctx).await;
    let request_id = paused["params"]["requestId"].as_str().unwrap().to_owned();

    ctx.process_async(json!({
        "id": 359,
        "method": "Fetch.continueRequest",
        "sessionId": "SID-1",
        "params": { "requestId": request_id, "interceptResponse": true }
    }))
    .await;
    ctx.expect_result(359, json!({}), Some("SID-1"));
    assert_eq!(ctx.take_one()["method"], "Fetch.requestPaused");

    ctx.process_async(json!({
        "id": 360,
        "method": "Fetch.takeResponseBodyAsStream",
        "sessionId": "SID-1",
        "params": { "requestId": "INT-1" }
    }))
    .await;
    ctx.expect_result(
        360,
        json!({ "stream": "BID-1:TID-1:STREAM-1" }),
        Some("SID-1"),
    );

    ctx.process_async(json!({
        "id": 361,
        "method": "Network.clearBrowserCache"
    }))
    .await;
    ctx.expect_result(361, json!({}), None);

    assert!(
        ctx.conn
            .browser_context
            .as_ref()
            .unwrap()
            .active_page_state()
            .fetch_owner
            .active_fetch_response_body_stream_request_id_for_test("BID-1:TID-1:STREAM-1")
            .is_none()
    );

    ctx.process_async(json!({
        "id": 362,
        "method": "IO.read",
        "params": { "handle": "BID-1:TID-1:STREAM-1" }
    }))
    .await;
    ctx.expect_error(362, -32000, "StreamHandleNotFound");

    ctx.process_async(json!({
        "id": 363,
        "method": "Fetch.continueResponse",
        "sessionId": "SID-1",
        "params": { "requestId": "INT-1" }
    }))
    .await;
    ctx.expect_error(363, -32000, "RequestNotFound");

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn fail_request_finishes_pending_navigation_with_error() {
    async fn handler() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body><main>unused</main></body></html>",
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, Router::new().route("/page", get(handler)))
            .await
            .unwrap();
    });

    let mut ctx = TestContext::new();
    ctx.conn.browser_context = Some(attached_browser_context());
    let url = format!("http://{addr}/page");

    ctx.process_async(json!({
        "id": 33,
        "method": "Fetch.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(33, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 34,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": url }
    }))
    .await;

    let paused = take_main_document_request_pause(&mut ctx).await;
    let request_id = paused["params"]["requestId"]
        .as_str()
        .expect("fetch request id")
        .to_owned();

    ctx.process_async(json!({
        "id": 35,
        "method": "Fetch.failRequest",
        "sessionId": "SID-1",
        "params": { "requestId": request_id, "errorReason": "Aborted" }
    }))
    .await;

    ctx.expect_result(35, json!({}), Some("SID-1"));
    ctx.expect_error(34, -32000, "Aborted");
    assert!(ctx.sent.is_empty());

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn fulfill_request_completes_navigation_with_synthetic_response() {
    let mut ctx = TestContext::new();
    let mut bc = attached_browser_context();
    bc.active_page_state_mut()
        .runtime_slot
        .enable_primary_network_events();
    ctx.conn.browser_context = Some(bc);
    ctx.enable_page_events_for_test(Some("SID-1"));
    ctx.enable_dom_events_for_test(Some("SID-1"));

    ctx.process_async(json!({
        "id": 36,
        "method": "Fetch.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(36, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 37,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": "http://example.test/fulfilled" }
    }))
    .await;

    let paused = take_main_document_request_pause(&mut ctx).await;
    assert_eq!(paused["method"], "Fetch.requestPaused");
    assert_eq!(paused["params"]["networkId"], LOADER_ID);
    let request_id = paused["params"]["requestId"]
        .as_str()
        .expect("fetch request id")
        .to_owned();

    ctx.process_async(json!({
        "id": 38,
        "method": "Fetch.fulfillRequest",
        "sessionId": "SID-1",
        "params": {
            "requestId": request_id,
            "responseCode": 201,
            "responseHeaders": [{ "name": "content-type", "value": "text/plain" }],
            "body": "ZnVsZmlsbGVk"
        }
    }))
    .await;

    ctx.expect_result(38, json!({}), Some("SID-1"));
    ctx.expect_result(
        37,
        json!({ "frameId": "TID-1", "loaderId": LOADER_ID }),
        Some("SID-1"),
    );
    wait_until_frame_stopped_loading(&mut ctx, "TID-1").await;

    let response = ctx.take_one();
    assert_eq!(response["method"], "Network.responseReceived");
    assert_eq!(response["params"]["requestId"], LOADER_ID);
    assert_eq!(response["params"]["response"]["status"], 201);
    assert_eq!(response["params"]["response"]["mimeType"], "text/plain");

    assert_eq!(ctx.take_one()["method"], "Page.frameNavigated");
    assert_eq!(ctx.take_one()["method"], "DOM.documentUpdated");
    assert_eq!(ctx.take_one()["method"], "DOM.documentUpdated");
    assert_eq!(ctx.take_one()["method"], "Page.domContentEventFired");

    let data = ctx.take_one();
    assert_eq!(data["method"], "Network.dataReceived");
    assert_eq!(data["params"]["requestId"], LOADER_ID);
    assert_eq!(data["params"]["dataLength"], json!(9));

    let finished = ctx.take_one();
    assert_eq!(finished["method"], "Network.loadingFinished");
    assert_eq!(finished["params"]["requestId"], LOADER_ID);

    assert_eq!(ctx.take_one()["method"], "Page.loadEventFired");
    assert_eq!(ctx.take_one()["method"], "Page.frameStoppedLoading");

    ctx.process_async(json!({
        "id": 39,
        "method": "Network.getResponseBody",
        "sessionId": "SID-1",
        "params": { "requestId": LOADER_ID }
    }))
    .await;
    ctx.expect_result(
        39,
        json!({ "body": "fulfilled", "base64Encoded": false }),
        Some("SID-1"),
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn fulfill_request_navigation_get_response_body_preserves_binary_bytes() {
    let mut ctx = TestContext::new();
    let mut bc = attached_browser_context();
    bc.active_page_state_mut()
        .runtime_slot
        .enable_primary_network_events();
    ctx.conn.browser_context = Some(bc);

    ctx.process_async(json!({
        "id": 44_001,
        "method": "Fetch.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(44_001, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 44_002,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": "http://example.test/fulfilled-binary" }
    }))
    .await;

    let paused = take_main_document_request_pause(&mut ctx).await;
    let request_id = paused["params"]["requestId"]
        .as_str()
        .expect("fetch request id")
        .to_owned();

    ctx.process_async(json!({
        "id": 44_003,
        "method": "Fetch.fulfillRequest",
        "sessionId": "SID-1",
        "params": {
            "requestId": request_id,
            "responseCode": 200,
            "responseHeaders": [{ "name": "content-type", "value": "text/html" }],
            "body": "AP9h"
        }
    }))
    .await;

    ctx.expect_result(44_003, json!({}), Some("SID-1"));
    ctx.expect_result(
        44_002,
        json!({ "frameId": "TID-1", "loaderId": LOADER_ID }),
        Some("SID-1"),
    );

    wait_until_messages(
        &mut ctx,
        Some("SID-1"),
        "binary synthetic navigation finished",
        |messages| {
            messages.iter().any(|message| {
                message["method"] == json!("Network.loadingFinished")
                    && message["params"]["requestId"] == json!(LOADER_ID)
            })
        },
    )
    .await;

    ctx.process_async(json!({
        "id": 44_004,
        "method": "Network.getResponseBody",
        "sessionId": "SID-1",
        "params": { "requestId": LOADER_ID }
    }))
    .await;
    ctx.expect_result(
        44_004,
        json!({ "body": "AP9h", "base64Encoded": true }),
        Some("SID-1"),
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn fulfill_request_document_body_uses_phase_one_parser_semantics() {
    let mut ctx = TestContext::new();
    let mut bc = attached_browser_context();
    bc.active_page_state_mut()
        .runtime_slot
        .enable_primary_network_events();
    ctx.conn.browser_context = Some(bc);

    ctx.process_async(json!({
        "id": 390,
        "method": "Fetch.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(390, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 391,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": "http://example.test/fulfilled-parser" }
    }))
    .await;

    let paused = take_main_document_request_pause(&mut ctx).await;
    let request_id = paused["params"]["requestId"]
        .as_str()
        .expect("fetch request id")
        .to_owned();

    ctx.process_async(json!({
        "id": 392,
        "method": "Fetch.fulfillRequest",
        "sessionId": "SID-1",
        "params": {
            "requestId": request_id,
            "responseCode": 200,
            "responseHeaders": [{ "name": "content-type", "value": "text/html" }],
            "body": "PCFkb2N0eXBlIGh0bWw+PGh0bWw+PGJvZHk+PHNjcmlwdD5kb2N1bWVudC53cml0ZSgiPG1haW4gaWQ9ZnJvbS13cml0ZT5waGFzZS1vbmU8L21haW4+Iik7PC9zY3JpcHQ+PHNlY3Rpb24gaWQ9dGFpbD50YWlsPC9zZWN0aW9uPjwvYm9keT48L2h0bWw+"
        }
    }))
    .await;

    ctx.expect_result(392, json!({}), Some("SID-1"));
    ctx.expect_result(
        391,
        json!({ "frameId": "TID-1", "loaderId": LOADER_ID }),
        Some("SID-1"),
    );
    while !ctx.sent.is_empty() {
        ctx.take_one();
    }

    ctx.process_async(json!({
        "id": 393,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": "JSON.stringify({write: document.getElementById('from-write')?.textContent, tail: document.getElementById('tail')?.textContent, order: Array.from(document.body.children).map(e => e.id).filter(Boolean).join(',')})"
        }
    }))
    .await;
    let evaluated = take_response_by_id(&mut ctx, 393);
    assert_eq!(
        evaluated["result"]["result"]["value"],
        json!("{\"write\":\"phase-one\",\"tail\":\"tail\",\"order\":\"from-write,tail\"}")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn fulfill_request_accepts_binary_response_headers() {
    let mut ctx = TestContext::new();
    let mut bc = attached_browser_context();
    bc.active_page_state_mut()
        .runtime_slot
        .enable_primary_network_events();
    ctx.conn.browser_context = Some(bc);

    ctx.process_async(json!({
        "id": 40,
        "method": "Fetch.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(40, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 41,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": "http://example.test/binary-fulfilled" }
    }))
    .await;

    let paused = take_main_document_request_pause(&mut ctx).await;
    let request_id = paused["params"]["requestId"]
        .as_str()
        .expect("fetch request id")
        .to_owned();

    ctx.process_async(json!({
        "id": 42,
        "method": "Fetch.fulfillRequest",
        "sessionId": "SID-1",
        "params": {
            "requestId": request_id,
            "responseCode": 202,
            "binaryResponseHeaders": "eC1iaW46IHllcwB4LXR3bzogMgA=",
            "body": "ZnVsZmlsbGVk"
        }
    }))
    .await;

    ctx.expect_result(42, json!({}), Some("SID-1"));
    ctx.expect_result(
        41,
        json!({ "frameId": "TID-1", "loaderId": LOADER_ID }),
        Some("SID-1"),
    );

    let response = ctx
        .sent
        .iter()
        .find(|message| message["method"] == json!("Network.responseReceived"))
        .cloned()
        .expect("response event");
    assert_eq!(response["params"]["response"]["status"], 202);
    assert_eq!(response["params"]["response"]["headers"]["x-bin"], "yes");
    assert_eq!(response["params"]["response"]["headers"]["x-two"], "2");
}

#[tokio::test(flavor = "multi_thread")]
async fn continue_request_applies_url_method_headers_and_post_data() {
    async fn handler(method: Method, headers: HeaderMap, body: String) -> impl IntoResponse {
        let marker = headers
            .get("x-fetch-test")
            .and_then(|value| value.to_str().ok())
            .unwrap_or("");
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            format!(
                "<!doctype html><html><body><main>{}|{}|{}</main></body></html>",
                method, marker, body
            ),
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/start", any(handler))
                .route("/continued", any(handler)),
        )
        .await
        .unwrap();
    });

    let start_url = format!("http://{addr}/start");
    let continued_url = format!("http://{addr}/continued");
    let mut ctx = TestContext::new();
    let mut bc = attached_browser_context();
    bc.active_page_state_mut()
        .runtime_slot
        .enable_primary_network_events();
    ctx.conn.browser_context = Some(bc);
    ctx.enable_page_events_for_test(Some("SID-1"));
    ctx.enable_dom_events_for_test(Some("SID-1"));

    ctx.process_async(json!({
        "id": 40,
        "method": "Fetch.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(40, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 41,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": start_url }
    }))
    .await;

    let paused = take_main_document_request_pause(&mut ctx).await;
    let request_id = paused["params"]["requestId"]
        .as_str()
        .expect("fetch request id")
        .to_owned();
    assert_eq!(paused["params"]["networkId"], LOADER_ID);

    ctx.process_async(json!({
        "id": 42,
        "method": "Fetch.continueRequest",
        "sessionId": "SID-1",
        "params": {
            "requestId": request_id,
            "url": continued_url,
            "method": "POST",
            "headers": [{ "name": "x-fetch-test", "value": "continued" }],
            "postData": "cGF5bG9hZA=="
        }
    }))
    .await;

    ctx.expect_result(42, json!({}), Some("SID-1"));
    assert!(
        !ctx.sent
            .iter()
            .any(|message| message["method"] == json!("Network.requestWillBeSent")),
        "continueRequest should not re-emit Network.requestWillBeSent after request-stage pause"
    );

    ctx.expect_result(
        41,
        json!({ "frameId": "TID-1", "loaderId": LOADER_ID }),
        Some("SID-1"),
    );
    wait_until_frame_stopped_loading(&mut ctx, "TID-1").await;

    let request_extra_info = ctx.take_one();
    assert_eq!(
        request_extra_info["method"],
        "Network.requestWillBeSentExtraInfo"
    );
    assert_eq!(request_extra_info["params"]["requestId"], LOADER_ID);
    let response_extra_info = ctx.take_one();
    assert_eq!(
        response_extra_info["method"],
        "Network.responseReceivedExtraInfo"
    );
    assert_eq!(response_extra_info["params"]["requestId"], LOADER_ID);
    assert_eq!(response_extra_info["params"]["statusCode"], 200);
    let response = ctx.take_one();
    assert_eq!(response["method"], "Network.responseReceived");
    assert_eq!(response["params"]["requestId"], LOADER_ID);
    assert_eq!(response["params"]["response"]["url"], continued_url);

    assert_eq!(ctx.take_one()["method"], "Page.frameNavigated");
    assert_eq!(ctx.take_one()["method"], "DOM.documentUpdated");
    assert_eq!(ctx.take_one()["method"], "DOM.documentUpdated");
    assert_eq!(ctx.take_one()["method"], "Page.domContentEventFired");
    let data = ctx.take_one();
    assert_eq!(data["method"], "Network.dataReceived");
    assert_eq!(data["params"]["requestId"], LOADER_ID);
    assert_eq!(ctx.take_one()["method"], "Network.loadingFinished");
    assert_eq!(ctx.take_one()["method"], "Page.loadEventFired");
    assert_eq!(ctx.take_one()["method"], "Page.frameStoppedLoading");

    ctx.process_async(json!({
        "id": 43,
        "method": "Network.getResponseBody",
        "sessionId": "SID-1",
        "params": { "requestId": LOADER_ID }
    }))
    .await;
    ctx.expect_result(
        43,
        json!({
            "body": "<!doctype html><html><body><main>POST|continued|payload</main></body></html>",
            "base64Encoded": false
        }),
        Some("SID-1"),
    );

    server.abort();
}
