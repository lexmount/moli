use super::*;
use base64::{Engine as _, engine::general_purpose::STANDARD};

#[tokio::test(flavor = "multi_thread")]
async fn browser_native_commands_do_not_require_a_live_inspector_session() {
    let mut ctx = dom_context().await;
    ctx.process_async(json!({"id": 1, "method": "Runtime.evaluate", "params": {
        "expression": "document.body.innerHTML = '<input id=field oninput=\"document.body.dataset.value=this.value\">'; document.getElementById('field').focus()",
    }})).await;
    assert!(ctx.take_response_by_id(1).get("error").is_none());
    let pending_capture = ctx
        .conn
        .loaded_page_mut_for_protocol_access(None)
        .unwrap()
        .start_capture_screenshot()
        .unwrap();
    // Seal the actual primary Inspector receiver without retiring the Browser
    // document. Browser operations must not silently join this closed session.
    ctx.conn
        .runtime_session_owner_slot(None)
        .unwrap()
        .current_renderer_inspection_binding()
        .unwrap()
        .detach_session(None)
        .unwrap();

    let completion = pending_capture.wait().await.unwrap();
    let page = ctx.conn.loaded_page_mut_for_protocol_access(None).unwrap();
    assert!(completion.is_from_page(page));
    assert!(
        page.finish_capture_screenshot(completion).is_ok(),
        "session detach must not cancel already admitted Browser capture"
    );

    ctx.process_async(json!({"id": 2, "method": "Input.insertText", "params": {"text": "native"}}))
        .await;
    ctx.expect_result(2, json!({}), None);
    let page = ctx.conn.loaded_page_mut_for_protocol_access(None).unwrap();
    assert!(
        page.serialize_html_async()
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
