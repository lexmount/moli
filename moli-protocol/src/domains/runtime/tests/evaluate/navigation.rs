// Tests grouped by behavior. Shared fixtures live in the parent module.
use super::*;

#[tokio::test]
async fn evaluate_without_page_errors() {
    let mut ctx = TestContext::new();
    ctx.process_async(json!({"id": 2, "method": "Runtime.evaluate",
                             "params": {"expression": "1 + 1"}}))
        .await;
    ctx.expect_error(2, -32000, "NoDocumentLoaded");
}

#[tokio::test]
async fn evaluate_rejects_default_context_while_main_document_navigation_is_pending() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(
        &mut ctx,
        "<!doctype html><html><head><title>previous</title></head><body>old</body></html>",
    )
    .await;
    let browser_context = ctx
        .conn
        .browser_context
        .as_mut()
        .expect("browser context should exist");
    browser_context.set_active_target_id("TID-1");
    browser_context.attach_active_session("SID-1");
    browser_context.set_target_url("data:text/html,previous".to_owned());
    browser_context
        .start_document_navigation_for_active_target("PENDING-LOADER".to_owned())
        .expect("active navigation should start");
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 3,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": "document.title",
            "returnByValue": true
        }
    }))
    .await;

    ctx.expect_error(3, -32000, "Navigation is changing the document");
}

#[tokio::test]
async fn evaluate_started_before_pending_navigation_can_complete() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(
        &mut ctx,
        "<!doctype html><html><head><title>previous</title></head><body>old</body></html>",
    )
    .await;
    let browser_context = ctx
        .conn
        .browser_context
        .as_mut()
        .expect("browser context should exist");
    browser_context.set_active_target_id("TID-1");
    browser_context.attach_active_session("SID-1");
    browser_context.set_target_url("data:text/html,previous".to_owned());
    ctx.conn.commit_declared_session_fixtures_for_test();
    ctx.sent.clear();

    let step = ctx.conn.start_command_dispatch(
        &json!({
            "id": 4,
            "method": "Runtime.evaluate",
            "sessionId": "SID-1",
            "params": {
                "expression": "21 + 21",
                "returnByValue": true
            }
        })
        .to_string(),
    );
    ctx.conn
        .browser_context
        .as_mut()
        .expect("browser context should exist")
        .start_document_navigation_for_active_target("PENDING-LOADER".to_owned())
        .expect("active navigation should start");

    let (messages, _scheduler_events) = complete_command_task_step_for_test(&mut ctx, step).await;

    assert!(
        messages.iter().any(|message| {
            message["id"] == json!(4) && message["result"]["result"]["value"] == json!(42)
        }),
        "started Runtime.evaluate should complete against its original page: {messages:?}"
    );
}

#[tokio::test]
async fn evaluate_rejects_attached_session_while_main_document_navigation_is_pending() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(
        &mut ctx,
        "<!doctype html><html><head><title>previous</title></head><body>old</body></html>",
    )
    .await;
    let browser_context = ctx
        .conn
        .browser_context
        .as_mut()
        .expect("browser context should exist");
    browser_context.set_active_target_id("TID-1");
    browser_context.attach_active_session("SID-1");
    assert!(
        browser_context.assign_attached_session_to_target("TID-1", "SID-attached".to_owned()),
        "attached session should attach to the active target"
    );
    browser_context.set_target_url("data:text/html,previous".to_owned());
    browser_context
        .start_document_navigation_for_active_target("PENDING-LOADER".to_owned())
        .expect("active navigation should start");
    ctx.conn.commit_declared_session_fixtures_for_test();
    assert!(
        ctx.conn
            .has_pending_document_navigation_for_session_owner(Some("SID-attached")),
        "attached session should inherit the active target document gate"
    );
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 4,
        "method": "Runtime.evaluate",
        "sessionId": "SID-attached",
        "params": {
            "expression": "document.title",
            "returnByValue": true
        }
    }))
    .await;

    ctx.expect_error(4, -32000, "Navigation is changing the document");
}

#[tokio::test]
async fn dom_get_document_rejects_while_main_document_navigation_is_pending() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(
        &mut ctx,
        "<!doctype html><html><head><title>previous</title></head><body>old</body></html>",
    )
    .await;
    let browser_context = ctx
        .conn
        .browser_context
        .as_mut()
        .expect("browser context should exist");
    browser_context.set_active_target_id("TID-1");
    browser_context.attach_active_session("SID-1");
    browser_context.set_target_url("data:text/html,previous".to_owned());
    browser_context
        .start_document_navigation_for_active_target("PENDING-LOADER".to_owned())
        .expect("active navigation should start");
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 5,
        "method": "DOM.getDocument",
        "sessionId": "SID-1"
    }))
    .await;

    ctx.expect_error(5, -32000, "Navigation is changing the document");
}

#[tokio::test]
async fn document_navigation_gate_is_scoped_to_background_target_owner() {
    let mut ctx = TestContext::new();
    let background_target = crate::conn::PageTargetHost::with_url(
        "TID-background".to_owned(),
        Some("SID-background".to_owned()),
        "about:blank".to_owned(),
    );

    let mut browser_context = BrowserContext::new("BID-1".to_owned());
    browser_context.set_active_target_id("TID-active");
    browser_context.attach_active_session("SID-active");
    browser_context.active_page_target_mut().devtools_sessions
        [moli_page_types::DevToolsSessionKey::Primary]
        .runtime_session_state
        .runtime_frontend_enabled = true;
    browser_context.insert_page_target_host(background_target);
    ctx.conn
        .install_browser_context_fixture_for_test(browser_context);
    ctx.install_navigation_fixture_for_session_owner(
        "data:text/html,<html><title>active</title></html>",
        Some("SID-active"),
    )
    .await;
    ctx.install_navigation_fixture_for_session_owner(
        "data:text/html,<html><title>background</title></html>",
        Some("SID-background"),
    )
    .await;
    let browser_context = ctx.conn.browser_context.as_mut().expect("browser context");
    browser_context
        .background_target_mut("TID-background")
        .expect("background target must exist")
        .devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
        .runtime_session_state
        .runtime_frontend_enabled = true;
    browser_context
        .start_document_navigation_for_target(
            "TID-background",
            "PENDING-BACKGROUND-LOADER".to_owned(),
        )
        .expect("background document navigation should start");
    assert!(
        ctx.conn
            .has_pending_document_navigation_for_session_owner(Some("SID-background")),
        "background session should see its own pending document navigation"
    );
    assert!(
        !ctx.conn
            .has_pending_document_navigation_for_session_owner(Some("SID-active")),
        "active session should not inherit a background target document gate"
    );
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 6,
        "method": "Runtime.evaluate",
        "sessionId": "SID-background",
        "params": {
            "expression": "document.title",
            "returnByValue": true
        }
    }))
    .await;
    ctx.expect_error(6, -32000, "Navigation is changing the document");

    ctx.process_async(json!({
        "id": 7,
        "method": "Runtime.evaluate",
        "sessionId": "SID-active",
        "params": {
            "expression": "document.title",
            "returnByValue": true
        }
    }))
    .await;
    let response = take_response_by_id(&mut ctx, 7);
    assert_eq!(response["result"]["result"]["value"], json!("active"));
}

#[tokio::test(flavor = "multi_thread")]
async fn page_navigate_commits_http_error_response_document() {
    async fn first() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html; charset=utf-8")],
            "<!doctype html><html><head><title>first</title></head><body>first page</body></html>",
        )
    }

    async fn server_error() -> impl IntoResponse {
        (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            [(CONTENT_TYPE.as_str(), "text/html; charset=utf-8")],
            "<!doctype html><html><head><title>server-error</title></head><body><main>error document</main></body></html>",
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/first", get(first))
                .route("/error", get(server_error)),
        )
        .await
        .unwrap();
    });

    let first_url = format!("http://{addr}/first");
    let error_url = format!("http://{addr}/error");
    let mut ctx = TestContext::new();
    with_loaded_http_document_async(&mut ctx, &first_url, "SID-1", "TID-1").await;
    ctx.conn
        .browser_context
        .as_mut()
        .expect("browser context should exist")
        .set_target_url(first_url);
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 4,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": error_url }
    }))
    .await;
    let navigate = take_response_by_id(&mut ctx, 4);
    assert_eq!(navigate["result"]["frameId"], json!("TID-1"));
    assert!(
        navigate["result"].get("errorText").is_none(),
        "HTTP error responses should commit as documents, not Page.navigate failures: {navigate:?}"
    );
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 5,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": "`${document.title}:${document.querySelector('main')?.textContent}`",
            "returnByValue": true
        }
    }))
    .await;
    let evaluate = take_response_by_id(&mut ctx, 5);
    assert_eq!(
        evaluate["result"]["result"]["value"],
        json!("server-error:error document")
    );
    assert_eq!(
        ctx.conn
            .browser_context
            .as_ref()
            .expect("browser context should exist")
            .target_url(),
        error_url
    );

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn page_navigate_empty_http_error_commits_browser_error_document() {
    async fn first() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html; charset=utf-8")],
            "<!doctype html><html><head><title>first</title></head><body>first page</body></html>",
        )
    }

    async fn empty_rate_limit() -> impl IntoResponse {
        (
            axum::http::StatusCode::TOO_MANY_REQUESTS,
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
            Router::new()
                .route("/first", get(first))
                .route("/empty-429", get(empty_rate_limit)),
        )
        .await
        .unwrap();
    });

    let first_url = format!("http://{addr}/first");
    let error_url = format!("http://{addr}/empty-429");
    let mut ctx = TestContext::new();
    with_loaded_http_document_async(&mut ctx, &first_url, "SID-1", "TID-1").await;
    ctx.conn
        .browser_context
        .as_mut()
        .expect("browser context should exist")
        .set_target_url(first_url);
    ctx.enable_background_navigation_scheduler_for_test();
    ctx.enable_page_events_for_test(Some("SID-1"));
    ctx.process_async(json!({
        "id": 40,
        "method": "Network.enable",
        "sessionId": "SID-1"
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 40);
    ctx.process_async(json!({
        "id": 41,
        "method": "Runtime.enable",
        "sessionId": "SID-1"
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 41);
    ctx.sent.clear();

    tokio::task::LocalSet::new()
        .run_until(assert_empty_http_error_navigation(&mut ctx, &error_url))
        .await;

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn page_navigate_network_failure_commits_error_document() {
    async fn first() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html; charset=utf-8")],
            "<!doctype html><html><head><title>first</title></head><body>first page</body></html>",
        )
    }

    let (failing_addr, failing_server) = spawn_connection_drop_server().await;

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, Router::new().route("/first", get(first)))
            .await
            .unwrap();
    });

    let first_url = format!("http://{addr}/first");
    let failing_url = format!("http://{failing_addr}/failed");
    let mut ctx = TestContext::new();
    with_loaded_http_document_async(&mut ctx, &first_url, "SID-1", "TID-1").await;
    ctx.conn
        .browser_context
        .as_mut()
        .expect("browser context should exist")
        .set_target_url(first_url);
    ctx.enable_page_events_for_test(Some("SID-1"));
    ctx.process_async(json!({
        "id": 5,
        "method": "Runtime.enable",
        "sessionId": "SID-1"
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 5);
    ctx.process_async(json!({
        "id": 5_1,
        "method": "Page.setLifecycleEventsEnabled",
        "sessionId": "SID-1",
        "params": { "enabled": true }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 5_1);
    ctx.process_async(json!({
        "id": 5_2,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": "globalThis.__beforeNetworkError = 'old realm'",
            "returnByValue": true
        }
    }))
    .await;
    assert_eq!(
        take_response_by_id(&mut ctx, 5_2)["result"]["result"]["value"],
        json!("old realm")
    );
    let old_document_token = ctx
        .conn
        .browser_context
        .as_mut()
        .expect("browser context")
        .start_document_navigation_for_active_target("LOADER-before-network-error".to_owned())
        .expect("loaded document token");
    ctx.conn
        .browser_context
        .as_mut()
        .expect("browser context")
        .commit_document_navigation_if_matches(&old_document_token);
    let before_target_page = ctx
        .conn
        .target_page_residence_identity_for_session(Some("SID-1"))
        .expect("loaded target Page residence");
    let before_renderer_page = ctx
        .conn
        .renderer_page_residence_identity_for_session_owner(Some("SID-1"))
        .expect("loaded renderer Page residence");
    let before_renderer_attachment = ctx
        .conn
        .current_renderer_agent_attachment_id_for_owner(
            &crate::conn::CommandOwnerScope::for_session("SID-1"),
        )
        .expect("loaded renderer attachment");
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 6,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": failing_url }
    }))
    .await;

    let navigate = take_response_by_id(&mut ctx, 6);
    assert_eq!(navigate["result"]["frameId"], json!("TID-1"));
    assert!(navigate["result"]["loaderId"].is_string());
    assert_eq!(navigate["result"]["isDownload"], json!(false));
    assert!(
        navigate["result"]["errorText"]
            .as_str()
            .is_some_and(|message| !message.is_empty()),
        "failed navigation should return the browser network error: {navigate:?}"
    );
    // The outgoing Document can still finish loading before the error Document
    // commits. This fixture can also reuse its loader id, so match the load
    // after the concrete error-page commit instead of accepting either page.
    wait_until_messages(
        &mut ctx,
        Some("SID-1"),
        "network error Document load after its commit",
        |messages| {
            let Some(commit_index) = messages.iter().position(|message| {
                message["method"] == json!("Page.frameNavigated")
                    && message["sessionId"] == json!("SID-1")
                    && message["params"]["frame"]["url"] == NETWORK_ERROR_PAGE_URL
                    && message["params"]["frame"]["loaderId"] == navigate["result"]["loaderId"]
            }) else {
                return false;
            };
            messages[commit_index + 1..].iter().any(|message| {
                message["method"] == json!("Page.lifecycleEvent")
                    && message["sessionId"] == json!("SID-1")
                    && message["params"]["loaderId"] == navigate["result"]["loaderId"]
                    && message["params"]["name"] == json!("load")
            })
        },
    )
    .await;
    assert!(
        ctx.sent.iter().any(|message| {
            message["method"] == json!("Network.loadingFailed")
                && message["params"]["type"] == json!("Document")
        }),
        "failed navigation should emit a document loadingFailed event: {:?}",
        ctx.sent
    );
    let commit_index = ctx
        .sent
        .iter()
        .position(|message| {
            message["method"] == json!("Page.frameNavigated")
                && message["sessionId"] == json!("SID-1")
                && message["params"]["frame"]["url"] == NETWORK_ERROR_PAGE_URL
                && message["params"]["frame"]["loaderId"] == navigate["result"]["loaderId"]
        })
        .unwrap_or_else(|| panic!("missing error Document frame commit: {:?}", ctx.sent));
    let frame_navigated = &ctx.sent[commit_index];
    assert_eq!(
        frame_navigated["params"]["frame"]["url"],
        NETWORK_ERROR_PAGE_URL
    );
    assert_eq!(
        frame_navigated["params"]["frame"]["unreachableUrl"],
        failing_url
    );
    assert_eq!(frame_navigated["params"]["frame"]["securityOrigin"], "://");
    assert_eq!(
        frame_navigated["params"]["frame"]["secureContextType"],
        "InsecureScheme"
    );
    assert!(
        ctx.sent
            .iter()
            .any(|message| { message["method"] == json!("Runtime.executionContextsCleared") })
    );
    assert!(ctx.sent.iter().any(|message| {
        message["method"] == json!("Runtime.executionContextCreated")
            && message["params"]["context"]["auxData"]["isDefault"] == json!(true)
    }));
    let error_document_events = &ctx.sent[commit_index + 1..];
    assert!(error_document_events.iter().any(|message| {
        message["method"] == json!("Page.loadEventFired") && message["sessionId"] == json!("SID-1")
    }));
    let lifecycle_names: Vec<_> = error_document_events
        .iter()
        .filter(|message| {
            message["method"] == json!("Page.lifecycleEvent")
                && message["sessionId"] == json!("SID-1")
                && message["params"]["loaderId"] == navigate["result"]["loaderId"]
        })
        .filter_map(|message| message["params"]["name"].as_str())
        .filter(|name| matches!(*name, "DOMContentLoaded" | "load"))
        .collect();
    assert_eq!(
        lifecycle_names,
        ["DOMContentLoaded", "load"],
        "{:#?}",
        ctx.sent
    );

    ctx.sent.clear();
    ctx.process_async(json!({
        "id": 7,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": "JSON.stringify([location.href, location.origin, typeof globalThis.__beforeNetworkError, document.title, document.readyState])",
            "returnByValue": true
        }
    }))
    .await;

    let evaluate = take_response_by_id(&mut ctx, 7);
    assert_eq!(
        evaluate["result"]["result"]["value"],
        json!(format!(
            "[\"{NETWORK_ERROR_PAGE_URL}\",\"null\",\"undefined\",\"127.0.0.1\",\"complete\"]"
        ))
    );
    assert_eq!(
        ctx.conn
            .browser_context
            .as_ref()
            .expect("browser context should exist")
            .target_url(),
        failing_url
    );
    let page = ctx
        .conn
        .browser_context
        .as_ref()
        .and_then(BrowserContext::loaded_page)
        .expect("network error Document should remain loaded");
    assert_eq!(page.final_url().as_str(), NETWORK_ERROR_PAGE_URL);
    assert!(
        !ctx.conn
            .browser_context
            .as_ref()
            .expect("browser context")
            .accepts_document_body_completion_event(&old_document_token),
        "the retired document generation must not accept late completion"
    );
    let error_document_loader_id = ctx
        .conn
        .target_session_owner_frame_tree_loader_id_for_owner(
            &crate::conn::CommandOwnerScope::for_session("SID-1"),
        )
        .expect("error Document loader id");
    let stale_request_id = "REQ-before-network-error";
    let stale_body_completion = BackgroundNavigationCompletion::main_document_body(
        old_document_token.clone(),
        crate::conn::NavigationDispatchState {
            auxiliary_document_response: None,
            redirect_chain: Vec::new(),
            redirect_headers: None,
            navigate_id: None,
            owner: crate::conn::CommandOwnerScope::for_session("SID-1"),
            result_projection: crate::conn::NavigationResultProjection::Cdp(json!({})),
            frame_id: "TID-1".to_owned(),
            session_id: Some("SID-1".to_owned()),
            request_id: Some(stale_request_id.to_owned()),
            loader_id: old_document_token.loader_id.clone(),
            request_announced: true,
            requested_url: url::Url::parse("https://stale.example.test/old-body").unwrap(),
            request_method: "GET".to_owned(),
            request_body: None,
            request_body_bytes: None,
            request_headers: Vec::new().into(),
            request_load_policy: crate::conn::NavigationRequestLoadPolicy::DocumentInitiated,
            timestamp: 0.0,
            source_document_security: crate::conn::NavigationSourceDocumentSecurityContext::new(
                "http://127.0.0.1".to_owned(),
                "InsecureScheme".to_owned(),
            ),
        },
        Ok(crate::conn::CapturedBody::from_string(
            "stale body".to_owned(),
        )),
        false,
        crate::domains::network::MainDocumentBodyProgressSource::default(),
        url::Url::parse("https://stale.example.test/old-body").unwrap(),
        vec![("content-type".to_owned(), b"text/plain".to_vec())],
        false,
    );
    let (stale_completion_messages, stale_completion_scheduler_events) = ctx
        .conn
        .drain_background_navigation_completion_turn_async(stale_body_completion)
        .await
        .into_parts();
    assert!(stale_completion_messages.is_empty());
    assert!(stale_completion_scheduler_events.is_empty());
    assert_eq!(
        ctx.conn
            .target_session_owner_frame_tree_loader_id_for_owner(
                &crate::conn::CommandOwnerScope::for_session("SID-1")
            )
            .as_deref(),
        Some(error_document_loader_id.as_str()),
        "stale body completion must not replace the error Document loader"
    );
    assert_eq!(
        ctx.conn
            .browser_context
            .as_ref()
            .and_then(BrowserContext::loaded_page)
            .expect("error Document should survive stale body completion")
            .final_url()
            .as_str(),
        NETWORK_ERROR_PAGE_URL
    );
    ctx.process_async(json!({
        "id": 91,
        "method": "Network.getResponseBody",
        "sessionId": "SID-1",
        "params": { "requestId": stale_request_id }
    }))
    .await;
    ctx.expect_error(91, -32000, "No resource with given identifier found");
    assert_eq!(
        ctx.conn
            .target_page_residence_identity_for_session(Some("SID-1")),
        Some(before_target_page),
        "the error Document must retain the target Page residence"
    );
    assert_eq!(
        ctx.conn
            .renderer_page_residence_identity_for_session_owner(Some("SID-1")),
        Some(before_renderer_page),
        "the error Document replaces the Document within the existing renderer Page"
    );
    assert_ne!(
        ctx.conn.current_renderer_agent_attachment_id_for_owner(
            &crate::conn::CommandOwnerScope::for_session("SID-1"),
        ),
        Some(before_renderer_attachment),
        "the error Document must own a replacement realm/Inspector attachment"
    );

    ctx.process_async(json!({
        "id": 8,
        "method": "Page.getFrameTree",
        "sessionId": "SID-1"
    }))
    .await;
    let frame_tree = take_response_by_id(&mut ctx, 8);
    assert_eq!(
        frame_tree["result"]["frameTree"]["frame"]["url"],
        NETWORK_ERROR_PAGE_URL
    );
    assert_eq!(
        frame_tree["result"]["frameTree"]["frame"]["unreachableUrl"],
        failing_url
    );

    ctx.process_async(json!({
        "id": 9,
        "method": "Page.getNavigationHistory",
        "sessionId": "SID-1"
    }))
    .await;
    let history = take_response_by_id(&mut ctx, 9);
    let current_index = history["result"]["currentIndex"]
        .as_u64()
        .expect("current history index") as usize;
    assert_eq!(
        history["result"]["entries"][current_index]["url"], failing_url,
        "history must expose the failed request URL, not the internal error URL"
    );

    ctx.sent.clear();
    ctx.process_async(json!({
        "id": 10,
        "method": "Fetch.enable",
        "sessionId": "SID-1",
        "params": {
            "patterns": [{ "urlPattern": "*", "resourceType": "Document" }]
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 10);

    let secure_url = "https://secure.example.test/after-error";
    ctx.process_async(json!({
        "id": 11,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": secure_url }
    }))
    .await;
    let paused = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Fetch.requestPaused")
                && message["params"]["resourceType"] == json!("Document")
                && message["params"]["request"]["url"] == json!(secure_url)
        })
        .cloned()
        .unwrap_or_else(|| panic!("missing HTTPS document request pause: {:?}", ctx.sent));
    let paused_request_id = paused["params"]["requestId"]
        .as_str()
        .expect("paused HTTPS request id")
        .to_owned();
    ctx.sent.clear();
    ctx.process_async(json!({
        "id": 12,
        "method": "Fetch.fulfillRequest",
        "sessionId": "SID-1",
        "params": {
            "requestId": paused_request_id,
            "responseCode": 200,
            "responseHeaders": [{ "name": "content-type", "value": "text/html" }],
            "body": "PCFkb2N0eXBlIGh0bWw+PHRpdGxlPnNlY3VyZS1hZnRlci1lcnJvcjwvdGl0bGU+"
        }
    }))
    .await;
    wait_until_message(
        &mut ctx,
        Some("SID-1"),
        "successful HTTPS navigation after error Document",
        |message| {
            message["method"] == json!("Page.loadEventFired")
                && message["sessionId"] == json!("SID-1")
        },
    )
    .await;

    ctx.process_async(json!({
        "id": 13,
        "method": "Page.getFrameTree",
        "sessionId": "SID-1"
    }))
    .await;
    let secure_frame_tree = take_response_by_id(&mut ctx, 13);
    let secure_frame = &secure_frame_tree["result"]["frameTree"]["frame"];
    assert_eq!(secure_frame["url"], secure_url);
    assert_eq!(
        secure_frame["securityOrigin"],
        "https://secure.example.test"
    );
    assert_eq!(secure_frame["secureContextType"], "Secure");
    assert!(secure_frame.get("unreachableUrl").is_none());

    ctx.process_async(json!({
        "id": 14,
        "method": "Fetch.disable",
        "sessionId": "SID-1"
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 14);
    ctx.sent.clear();
    ctx.process_async(json!({
        "id": 15,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": "about:blank" }
    }))
    .await;
    wait_until_message(
        &mut ctx,
        Some("SID-1"),
        "about:blank navigation inheriting the secure source Document",
        |message| {
            message["method"] == json!("Page.loadEventFired")
                && message["sessionId"] == json!("SID-1")
        },
    )
    .await;
    ctx.process_async(json!({
        "id": 16,
        "method": "Page.getFrameTree",
        "sessionId": "SID-1"
    }))
    .await;
    let inherited_frame_tree = take_response_by_id(&mut ctx, 16);
    let inherited_frame = &inherited_frame_tree["result"]["frameTree"]["frame"];
    assert_eq!(inherited_frame["url"], "about:blank");
    assert_eq!(
        inherited_frame["securityOrigin"],
        "https://secure.example.test"
    );
    assert_eq!(inherited_frame["secureContextType"], "Secure");

    failing_server.abort();
    server.abort();
}
