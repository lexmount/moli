use super::*;
use crate::conn::{Cmd, CommandDispatchContext};
use crate::domains::page::{
    PageCommandTaskStep, complete_pending_page_command, try_start_page_command_dispatch,
};
use serde_json::Value;

/// cdp.page: captureScreenshot – invalid image format
#[tokio::test(flavor = "multi_thread")]
async fn capture_screenshot_bad_format() {
    let mut ctx = TestContext::new();
    for (id, format) in [(10, "jpg"), (12, "pcx")] {
        ctx.process_async(json!({
            "id": id,
            "method": "Page.captureScreenshot",
            "params": {"format": format}
        }))
        .await;
        ctx.expect_error(id, -32602, "Invalid image format");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn capture_screenshot_encodes_jpeg_and_rejects_webp() {
    let mut ctx = TestContext::new();
    install_active_screenshot_page(
        &mut ctx,
        "BID-SCREENSHOT-JPEG",
        "TID-SCREENSHOT-JPEG",
        "SID-SCREENSHOT-JPEG",
        "data:text/html,<style>html%7Bbackground-color%3Argb(255,0,0)%7D</style>",
    )
    .await;
    set_screenshot_viewport(&mut ctx, "SID-SCREENSHOT-JPEG", 4, 3, 1.0, 149).await;

    ctx.process_async(json!({
        "id": 13,
        "method": "Page.captureScreenshot",
        "sessionId": "SID-SCREENSHOT-JPEG",
        "params": {"format": "jpeg", "quality": 70}
    }))
    .await;
    let jpeg = screenshot_bytes(&take_response_by_id(&mut ctx, 13));
    assert_eq!(&jpeg[..2], &[0xff, 0xd8]);
    assert_eq!(&jpeg[jpeg.len() - 2..], &[0xff, 0xd9]);
    let decoded = moli_image::decode_jpeg(&jpeg)
        .expect("captureScreenshot JPEG should decode through moli-image");
    assert_eq!((decoded.width, decoded.height), (4, 3));

    ctx.process_async(json!({
        "id": 14,
        "method": "Page.captureScreenshot",
        "sessionId": "SID-SCREENSHOT-JPEG",
        "params": {"format": "webp"}
    }))
    .await;
    ctx.expect_error(
        14,
        -32000,
        "Page.captureScreenshot option 'format' is not supported.",
    );
}

/// cdp.page: captureScreenshot – a committed document is required
#[tokio::test(flavor = "multi_thread")]
async fn capture_screenshot_without_document_reports_no_document_loaded() {
    let mut ctx = TestContext::new();
    ctx.process_async(json!({"id": 11, "method": "Page.captureScreenshot"}))
        .await;
    ctx.expect_error(11, -32000, "NoDocumentLoaded");
}

#[tokio::test(flavor = "multi_thread")]
async fn capture_screenshot_accepts_default_equivalent_options() {
    let mut ctx = TestContext::new();
    install_active_screenshot_page(
        &mut ctx,
        "BID-SCREENSHOT-DEFAULT",
        "TID-SCREENSHOT-DEFAULT",
        "SID-SCREENSHOT-DEFAULT",
        "data:text/html,<style>html%7Bbackground-color%3Argb(255,0,0)%7D</style>",
    )
    .await;
    set_screenshot_viewport(&mut ctx, "SID-SCREENSHOT-DEFAULT", 4, 3, 1.0, 150).await;
    ctx.process_async(json!({
        "id": 15,
        "method": "Page.captureScreenshot",
        "sessionId": "SID-SCREENSHOT-DEFAULT",
        "params": {
            "format": "png",
            "quality": 100,
            "fromSurface": true,
            "captureBeyondViewport": false,
            "optimizeForSpeed": false
        }
    }))
    .await;
    let response = take_response_by_id(&mut ctx, 15);
    let png = screenshot_png_bytes(&response);
    assert_png_dimensions(&png, 4, 3);
    assert_eq!(response["sessionId"], json!("SID-SCREENSHOT-DEFAULT"));
}

#[tokio::test(flavor = "multi_thread")]
async fn capture_screenshot_uses_pending_renderer_page_command_residence() {
    let mut ctx = TestContext::new();
    install_active_screenshot_page(
        &mut ctx,
        "BID-SCREENSHOT-FENCE",
        "TID-SCREENSHOT-FENCE",
        "SID-SCREENSHOT-FENCE",
        "data:text/html,<style>html%7Bbackground-color%3Ablue%7D</style>",
    )
    .await;

    let params = Value::Null;
    let cmd = Cmd::for_test(
        Some(151),
        "Page.captureScreenshot",
        &params,
        Some("SID-SCREENSHOT-FENCE"),
        r#"{"id":151,"method":"Page.captureScreenshot","sessionId":"SID-SCREENSHOT-FENCE"}"#,
    );
    let PageCommandTaskStep::Pending(pending) =
        try_start_page_command_dispatch(&mut ctx.conn, &cmd)
            .expect("Page.captureScreenshot should be handled by Page domain")
    else {
        panic!("captureScreenshot should use the pending renderer page-command lane");
    };

    let mut command_context = CommandDispatchContext::default();
    let PageCommandTaskStep::Complete(plan) =
        complete_pending_page_command(&mut ctx.conn, pending.wait().await, &mut command_context)
            .await
    else {
        panic!("captureScreenshot should complete after one renderer page command");
    };
    if let Some(predecessor) = command_context.take_renderer_output_predecessor() {
        ctx.route_direct_command_renderer_predecessor_for_test(predecessor)
            .await;
    }

    let mut out = Vec::new();
    plan.emit_into(&mut out, cmd.id, cmd.session_id);
    assert_png_dimensions(&screenshot_png_bytes(&out[0]), 1920, 1080);
}

#[tokio::test(flavor = "multi_thread")]
async fn capture_screenshot_rejects_completion_from_replaced_renderer_attachment() {
    let mut ctx = TestContext::new();
    install_active_screenshot_page(
        &mut ctx,
        "BID-SCREENSHOT-STALE",
        "TID-SCREENSHOT-STALE",
        "SID-SCREENSHOT-STALE",
        "data:text/html,<style>html%7Bbackground-color%3Ared%7D</style>",
    )
    .await;

    let params = Value::Null;
    let cmd = Cmd::for_test(
        Some(152),
        "Page.captureScreenshot",
        &params,
        Some("SID-SCREENSHOT-STALE"),
        r#"{"id":152,"method":"Page.captureScreenshot","sessionId":"SID-SCREENSHOT-STALE"}"#,
    );
    let PageCommandTaskStep::Pending(pending) =
        try_start_page_command_dispatch(&mut ctx.conn, &cmd)
            .expect("Page.captureScreenshot should be handled by Page domain")
    else {
        panic!("captureScreenshot should use the pending renderer page-command lane");
    };

    let replacement = ctx
        .conn
        .load_page_via_runtime_async(
            "data:text/html,<style>html%7Bbackground-color%3Agreen%7D</style>",
        )
        .await
        .expect("replacement page should load");
    ctx.conn
        .browser_context
        .as_mut()
        .expect("browser context")
        .active_target
        .runtime_slot
        .replace_loaded_page(Some(replacement));

    let mut command_context = CommandDispatchContext::default();
    let PageCommandTaskStep::Complete(plan) =
        complete_pending_page_command(&mut ctx.conn, pending.wait().await, &mut command_context)
            .await
    else {
        panic!("stale captureScreenshot completion should settle as an error");
    };
    let mut out = Vec::new();
    plan.emit_into(&mut out, cmd.id, cmd.session_id);
    assert_eq!(out[0]["error"]["code"], json!(-32000));
    assert_eq!(
        out[0]["error"]["message"],
        json!(
            "Failed to capture page screenshot: capture screenshot completed for a stale renderer attachment"
        )
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn capture_screenshot_rejects_window_surface_capture_explicitly() {
    let mut ctx = TestContext::new();
    ctx.process_async(json!({
        "id": 17,
        "method": "Page.captureScreenshot",
        "params": {"fromSurface": false}
    }))
    .await;
    ctx.expect_error(
        17,
        -32000,
        "Page.captureScreenshot option 'fromSurface' is not supported.",
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn capture_screenshot_rejects_invalid_quality_and_clip() {
    let mut ctx = TestContext::new();
    for (id, params, expected) in [
        (
            16,
            json!({"quality": -1}),
            "Page.captureScreenshot quality must be between 0 and 100.",
        ),
        (
            18,
            json!({"quality": 101}),
            "Page.captureScreenshot quality must be between 0 and 100.",
        ),
        (
            19,
            json!({"clip": {"x": 0, "y": 0, "width": 0, "height": 1, "scale": 1}}),
            "Page.captureScreenshot clip must have a finite origin and positive finite width, height, and scale.",
        ),
    ] {
        ctx.process_async(json!({
            "id": id,
            "method": "Page.captureScreenshot",
            "params": params
        }))
        .await;
        ctx.expect_error(id, -32602, expected);
    }
}
#[tokio::test(flavor = "multi_thread")]
async fn capture_snapshot_returns_minimal_mhtml_for_loaded_page() {
    // Chromium source:
    // third_party/blink/web_tests/inspector-protocol/page/capture-snapshot.js
    let mut ctx = TestContext::new();
    let page_url = "data:text/html,<div id='x' class='container'><p>Text</p></div>";
    load_bc_with_target(&mut ctx, "BID-MHTML", "TID-MHTML", page_url);
    let page = ctx
        .conn
        .load_page_via_runtime_async(page_url)
        .await
        .expect("page should load");
    ctx.conn
        .browser_context
        .as_mut()
        .expect("browser context")
        .active_target
        .runtime_slot
        .replace_loaded_page(Some(page));

    ctx.process_async(json!({
        "id": 1110,
        "method": "Page.captureSnapshot"
    }))
    .await;
    let response = take_response_by_id(&mut ctx, 1110);
    let data = response["result"]["data"]
        .as_str()
        .expect("mhtml data should be returned");
    assert!(data.contains("Content-Type: multipart/related;"));
    assert!(data.contains("Content-Transfer-Encoding: base64"));
    assert!(data.contains("Content-Location: data:text/html,"));
    let encoded_html = data
        .split("\r\n\r\n")
        .nth(2)
        .and_then(|part_body| part_body.split("\r\n--").next())
        .expect("mhtml html part body");
    let decoded_html = String::from_utf8(
        BASE64_STANDARD
            .decode(encoded_html)
            .expect("mhtml html part should be base64"),
    )
    .expect("mhtml html part should be utf-8");
    assert!(decoded_html.contains("<p>Text</p>"));

    ctx.process_async(json!({
        "id": 1111,
        "method": "Page.captureSnapshot",
        "params": { "format": "foo" }
    }))
    .await;
    ctx.expect_error(1111, -32000, "unsupported snapshot format.");
}
#[tokio::test(flavor = "multi_thread")]
async fn capture_snapshot_dispatch_serializes_html_in_renderer_owner() {
    let mut ctx = TestContext::new();
    let page_url =
        "data:text/html,<!doctype html><html><body><main id='live'>renderer</main></body></html>";
    load_bc_with_target(&mut ctx, "BID-MHTML-LIVE", "TID-MHTML-LIVE", page_url);
    let page = ctx
        .conn
        .load_page_via_runtime_async(page_url)
        .await
        .expect("page should load");
    ctx.conn
        .browser_context
        .as_mut()
        .expect("browser context")
        .active_target
        .runtime_slot
        .replace_loaded_page(Some(page));

    let params = serde_json::Value::Null;
    let cmd = Cmd::for_test(
        Some(1120),
        "Page.captureSnapshot",
        &params,
        None,
        r#"{"id":1120,"method":"Page.captureSnapshot"}"#,
    );
    let step = try_start_page_command_dispatch(&mut ctx.conn, &cmd)
        .expect("Page.captureSnapshot should be handled by Page domain");
    let PageCommandTaskStep::Pending(pending) = step else {
        panic!("Page.captureSnapshot should serialize HTML with a renderer page command");
    };

    let mut command_context = CommandDispatchContext::default();
    let step =
        complete_pending_page_command(&mut ctx.conn, pending.wait().await, &mut command_context)
            .await;
    let PageCommandTaskStep::Complete(plan) = step else {
        panic!("Page.captureSnapshot serialize command should complete in one pending step");
    };

    let mut out = Vec::new();
    plan.emit_into(&mut out, cmd.id, cmd.session_id);
    assert_eq!(out.len(), 1);
    assert_eq!(out[0]["id"], json!(1120));
    let data = out[0]["result"]["data"]
        .as_str()
        .expect("mhtml data should be returned");
    let encoded_html = data
        .split("\r\n\r\n")
        .nth(2)
        .and_then(|part_body| part_body.split("\r\n--").next())
        .expect("mhtml html part body");
    let decoded_html = String::from_utf8(
        BASE64_STANDARD
            .decode(encoded_html)
            .expect("mhtml html part should be base64"),
    )
    .expect("mhtml html part should be utf-8");
    assert!(decoded_html.contains("<main id=\"live\">renderer</main>"));
}
#[test]
fn mhtml_snapshot_base64_encodes_html_and_sanitizes_header_url() {
    let html = "<main>----MultipartBoundary--moli</main>";
    let mhtml =
        super::build_mhtml_snapshot("https://example.test/page\r\nInjected-Header: yes", html);
    assert!(mhtml.contains("Content-Transfer-Encoding: base64"));
    assert!(mhtml.contains(&BASE64_STANDARD.encode(html)));
    assert!(!mhtml.contains("\r\nInjected-Header: yes"));
    assert!(!mhtml.contains(html));
}
#[tokio::test(flavor = "multi_thread")]
async fn print_to_pdf_returns_base64_pdf() {
    let mut ctx = TestContext::new();
    install_active_screenshot_page(
        &mut ctx,
        "BID-PDF-BASE64",
        "TID-PDF-BASE64",
        "SID-PDF-BASE64",
        "data:text/html,<style>html,body%7Bmargin%3A0%7D</style><main>pdf</main>",
    )
    .await;
    ctx.process_async(json!({
        "id": 1112,
        "method": "Page.printToPDF",
        "sessionId": "SID-PDF-BASE64"
    }))
    .await;
    let pdf = screenshot_bytes(&take_response_by_id(&mut ctx, 1112));
    assert!(pdf.starts_with(b"%PDF-1.7"));
    assert!(pdf.ends_with(b"%%EOF\n"));
}
#[tokio::test(flavor = "multi_thread")]
async fn print_to_pdf_return_as_stream_reads_through_io_domain() {
    let mut ctx = TestContext::new();
    install_active_screenshot_page(
        &mut ctx,
        "BID-PDF-STREAM",
        "TID-PDF-STREAM",
        "SID-PDF-STREAM",
        "data:text/html,<style>html,body%7Bmargin%3A0%7D</style><main>stream</main>",
    )
    .await;
    ctx.process_async(json!({
        "id": 1113,
        "method": "Page.printToPDF",
        "sessionId": "SID-PDF-STREAM",
        "params": { "transferMode": "ReturnAsStream" }
    }))
    .await;
    let response = take_response_by_id(&mut ctx, 1113);
    assert_eq!(response["result"]["data"], json!(""));
    let handle = response["result"]["stream"]
        .as_str()
        .expect("printToPDF stream handle")
        .to_owned();

    ctx.process_async(json!({
        "id": 1114,
        "method": "IO.read",
        "sessionId": "SID-PDF-STREAM",
        "params": { "handle": handle.clone() }
    }))
    .await;
    let read = take_response_by_id(&mut ctx, 1114);
    assert_eq!(read["result"]["base64Encoded"], json!(true));
    assert_eq!(read["result"]["eof"], json!(true));
    let pdf = BASE64_STANDARD
        .decode(read["result"]["data"].as_str().expect("IO.read data"))
        .expect("IO.read PDF should be base64");
    assert!(pdf.starts_with(b"%PDF-1.7"));
    assert!(pdf.ends_with(b"%%EOF\n"));

    ctx.process_async(json!({
        "id": 1116,
        "method": "IO.close",
        "sessionId": "SID-PDF-STREAM",
        "params": { "handle": handle }
    }))
    .await;
    assert_eq!(take_response_by_id(&mut ctx, 1116)["result"], json!({}));
}
#[tokio::test(flavor = "multi_thread")]
async fn print_to_pdf_rejects_tiny_page_with_default_margins() {
    // Chromium source:
    // components/headless/test/data/protocol/shared/print-to-pdf-tiny-page.js
    let mut ctx = TestContext::new();
    ctx.process_async(json!({
        "id": 1115,
        "method": "Page.printToPDF",
        "params": { "paperWidth": 0.1, "paperHeight": 0.1 }
    }))
    .await;
    ctx.expect_error(
        1115,
        -32602,
        "invalid print parameters: printable area is empty",
    );
}
#[tokio::test(flavor = "multi_thread")]
async fn capture_screenshot_uses_emulated_device_metrics() {
    let mut ctx = TestContext::new();
    install_active_screenshot_page(
        &mut ctx,
        "BID-SCREENSHOT-EMU",
        "TID-SCREENSHOT-EMU",
        "SID-SCREENSHOT-EMU",
        "data:text/html,<style>html%7Bbackground-color%3Argb(0,255,0)%7D</style>",
    )
    .await;
    set_screenshot_viewport(&mut ctx, "SID-SCREENSHOT-EMU", 800, 600, 2.0, 111).await;

    ctx.process_async(json!({
        "id": 112,
        "method": "Page.captureScreenshot",
        "sessionId": "SID-SCREENSHOT-EMU"
    }))
    .await;
    let response = take_response_by_id(&mut ctx, 112);
    assert_png_dimensions(&screenshot_png_bytes(&response), 1600, 1200);
}
#[tokio::test(flavor = "multi_thread")]
async fn capture_screenshot_targets_loaded_background_owner_without_promotion() {
    let mut ctx = TestContext::new();
    let background = PageTargetHost::with_url(
        "TID-background".to_owned(),
        Some("SID-background".to_owned()),
        "about:blank".to_owned(),
    );

    let mut bc = BrowserContext::new("BID-screenshot-background".to_owned());
    bc.set_active_target_id("TID-active".to_owned());
    bc.attach_active_session("SID-active".to_owned());
    bc.insert_page_target_host(background);
    ctx.conn.browser_context = Some(bc);
    ctx.install_navigation_fixture_for_session_owner(
        "data:text/html,<title>Background Screenshot</title><main>background</main>",
        Some("SID-background"),
    )
    .await;
    ctx.wait_for_scheduler_message("background screenshot fixture load", |message| {
        message["method"] == json!("Page.loadEventFired")
            && message["sessionId"] == json!("SID-background")
    })
    .await;
    ctx.sent.clear();
    ctx.conn
        .browser_context
        .as_mut()
        .expect("browser context")
        .mutate_parked_page_session_state("TID-background", |state| {
            state.emulated_device_metrics = Some(EmulatedDeviceMetrics {
                width: 320,
                height: 240,
                device_scale_factor: 2.0,
                screen_width: 320,
                screen_height: 240,
            });
        });

    ctx.process_async(json!({
        "id": 114,
        "method": "Page.captureScreenshot",
        "sessionId": "SID-background"
    }))
    .await;
    let response = take_response_by_id(&mut ctx, 114);
    assert_png_dimensions(&screenshot_png_bytes(&response), 1920, 1080);
    assert_eq!(
        ctx.conn
            .browser_context
            .as_ref()
            .and_then(|browser_context| browser_context.active_target_id()),
        Some("TID-active"),
        "background Page.captureScreenshot should not promote the target"
    );
}
#[tokio::test(flavor = "multi_thread")]
async fn capture_screenshot_targets_inactive_loaded_owner_without_activation() {
    let mut ctx = TestContext::new();
    load_bc_with_session(
        &mut ctx,
        "BID-active",
        "TID-active",
        "SID-active",
        "about:blank",
    );
    let page = ctx
        .conn
        .load_page_via_runtime_async(
            "data:text/html,<title>Inactive Screenshot</title><main>inactive</main>",
        )
        .await
        .expect("inactive page should load");
    let mut inactive = BrowserContext::new("BID-inactive-screenshot".to_owned());
    inactive.set_active_target_id("TID-inactive".to_owned());
    inactive.attach_active_session("SID-inactive".to_owned());
    inactive.emulated_device_metrics = Some(EmulatedDeviceMetrics {
        width: 500,
        height: 300,
        device_scale_factor: 1.5,
        screen_width: 500,
        screen_height: 300,
    });
    inactive.replace_loaded_page(Some(page));
    ctx.conn.inactive_browser_contexts.push(inactive);

    ctx.process_async(json!({
        "id": 115,
        "method": "Page.captureScreenshot",
        "sessionId": "SID-inactive"
    }))
    .await;
    let response = take_response_by_id(&mut ctx, 115);
    assert_png_dimensions(&screenshot_png_bytes(&response), 1920, 1080);
    assert_eq!(
        ctx.conn
            .browser_context
            .as_ref()
            .map(|browser_context| browser_context.id.as_str()),
        Some("BID-active"),
        "inactive Page.captureScreenshot should not activate its browser context"
    );
}
#[tokio::test(flavor = "multi_thread")]
async fn capture_screenshot_clip_uses_page_coordinates_scale_and_live_dpr() {
    let mut ctx = TestContext::new();
    let url = screenshot_data_url(
        "<!doctype html><style>html,body{margin:0;background:rgb(255,0,0)}#target{position:absolute;left:10px;top:20px;width:30px;height:20px;background:rgb(0,255,0)}</style><div id=target></div>",
    );
    install_active_screenshot_page(
        &mut ctx,
        "BID-SCREENSHOT-CLIP",
        "TID-SCREENSHOT-CLIP",
        "SID-SCREENSHOT-CLIP",
        &url,
    )
    .await;
    set_screenshot_viewport(&mut ctx, "SID-SCREENSHOT-CLIP", 40, 30, 2.0, 116).await;
    ctx.process_async(json!({
        "id": 113,
        "method": "Page.captureScreenshot",
        "sessionId": "SID-SCREENSHOT-CLIP",
        "params": {
            "captureBeyondViewport": true,
            "optimizeForSpeed": true,
            "clip": {
                "x": 10,
                "y": 20,
                "width": 30,
                "height": 20,
                "scale": 0.5
            }
        }
    }))
    .await;
    let response = take_response_by_id(&mut ctx, 113);
    let png = screenshot_png_bytes(&response);
    assert_png_dimensions(&png, 30, 20);
    assert_eq!(decode_png_pixel(&png, 15, 10), [0, 255, 0, 255]);
}

#[tokio::test(flavor = "multi_thread")]
async fn capture_screenshot_beyond_viewport_uses_real_document_extent() {
    let mut ctx = TestContext::new();
    let url = screenshot_data_url(
        "<!doctype html><style>html,body{margin:0}.top,.bottom{width:20px;height:20px}.top{background:red}.bottom{background:lime}</style><div class=top></div><div class=bottom></div>",
    );
    install_active_screenshot_page(
        &mut ctx,
        "BID-SCREENSHOT-FULL",
        "TID-SCREENSHOT-FULL",
        "SID-SCREENSHOT-FULL",
        &url,
    )
    .await;
    set_screenshot_viewport(&mut ctx, "SID-SCREENSHOT-FULL", 20, 20, 2.0, 117).await;
    ctx.process_async(json!({
        "id": 118,
        "method": "Page.captureScreenshot",
        "sessionId": "SID-SCREENSHOT-FULL",
        "params": {
            "captureBeyondViewport": true,
            "optimizeForSpeed": true
        }
    }))
    .await;
    let png = screenshot_png_bytes(&take_response_by_id(&mut ctx, 118));
    assert_png_dimensions(&png, 40, 80);
    assert_eq!(decode_png_pixel(&png, 20, 10), [255, 0, 0, 255]);
    assert_eq!(decode_png_pixel(&png, 20, 60), [0, 255, 0, 255]);
}

#[tokio::test(flavor = "multi_thread")]
async fn devtools_node_screenshot_chain_uses_real_box_and_layout_metrics() {
    let mut ctx = TestContext::new();
    let url = screenshot_data_url(
        "<!doctype html><style>html,body{margin:0;background:red;height:120px}#target{position:absolute;left:7px;top:45px;width:13px;height:9px;background:lime}</style><div id=target></div>",
    );
    install_active_screenshot_page(
        &mut ctx,
        "BID-SCREENSHOT-NODE",
        "TID-SCREENSHOT-NODE",
        "SID-SCREENSHOT-NODE",
        &url,
    )
    .await;
    set_screenshot_viewport(&mut ctx, "SID-SCREENSHOT-NODE", 40, 30, 1.0, 119).await;

    ctx.process_async(json!({
        "id": 120,
        "method": "Runtime.evaluate",
        "sessionId": "SID-SCREENSHOT-NODE",
        "params": {"expression": "window.scrollTo(0, 30)"}
    }))
    .await;
    take_response_by_id(&mut ctx, 120);

    ctx.process_async(json!({
        "id": 121,
        "method": "DOM.getDocument",
        "sessionId": "SID-SCREENSHOT-NODE"
    }))
    .await;
    let document = take_response_by_id(&mut ctx, 121);
    let root_id = document["result"]["root"]["nodeId"]
        .as_u64()
        .expect("DOM.getDocument root nodeId");
    ctx.process_async(json!({
        "id": 122,
        "method": "DOM.querySelector",
        "sessionId": "SID-SCREENSHOT-NODE",
        "params": {"nodeId": root_id, "selector": "#target"}
    }))
    .await;
    let node_id = take_response_by_id(&mut ctx, 122)["result"]["nodeId"]
        .as_u64()
        .expect("DOM.querySelector target nodeId");

    ctx.process_async(json!({
        "id": 123,
        "method": "DOM.getBoxModel",
        "sessionId": "SID-SCREENSHOT-NODE",
        "params": {"nodeId": node_id}
    }))
    .await;
    let model = take_response_by_id(&mut ctx, 123);
    let border = model["result"]["model"]["border"]
        .as_array()
        .expect("DOM.getBoxModel border quad");
    let viewport_x = border[0].as_f64().expect("border x");
    let viewport_y = border[1].as_f64().expect("border y");
    let width = border[2].as_f64().expect("border right") - viewport_x;
    let height = border[5].as_f64().expect("border bottom") - viewport_y;

    ctx.process_async(json!({
        "id": 124,
        "method": "Page.getLayoutMetrics",
        "sessionId": "SID-SCREENSHOT-NODE"
    }))
    .await;
    let metrics = take_response_by_id(&mut ctx, 124);
    let page_x = metrics["result"]["layoutViewport"]["pageX"]
        .as_f64()
        .expect("layout viewport pageX");
    let page_y = metrics["result"]["layoutViewport"]["pageY"]
        .as_f64()
        .expect("layout viewport pageY");

    // This is the same chain used by the Chromium DevTools frontend for
    // "Capture node screenshot": viewport box quad + live scroll offset ->
    // Page.captureScreenshot document-coordinate clip.
    ctx.process_async(json!({
        "id": 125,
        "method": "Page.captureScreenshot",
        "sessionId": "SID-SCREENSHOT-NODE",
        "params": {
            "captureBeyondViewport": true,
            "clip": {
                "x": viewport_x + page_x,
                "y": viewport_y + page_y,
                "width": width,
                "height": height,
                "scale": 1
            }
        }
    }))
    .await;
    let png = screenshot_png_bytes(&take_response_by_id(&mut ctx, 125));
    assert_png_dimensions(&png, 13, 9);
    assert_eq!(decode_png_pixel(&png, 6, 4), [0, 255, 0, 255]);
}

async fn install_active_screenshot_page(
    ctx: &mut TestContext,
    browser_context_id: &str,
    target_id: &str,
    session_id: &str,
    url: &str,
) {
    load_bc_with_session(ctx, browser_context_id, target_id, session_id, url);
    ctx.install_navigation_fixture_for_session_owner(url, Some(session_id))
        .await;
    ctx.wait_for_scheduler_message("screenshot fixture load", |message| {
        message["method"] == json!("Page.loadEventFired")
            && message["sessionId"] == json!(session_id)
    })
    .await;
    ctx.sent.clear();
}

async fn set_screenshot_viewport(
    ctx: &mut TestContext,
    session_id: &str,
    width: u32,
    height: u32,
    device_scale_factor: f64,
    command_id: u64,
) {
    ctx.process_async(json!({
        "id": command_id,
        "method": "Emulation.setDeviceMetricsOverride",
        "sessionId": session_id,
        "params": {
            "width": width,
            "height": height,
            "deviceScaleFactor": device_scale_factor,
            "mobile": false
        }
    }))
    .await;
    take_response_by_id(ctx, command_id);
}

fn screenshot_png_bytes(response: &Value) -> Vec<u8> {
    screenshot_bytes(response)
}

fn screenshot_bytes(response: &Value) -> Vec<u8> {
    let data = response["result"]["data"]
        .as_str()
        .expect("captureScreenshot should return base64 data");
    BASE64_STANDARD
        .decode(data)
        .expect("captureScreenshot data should be standard base64")
}

fn assert_png_dimensions(bytes: &[u8], width: u32, height: u32) {
    assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n");
    assert_eq!(&bytes[12..16], b"IHDR");
    assert_eq!(u32::from_be_bytes(bytes[16..20].try_into().unwrap()), width);
    assert_eq!(
        u32::from_be_bytes(bytes[20..24].try_into().unwrap()),
        height
    );
}

fn decode_png_pixel(bytes: &[u8], x: u32, y: u32) -> [u8; 4] {
    let image = moli_image::decode_png(bytes)
        .expect("captureScreenshot PNG should decode through moli-image");
    assert!(x < image.width && y < image.height);
    let offset = ((y * image.width + x) * 4) as usize;
    image.rgba[offset..offset + 4]
        .try_into()
        .expect("one decoded RGBA pixel")
}

fn screenshot_data_url(html: &str) -> String {
    format!(
        "data:text/html,{}",
        percent_encoding::percent_encode(html.as_bytes(), percent_encoding::NON_ALPHANUMERIC)
    )
}
/// cdp.page: getLayoutMetrics – falls back to viewport metrics without a live page
#[tokio::test(flavor = "multi_thread")]
async fn get_layout_metrics() {
    let mut ctx = TestContext::new();
    load_bc_with_target(&mut ctx, "BID-9", "FID-000000000X", "about:blank");
    ctx.process_async(json!({"id": 12, "method": "Page.getLayoutMetrics"}))
        .await;
    let msg = ctx.take_one();
    let r = &msg["result"];
    assert_eq!(r["layoutViewport"]["clientWidth"], 1920);
    assert_eq!(r["layoutViewport"]["clientHeight"], 1080);
    assert_eq!(r["contentSize"]["width"], 1920.0);
    assert_eq!(r["contentSize"]["height"], 1080.0);
}
#[tokio::test(flavor = "multi_thread")]
async fn get_layout_metrics_uses_viewport_fallback_without_live_page() {
    let mut ctx = TestContext::new();
    load_bc_with_session(
        &mut ctx,
        "BID-LIVE-METRICS",
        "TID-LIVE-METRICS",
        "SID-LIVE-METRICS",
        "about:blank",
    );

    ctx.process_async(json!({
        "id": 120,
        "method": "Emulation.setDeviceMetricsOverride",
        "sessionId": "SID-LIVE-METRICS",
        "params": {
            "width": 800,
            "height": 600,
            "deviceScaleFactor": 1,
            "screenWidth": 800,
            "screenHeight": 600,
            "mobile": false
        }
    }))
    .await;
    ctx.expect_result(120, json!({}), Some("SID-LIVE-METRICS"));

    ctx.process_async(json!({
        "id": 122,
        "method": "Page.getLayoutMetrics",
        "sessionId": "SID-LIVE-METRICS",
    }))
    .await;
    let metrics = take_response_by_id(&mut ctx, 122);
    let result = &metrics["result"];
    assert_eq!(result["layoutViewport"]["clientWidth"], json!(800));
    assert_eq!(result["layoutViewport"]["clientHeight"], json!(600));
    assert_eq!(result["visualViewport"]["scale"], json!(1.0));
    assert_eq!(
        result["contentSize"],
        json!({ "x": 0, "y": 0, "width": 800.0, "height": 600.0 }),
        "without a live renderer page, content size should use the owner viewport"
    );
    assert_eq!(
        result["cssContentSize"]["width"],
        result["contentSize"]["width"]
    );
    assert_eq!(
        result["cssContentSize"]["height"],
        result["contentSize"]["height"]
    );
}
#[tokio::test(flavor = "multi_thread")]
async fn get_layout_metrics_queries_live_renderer_for_loaded_pages() {
    let mut ctx = TestContext::new();
    load_bc_with_session(
        &mut ctx,
        "BID-PENDING-LAYOUT-METRICS",
        "TID-PENDING-LAYOUT-METRICS",
        "SID-PENDING-LAYOUT-METRICS",
        "about:blank",
    );
    let page_url = "data:text/html,<html style='width:2300px;height:1500px'><body style='margin:0;width:2300px;height:1500px'><div style='width:2300px;height:1500px'></div></body></html>";
    let page = ctx
        .conn
        .load_page_via_runtime_async(page_url)
        .await
        .expect("page should load");
    ctx.conn
        .browser_context
        .as_mut()
        .expect("browser context")
        .active_target
        .runtime_slot
        .replace_loaded_page(Some(page));

    ctx.process_async(json!({
        "id": 125,
        "method": "Page.getLayoutMetrics",
        "sessionId": "SID-PENDING-LAYOUT-METRICS"
    }))
    .await;
    let metrics = take_response_by_id(&mut ctx, 125);
    assert_eq!(
        metrics["sessionId"],
        json!("SID-PENDING-LAYOUT-METRICS"),
        "response should stay scoped to the session"
    );
    assert_eq!(
        metrics["result"]["contentSize"],
        json!({ "x": 0, "y": 0, "width": 2300.0, "height": 1500.0 }),
        "content size should come from a one-shot live layout: {metrics:?}"
    );
}
#[tokio::test(flavor = "multi_thread")]
async fn get_layout_metrics_targets_loaded_background_owner_without_promotion() {
    let mut ctx = TestContext::new();
    let page_url = "data:text/html,<html style='width:2300px;height:1700px'><body style='margin:0;width:2300px;height:1700px'><div style='width:2300px;height:1700px'></div></body></html>";
    let background = PageTargetHost::with_url(
        "TID-background".to_owned(),
        Some("SID-background".to_owned()),
        "about:blank".to_owned(),
    );

    let mut bc = BrowserContext::new("BID-1".to_owned());
    bc.set_active_target_id("TID-active".to_owned());
    bc.attach_active_session("SID-active".to_owned());
    bc.set_target_url("data:text/html,<body>active</body>".to_owned());
    bc.insert_page_target_host(background);
    ctx.conn.browser_context = Some(bc);
    ctx.install_navigation_fixture_for_session_owner(page_url, Some("SID-background"))
        .await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 121,
        "method": "Emulation.setDeviceMetricsOverride",
        "sessionId": "SID-background",
        "params": {
            "width": 640,
            "height": 480,
            "deviceScaleFactor": 1.5,
            "screenWidth": 640,
            "screenHeight": 480,
            "mobile": false
        }
    }))
    .await;
    ctx.expect_result(121, json!({}), Some("SID-background"));

    ctx.process_async(json!({
        "id": 123,
        "method": "Page.getLayoutMetrics",
        "sessionId": "SID-background"
    }))
    .await;
    let metrics = take_response_by_id(&mut ctx, 123);
    let result = &metrics["result"];
    assert_eq!(result["visualViewport"]["scale"], json!(1.5));
    assert_eq!(
        result["contentSize"],
        json!({ "x": 0, "y": 0, "width": 2300.0, "height": 1700.0 }),
        "content size should come from the background owner's live document"
    );
    assert_eq!(
        ctx.conn
            .browser_context
            .as_ref()
            .and_then(|browser_context| browser_context.active_target_id()),
        Some("TID-active"),
        "background Page.getLayoutMetrics should not promote the target"
    );
}
#[tokio::test(flavor = "multi_thread")]
async fn get_layout_metrics_targets_inactive_loaded_owner_without_activation() {
    let mut ctx = TestContext::new();
    let page_url = "data:text/html,<html style='width:2100px;height:1600px'><body style='margin:0;width:2100px;height:1600px'><div style='width:2100px;height:1600px'></div></body></html>";
    let mut inactive = BrowserContext::new("BID-inactive".to_owned());
    inactive.set_active_target_id("TID-inactive".to_owned());
    inactive.attach_active_session("SID-inactive".to_owned());
    inactive.set_target_url("about:blank".to_owned());
    ctx.conn.inactive_browser_contexts.push(inactive);
    ctx.install_navigation_fixture_for_session_owner(page_url, Some("SID-inactive"))
        .await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 121,
        "method": "Emulation.setDeviceMetricsOverride",
        "sessionId": "SID-inactive",
        "params": {
            "width": 700,
            "height": 500,
            "deviceScaleFactor": 2.0,
            "screenWidth": 700,
            "screenHeight": 500,
            "mobile": false
        }
    }))
    .await;
    ctx.expect_result(121, json!({}), Some("SID-inactive"));

    ctx.process_async(json!({
        "id": 124,
        "method": "Page.getLayoutMetrics",
        "sessionId": "SID-inactive"
    }))
    .await;
    let metrics = take_response_by_id(&mut ctx, 124);
    let result = &metrics["result"];
    assert_eq!(result["visualViewport"]["scale"], json!(2.0));
    assert_eq!(
        result["contentSize"],
        json!({ "x": 0, "y": 0, "width": 2100.0, "height": 1600.0 }),
        "content size should come from the inactive owner's live document"
    );
    assert!(
        ctx.conn.browser_context.is_none(),
        "inactive Page.getLayoutMetrics should not activate its browser context"
    );
}
