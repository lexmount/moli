use super::*;
use base64::{Engine as _, engine::general_purpose::STANDARD};

async fn two_native_documents() -> TestContext {
    let mut ctx = dom_context().await;
    let context = ctx.conn.browser_context.as_mut().unwrap();
    context.attach_active_session("SID-native-original");
    assert!(context.register_page_target_url_fixture(
        "TID-native-background".into(),
        Some("SID-native-background".into()),
        "about:blank".into(),
    ));
    ctx.install_navigation_fixture_for_session_owner(
        "data:text/html,<title>background</title><body>background</body>",
        Some("SID-native-background"),
    )
    .await;
    ctx
}

#[tokio::test(flavor = "multi_thread")]
async fn native_history_reset_completion_survives_selection_and_session_detach() {
    let mut ctx = two_native_documents().await;
    ctx.install_buffered_navigation_fixture_for_session_owner(
        url::Url::parse("https://history.example/").unwrap(),
        "<title>history</title>".into(),
        Some("SID-native-original"),
    )
    .await;
    let initial = ctx
        .conn
        .browser_context
        .as_ref()
        .unwrap()
        .target_navigation_history_snapshot("TID-dom-inspection")
        .unwrap();
    ctx.process_async(json!({"id": 701, "sessionId": "SID-native-original", "method": "Runtime.evaluate", "params": {
        "expression": "history.pushState(null, '', '#one'); history.pushState(null, '', '#two')",
    }})).await;
    let response = ctx.take_response_by_id(701);
    assert!(
        response["result"]["exceptionDetails"].is_null(),
        "{response}"
    );
    let before = ctx
        .conn
        .browser_context
        .as_mut()
        .unwrap()
        .target_navigation_history_snapshot("TID-dom-inspection")
        .unwrap();
    assert_eq!(before.1.len(), initial.1.len() + 2);
    let raw = json!({"id": 702, "sessionId": "SID-native-original", "method": "Page.resetNavigationHistory"}).to_string();
    let CdpCommandTaskStep::Pending(pending) = ctx.conn.start_command_dispatch(&raw) else {
        panic!("history reset must execute on the Browser Document");
    };
    let completed = pending.wait().await;
    assert!({
        let handle = ctx
            .conn
            .browser_web_contents_for_target("TID-native-background")
            .unwrap();
        ctx.conn
            .select_browser_web_contents_async(handle)
            .await
            .is_ok()
    });
    let peer = ctx
        .conn
        .browser_context
        .as_mut()
        .unwrap()
        .target_navigation_history_snapshot("TID-native-background")
        .unwrap();
    ctx.process_async(
        json!({"id": 703, "method": "Target.detachFromTarget", "params": {
            "targetId": "TID-dom-inspection", "sessionId": "SID-native-original",
        }}),
    )
    .await;
    assert!(ctx.take_response_by_id(703).get("error").is_none());
    assert!(
        ctx.conn
            .session_route(Some("SID-native-original"))
            .is_none()
    );
    let CdpCommandTaskStep::Complete(outcome) =
        ctx.conn.complete_pending_command_dispatch(completed).await
    else {
        panic!("history reset must complete without its frontend session");
    };
    let (messages, _) = ctx.route_completed_command_outcome_for_test(outcome).await;
    assert!(
        messages
            .iter()
            .any(|message| message["id"] == json!(702) && message["result"] == json!({})),
        "{messages:?}"
    );
    let context = ctx.conn.browser_context.as_mut().unwrap();
    assert_eq!(context.active_target_id(), Some("TID-native-background"));
    assert_eq!(
        context.target_navigation_history_snapshot("TID-native-background"),
        Some(peer)
    );
    assert_eq!(
        context.target_navigation_history_snapshot("TID-dom-inspection"),
        Some((0, vec![before.1[before.0].clone()]))
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn native_input_completion_keeps_its_document_after_selection_and_session_detach() {
    let mut ctx = two_native_documents().await;
    ctx.process_async(json!({"id": 21, "sessionId": "SID-native-original", "method": "Runtime.evaluate", "params": {
        "expression": "document.body.innerHTML = '<input id=field>'; const field = document.getElementById('field'); field.oninput = () => document.title = field.value; field.focus()",
    }})).await;
    assert!(ctx.take_response_by_id(21).get("error").is_none());
    let raw = json!({"id": 22, "sessionId": "SID-native-original", "method": "Input.insertText", "params": {"text": "original-input"}}).to_string();
    let CdpCommandTaskStep::Pending(pending) = ctx.conn.start_command_dispatch(&raw) else {
        panic!("input must be admitted to the original Browser document");
    };
    let completed = pending.wait().await;
    let handle = ctx
        .conn
        .browser_web_contents_for_target("TID-native-background")
        .unwrap();
    ctx.conn
        .select_browser_web_contents_async(handle)
        .await
        .unwrap();
    ctx.process_async(
        json!({"id": 23, "method": "Target.detachFromTarget", "params": {
            "targetId": "TID-dom-inspection", "sessionId": "SID-native-original",
        }}),
    )
    .await;
    assert!(ctx.take_response_by_id(23).get("error").is_none());
    assert!(
        ctx.conn
            .session_route(Some("SID-native-original"))
            .is_none()
    );
    let CdpCommandTaskStep::Complete(outcome) =
        ctx.conn.complete_pending_command_dispatch(completed).await
    else {
        panic!("native input completion must not depend on the detached session");
    };
    let (messages, _) = ctx.route_completed_command_outcome_for_test(outcome).await;
    assert!(
        messages
            .iter()
            .any(|message| message["id"] == json!(22) && message["result"] == json!({}))
    );
    let context = ctx.conn.browser_context.as_ref().unwrap();
    assert_eq!(context.active_target_id(), Some("TID-native-background"));
    assert_eq!(
        context
            .target_document_title("TID-native-background")
            .as_deref(),
        Some("background")
    );
    assert_eq!(
        context
            .target_document_title("TID-dom-inspection")
            .as_deref(),
        Some("original-input")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn native_diagnostics_completion_keeps_exact_documents_after_selection_and_detach() {
    let mut ctx = two_native_documents().await;
    let completed = ctx
        .conn
        .start_moli_diagnostics()
        .unwrap()
        .wait()
        .await
        .unwrap();
    let handle = ctx
        .conn
        .browser_web_contents_for_target("TID-native-background")
        .unwrap();
    ctx.conn
        .select_browser_web_contents_async(handle)
        .await
        .unwrap();
    ctx.process_async(
        json!({"id": 31, "method": "Target.detachFromTarget", "params": {
            "targetId": "TID-dom-inspection", "sessionId": "SID-native-original",
        }}),
    )
    .await;
    assert!(ctx.take_response_by_id(31).get("error").is_none());
    let diagnostics = ctx.conn.complete_moli_diagnostics(completed);
    assert_eq!(
        diagnostics["isolateScope"]["dedicatedWorkerDiagnosticsFailedPageSnapshotCount"],
        json!(0),
        "selection/detach cannot redirect or invalidate Browser diagnostics: {diagnostics}"
    );
    assert_eq!(
        diagnostics["isolateScope"]["documentContextCount"],
        json!(2)
    );
    assert_eq!(
        ctx.conn
            .browser_context
            .as_ref()
            .unwrap()
            .active_target_id(),
        Some("TID-native-background")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn native_diagnostics_rejects_only_the_replaced_document_snapshot() {
    let mut ctx = two_native_documents().await;
    let completed = ctx
        .conn
        .start_moli_diagnostics()
        .unwrap()
        .wait()
        .await
        .unwrap();
    ctx.install_navigation_fixture_for_session_owner(
        "data:text/html,<title>replacement</title>",
        Some("SID-native-original"),
    )
    .await;
    let diagnostics = ctx.conn.complete_moli_diagnostics(completed);
    assert_eq!(
        diagnostics["isolateScope"]["dedicatedWorkerDiagnosticsFailedPageSnapshotCount"],
        json!(1),
        "only the replaced Document snapshot must be rejected: {diagnostics}"
    );
    assert_eq!(
        diagnostics["isolateScope"]["documentContextCount"],
        json!(1)
    );
    assert_eq!(
        ctx.conn
            .browser_context
            .as_ref()
            .unwrap()
            .target_document_title("TID-dom-inspection")
            .as_deref(),
        Some("replacement")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn browser_native_commands_do_not_require_a_live_inspector_session() {
    let mut ctx = dom_context().await;
    ctx.process_async(json!({"id": 1, "method": "Runtime.evaluate", "params": {
        "expression": "document.body.innerHTML = '<input id=field oninput=\"document.body.dataset.value=this.value\">'; document.getElementById('field').focus()",
    }})).await;
    assert!(ctx.take_response_by_id(1).get("error").is_none());
    let owner = CommandOwnerScope::capture(&ctx.conn, None);
    let (context_id, target_id) = ctx.conn.resolve_document_command_owner(&owner).unwrap();
    let context = ctx.conn.browser_context_by_id(&context_id).unwrap();
    let residence = context
        .target_renderer_page_residence_identity(&target_id)
        .unwrap();
    let document = ctx.conn.resolve_browser_document_for_owner(&owner).unwrap();
    let pending_capture = ctx
        .conn
        .start_capture_document_image(
            document,
            moli_core::page::RendererCaptureScreenshotRequest::viewport_png(),
        )
        .unwrap();
    // Seal the actual primary Inspector receiver without retiring the Browser
    // document. Browser operations must not silently join this closed session.
    ctx.conn
        .runtime_session_owner_slot(None)
        .unwrap()
        .current_renderer_inspection_binding()
        .unwrap()
        .detach_session(None, None)
        .await
        .unwrap();

    let completion = pending_capture.wait().await;
    assert!(
        ctx.conn.finish_capture_document_image(completion).is_ok(),
        "session detach must not cancel already admitted Browser capture"
    );
    assert_eq!(
        ctx.conn
            .browser_context_by_id(&context_id)
            .unwrap()
            .target_renderer_page_residence_identity(&target_id),
        Some(residence)
    );

    ctx.process_async(json!({"id": 2, "method": "Input.insertText", "params": {"text": "native"}}))
        .await;
    ctx.expect_result(2, json!({}), None);
    assert!(
        ctx.conn
            .browser_context_by_id_mut(&context_id)
            .unwrap()
            .serialize_target_html_for_test(&target_id)
            .await
            .unwrap()
            .contains("data-value=\"native\"")
    );

    for (id, method, signature) in [
        (3, "Page.captureScreenshot", b"\x89PNG\r\n\x1a\n".as_slice()),
        (4, "Page.printToPDF", b"%PDF-".as_slice()),
    ] {
        ctx.process_async(json!({"id": id, "method": method})).await;
        let response = ctx.take_response_by_id(id);
        assert!(response.get("error").is_none(), "{method}: {response}");
        let bytes = STANDARD
            .decode(response["result"]["data"].as_str().unwrap())
            .unwrap();
        assert!(
            bytes.starts_with(signature),
            "{method} must produce the actual capture"
        );
    }
    ctx.process_async(
        json!({"id": 5, "method": "Network.setExtraHTTPHeaders", "params": {
            "headers": {"X-Native-Owner": "retained"},
        }}),
    )
    .await;
    ctx.expect_result(5, json!({}), None);
}

#[tokio::test(flavor = "multi_thread")]
async fn native_capture_rejects_a_foreign_page_without_devtools_attachments() {
    use moli_core::runtime::{Browser, BrowserConfig};

    let browser = Browser::new(BrowserConfig::default()).unwrap();
    let mut first = browser
        .fetch("data:text/html,<title>first</title>")
        .await
        .unwrap();
    let mut second = browser
        .fetch("data:text/html,<title>second</title>")
        .await
        .unwrap();
    let completion = first
        .start_capture_screenshot()
        .unwrap()
        .wait()
        .await
        .unwrap();
    assert!(completion.is_from_page(&first));
    assert!(!completion.is_from_page(&second));
    assert!(second.finish_capture_screenshot(completion).is_err());
    assert_eq!(second.document_title(), "second");
    let completion = first
        .start_capture_screenshot()
        .unwrap()
        .wait()
        .await
        .unwrap();
    assert!(first.finish_capture_screenshot(completion).is_ok());
}

#[tokio::test(flavor = "multi_thread")]
async fn completed_browser_document_command_cannot_retarget_a_replacement_document() {
    let mut ctx = dom_context().await;
    let owner = CommandOwnerScope::capture(&ctx.conn, None);
    let document = ctx.conn.resolve_browser_document_for_owner(&owner).unwrap();
    let completed = ctx
        .conn
        .start_capture_document_snapshot(document)
        .unwrap()
        .wait()
        .await;

    ctx.install_navigation_fixture_for_session_owner(
        "data:text/html,<title>replacement-document</title>",
        None,
    )
    .await;

    assert!(matches!(
        ctx.conn.finish_capture_document_snapshot(completed),
        Err(error) if error == "Document changed"
    ));
    assert_eq!(
        ctx.conn
            .browser_context
            .as_ref()
            .unwrap()
            .target_document_title("TID-dom-inspection")
            .as_deref(),
        Some("replacement-document")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn app_manifest_fetch_cannot_publish_into_a_replacement_document() {
    let mut ctx = dom_context().await;
    ctx.install_buffered_navigation_fixture_for_session_owner(
        url::Url::parse("https://manifest.example/page").unwrap(),
        r#"<!doctype html><link rel="manifest" href="data:application/manifest+json,%7B%7D"><title>manifest-owner</title>"#
            .into(),
        None,
    )
    .await;

    let raw = json!({"id": 801, "method": "Page.getAppManifest"}).to_string();
    let CdpCommandTaskStep::Pending(prepare) = ctx.conn.start_command_dispatch(&raw) else {
        panic!("app manifest inspection must start on the original document");
    };
    let CdpCommandTaskStep::Pending(fetch) = ctx
        .conn
        .complete_pending_command_dispatch(prepare.wait().await)
        .await
    else {
        panic!("an external app manifest must enter the browser fetch stage");
    };
    let fetched = fetch.wait().await;

    ctx.install_navigation_fixture_for_session_owner(
        "data:text/html,<title>replacement-manifest-owner</title>",
        None,
    )
    .await;

    let CdpCommandTaskStep::Complete(outcome) =
        ctx.conn.complete_pending_command_dispatch(fetched).await
    else {
        panic!("a stale app manifest fetch must not publish into the replacement document");
    };
    let (messages, _) = ctx.route_completed_command_outcome_for_test(outcome).await;
    let response = messages
        .iter()
        .find(|message| message["id"] == json!(801))
        .expect("app manifest response");
    assert_eq!(response["error"]["code"], json!(-32000), "{response}");
    assert_eq!(
        response["error"]["message"],
        json!("Failed to publish app manifest result: Document changed"),
        "the fetched result must retain its originating Browser Document: {response}"
    );
    assert_eq!(
        ctx.conn
            .browser_context
            .as_ref()
            .unwrap()
            .target_document_title("TID-dom-inspection")
            .as_deref(),
        Some("replacement-manifest-owner")
    );
}
