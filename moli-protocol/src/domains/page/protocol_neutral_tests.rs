use crate::devtools_runtime::{
    DevToolsCaptureScreenshotClip, DevToolsCaptureScreenshotCommand, DevToolsCommand,
    DevToolsCommandContext, DevToolsPrintToPdfCommand, DevToolsPrintToPdfTransferMode,
    DevToolsProtocol, DevToolsScreenshotClip, DevToolsTargetId,
};
use serde_json::{Value, json};

use crate::conn::{CdpConnection, Cmd};

use super::{
    PageCommandTaskStep, build_cdp_capture_screenshot_command, build_cdp_get_frame_tree_command,
    build_cdp_get_layout_metrics_command, build_cdp_handle_javascript_dialog_command,
    build_cdp_print_to_pdf_command, start_devtools_page_command,
};

#[test]
fn cdp_get_frame_tree_builds_protocol_neutral_command() {
    let conn = CdpConnection::new();
    let params = Value::Null;
    let cmd = Cmd::for_test(
        Some(120),
        "Page.getFrameTree",
        &params,
        Some("SID-page"),
        r#"{"id":120,"method":"Page.getFrameTree"}"#,
    );

    let command = build_cdp_get_frame_tree_command(&conn, &cmd);

    assert_eq!(command.context.protocol, DevToolsProtocol::Cdp);
    assert_eq!(
        command.context.session_id.as_ref().map(|id| id.as_str()),
        Some("SID-page")
    );
    assert_eq!(command.context.target_id, None);
    assert_eq!(command.context.browser_context_id, None);
}

#[test]
fn devtools_page_entry_routes_get_frame_tree_command_to_page_owner() {
    let mut conn = CdpConnection::new();
    let params = Value::Null;
    let cmd = Cmd::for_test(
        Some(121),
        "Page.getFrameTree",
        &params,
        None,
        r#"{"id":121,"method":"Page.getFrameTree"}"#,
    );
    let command = build_cdp_get_frame_tree_command(&conn, &cmd);

    let step =
        start_devtools_page_command(&mut conn, cmd.id, DevToolsCommand::GetFrameTree(command));

    let PageCommandTaskStep::Complete(plan) = step else {
        panic!("missing browser context should complete through the unified page entry");
    };
    let mut out = Vec::new();
    plan.emit_into(&mut out, cmd.id, cmd.session_id);
    assert_eq!(out[0]["id"], json!(121));
    assert_eq!(out[0]["error"]["code"], json!(-31998));
    assert_eq!(out[0]["error"]["message"], json!("BrowserContextNotLoaded"));
}

#[test]
fn cdp_get_layout_metrics_builds_protocol_neutral_command() {
    let conn = CdpConnection::new();
    let params = Value::Null;
    let cmd = Cmd::for_test(
        Some(122),
        "Page.getLayoutMetrics",
        &params,
        Some("SID-page"),
        r#"{"id":122,"method":"Page.getLayoutMetrics"}"#,
    );

    let command = build_cdp_get_layout_metrics_command(&conn, &cmd);

    assert_eq!(command.context.protocol, DevToolsProtocol::Cdp);
    assert_eq!(
        command.context.session_id.as_ref().map(|id| id.as_str()),
        Some("SID-page")
    );
    assert_eq!(command.context.target_id, None);
    assert_eq!(command.context.browser_context_id, None);
}

#[test]
fn devtools_page_entry_routes_get_layout_metrics_command_to_page_owner() {
    let mut conn = CdpConnection::new();
    let params = Value::Null;
    let cmd = Cmd::for_test(
        Some(123),
        "Page.getLayoutMetrics",
        &params,
        None,
        r#"{"id":123,"method":"Page.getLayoutMetrics"}"#,
    );
    let command = build_cdp_get_layout_metrics_command(&conn, &cmd);

    let step = start_devtools_page_command(
        &mut conn,
        cmd.id,
        DevToolsCommand::GetLayoutMetrics(command),
    );

    let PageCommandTaskStep::Complete(plan) = step else {
        panic!("missing page should complete through the unified page entry");
    };
    let mut out = Vec::new();
    plan.emit_into(&mut out, cmd.id, cmd.session_id);
    assert_eq!(out[0]["id"], json!(123));
    assert_eq!(out[0]["error"]["code"], -32000);
    assert_eq!(out[0]["error"]["message"], "NoDocumentLoaded");
}

#[test]
fn cdp_handle_javascript_dialog_builds_protocol_neutral_command() {
    let conn = CdpConnection::new();
    let params = json!({
        "accept": false,
        "promptText": "typed text"
    });
    let cmd = Cmd::for_test(
        Some(124),
        "Page.handleJavaScriptDialog",
        &params,
        Some("SID-page"),
        r#"{"id":124,"method":"Page.handleJavaScriptDialog"}"#,
    );

    let command = build_cdp_handle_javascript_dialog_command(&conn, &cmd);
    let Ok(command) = command else {
        panic!("valid handleJavaScriptDialog command");
    };

    assert_eq!(command.context.protocol, DevToolsProtocol::Cdp);
    assert_eq!(
        command.context.session_id.as_ref().map(|id| id.as_str()),
        Some("SID-page")
    );
    assert!(!command.accept);
    assert_eq!(command.prompt_text, "typed text");
}

#[test]
fn devtools_page_entry_routes_handle_javascript_dialog_command_to_page_owner() {
    let mut conn = CdpConnection::new();
    let params = json!({
        "accept": true
    });
    let cmd = Cmd::for_test(
        Some(125),
        "Page.handleJavaScriptDialog",
        &params,
        Some("SID-page"),
        r#"{"id":125,"method":"Page.handleJavaScriptDialog"}"#,
    );
    let command = build_cdp_handle_javascript_dialog_command(&conn, &cmd);
    let Ok(command) = command else {
        panic!("valid handleJavaScriptDialog command");
    };

    let step = start_devtools_page_command(
        &mut conn,
        cmd.id,
        DevToolsCommand::HandleJavaScriptDialog(command),
    );

    let PageCommandTaskStep::Complete(plan) = step else {
        panic!("dialog command should complete through the unified page entry");
    };
    let mut out = Vec::new();
    plan.emit_into(&mut out, cmd.id, cmd.session_id);
    assert_eq!(out[0]["id"], json!(125));
    assert_eq!(out[0]["error"]["code"], json!(-32602));
    assert_eq!(out[0]["error"]["message"], json!("No dialog is showing"));
}

#[test]
fn cdp_capture_screenshot_builds_requested_capture_command() {
    let conn = CdpConnection::new();
    let params = json!({
        "format": "png",
        "quality": 100,
        "fromSurface": true,
        "captureBeyondViewport": false,
        "optimizeForSpeed": false
    });
    let cmd = Cmd::for_test(
        Some(126),
        "Page.captureScreenshot",
        &params,
        Some("SID-page"),
        r#"{"id":126,"method":"Page.captureScreenshot"}"#,
    );

    let command = build_cdp_capture_screenshot_command(&conn, &cmd);
    let Ok(command) = command else {
        panic!("valid captureScreenshot command");
    };

    assert_eq!(command.context.protocol, DevToolsProtocol::Cdp);
    assert_eq!(
        command.context.session_id.as_ref().map(|id| id.as_str()),
        Some("SID-page")
    );
    assert_eq!(command.format.as_deref(), Some("png"));
    assert_eq!(command.quality, Some(100));
    assert_eq!(command.clip, None);
    assert!(!command.capture_beyond_viewport);
    assert!(!command.optimize_for_speed);
}

#[test]
fn cdp_capture_screenshot_preserves_page_clip() {
    let conn = CdpConnection::new();
    let params = json!({
        "format": "png",
        "clip": {
            "x": 0.0,
            "y": 0.0,
            "width": 2.0,
            "height": 3.0,
            "scale": 1.0
        }
    });
    let cmd = Cmd::for_test(
        Some(127),
        "Page.captureScreenshot",
        &params,
        Some("SID-page"),
        r#"{"id":127,"method":"Page.captureScreenshot"}"#,
    );
    let Ok(command) = build_cdp_capture_screenshot_command(&conn, &cmd) else {
        panic!("valid clip should enter the protocol-neutral screenshot command");
    };
    assert_eq!(
        command.clip,
        Some(DevToolsCaptureScreenshotClip::Box(DevToolsScreenshotClip {
            x: 0.0,
            y: 0.0,
            width: 2.0,
            height: 3.0,
            scale: 1.0,
        }))
    );
}

#[test]
fn devtools_page_entry_rejects_unsupported_capture_screenshot_format() {
    let conn = CdpConnection::new();
    let params = json!({
        "format": "webp"
    });
    let cmd = Cmd::for_test(
        Some(128),
        "Page.captureScreenshot",
        &params,
        None,
        r#"{"id":128,"method":"Page.captureScreenshot"}"#,
    );
    let Err(plan) = build_cdp_capture_screenshot_command(&conn, &cmd) else {
        panic!("webp should be rejected at the CDP capability boundary");
    };
    let mut out = Vec::new();
    plan.emit_into(&mut out, cmd.id, cmd.session_id);
    assert_eq!(out[0]["id"], json!(128));
    assert_eq!(out[0]["error"]["code"], json!(-32000));
    assert_eq!(
        out[0]["error"]["message"],
        json!("Page.captureScreenshot option 'format' is not supported.")
    );
}

#[test]
fn cdp_capture_screenshot_rejects_invalid_quality_and_clip() {
    let conn = CdpConnection::new();
    for (id, params, expected_message) in [
        (
            132,
            json!({ "quality": 101 }),
            "Page.captureScreenshot quality must be between 0 and 100.",
        ),
        (
            133,
            json!({
                "clip": {
                    "x": 0.0,
                    "y": 0.0,
                    "width": 0.0,
                    "height": 3.0,
                    "scale": 1.0
                }
            }),
            "Page.captureScreenshot clip must have a finite origin and positive finite width, height, and scale.",
        ),
    ] {
        let raw = format!(r#"{{"id":{id},"method":"Page.captureScreenshot"}}"#);
        let cmd = Cmd::for_test(
            Some(id),
            "Page.captureScreenshot",
            &params,
            Some("SID-page"),
            &raw,
        );
        let Err(plan) = build_cdp_capture_screenshot_command(&conn, &cmd) else {
            panic!("invalid screenshot parameters should be rejected");
        };
        let mut out = Vec::new();
        plan.emit_into(&mut out, cmd.id, cmd.session_id);
        assert_eq!(out[0]["error"]["code"], json!(-32602));
        assert_eq!(out[0]["error"]["message"], json!(expected_message));
    }
}

#[test]
fn devtools_page_entry_validates_capture_screenshot_target_before_unsupported() {
    let mut conn = CdpConnection::new();
    let command = DevToolsCaptureScreenshotCommand {
        context: DevToolsCommandContext {
            protocol: DevToolsProtocol::WebDriverBidi,
            session_id: None,
            target_id: Some(DevToolsTargetId::from("missing-target")),
            browser_context_id: None,
        },
        format: Some("png".to_owned()),
        quality: None,
        clip: None,
        capture_beyond_viewport: false,
        optimize_for_speed: false,
    };

    let step = start_devtools_page_command(
        &mut conn,
        Some(131),
        DevToolsCommand::CaptureScreenshot(command),
    );

    let PageCommandTaskStep::Complete(plan) = step else {
        panic!("screenshot target validation should complete synchronously");
    };
    let mut out = Vec::new();
    plan.emit_into(&mut out, Some(131), None);
    assert_eq!(out[0]["id"], json!(131));
    assert_eq!(out[0]["error"]["code"], json!(-31998));
    assert_eq!(out[0]["error"]["message"], json!("NoSuchTarget"));
}

#[test]
fn cdp_print_to_pdf_builds_protocol_neutral_command() {
    let conn = CdpConnection::new();
    let params = json!({
        "landscape": true,
        "printBackground": true,
        "scale": 1.25,
        "paperWidth": 8.0,
        "paperHeight": 10.0,
        "marginTop": 0.25,
        "marginBottom": 0.5,
        "marginLeft": 0.75,
        "marginRight": 1.0,
        "pageRanges": "1-2,4",
        "transferMode": "ReturnAsStream"
    });
    let cmd = Cmd::for_test(
        Some(129),
        "Page.printToPDF",
        &params,
        Some("SID-page"),
        r#"{"id":129,"method":"Page.printToPDF"}"#,
    );

    let command = build_cdp_print_to_pdf_command(&conn, &cmd);
    let Ok(command) = command else {
        panic!("valid printToPDF command");
    };

    assert_eq!(command.context.protocol, DevToolsProtocol::Cdp);
    assert_eq!(
        command.context.session_id.as_ref().map(|id| id.as_str()),
        Some("SID-page")
    );
    assert_eq!(command.landscape, Some(true));
    assert_eq!(command.print_background, Some(true));
    assert_eq!(command.scale, Some(1.25));
    assert_eq!(command.paper_width, Some(8.0));
    assert_eq!(command.paper_height, Some(10.0));
    assert_eq!(command.margin_top, Some(0.25));
    assert_eq!(command.margin_bottom, Some(0.5));
    assert_eq!(command.margin_left, Some(0.75));
    assert_eq!(command.margin_right, Some(1.0));
    assert_eq!(command.page_ranges, Some("1-2,4".to_owned()));
    assert_eq!(
        command.transfer_mode,
        Some(DevToolsPrintToPdfTransferMode::ReturnAsStream)
    );
}

#[test]
fn devtools_page_entry_reports_layout_disabled_without_placeholder_payload() {
    let mut conn = CdpConnection::new();
    let params = Value::Null;
    let cmd = Cmd::for_test(
        Some(130),
        "Page.printToPDF",
        &params,
        None,
        r#"{"id":130,"method":"Page.printToPDF"}"#,
    );
    let command = build_cdp_print_to_pdf_command(&conn, &cmd);
    let Ok(command) = command else {
        panic!("default printToPDF command should build");
    };

    let step = start_devtools_page_command(&mut conn, cmd.id, DevToolsCommand::PrintToPdf(command));

    let PageCommandTaskStep::Complete(plan) = step else {
        panic!("printToPDF command should complete through the unified page entry");
    };
    let mut out = Vec::new();
    plan.emit_into(&mut out, cmd.id, cmd.session_id);
    assert_eq!(out[0]["id"], json!(130));
    assert_eq!(out[0]["error"]["code"], json!(-32000));
    assert_eq!(
        out[0]["error"]["message"],
        json!(
            "Page.printToPDF is not supported: renderer layout is disabled; start Moli with --layout."
        )
    );
}

#[test]
fn devtools_page_entry_validates_print_to_pdf_target_before_unsupported() {
    let mut conn = CdpConnection::new();
    let command = DevToolsPrintToPdfCommand {
        context: DevToolsCommandContext {
            protocol: DevToolsProtocol::WebDriverBidi,
            session_id: None,
            target_id: Some(DevToolsTargetId::from("missing-target")),
            browser_context_id: None,
        },
        landscape: None,
        print_background: None,
        scale: None,
        paper_width: None,
        paper_height: None,
        margin_top: None,
        margin_bottom: None,
        margin_left: None,
        margin_right: None,
        page_ranges: None,
        shrink_to_fit: None,
        transfer_mode: None,
    };

    let step =
        start_devtools_page_command(&mut conn, Some(132), DevToolsCommand::PrintToPdf(command));

    let PageCommandTaskStep::Complete(plan) = step else {
        panic!("print target validation should complete synchronously");
    };
    let mut out = Vec::new();
    plan.emit_into(&mut out, Some(132), None);
    assert_eq!(out[0]["id"], json!(132));
    assert_eq!(out[0]["error"]["code"], json!(-31998));
    assert_eq!(out[0]["error"]["message"], json!("NoSuchTarget"));
}
