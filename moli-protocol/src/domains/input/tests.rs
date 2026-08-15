use super::*;
use crate::conn::{BrowserContext, CdpCommandTaskStep, CommandDispatchContext};
use crate::testing::{TestContext, wait_until_frame_stopped_loading};
use moli_core::LayoutPolicy;

const INPUT_HIT_X: u32 = 20;
const INPUT_HIT_Y: u32 = 20;

async fn with_loaded_document(ctx: &mut TestContext, html: &str) {
    let mut bc = BrowserContext::new("BID-I".into());
    bc.set_active_target_id("TID-1");
    ctx.conn.browser_context = Some(bc);
    let data_url = format!("data:text/html,{html}");
    // Input commands can execute JS and therefore publish concrete renderer
    // output. Install the fixture through the same Page-owner transaction as
    // production navigation instead of exposing a bare Page without an output
    // stream owner or committed Document binding.
    ctx.install_navigation_fixture_for_session_owner(&data_url, None)
        .await;
}

async fn evaluate_string(ctx: &mut TestContext, expression: &str) -> String {
    ctx.conn
        .evaluate_runtime_expression_with_await_async(expression, false)
        .await
        .expect("expression must succeed")["value"]
        .as_str()
        .expect("expression must return a string")
        .to_owned()
}

async fn evaluate_bool(ctx: &mut TestContext, expression: &str) -> bool {
    ctx.conn
        .evaluate_runtime_expression_with_await_async(expression, false)
        .await
        .expect("expression must succeed")["value"]
        .as_bool()
        .expect("expression must return a boolean")
}

async fn resolve_selector_object_id(ctx: &mut TestContext, selector: &str, id_base: u64) -> String {
    ctx.process_async(json!({
        "id": id_base,
        "method": "DOM.getDocument"
    }))
    .await;
    let root_id = ctx.take_response_by_id(id_base)["result"]["root"]["nodeId"]
        .as_u64()
        .expect("DOM.getDocument should return a root node id");

    ctx.process_async(json!({
        "id": id_base + 1,
        "method": "DOM.querySelector",
        "params": {
            "nodeId": root_id,
            "selector": selector
        }
    }))
    .await;
    let node_id = ctx.take_response_by_id(id_base + 1)["result"]["nodeId"]
        .as_u64()
        .unwrap_or(0);
    assert!(node_id > 0, "DOM.querySelector should find {selector}");

    ctx.process_async(json!({
        "id": id_base + 2,
        "method": "DOM.resolveNode",
        "params": {
            "nodeId": node_id
        }
    }))
    .await;
    ctx.take_response_by_id(id_base + 2)["result"]["object"]["objectId"]
        .as_str()
        .unwrap_or_else(|| panic!("DOM.resolveNode should return objectId for {selector}"))
        .to_owned()
}

async fn call_function_on_object(
    ctx: &mut TestContext,
    id: u64,
    object_id: &str,
    function_declaration: &str,
) -> serde_json::Value {
    ctx.process_async(json!({
        "id": id,
        "method": "Runtime.callFunctionOn",
        "params": {
            "objectId": object_id,
            "functionDeclaration": function_declaration,
            "returnByValue": true
        }
    }))
    .await;
    ctx.take_response_by_id(id)
}

#[tokio::test(flavor = "multi_thread")]
async fn selector_runtime_click_and_check_uncheck_do_not_require_geometry() {
    let mut ctx = TestContext::new();
    with_loaded_document(
        &mut ctx,
        r#"<html><body>
                <button id='btn' onclick="document.body.dataset.clicked = String(Number(document.body.dataset.clicked || '0') + 1)">go</button>
                <input id='box' type='checkbox'>
               </body></html>"#,
    )
    .await;

    let button_object_id = resolve_selector_object_id(&mut ctx, "#btn", 10).await;
    let clicked = call_function_on_object(
        &mut ctx,
        20,
        &button_object_id,
        "function() { this.click(); return document.body.dataset.clicked; }",
    )
    .await;
    assert_eq!(clicked["result"]["result"]["value"], json!("1"));

    let checkbox_object_id = resolve_selector_object_id(&mut ctx, "#box", 30).await;
    let checked = call_function_on_object(
        &mut ctx,
        40,
        &checkbox_object_id,
        "function() { if (!this.checked) this.click(); return this.checked; }",
    )
    .await;
    assert_eq!(checked["result"]["result"]["value"], json!(true));

    let unchecked = call_function_on_object(
        &mut ctx,
        41,
        &checkbox_object_id,
        "function() { if (this.checked) this.click(); return this.checked; }",
    )
    .await;
    assert_eq!(unchecked["result"]["result"]["value"], json!(false));

    ctx.process_async(json!({
        "id": 42,
        "method": "Runtime.evaluate",
        "params": {
            "expression": "JSON.stringify({ clicked: document.body.dataset.clicked, checked: document.getElementById('box').checked })",
            "returnByValue": true
        }
    }))
    .await;
    assert_eq!(
        ctx.take_response_by_id(42)["result"]["result"]["value"],
        json!(r#"{"clicked":"1","checked":false}"#)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn coordinate_mouse_commands_hit_test_real_layout_and_dispatch_dom_events() {
    let mut ctx = TestContext::new();
    with_loaded_document(
        &mut ctx,
        r#"<html><body style='margin:0'>
                <button id='btn' style='position:absolute;left:0;top:0;width:80px;height:80px'>go</button>
                <script>
                  window.__events = [];
                  const btn = document.getElementById('btn');
                  ['mousedown', 'mouseup', 'mousemove', 'wheel', 'click'].forEach((type) => {
                    btn.addEventListener(type, () => window.__events.push(type));
                  });
                </script>
               </body></html>"#,
    )
    .await;

    let commands = [
        json!({
            "id": 1,
            "method": "Input.dispatchMouseEvent",
            "params": { "type": "mousePressed", "x": INPUT_HIT_X, "y": INPUT_HIT_Y, "button": "left", "buttons": 1, "clickCount": 1 }
        }),
        json!({
            "id": 2,
            "method": "Input.dispatchMouseEvent",
            "params": { "type": "mouseReleased", "x": INPUT_HIT_X, "y": INPUT_HIT_Y, "button": "left", "buttons": 0, "clickCount": 1 }
        }),
        json!({
            "id": 3,
            "method": "Input.dispatchMouseEvent",
            "params": { "type": "mouseWheel", "x": INPUT_HIT_X, "y": INPUT_HIT_Y, "deltaX": 4, "deltaY": -12 }
        }),
        json!({
            "id": 9,
            "method": "Input.dispatchMouseEvent",
            "params": { "type": "mouseMoved", "x": INPUT_HIT_X, "y": INPUT_HIT_Y, "modifiers": 8 }
        }),
    ];

    for command in commands {
        let id = command["id"].as_u64().expect("command id");
        ctx.process_async(command).await;
        ctx.expect_result(id, json!({}), None);
    }

    assert_eq!(
        evaluate_string(&mut ctx, "JSON.stringify(window.__events)").await,
        r#"["mousedown","mouseup","click","wheel","mousemove"]"#
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn coordinate_mouse_release_acknowledges_real_link_navigation() {
    tokio::task::LocalSet::new()
        .run_until(async {
            let mut ctx = TestContext::new();
            ctx.enable_background_navigation_scheduler_for_test();
            with_loaded_document(
                &mut ctx,
                r#"<html><body style='margin:0'>
                <a id='next' href='data:text/html,destination'
                   style='display:block;width:80px;height:80px'>next</a>
               </body></html>"#,
            )
            .await;
            ctx.enable_page_events_for_test(None);

            ctx.process_and_wait_for_response_async(json!({
                "id": 101,
                "method": "Input.dispatchMouseEvent",
                "params": {
                    "type": "mousePressed",
                    "x": INPUT_HIT_X,
                    "y": INPUT_HIT_Y,
                    "button": "left",
                    "buttons": 1,
                    "clickCount": 1
                }
            }))
            .await;
            ctx.expect_result(101, json!({}), None);

            ctx.process_and_wait_for_response_async(json!({
                "id": 102,
                "method": "Input.dispatchMouseEvent",
                "params": {
                    "type": "mouseReleased",
                    "x": INPUT_HIT_X,
                    "y": INPUT_HIT_Y,
                    "button": "left",
                    "buttons": 0,
                    "clickCount": 1
                }
            }))
            .await;

            // Chromium acknowledges an input event once its renderer has consumed
            // it. The resulting Document replacement is an independent lifecycle and
            // must not retroactively turn this response into NoDocumentLoaded.
            ctx.expect_result(102, json!({}), None);
            wait_until_frame_stopped_loading(&mut ctx, "TID-1").await;
            ctx.expect_event("Page.frameNavigated", None);
            assert_eq!(
                evaluate_string(&mut ctx, "document.body.textContent").await,
                "destination"
            );
        })
        .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn completed_mouse_event_does_not_restore_replaced_page_state() {
    let mut ctx = TestContext::new();
    with_loaded_document(&mut ctx, "<body>origin</body>").await;

    let original_owner = ctx
        .conn
        .target_page_residence_identity_for_session(None)
        .expect("the original Page should have a residence identity");
    let pending = loaded_page_mut(&mut ctx.conn, None)
        .expect("the original Page should be loaded")
        .start_dispatch_mouse_event_at_point_with_outcome(
            INPUT_HIT_X.into(),
            INPUT_HIT_Y.into(),
            "mousemove",
            -1,
            None,
            0.0,
            0.0,
        )
        .expect("the original Page should admit the mouse event");
    let completed = PendingInputCommandDispatch {
        command_id: Some(104),
        session_id: None,
        owner: original_owner.clone(),
        page_residence_token: None,
        kind: PendingInputCommandKind::DispatchMouseEvent,
        pending: PendingInputOperation::Page(pending),
    }
    .wait()
    .await;

    let replacement_url = "data:text/html,<body>replacement</body>";
    ctx.install_navigation_fixture_for_session_owner(replacement_url, None)
        .await;
    let replacement_owner = ctx
        .conn
        .target_page_residence_identity_for_session(None)
        .expect("the replacement Page should have a residence identity");
    assert_ne!(original_owner, replacement_owner);

    let completed = complete_pending_input_command(
        &mut ctx.conn,
        completed,
        &mut CommandDispatchContext::default(),
    )
    .await;
    assert!(matches!(completed.result, Ok(DevToolsCommandResult::Empty)));
    assert!(completed.protocol_events.is_empty());
    assert!(
        ctx.conn
            .browser_context
            .as_ref()
            .and_then(|context| context.active_target.runtime_slot.loaded_page())
            .is_some_and(|page| page.final_url().as_str() == replacement_url),
        "settling the original input command must not install its Page state into the replacement"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn pending_mouse_event_acknowledges_when_page_is_replaced_before_renderer_completion() {
    let mut ctx = TestContext::new();
    with_loaded_document(&mut ctx, "<body>origin</body>").await;

    let original_owner = ctx
        .conn
        .target_page_residence_identity_for_session(None)
        .expect("the original Page should have a residence identity");
    let page_residence_token = ctx
        .conn
        .capture_target_page_residence_token_for_session(None)
        .expect("the original Page should expose its attachment lifetime");

    let replacement_url = "data:text/html,<body>replacement-before-completion</body>";
    ctx.install_navigation_fixture_for_session_owner(replacement_url, None)
        .await;

    let wait = wait_for_renderer_input_or_page_replacement(
        std::future::pending::<()>(),
        Some(page_residence_token),
    );
    let outcome = tokio::time::timeout(std::time::Duration::from_secs(1), wait)
        .await
        .expect("Page replacement should settle the pending input wait");
    assert!(matches!(
        outcome,
        RendererInputWaitOutcome::PageResidence(TargetPageResidenceObservation::Superseded)
    ));

    let completed = complete_pending_input_command(
        &mut ctx.conn,
        CompletedInputCommandDispatch {
            command_id: Some(105),
            session_id: None,
            owner: original_owner,
            kind: PendingInputCommandKind::DispatchMouseEvent,
            completed: CompletedInputOperation::PageResidenceSuperseded,
        },
        &mut CommandDispatchContext::default(),
    )
    .await;
    assert!(matches!(completed.result, Ok(DevToolsCommandResult::Empty)));
    assert!(completed.protocol_events.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn completed_renderer_ack_wins_when_page_replacement_is_already_observable() {
    let mut ctx = TestContext::new();
    with_loaded_document(&mut ctx, "<body>origin</body>").await;

    let page_residence_token = ctx
        .conn
        .capture_target_page_residence_token_for_session(None)
        .expect("the original Page should expose its attachment lifetime");
    ctx.install_navigation_fixture_for_session_owner(
        "data:text/html,<body>replacement-after-ack</body>",
        None,
    )
    .await;

    let outcome = wait_for_renderer_input_or_page_replacement(
        std::future::ready("renderer-ack"),
        Some(page_residence_token),
    )
    .await;
    assert!(matches!(
        outcome,
        RendererInputWaitOutcome::Completed("renderer-ack")
    ));
}

#[test]
fn renderer_host_ack_cleanup_is_limited_to_mouse_and_key_callbacks() {
    for kind in [
        PendingInputCommandKind::DispatchMouseEvent,
        PendingInputCommandKind::DispatchKeyEvent,
    ] {
        assert!(kind.uses_renderer_host_ack_cleanup());
    }

    for kind in [
        PendingInputCommandKind::DispatchTouchEvent,
        PendingInputCommandKind::DispatchDragEvent,
        PendingInputCommandKind::SynthesizeTapGesture,
        PendingInputCommandKind::InsertText,
    ] {
        assert!(!kind.uses_renderer_host_ack_cleanup());
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn coordinate_mouse_event_without_document_still_reports_no_document_loaded() {
    let mut ctx = TestContext::new();
    let mut bc = BrowserContext::new("BID-I".into());
    bc.set_active_target_id("TID-1");
    ctx.conn.browser_context = Some(bc);

    ctx.process_async(json!({
        "id": 103,
        "method": "Input.dispatchMouseEvent",
        "params": {
            "type": "mouseReleased",
            "x": INPUT_HIT_X,
            "y": INPUT_HIT_Y,
            "button": "left",
            "buttons": 0,
            "clickCount": 1
        }
    }))
    .await;

    ctx.expect_error(103, -32000, "NoDocumentLoaded");
}

#[tokio::test(flavor = "multi_thread")]
async fn coordinate_mouse_down_applies_chromium_focus_default_action() {
    let mut ctx = TestContext::new();
    with_loaded_document(
        &mut ctx,
        r#"<html><body style='margin:0'>
                <textarea id='field' style='position:absolute;left:0;top:0;width:80px;height:80px'></textarea>
                <button id='before' style='position:absolute;left:100px;top:0;width:80px;height:80px'>before</button>
                <script>
                  window.__focusEvents = [];
                  const field = document.getElementById('field');
                  const before = document.getElementById('before');
                  before.focus();
                  field.addEventListener('mousedown', () => {
                    window.__focusEvents.push(`mousedown:${document.activeElement.id}`);
                  });
                  before.addEventListener('blur', () => window.__focusEvents.push('before-blur'));
                  field.addEventListener('focus', () => window.__focusEvents.push('field-focus'));
                </script>
               </body></html>"#,
    )
    .await;

    ctx.process_async(json!({
        "id": 31,
        "method": "Input.dispatchMouseEvent",
        "params": {
            "type": "mousePressed",
            "x": INPUT_HIT_X,
            "y": INPUT_HIT_Y,
            "button": "left",
            "buttons": 1,
            "clickCount": 1
        }
    }))
    .await;
    ctx.expect_result(31, json!({}), None);

    assert_eq!(
        evaluate_string(
            &mut ctx,
            "JSON.stringify({ active: document.activeElement.id, events: window.__focusEvents })",
        )
        .await,
        r#"{"active":"field","events":["mousedown:before","before-blur","field-focus"]}"#
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn canceled_coordinate_mouse_down_keeps_existing_focus() {
    let mut ctx = TestContext::new();
    with_loaded_document(
        &mut ctx,
        r#"<html><body style='margin:0'>
                <textarea id='field' style='position:absolute;left:0;top:0;width:80px;height:80px'></textarea>
                <button id='before' style='position:absolute;left:100px;top:0;width:80px;height:80px'>before</button>
                <script>
                  const field = document.getElementById('field');
                  document.getElementById('before').focus();
                  field.addEventListener('mousedown', event => event.preventDefault());
                </script>
               </body></html>"#,
    )
    .await;

    ctx.process_async(json!({
        "id": 32,
        "method": "Input.dispatchMouseEvent",
        "params": {
            "type": "mousePressed",
            "x": INPUT_HIT_X,
            "y": INPUT_HIT_Y,
            "button": "left",
            "buttons": 1,
            "clickCount": 1
        }
    }))
    .await;
    ctx.expect_result(32, json!({}), None);

    assert_eq!(
        evaluate_string(&mut ctx, "document.activeElement.id").await,
        "before"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn touch_tap_and_drag_commands_hit_test_real_layout() {
    let mut ctx = TestContext::new();
    with_loaded_document(
        &mut ctx,
        r#"<html><body style='margin:0'>
                <button id='btn' style='position:absolute;left:0;top:0;width:80px;height:80px'>go</button>
                <div id='drop' style='position:absolute;left:100px;top:0;width:80px;height:80px'></div>
                <script>
                  window.__events = [];
                  const btn = document.getElementById('btn');
                  const drop = document.getElementById('drop');
                  ['touchstart', 'touchend', 'click'].forEach((type) => {
                    btn.addEventListener(type, () => window.__events.push(type));
                  });
                  ['dragenter', 'dragover', 'drop'].forEach((type) => {
                    drop.addEventListener(type, () => window.__events.push(type));
                  });
                </script>
               </body></html>"#,
    )
    .await;

    let commands = [
        json!({
            "id": 4,
            "method": "Input.dispatchTouchEvent",
            "params": { "type": "touchStart", "touchPoints": [{ "x": INPUT_HIT_X, "y": INPUT_HIT_Y }] }
        }),
        json!({
            "id": 5,
            "method": "Input.dispatchTouchEvent",
            "params": { "type": "touchEnd", "touchPoints": [] }
        }),
        json!({
            "id": 6,
            "method": "Input.emulateTouchFromMouseEvent",
            "params": { "type": "mousePressed", "x": INPUT_HIT_X, "y": INPUT_HIT_Y, "button": "left" }
        }),
        json!({
            "id": 7,
            "method": "Input.emulateTouchFromMouseEvent",
            "params": { "type": "mouseReleased", "x": INPUT_HIT_X, "y": INPUT_HIT_Y, "button": "left" }
        }),
        json!({
            "id": 8,
            "method": "Input.synthesizeTapGesture",
            "params": { "x": INPUT_HIT_X, "y": INPUT_HIT_Y }
        }),
        json!({
            "id": 9,
            "method": "Input.dispatchDragEvent",
            "params": {
                "type": "dragEnter",
                "x": 120,
                "y": INPUT_HIT_Y,
                "data": {
                    "items": [{ "mimeType": "text/plain", "data": "drag-text" }],
                    "files": [],
                    "dragOperationsMask": 1
                }
            }
        }),
    ];

    for command in commands {
        let id = command["id"].as_u64().expect("command id");
        ctx.process_async(command).await;
        ctx.expect_result(id, json!({}), None);
    }

    assert_eq!(
        evaluate_string(&mut ctx, "JSON.stringify(window.__events)").await,
        r#"["touchstart","touchend","touchstart","touchend","touchstart","touchend","click","dragenter"]"#
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn coordinate_input_invalid_params_keep_session_id() {
    let mut ctx = TestContext::new();
    let mut bc = BrowserContext::new("BID-I".into());
    bc.set_active_target_id("TID-1");
    bc.attach_active_session("SID-1");
    ctx.conn.browser_context = Some(bc);

    for (id, method) in [
        (9001, "Input.dispatchMouseEvent"),
        (9002, "Input.dispatchTouchEvent"),
        (9003, "Input.emulateTouchFromMouseEvent"),
        (9004, "Input.synthesizeTapGesture"),
        (9005, "Input.dispatchDragEvent"),
    ] {
        ctx.process_async(json!({
            "id": id,
            "method": method,
            "sessionId": "SID-1"
        }))
        .await;
        assert_eq!(
            ctx.take_response_by_id(id),
            json!({
                "id": id,
                "sessionId": "SID-1",
                "error": { "code": -32602, "message": "InvalidParams" }
            })
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn coordinate_input_static_validation_precedes_unsupported_errors() {
    let mut ctx = TestContext::new();
    let invalid_commands = [
        json!({
            "id": 9008,
            "method": "Input.dispatchMouseEvent",
            "params": { "type": "mouseMoved", "x": 1, "y": 2, "force": 2 }
        }),
        json!({
            "id": 9009,
            "method": "Input.dispatchMouseEvent",
            "params": { "type": "mouseWheel", "x": 1, "y": 2 }
        }),
        json!({
            "id": 9010,
            "method": "Input.dispatchTouchEvent",
            "params": { "type": "touchStart", "touchPoints": [] }
        }),
        json!({
            "id": 9011,
            "method": "Input.dispatchTouchEvent",
            "params": { "type": "touchCancel", "touchPoints": [{ "x": 1, "y": 2 }] }
        }),
        json!({
            "id": 9012,
            "method": "Input.emulateTouchFromMouseEvent",
            "params": { "type": "mouseReleased", "x": 1, "y": 2 }
        }),
        json!({
            "id": 9013,
            "method": "Input.emulateTouchFromMouseEvent",
            "params": { "type": "mouseWheel", "x": 1, "y": 2, "button": "none" }
        }),
        json!({
            "id": 9014,
            "method": "Input.dispatchDragEvent",
            "params": {
                "type": "dragEnter",
                "x": 1,
                "y": 2,
                "data": { "dragOperationsMask": 1 }
            }
        }),
        json!({
            "id": 9015,
            "method": "Input.dispatchTouchEvent",
            "params": {
                "type": "touchStart",
                "touchPoints": [{ "id": 1, "x": 1, "y": 2 }, { "x": 3, "y": 4 }]
            }
        }),
        json!({
            "id": 9016,
            "method": "Input.dispatchTouchEvent",
            "params": { "type": "touchStart", "touchPoints": [{ "x": 1, "y": 2, "force": 2 }] }
        }),
        json!({
            "id": 9017,
            "method": "Input.dispatchMouseEvent",
            "params": { "type": "MouseMoved", "x": 1, "y": 2 }
        }),
        json!({
            "id": 9018,
            "method": "Input.dispatchTouchEvent",
            "params": { "type": "TouchStart", "touchPoints": [{ "x": 1, "y": 2 }] }
        }),
        json!({
            "id": 9019,
            "method": "Input.emulateTouchFromMouseEvent",
            "params": { "type": "MouseMoved", "x": 1, "y": 2, "button": "Left" }
        }),
        json!({
            "id": 9020,
            "method": "Input.synthesizeTapGesture",
            "params": { "x": 1, "y": 2, "gestureSourceType": "Touch" }
        }),
        json!({
            "id": 9021,
            "method": "Input.dispatchDragEvent",
            "params": {
                "type": "DragEnter",
                "x": 1,
                "y": 2,
                "data": { "items": [], "files": [], "dragOperationsMask": 0 }
            }
        }),
    ];

    for command in invalid_commands {
        let id = command["id"].as_u64().expect("command id");
        ctx.process_async(command).await;
        ctx.expect_error(id, -32602, "InvalidParams");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn mock_layout_policy_rejects_coordinate_input_without_dispatching() {
    let mut ctx = TestContext::new_with_layout_policy(LayoutPolicy::Mock);
    with_loaded_document(
        &mut ctx,
        r#"<button onclick="window.__clicked = true">go</button>"#,
    )
    .await;

    ctx.process_async(json!({
        "id": 9022,
        "method": "Input.dispatchMouseEvent",
        "params": {
            "type": "mousePressed",
            "x": INPUT_HIT_X,
            "y": INPUT_HIT_Y,
            "button": "left",
            "buttons": 1
        }
    }))
    .await;
    ctx.expect_error(9022, -32000, DISPATCH_MOUSE_EVENT_UNSUPPORTED_MESSAGE);
    assert_eq!(
        evaluate_string(&mut ctx, "String(window.__clicked)").await,
        "undefined"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn set_intercept_drags_fails_explicitly_without_drag_interception_events() {
    let mut ctx = TestContext::new();
    with_loaded_document(
        &mut ctx,
        r#"<html><body><div id='target' draggable='true'>drag</div></body></html>"#,
    )
    .await;

    ctx.process_async(json!({
        "id": 12,
        "method": "Input.setInterceptDrags",
        "params": { "enabled": true }
    }))
    .await;

    ctx.expect_error(12, -32000, SET_INTERCEPT_DRAGS_UNSUPPORTED_MESSAGE);
}

#[tokio::test(flavor = "multi_thread")]
async fn cancel_dragging_clears_reachable_drag_state_and_succeeds_when_idle() {
    let mut ctx = TestContext::new();
    with_loaded_document(&mut ctx, "<html><body>drag state</body></html>").await;
    ctx.conn
        .browser_context
        .as_mut()
        .expect("browser context")
        .input_drag_intercepted = true;

    ctx.process_async(json!({
        "id": 13,
        "method": "Input.cancelDragging"
    }))
    .await;
    ctx.expect_result(13, json!({}), None);
    assert!(
        !ctx.conn
            .browser_context
            .as_ref()
            .expect("browser context")
            .input_drag_intercepted
    );

    ctx.process_async(json!({
        "id": 14,
        "method": "Input.cancelDragging",
        "params": {}
    }))
    .await;
    ctx.expect_result(14, json!({}), None);
}

#[tokio::test(flavor = "multi_thread")]
async fn set_ignore_input_events_rejects_missing_or_non_boolean_ignore() {
    let mut ctx = TestContext::new();
    with_loaded_document(&mut ctx, "<html><body>ignore input</body></html>").await;

    for (id, params) in [
        (20, None),
        (21, Some(json!({}))),
        (22, Some(json!({ "ignore": 1 }))),
        (23, Some(json!({ "ignore": null }))),
    ] {
        let mut command = json!({
            "id": id,
            "method": "Input.setIgnoreInputEvents"
        });
        if let Some(params) = params {
            command["params"] = params;
        }
        ctx.process_async(command).await;
        ctx.expect_error(id, -32602, "InvalidParams");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn ignore_input_events_is_target_aggregated_and_does_not_block_insert_text() {
    let mut ctx = TestContext::new();
    with_loaded_document(
        &mut ctx,
        r#"<html><body>
                <input id='field' value=''>
                <script>
                  window.__inputEvents = [];
                  const field = document.getElementById('field');
                  field.addEventListener('keydown', event => window.__inputEvents.push(`keydown:${event.key}`));
                  field.addEventListener('input', () => window.__inputEvents.push(`input:${field.value}`));
                  field.focus();
                </script>
               </body></html>"#,
    )
    .await;
    let browser_context = ctx.conn.browser_context.as_mut().expect("browser context");
    browser_context.attach_active_session("SID-primary");
    assert!(browser_context.assign_auxiliary_session_to_target("TID-1", "SID-aux".to_owned()));

    ctx.process_async(json!({
        "id": 30,
        "method": "Input.setIgnoreInputEvents",
        "sessionId": "SID-aux",
        "params": { "ignore": true }
    }))
    .await;
    ctx.expect_result(30, json!({}), Some("SID-aux"));
    assert!(
        ctx.conn
            .target_page_session_state_for_session(Some("SID-aux"))
            .is_some_and(|state| state.input_events_ignored)
    );
    assert!(
        !ctx.conn
            .target_page_session_state_for_session(Some("SID-primary"))
            .is_some_and(|state| state.input_events_ignored)
    );

    ctx.process_async(json!({
        "id": 31,
        "method": "Input.dispatchKeyEvent",
        "sessionId": "SID-primary",
        "params": { "type": "keyDown", "key": "a", "code": "KeyA", "text": "a" }
    }))
    .await;
    ctx.expect_result(31, json!({}), Some("SID-primary"));
    assert_eq!(
        evaluate_string(
            &mut ctx,
            "field.value + '|' + JSON.stringify(window.__inputEvents)"
        )
        .await,
        "|[]"
    );

    ctx.process_async(json!({
        "id": 32,
        "method": "Input.dispatchKeyEvent",
        "sessionId": "SID-primary",
        "params": { "type": "not-a-key-event" }
    }))
    .await;
    assert_eq!(
        ctx.take_response_by_id(32),
        json!({
            "id": 32,
            "sessionId": "SID-primary",
            "error": { "code": -32602, "message": "InvalidParams" }
        }),
        "ignored input must still validate command parameters"
    );

    ctx.process_async(json!({
        "id": 33,
        "method": "Input.insertText",
        "sessionId": "SID-primary",
        "params": { "text": "b" }
    }))
    .await;
    ctx.expect_result(33, json!({}), Some("SID-primary"));
    assert_eq!(
        evaluate_string(
            &mut ctx,
            "field.value + '|' + JSON.stringify(window.__inputEvents)"
        )
        .await,
        "b|[\"input:b\"]",
        "Chromium does not apply setIgnoreInputEvents to Input.insertText"
    );

    ctx.process_async(json!({
        "id": 34,
        "method": "Input.setIgnoreInputEvents",
        "sessionId": "SID-primary",
        "params": { "ignore": false }
    }))
    .await;
    ctx.expect_result(34, json!({}), Some("SID-primary"));
    ctx.process_async(json!({
        "id": 35,
        "method": "Input.dispatchKeyEvent",
        "sessionId": "SID-primary",
        "params": { "type": "keyDown", "key": "d", "code": "KeyD", "text": "d" }
    }))
    .await;
    ctx.expect_result(35, json!({}), Some("SID-primary"));
    assert_eq!(evaluate_string(&mut ctx, "field.value").await, "b");

    ctx.process_async(json!({
        "id": 36,
        "method": "Input.setIgnoreInputEvents",
        "sessionId": "SID-aux",
        "params": { "ignore": false }
    }))
    .await;
    ctx.expect_result(36, json!({}), Some("SID-aux"));
    ctx.process_async(json!({
        "id": 37,
        "method": "Input.dispatchKeyEvent",
        "sessionId": "SID-primary",
        "params": { "type": "keyDown", "key": "e", "code": "KeyE", "text": "e" }
    }))
    .await;
    ctx.expect_result(37, json!({}), Some("SID-primary"));
    assert_eq!(evaluate_string(&mut ctx, "field.value").await, "be");

    ctx.process_async(json!({
        "id": 38,
        "method": "Input.setIgnoreInputEvents",
        "sessionId": "SID-aux",
        "params": { "ignore": true }
    }))
    .await;
    ctx.expect_result(38, json!({}), Some("SID-aux"));
    assert_eq!(
        ctx.conn
            .browser_context
            .as_mut()
            .expect("browser context")
            .remove_auxiliary_session("SID-aux"),
        Some("TID-1".to_owned())
    );
    ctx.process_async(json!({
        "id": 39,
        "method": "Input.dispatchKeyEvent",
        "sessionId": "SID-primary",
        "params": { "type": "keyDown", "key": "f", "code": "KeyF", "text": "f" }
    }))
    .await;
    ctx.expect_result(39, json!({}), Some("SID-primary"));
    assert_eq!(evaluate_string(&mut ctx, "field.value").await, "bef");
}

#[tokio::test(flavor = "multi_thread")]
async fn dispatch_key_event_char_inserts_text_into_active_control() {
    let mut ctx = TestContext::new();
    with_loaded_document(
        &mut ctx,
        r#"<html><body>
                <input id='field' value=''>
                <script>
                  window.__keyEvents = [];
                  const field = document.getElementById('field');
                  field.addEventListener('keypress', (event) => {
                    window.__keyEvents.push({
                      type: event.type,
                      key: event.key,
                      code: event.code,
                      shiftKey: event.shiftKey,
                    });
                  });
                  field.focus();
                </script>
               </body></html>"#,
    )
    .await;

    ctx.process_async(json!({
        "id": 5,
        "method": "Input.dispatchKeyEvent",
        "params": {
            "type": "char",
            "key": "A",
            "code": "KeyA",
            "modifiers": 8,
            "text": "A"
        }
    }))
    .await;

    ctx.expect_result(5, json!({}), None);
    assert_eq!(
        evaluate_string(&mut ctx, "document.getElementById('field').value").await,
        "A"
    );
    assert_eq!(
        evaluate_string(
            &mut ctx,
            "JSON.stringify({start: field.selectionStart, end: field.selectionEnd})"
                .replace("field", "document.getElementById('field')")
                .as_str()
        )
        .await,
        r#"{"start":1,"end":1}"#
    );
    assert_eq!(
        evaluate_string(&mut ctx, "JSON.stringify(window.__keyEvents)").await,
        r#"[{"type":"keypress","key":"A","code":"KeyA","shiftKey":true}]"#
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn dispatch_key_event_keydown_with_text_inserts_text_into_active_control() {
    let mut ctx = TestContext::new();
    with_loaded_document(
        &mut ctx,
        r#"<html><body>
                <input id='field' value=''>
                <script>
                  window.__keyEvents = [];
                  const field = document.getElementById('field');
                  field.addEventListener('keydown', (event) => {
                    window.__keyEvents.push({
                      type: event.type,
                      key: event.key,
                      code: event.code,
                      shiftKey: event.shiftKey,
                    });
                  });
                  field.focus();
                </script>
               </body></html>"#,
    )
    .await;

    ctx.process_async(json!({
        "id": 5,
        "method": "Input.dispatchKeyEvent",
        "params": {
            "type": "keyDown",
            "key": "A",
            "code": "KeyA",
            "modifiers": 8,
            "text": "A"
        }
    }))
    .await;

    ctx.expect_result(5, json!({}), None);
    assert_eq!(
        evaluate_string(&mut ctx, "document.getElementById('field').value").await,
        "A"
    );
    assert_eq!(
        evaluate_string(&mut ctx, "JSON.stringify(window.__keyEvents)").await,
        r#"[{"type":"keydown","key":"A","code":"KeyA","shiftKey":true}]"#
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn dispatch_key_event_text_control_change_commits_on_blur() {
    let mut ctx = TestContext::new();
    with_loaded_document(
        &mut ctx,
        r#"<html><body>
                <input id='field' value=''>
                <input id='other' value=''>
                <script>
                  window.__changeEvents = [];
                  const field = document.getElementById('field');
                  for (const type of ['input', 'change']) {
                    field.addEventListener(type, (event) => {
                      window.__changeEvents.push(`${event.type}:${event.composed}:${field.value}`);
                    });
                  }
                  field.focus();
                </script>
               </body></html>"#,
    )
    .await;

    ctx.process_async(json!({
        "id": 51,
        "method": "Input.dispatchKeyEvent",
        "params": {
            "type": "keyDown",
            "key": "a",
            "code": "KeyA",
            "text": "a"
        }
    }))
    .await;
    ctx.expect_result(51, json!({}), None);
    assert_eq!(
        evaluate_string(&mut ctx, "JSON.stringify(window.__changeEvents)").await,
        r#"["input:true:a"]"#
    );

    assert_eq!(
        evaluate_string(
            &mut ctx,
            "document.getElementById('other').focus(); JSON.stringify(window.__changeEvents)"
        )
        .await,
        r#"["input:true:a","change:false:a"]"#
    );

    assert_eq!(
        evaluate_string(
            &mut ctx,
            "window.__changeEvents = []; document.getElementById('field').focus(); document.getElementById('field').value = 'script'; document.getElementById('other').focus(); JSON.stringify(window.__changeEvents)"
        )
        .await,
        r#"[]"#
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn dispatch_key_event_autorepeat_sets_keyboard_event_repeat() {
    let mut ctx = TestContext::new();
    with_loaded_document(
        &mut ctx,
        r#"<html><body>
                <input id='field' value=''>
                <script>
                  window.__keyEvents = [];
                  const field = document.getElementById('field');
                  field.addEventListener('keydown', (event) => {
                    window.__keyEvents.push({
                      key: event.key,
                      repeat: event.repeat,
                    });
                  });
                  field.focus();
                </script>
               </body></html>"#,
    )
    .await;

    ctx.process_async(json!({
        "id": 6,
        "method": "Input.dispatchKeyEvent",
        "params": {
            "type": "keyDown",
            "key": "a",
            "code": "KeyA",
            "autoRepeat": true,
            "text": "a"
        }
    }))
    .await;

    ctx.expect_result(6, json!({}), None);
    assert_eq!(
        evaluate_string(&mut ctx, "JSON.stringify(window.__keyEvents)").await,
        r#"[{"key":"a","repeat":true}]"#
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn dispatch_key_event_populates_legacy_keyboard_event_codes() {
    let mut ctx = TestContext::new();
    with_loaded_document(
        &mut ctx,
        r#"<html><body>
                <input id='field' value=''>
                <script>
                  window.__keyEvents = [];
                  const field = document.getElementById('field');
                  for (const type of ['keydown', 'keypress', 'keyup']) {
                    field.addEventListener(type, (event) => {
                      window.__keyEvents.push({
                        type: event.type,
                        key: event.key,
                        code: event.code,
                        keyCode: event.keyCode,
                        which: event.which,
                        charCode: event.charCode,
                      });
                    });
                  }
                  field.focus();
                </script>
               </body></html>"#,
    )
    .await;

    ctx.process_async(json!({
        "id": 7,
        "method": "Input.dispatchKeyEvent",
        "params": {
            "type": "keyDown",
            "key": "ArrowDown",
            "code": "ArrowDown"
        }
    }))
    .await;
    ctx.expect_result(7, json!({}), None);

    ctx.process_async(json!({
        "id": 8,
        "method": "Input.dispatchKeyEvent",
        "params": {
            "type": "keyUp",
            "key": "ArrowDown",
            "code": "ArrowDown"
        }
    }))
    .await;
    ctx.expect_result(8, json!({}), None);

    ctx.process_async(json!({
        "id": 9,
        "method": "Input.dispatchKeyEvent",
        "params": {
            "type": "char",
            "key": "a",
            "code": "KeyA",
            "text": "a"
        }
    }))
    .await;
    ctx.expect_result(9, json!({}), None);

    assert_eq!(
        evaluate_string(&mut ctx, "JSON.stringify(window.__keyEvents)").await,
        r#"[{"type":"keydown","key":"ArrowDown","code":"ArrowDown","keyCode":40,"which":40,"charCode":0},{"type":"keyup","key":"ArrowDown","code":"ArrowDown","keyCode":40,"which":40,"charCode":0},{"type":"keypress","key":"a","code":"KeyA","keyCode":97,"which":97,"charCode":97}]"#
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn dispatch_key_event_keydown_enter_text_normalizes_textarea_newline() {
    let mut ctx = TestContext::new();
    with_loaded_document(
        &mut ctx,
        r#"<html><body>
                <textarea id='field'>one</textarea>
                <script>
                  const field = document.getElementById('field');
                  field.focus();
                  field.setSelectionRange(3, 3);
                </script>
               </body></html>"#,
    )
    .await;

    ctx.process_async(json!({
        "id": 5,
        "method": "Input.dispatchKeyEvent",
        "params": {
            "type": "keyDown",
            "key": "Enter",
            "code": "Enter",
            "text": "\r"
        }
    }))
    .await;

    ctx.expect_result(5, json!({}), None);
    assert_eq!(
        evaluate_string(&mut ctx, "document.getElementById('field').value").await,
        "one\n"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn insert_text_updates_selection_and_emits_input() {
    let mut ctx = TestContext::new();
    with_loaded_document(
        &mut ctx,
        r#"<html><body>
                <input id='field' value='xy'>
                <script>
                  window.__inputEvents = [];
                  const field = document.getElementById('field');
                  field.addEventListener('input', (event) => window.__inputEvents.push(field.value + ':' + event.composed));
                  field.focus();
                  field.setSelectionRange(1, 2);
                </script>
               </body></html>"#,
    )
    .await;

    ctx.process_async(json!({
        "id": 4,
        "method": "Input.insertText",
        "params": { "text": "Z" }
    }))
    .await;

    ctx.expect_result(4, json!({}), None);
    assert_eq!(
        evaluate_string(&mut ctx, "document.getElementById('field').value").await,
        "xZ"
    );
    assert_eq!(
        evaluate_string(
            &mut ctx,
            "JSON.stringify({start: document.getElementById('field').selectionStart, end: document.getElementById('field').selectionEnd})"
        ).await,
        r#"{"start":2,"end":2}"#
    );
    assert_eq!(
        evaluate_string(&mut ctx, "JSON.stringify(window.__inputEvents)").await,
        r#"["xZ:true"]"#
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn insert_text_updates_focused_contenteditable() {
    let mut ctx = TestContext::new();
    with_loaded_document(
        &mut ctx,
        r#"<html><body>
                <div id='editor' contenteditable='true'>xy</div>
                <script>
                  window.__inputEvents = [];
                  const editor = document.getElementById('editor');
                  editor.addEventListener('beforeinput', (event) => window.__inputEvents.push('beforeinput:' + event.composed + ':' + editor.textContent));
                  editor.addEventListener('input', (event) => window.__inputEvents.push('input:' + event.composed + ':' + editor.textContent));
                  editor.focus();
                  const range = document.createRange();
                  range.setStart(editor.firstChild, 1);
                  range.setEnd(editor.firstChild, 2);
                  const selection = window.getSelection();
                  selection.removeAllRanges();
                  selection.addRange(range);
                </script>
               </body></html>"#,
    )
    .await;

    ctx.process_async(json!({
        "id": 44,
        "method": "Input.insertText",
        "params": { "text": "Z" }
    }))
    .await;

    ctx.expect_result(44, json!({}), None);
    assert_eq!(
        evaluate_string(&mut ctx, "document.getElementById('editor').textContent").await,
        "xZ"
    );
    assert_eq!(
        evaluate_string(&mut ctx, "JSON.stringify(window.__inputEvents)").await,
        r#"["beforeinput:true:xy","input:true:xZ"]"#
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn dispatch_key_event_keydown_with_text_inserts_text_into_contenteditable() {
    let mut ctx = TestContext::new();
    with_loaded_document(
        &mut ctx,
        r#"<html><body>
                <div id='editor' contenteditable='true'></div>
                <script>
                  window.__keyEvents = [];
                  const editor = document.getElementById('editor');
                  editor.addEventListener('keydown', (event) => {
                    window.__keyEvents.push({
                      type: event.type,
                      key: event.key,
                      code: event.code,
                      shiftKey: event.shiftKey,
                    });
                  });
                  editor.focus();
                </script>
               </body></html>"#,
    )
    .await;

    ctx.process_async(json!({
        "id": 45,
        "method": "Input.dispatchKeyEvent",
        "params": {
            "type": "keyDown",
            "key": "A",
            "code": "KeyA",
            "modifiers": 8,
            "text": "A"
        }
    }))
    .await;

    ctx.expect_result(45, json!({}), None);
    assert_eq!(
        evaluate_string(&mut ctx, "document.getElementById('editor').textContent").await,
        "A"
    );
    assert_eq!(
        evaluate_string(&mut ctx, "JSON.stringify(window.__keyEvents)").await,
        r#"[{"type":"keydown","key":"A","code":"KeyA","shiftKey":true}]"#
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn dispatch_key_event_selects_and_replaces_contenteditable_text() {
    let mut ctx = TestContext::new();
    with_loaded_document(
        &mut ctx,
        r#"<html><body>
                <div id='editor' contenteditable='true'>old</div>
                <script>document.getElementById('editor').focus();</script>
               </body></html>"#,
    )
    .await;

    ctx.process_async(json!({
        "id": 451,
        "method": "Input.dispatchKeyEvent",
        "params": {
            "type": "keyDown",
            "key": "a",
            "code": "KeyA",
            "modifiers": 2
        }
    }))
    .await;
    ctx.expect_result(451, json!({}), None);

    ctx.process_async(json!({
        "id": 452,
        "method": "Input.dispatchKeyEvent",
        "params": {
            "type": "keyDown",
            "key": "N",
            "code": "KeyN",
            "text": "N"
        }
    }))
    .await;
    ctx.expect_result(452, json!({}), None);

    assert_eq!(
        evaluate_string(&mut ctx, "document.getElementById('editor').textContent").await,
        "N"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn coordinate_mouse_event_completes_through_pending_layout_dispatch() {
    let mut ctx = TestContext::new();
    with_loaded_document(
        &mut ctx,
        r#"<body style='margin:0'><button style='width:80px;height:80px' onclick="window.__clicked = true">go</button></body>"#,
    )
    .await;

    for (id, event_type, buttons) in [(4001, "mousePressed", 1), (4002, "mouseReleased", 0)] {
        let raw = json!({
            "id": id,
            "method": "Input.dispatchMouseEvent",
            "params": {
                "type": event_type,
                "x": INPUT_HIT_X,
                "y": INPUT_HIT_Y,
                "button": "left",
                "buttons": buttons
            }
        })
        .to_string();
        let pending = match ctx.conn.start_command_dispatch(&raw) {
            CdpCommandTaskStep::Pending(pending) => pending,
            CdpCommandTaskStep::Complete(_) => {
                panic!("coordinate mouse dispatch should wait for renderer layout")
            }
        };
        let outcome = ctx
            .conn
            .complete_pending_command_dispatch(pending.wait().await)
            .await;
        let (messages, scheduler_events) = ctx.complete_command_task_step_for_test(outcome).await;

        assert!(
            scheduler_events.is_empty(),
            "plain mouse dispatch should not enqueue scheduler work"
        );
        assert_eq!(messages, vec![json!({ "id": id, "result": {} })]);
    }

    assert_eq!(
        evaluate_string(&mut ctx, "String(window.__clicked)").await,
        "true"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn dispatch_key_event_can_complete_through_pending_command_dispatch() {
    let mut ctx = TestContext::new();
    with_loaded_document(
        &mut ctx,
        r#"<html><body>
                <input id='field' value=''>
                <script>document.getElementById('field').focus();</script>
               </body></html>"#,
    )
    .await;

    let raw = json!({
        "id": 4101,
        "method": "Input.dispatchKeyEvent",
        "params": {
            "type": "char",
            "key": "B",
            "code": "KeyB",
            "text": "B"
        }
    })
    .to_string();
    let pending = ctx
        .conn
        .try_start_pending_command_dispatch(&raw)
        .expect("Input.dispatchKeyEvent should start as a pending command");
    let outcome = ctx
        .conn
        .complete_pending_command_dispatch(pending.wait().await)
        .await;
    let (messages, scheduler_events) = ctx.complete_command_task_step_for_test(outcome).await;

    assert!(
        scheduler_events.is_empty(),
        "plain key dispatch should not enqueue scheduler work"
    );
    assert_eq!(messages, vec![json!({"id": 4101, "result": {}})]);
    assert_eq!(
        evaluate_string(&mut ctx, "document.getElementById('field').value").await,
        "B"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn insert_text_can_complete_through_pending_command_dispatch() {
    let mut ctx = TestContext::new();
    with_loaded_document(
        &mut ctx,
        r#"<html><body>
                <input id='field' value='xy'>
                <script>
                  const field = document.getElementById('field');
                  field.focus();
                  field.setSelectionRange(1, 2);
                </script>
               </body></html>"#,
    )
    .await;

    let raw = json!({
        "id": 4102,
        "method": "Input.insertText",
        "params": { "text": "Z" }
    })
    .to_string();
    let pending = ctx
        .conn
        .try_start_pending_command_dispatch(&raw)
        .expect("Input.insertText should start as a pending command");
    let outcome = ctx
        .conn
        .complete_pending_command_dispatch(pending.wait().await)
        .await;
    let (messages, scheduler_events) = ctx.complete_command_task_step_for_test(outcome).await;

    assert!(
        scheduler_events.is_empty(),
        "plain insertText should not enqueue scheduler work"
    );
    assert_eq!(messages, vec![json!({"id": 4102, "result": {}})]);
    assert_eq!(
        evaluate_string(&mut ctx, "document.getElementById('field').value").await,
        "xZ"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn coordinate_touch_commands_complete_through_pending_layout_dispatch() {
    let mut ctx = TestContext::new();
    with_loaded_document(
        &mut ctx,
        r#"<html><body style='margin:0'>
                <button id='btn' style='width:80px;height:80px'>tap</button>
                <script>
                  window.__touchPending = [];
                  const btn = document.getElementById('btn');
                  ['touchstart', 'touchend', 'click'].forEach((type) => {
                    btn.addEventListener(type, () => window.__touchPending.push(type));
                  });
                </script>
               </body></html>"#,
    )
    .await;

    for (id, event_type, touch_points) in [
        (
            4103,
            "touchStart",
            json!([{ "x": INPUT_HIT_X, "y": INPUT_HIT_Y }]),
        ),
        (4104, "touchEnd", json!([])),
    ] {
        let raw = json!({
            "id": id,
            "method": "Input.dispatchTouchEvent",
            "params": {
                "type": event_type,
                "touchPoints": touch_points
            }
        })
        .to_string();
        let pending = match ctx.conn.start_command_dispatch(&raw) {
            CdpCommandTaskStep::Pending(pending) => pending,
            CdpCommandTaskStep::Complete(_) => {
                panic!("coordinate touch dispatch should wait for renderer layout")
            }
        };
        let outcome = ctx
            .conn
            .complete_pending_command_dispatch(pending.wait().await)
            .await;
        let (messages, scheduler_events) = ctx.complete_command_task_step_for_test(outcome).await;

        assert!(
            scheduler_events.is_empty(),
            "plain touch dispatch should not enqueue scheduler work"
        );
        assert_eq!(messages, vec![json!({ "id": id, "result": {} })]);
    }

    for (id, event_type) in [(4105, "mousePressed"), (4106, "mouseReleased")] {
        ctx.process_async(json!({
            "id": id,
            "method": "Input.emulateTouchFromMouseEvent",
            "params": {
                "type": event_type,
                "x": INPUT_HIT_X,
                "y": INPUT_HIT_Y,
                "button": "left"
            }
        }))
        .await;
        ctx.expect_result(id, json!({}), None);
    }

    ctx.conn
        .browser_context
        .as_mut()
        .expect("loaded browser context")
        .emit_touch_events_for_mouse = true;
    for (id, event_type, buttons) in [(4107, "mousePressed", 1), (4108, "mouseReleased", 0)] {
        ctx.process_async(json!({
            "id": id,
            "method": "Input.dispatchMouseEvent",
            "params": {
                "type": event_type,
                "x": INPUT_HIT_X,
                "y": INPUT_HIT_Y,
                "button": "left",
                "buttons": buttons
            }
        }))
        .await;
        ctx.expect_result(id, json!({}), None);
    }

    assert_eq!(
        evaluate_string(&mut ctx, "JSON.stringify(window.__touchPending)").await,
        r#"["touchstart","touchend","touchstart","touchend","touchstart","touchend"]"#
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn coordinate_drag_event_completes_through_pending_layout_dispatch() {
    let mut ctx = TestContext::new();
    with_loaded_document(
        &mut ctx,
        r#"<html><body>
                <div id='target' style='position:absolute;left:0;top:0;width:200px;height:200px'></div>
                <script>
                  window.__dragPending = [];
                  document.getElementById('target').addEventListener('dragenter', event => {
                    window.__dragPending.push([
                      event.type,
                      event instanceof DragEvent,
                      event.dataTransfer.getData('text/plain')
                    ].join('|'));
                  });
                </script>
               </body></html>"#,
    )
    .await;

    let raw = json!({
        "id": 4107,
        "method": "Input.dispatchDragEvent",
        "params": {
            "type": "dragEnter",
            "x": INPUT_HIT_X,
            "y": INPUT_HIT_Y,
            "data": {
                "items": [{ "mimeType": "text/plain", "data": "drag-pending" }],
                "files": [],
                "dragOperationsMask": 1
            }
        }
    })
    .to_string();
    let pending = match ctx.conn.start_command_dispatch(&raw) {
        CdpCommandTaskStep::Pending(pending) => pending,
        CdpCommandTaskStep::Complete(_) => {
            panic!("coordinate drag dispatch should wait for renderer layout")
        }
    };
    let outcome = ctx
        .conn
        .complete_pending_command_dispatch(pending.wait().await)
        .await;
    let (messages, scheduler_events) = ctx.complete_command_task_step_for_test(outcome).await;
    assert!(
        scheduler_events.is_empty(),
        "drag event dispatch should not enqueue scheduler work"
    );
    assert_eq!(messages, vec![json!({ "id": 4107, "result": {} })]);
    assert_eq!(
        evaluate_string(&mut ctx, "JSON.stringify(window.__dragPending)").await,
        r#"["dragenter|true|drag-pending"]"#
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn insert_text_marks_text_controls_user_edited_for_length_validity() {
    let mut ctx = TestContext::new();
    with_loaded_document(
        &mut ctx,
        r#"<html><body>
                <form id='form'>
                  <input id='field' minlength='3' maxlength='4' value='seed'>
                  <textarea id='bio' minlength='2' maxlength='3'>seed</textarea>
                  <input id='emoji' maxlength='1'>
                </form>
                <script>
                  const field = document.getElementById('field');
                  const bio = document.getElementById('bio');
                  field.value = 'a';
                  bio.value = 'a';
                  window.__scriptSetLengthValidity = {
                    inputTooShort: field.validity.tooShort,
                    inputValid: field.validity.valid,
                    textareaTooShort: bio.validity.tooShort,
                    textareaValid: bio.validity.valid,
                  };
                </script>
               </body></html>"#,
    )
    .await;

    assert_eq!(
        evaluate_string(&mut ctx, "JSON.stringify(window.__scriptSetLengthValidity)").await,
        r#"{"inputTooShort":false,"inputValid":true,"textareaTooShort":false,"textareaValid":true}"#
    );

    evaluate_string(
        &mut ctx,
        "(() => { const field = document.getElementById('field'); field.focus(); field.setSelectionRange(0, field.value.length); return 'ready'; })()",
    )
    .await;
    ctx.process_async(json!({
        "id": 40,
        "method": "Input.insertText",
        "params": { "text": "ab" }
    }))
    .await;
    ctx.expect_result(40, json!({}), None);
    assert_eq!(
        evaluate_string(
            &mut ctx,
            "JSON.stringify({value: field.value, tooShort: field.validity.tooShort, valid: field.validity.valid})"
        )
        .await,
        r#"{"value":"ab","tooShort":true,"valid":false}"#
    );

    evaluate_string(
        &mut ctx,
        "field.setSelectionRange(0, field.value.length); 'ready'",
    )
    .await;
    ctx.process_async(json!({
        "id": 41,
        "method": "Input.insertText",
        "params": { "text": "abcde" }
    }))
    .await;
    ctx.expect_result(41, json!({}), None);
    assert_eq!(
        evaluate_string(
            &mut ctx,
            "JSON.stringify({value: field.value, tooLong: field.validity.tooLong, valid: field.validity.valid})"
        )
        .await,
        r#"{"value":"abcde","tooLong":true,"valid":false}"#
    );

    assert_eq!(
        evaluate_string(
            &mut ctx,
            "field.value = 'abcdef'; JSON.stringify({tooLong: field.validity.tooLong, valid: field.validity.valid})"
        )
        .await,
        r#"{"tooLong":false,"valid":true}"#
    );

    evaluate_string(
        &mut ctx,
        "(() => { const bio = document.getElementById('bio'); bio.focus(); bio.setSelectionRange(0, bio.value.length); return 'ready'; })()",
    )
    .await;
    ctx.process_async(json!({
        "id": 42,
        "method": "Input.insertText",
        "params": { "text": "abcd" }
    }))
    .await;
    ctx.expect_result(42, json!({}), None);
    assert_eq!(
        evaluate_string(
            &mut ctx,
            "JSON.stringify({value: bio.value, tooLong: bio.validity.tooLong, valid: bio.validity.valid})"
        )
        .await,
        r#"{"value":"abcd","tooLong":true,"valid":false}"#
    );

    evaluate_string(
        &mut ctx,
        "(() => { const emoji = document.getElementById('emoji'); emoji.focus(); return 'ready'; })()",
    )
    .await;
    ctx.process_async(json!({
        "id": 43,
        "method": "Input.insertText",
        "params": { "text": "😀" }
    }))
    .await;
    ctx.expect_result(43, json!({}), None);
    assert_eq!(
        evaluate_string(
            &mut ctx,
            "JSON.stringify({value: emoji.value, tooLong: emoji.validity.tooLong, valid: emoji.validity.valid})"
        )
        .await,
        r#"{"value":"😀","tooLong":true,"valid":false}"#
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn dispatch_key_event_space_toggles_checkbox() {
    let mut ctx = TestContext::new();
    with_loaded_document(
        &mut ctx,
        r#"<html><body>
                <input id='box' type='checkbox'>
                <script>
                  const box = document.getElementById('box');
                  box.focus();
                  window.__spaceEvents = [];
                  ['keydown', 'keyup', 'click', 'input', 'change'].forEach((type) => {
                    box.addEventListener(type, () => window.__spaceEvents.push(type));
                  });
                </script>
               </body></html>"#,
    )
    .await;

    ctx.process_async(json!({
        "id": 15,
        "method": "Input.dispatchKeyEvent",
        "params": {
            "type": "keyDown",
            "key": " ",
            "code": "Space"
        }
    }))
    .await;
    ctx.expect_result(15, json!({}), None);

    assert!(!evaluate_bool(&mut ctx, "document.getElementById('box').checked").await);
    assert_eq!(
        evaluate_string(&mut ctx, "JSON.stringify(window.__spaceEvents)").await,
        r#"["keydown"]"#
    );

    ctx.process_async(json!({
        "id": 150,
        "method": "Input.dispatchKeyEvent",
        "params": {
            "type": "keyUp",
            "key": " ",
            "code": "Space"
        }
    }))
    .await;
    ctx.expect_result(150, json!({}), None);

    assert!(evaluate_bool(&mut ctx, "document.getElementById('box').checked").await);
    assert_eq!(
        evaluate_string(&mut ctx, "JSON.stringify(window.__spaceEvents)").await,
        r#"["keydown","keyup","click","input","change"]"#
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn dispatch_key_event_raw_key_down_space_defers_checkbox_toggle_until_keyup() {
    let mut ctx = TestContext::new();
    with_loaded_document(
        &mut ctx,
        r#"<html><body>
                <input id='box' type='checkbox'>
                <script>
                  const box = document.getElementById('box');
                  box.focus();
                  window.__rawSpaceEvents = [];
                  ['keydown', 'keyup', 'click', 'input', 'change'].forEach((type) => {
                    box.addEventListener(type, () => window.__rawSpaceEvents.push(type));
                  });
                </script>
               </body></html>"#,
    )
    .await;

    ctx.process_async(json!({
        "id": 153,
        "method": "Input.dispatchKeyEvent",
        "params": {
            "type": "rawKeyDown",
            "key": " ",
            "code": "Space"
        }
    }))
    .await;
    ctx.expect_result(153, json!({}), None);

    assert!(!evaluate_bool(&mut ctx, "document.getElementById('box').checked").await);
    assert_eq!(
        evaluate_string(&mut ctx, "JSON.stringify(window.__rawSpaceEvents)").await,
        r#"["keydown"]"#
    );

    ctx.process_async(json!({
        "id": 154,
        "method": "Input.dispatchKeyEvent",
        "params": {
            "type": "keyUp",
            "key": " ",
            "code": "Space"
        }
    }))
    .await;
    ctx.expect_result(154, json!({}), None);

    assert!(evaluate_bool(&mut ctx, "document.getElementById('box').checked").await);
    assert_eq!(
        evaluate_string(&mut ctx, "JSON.stringify(window.__rawSpaceEvents)").await,
        r#"["keydown","keyup","click","input","change"]"#
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn dispatch_key_event_normalizes_rod_enter_control_character_like_chromium() {
    let mut ctx = TestContext::new();
    with_loaded_document(
        &mut ctx,
        r#"<html><body>
                <input id='field'>
                <script>
                  const field = document.getElementById('field');
                  field.focus();
                  window.__keyEvents = [];
                  for (const type of ['keydown', 'keyup']) {
                    field.addEventListener(type, (event) => {
                      window.__keyEvents.push({
                        type: event.type,
                        key: event.key,
                        code: event.code,
                        keyCode: event.keyCode,
                      });
                    });
                  }
                </script>
               </body></html>"#,
    )
    .await;

    for (id, event_type) in [(155, "rawKeyDown"), (156, "keyUp")] {
        ctx.process_async(json!({
            "id": id,
            "method": "Input.dispatchKeyEvent",
            "params": {
                "type": event_type,
                "key": "\r",
                "code": "Enter",
                "text": "",
                "unmodifiedText": "",
                "windowsVirtualKeyCode": 13,
                "location": 0,
                "isKeypad": false,
                "commands": []
            }
        }))
        .await;
        ctx.expect_result(id, json!({}), None);
    }

    assert_eq!(
        evaluate_string(&mut ctx, "JSON.stringify(window.__keyEvents)").await,
        r#"[{"type":"keydown","key":"Enter","code":"Enter","keyCode":13},{"type":"keyup","key":"Enter","code":"Enter","keyCode":13}]"#
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn dispatch_key_event_space_selects_radio_on_keyup() {
    let mut ctx = TestContext::new();
    with_loaded_document(
        &mut ctx,
        r#"<html><body>
                <input id='one' type='radio' name='group'>
                <input id='two' type='radio' name='group'>
                <script>
                  const one = document.getElementById('one');
                  one.focus();
                  window.__radioSpaceEvents = [];
                  ['keydown', 'keyup', 'click', 'input', 'change'].forEach((type) => {
                    one.addEventListener(type, () => window.__radioSpaceEvents.push(type));
                  });
                </script>
               </body></html>"#,
    )
    .await;

    ctx.process_async(json!({
        "id": 151,
        "method": "Input.dispatchKeyEvent",
        "params": {
            "type": "keyDown",
            "key": " ",
            "code": "Space"
        }
    }))
    .await;
    ctx.expect_result(151, json!({}), None);

    ctx.process_async(json!({
        "id": 152,
        "method": "Input.dispatchKeyEvent",
        "params": {
            "type": "keyUp",
            "key": " ",
            "code": "Space"
        }
    }))
    .await;
    ctx.expect_result(152, json!({}), None);

    assert!(evaluate_bool(&mut ctx, "document.getElementById('one').checked").await);
    assert_eq!(
        evaluate_string(&mut ctx, "JSON.stringify(window.__radioSpaceEvents)").await,
        r#"["keydown","keyup","click","input","change"]"#
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn dispatch_key_event_enter_inserts_newline_in_textarea() {
    let mut ctx = TestContext::new();
    with_loaded_document(
        &mut ctx,
        r#"<html><body>
                <textarea id='field'>ab</textarea>
                <script>
                  const field = document.getElementById('field');
                  field.focus();
                  field.setSelectionRange(1, 1);
                  window.__textareaEvents = [];
                  ['keydown', 'beforeinput', 'input'].forEach((type) => {
                    field.addEventListener(type, () => window.__textareaEvents.push(type));
                  });
                </script>
               </body></html>"#,
    )
    .await;

    ctx.process_async(json!({
        "id": 16,
        "method": "Input.dispatchKeyEvent",
        "params": {
            "type": "keyDown",
            "key": "Enter",
            "code": "Enter"
        }
    }))
    .await;
    ctx.expect_result(16, json!({}), None);

    assert_eq!(
        evaluate_string(&mut ctx, "document.getElementById('field').value").await,
        "a\nb"
    );
    assert_eq!(
        evaluate_string(
            &mut ctx,
            "JSON.stringify({start: document.getElementById('field').selectionStart, end: document.getElementById('field').selectionEnd})"
        ).await,
        r#"{"start":2,"end":2}"#
    );
    assert_eq!(
        evaluate_string(&mut ctx, "JSON.stringify(window.__textareaEvents)").await,
        r#"["keydown","beforeinput","input"]"#
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn dispatch_key_event_space_clicks_button_only_on_keyup() {
    let mut ctx = TestContext::new();
    with_loaded_document(
        &mut ctx,
        r#"<html><body>
                <button id='btn'>go</button>
                <script>
                  window.__buttonEvents = [];
                  const btn = document.getElementById('btn');
                  btn.focus();
                  btn.addEventListener('keydown', () => window.__buttonEvents.push('keydown'));
                  btn.addEventListener('keyup', () => window.__buttonEvents.push('keyup'));
                  btn.addEventListener('click', () => window.__buttonEvents.push('click'));
                </script>
               </body></html>"#,
    )
    .await;

    ctx.process_async(json!({
        "id": 30,
        "method": "Input.dispatchKeyEvent",
        "params": {
            "type": "keyDown",
            "key": " ",
            "code": "Space"
        }
    }))
    .await;
    ctx.expect_result(30, json!({}), None);
    assert_eq!(
        evaluate_string(&mut ctx, "JSON.stringify(window.__buttonEvents)").await,
        r#"["keydown"]"#
    );

    ctx.process_async(json!({
        "id": 31,
        "method": "Input.dispatchKeyEvent",
        "params": {
            "type": "keyUp",
            "key": " ",
            "code": "Space"
        }
    }))
    .await;
    ctx.expect_result(31, json!({}), None);
    assert_eq!(
        evaluate_string(&mut ctx, "JSON.stringify(window.__buttonEvents)").await,
        r#"["keydown","keyup","click"]"#
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn dispatch_key_event_raw_key_down_space_clicks_button_only_after_keyup() {
    let mut ctx = TestContext::new();
    with_loaded_document(
        &mut ctx,
        r#"<html><body>
                <button id='btn'>go</button>
                <script>
                  window.__rawButtonEvents = [];
                  const btn = document.getElementById('btn');
                  btn.focus();
                  btn.addEventListener('keydown', () => window.__rawButtonEvents.push('keydown'));
                  btn.addEventListener('keyup', () => window.__rawButtonEvents.push('keyup'));
                  btn.addEventListener('click', () => window.__rawButtonEvents.push('click'));
                </script>
               </body></html>"#,
    )
    .await;

    ctx.process_async(json!({
        "id": 311,
        "method": "Input.dispatchKeyEvent",
        "params": {
            "type": "rawKeyDown",
            "key": " ",
            "code": "Space"
        }
    }))
    .await;
    ctx.expect_result(311, json!({}), None);
    assert_eq!(
        evaluate_string(&mut ctx, "JSON.stringify(window.__rawButtonEvents)").await,
        r#"["keydown"]"#
    );

    ctx.process_async(json!({
        "id": 312,
        "method": "Input.dispatchKeyEvent",
        "params": {
            "type": "keyUp",
            "key": " ",
            "code": "Space"
        }
    }))
    .await;
    ctx.expect_result(312, json!({}), None);
    assert_eq!(
        evaluate_string(&mut ctx, "JSON.stringify(window.__rawButtonEvents)").await,
        r#"["keydown","keyup","click"]"#
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn dispatch_key_event_keyup_enter_does_not_click_button_without_keydown() {
    let mut ctx = TestContext::new();
    with_loaded_document(
        &mut ctx,
        r#"<html><body>
                <button id='btn'>go</button>
                <script>
                  window.__enterKeyupOnlyEvents = [];
                  const btn = document.getElementById('btn');
                  btn.focus();
                  btn.addEventListener('keydown', () => window.__enterKeyupOnlyEvents.push('keydown'));
                  btn.addEventListener('keyup', () => window.__enterKeyupOnlyEvents.push('keyup'));
                  btn.addEventListener('click', () => window.__enterKeyupOnlyEvents.push('click'));
                </script>
               </body></html>"#,
    ).await;

    ctx.process_async(json!({
        "id": 313,
        "method": "Input.dispatchKeyEvent",
        "params": {
            "type": "keyUp",
            "key": "Enter",
            "code": "Enter"
        }
    }))
    .await;
    ctx.expect_result(313, json!({}), None);
    assert_eq!(
        evaluate_string(&mut ctx, "JSON.stringify(window.__enterKeyupOnlyEvents)").await,
        r#"["keyup"]"#
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn dispatch_key_event_enter_clicks_link_only_on_keydown() {
    let mut ctx = TestContext::new();
    with_loaded_document(
        &mut ctx,
        r#"<html><body>
                <a id='link' href='#done'>go</a>
                <script>
                  window.__linkEvents = [];
                  window.__linkClicks = 0;
                  const link = document.getElementById('link');
                  link.focus();
                  link.addEventListener('keydown', () => window.__linkEvents.push('keydown'));
                  link.addEventListener('keyup', () => window.__linkEvents.push('keyup'));
                  link.addEventListener('click', (event) => {
                    event.preventDefault();
                    window.__linkClicks += 1;
                    window.__linkEvents.push('click');
                  });
                </script>
               </body></html>"#,
    )
    .await;

    ctx.process_async(json!({
        "id": 31,
        "method": "Input.dispatchKeyEvent",
        "params": {
            "type": "keyDown",
            "key": "Enter",
            "code": "Enter"
        }
    }))
    .await;
    ctx.expect_result(31, json!({}), None);
    assert_eq!(
        evaluate_string(&mut ctx, "JSON.stringify(window.__linkEvents)").await,
        r#"["keydown","click"]"#
    );
    assert_eq!(
        evaluate_string(&mut ctx, "String(window.__linkClicks)").await,
        "1"
    );

    ctx.process_async(json!({
        "id": 32,
        "method": "Input.dispatchKeyEvent",
        "params": {
            "type": "keyUp",
            "key": "Enter",
            "code": "Enter"
        }
    }))
    .await;
    ctx.expect_result(32, json!({}), None);
    assert_eq!(
        evaluate_string(&mut ctx, "JSON.stringify(window.__linkEvents)").await,
        r#"["keydown","click","keyup"]"#
    );
    assert_eq!(
        evaluate_string(&mut ctx, "String(window.__linkClicks)").await,
        "1"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn dispatch_key_event_backspace_deletes_text_in_input() {
    let mut ctx = TestContext::new();
    with_loaded_document(
        &mut ctx,
        r#"<html><body>
                <input id='field' value='abc'>
                <script>
                  const field = document.getElementById('field');
                  window.__editEvents = [];
                  field.focus();
                  field.setSelectionRange(2, 2);
                  field.addEventListener('input', () => window.__editEvents.push(field.value));
                </script>
               </body></html>"#,
    )
    .await;

    ctx.process_async(json!({
        "id": 32,
        "method": "Input.dispatchKeyEvent",
        "params": {
            "type": "keyDown",
            "key": "Backspace",
            "code": "Backspace"
        }
    }))
    .await;
    ctx.expect_result(32, json!({}), None);
    assert_eq!(
        evaluate_string(&mut ctx, "document.getElementById('field').value").await,
        "ac"
    );
    assert_eq!(
        evaluate_string(
            &mut ctx,
            "JSON.stringify({start: document.getElementById('field').selectionStart, end: document.getElementById('field').selectionEnd})"
        ).await,
        r#"{"start":1,"end":1}"#
    );
    assert_eq!(
        evaluate_string(&mut ctx, "JSON.stringify(window.__editEvents)").await,
        r#"["ac"]"#
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn dispatch_key_event_keyup_backspace_does_not_edit_text_control() {
    let mut ctx = TestContext::new();
    with_loaded_document(
        &mut ctx,
        r#"<html><body>
                <input id='field' value='abc'>
                <script>
                  const field = document.getElementById('field');
                  window.__keyupEditEvents = [];
                  field.focus();
                  field.setSelectionRange(2, 2);
                  field.addEventListener('input', () => window.__keyupEditEvents.push(field.value));
                </script>
               </body></html>"#,
    )
    .await;

    ctx.process_async(json!({
        "id": 314,
        "method": "Input.dispatchKeyEvent",
        "params": {
            "type": "keyUp",
            "key": "Backspace",
            "code": "Backspace"
        }
    }))
    .await;
    ctx.expect_result(314, json!({}), None);
    assert_eq!(
        evaluate_string(&mut ctx, "document.getElementById('field').value").await,
        "abc"
    );
    assert_eq!(
        evaluate_string(&mut ctx, "JSON.stringify(window.__keyupEditEvents)").await,
        "[]"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn dispatch_key_event_tab_moves_focus_forward_skipping_disabled_controls() {
    let mut ctx = TestContext::new();
    with_loaded_document(
        &mut ctx,
        r#"<html><body>
                <input id='first'>
                <input id='disabled' disabled>
                <button id='second'>next</button>
                <script>
                  document.getElementById('first').focus();
                </script>
               </body></html>"#,
    )
    .await;

    ctx.process_async(json!({
        "id": 33,
        "method": "Input.dispatchKeyEvent",
        "params": {
            "type": "keyDown",
            "key": "Tab",
            "code": "Tab"
        }
    }))
    .await;
    ctx.expect_result(33, json!({}), None);

    assert_eq!(
        evaluate_string(
            &mut ctx,
            "document.activeElement && document.activeElement.id"
        )
        .await,
        "second"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn dispatch_key_event_shift_tab_moves_focus_backward() {
    let mut ctx = TestContext::new();
    with_loaded_document(
        &mut ctx,
        r#"<html><body>
                <input id='first'>
                <button id='second'>next</button>
                <input id='third'>
                <script>
                  document.getElementById('third').focus();
                </script>
               </body></html>"#,
    )
    .await;

    ctx.process_async(json!({
        "id": 34,
        "method": "Input.dispatchKeyEvent",
        "params": {
            "type": "keyDown",
            "key": "Tab",
            "code": "Tab",
            "modifiers": 8
        }
    }))
    .await;
    ctx.expect_result(34, json!({}), None);

    assert_eq!(
        evaluate_string(
            &mut ctx,
            "document.activeElement && document.activeElement.id"
        )
        .await,
        "second"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn dispatch_key_event_tab_prefers_positive_tabindex_order() {
    let mut ctx = TestContext::new();
    with_loaded_document(
        &mut ctx,
        r#"<html><body>
                <button id='regular'>regular</button>
                <button id='two' tabindex='2'>two</button>
                <button id='one' tabindex='1'>one</button>
                <script>
                  document.getElementById('regular').focus();
                </script>
               </body></html>"#,
    )
    .await;

    ctx.process_async(json!({
        "id": 214,
        "method": "Input.dispatchKeyEvent",
        "params": {
            "type": "keyDown",
            "key": "Tab",
            "code": "Tab"
        }
    }))
    .await;
    ctx.expect_result(214, json!({}), None);
    assert_eq!(
        evaluate_string(
            &mut ctx,
            "document.activeElement && document.activeElement.id"
        )
        .await,
        "one"
    );

    ctx.process_async(json!({
        "id": 215,
        "method": "Input.dispatchKeyEvent",
        "params": {
            "type": "keyDown",
            "key": "Tab",
            "code": "Tab"
        }
    }))
    .await;
    ctx.expect_result(215, json!({}), None);
    assert_eq!(
        evaluate_string(
            &mut ctx,
            "document.activeElement && document.activeElement.id"
        )
        .await,
        "two"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn dispatch_key_event_shift_tab_reverses_positive_tabindex_order() {
    let mut ctx = TestContext::new();
    with_loaded_document(
        &mut ctx,
        r#"<html><body>
                <button id='regular'>regular</button>
                <button id='two' tabindex='2'>two</button>
                <button id='one' tabindex='1'>one</button>
                <script>
                  document.getElementById('regular').focus();
                </script>
               </body></html>"#,
    )
    .await;

    ctx.process_async(json!({
        "id": 216,
        "method": "Input.dispatchKeyEvent",
        "params": {
            "type": "keyDown",
            "key": "Tab",
            "code": "Tab",
            "modifiers": 8
        }
    }))
    .await;
    ctx.expect_result(216, json!({}), None);
    assert_eq!(
        evaluate_string(
            &mut ctx,
            "document.activeElement && document.activeElement.id"
        )
        .await,
        "two"
    );

    ctx.process_async(json!({
        "id": 217,
        "method": "Input.dispatchKeyEvent",
        "params": {
            "type": "keyDown",
            "key": "Tab",
            "code": "Tab",
            "modifiers": 8
        }
    }))
    .await;
    ctx.expect_result(217, json!({}), None);
    assert_eq!(
        evaluate_string(
            &mut ctx,
            "document.activeElement && document.activeElement.id"
        )
        .await,
        "one"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn dispatch_key_event_arrowleft_collapses_selection_to_start_in_text_input() {
    let mut ctx = TestContext::new();
    with_loaded_document(
        &mut ctx,
        r#"<html><body>
                <input id='field' value='hello'>
                <script>
                  const field = document.getElementById('field');
                  field.focus();
                  field.setSelectionRange(2, 4);
                </script>
               </body></html>"#,
    )
    .await;

    ctx.process_async(json!({
        "id": 218,
        "method": "Input.dispatchKeyEvent",
        "params": {
            "type": "keyDown",
            "key": "ArrowLeft",
            "code": "ArrowLeft"
        }
    }))
    .await;
    ctx.expect_result(218, json!({}), None);

    assert_eq!(
        evaluate_string(
            &mut ctx,
            "JSON.stringify([document.getElementById('field').selectionStart, document.getElementById('field').selectionEnd])"
        ).await,
        "[2,2]"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn dispatch_key_event_arrowright_collapses_selection_to_end_in_text_input() {
    let mut ctx = TestContext::new();
    with_loaded_document(
        &mut ctx,
        r#"<html><body>
                <input id='field' value='hello'>
                <script>
                  const field = document.getElementById('field');
                  field.focus();
                  field.setSelectionRange(1, 3);
                </script>
               </body></html>"#,
    )
    .await;

    ctx.process_async(json!({
        "id": 219,
        "method": "Input.dispatchKeyEvent",
        "params": {
            "type": "keyDown",
            "key": "ArrowRight",
            "code": "ArrowRight"
        }
    }))
    .await;
    ctx.expect_result(219, json!({}), None);

    assert_eq!(
        evaluate_string(
            &mut ctx,
            "JSON.stringify([document.getElementById('field').selectionStart, document.getElementById('field').selectionEnd])"
        ).await,
        "[3,3]"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn dispatch_key_event_shift_arrowleft_extends_text_input_selection() {
    let mut ctx = TestContext::new();
    with_loaded_document(
        &mut ctx,
        r#"<html><body>
                <input id='field' value='abcd efgh'>
                <script>
                  const field = document.getElementById('field');
                  field.focus();
                  field.setSelectionRange(field.value.length, field.value.length);
                </script>
               </body></html>"#,
    )
    .await;

    for id in 240..243 {
        ctx.process_async(json!({
            "id": id,
            "method": "Input.dispatchKeyEvent",
            "params": {
                "type": "keyDown",
                "key": "ArrowLeft",
                "code": "ArrowLeft",
                "modifiers": 8
            }
        }))
        .await;
        ctx.expect_result(id, json!({}), None);
    }

    assert_eq!(
        evaluate_string(
            &mut ctx,
            "JSON.stringify([document.getElementById('field').selectionStart, document.getElementById('field').selectionEnd, document.getElementById('field').selectionDirection])"
        ).await,
        r#"[6,9,"backward"]"#
    );

    ctx.process_async(json!({
        "id": 243,
        "method": "Input.dispatchKeyEvent",
        "params": {
            "type": "keyDown",
            "key": "Delete",
            "code": "Delete",
            "modifiers": 8
        }
    }))
    .await;
    ctx.expect_result(243, json!({}), None);

    assert_eq!(
        evaluate_string(
            &mut ctx,
            "JSON.stringify([document.getElementById('field').value, document.getElementById('field').selectionStart, document.getElementById('field').selectionEnd])"
        ).await,
        r#"["abcd e",6,6]"#
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn dispatch_key_event_home_moves_text_control_caret_to_start() {
    let mut ctx = TestContext::new();
    with_loaded_document(
        &mut ctx,
        r#"<html><body>
                <textarea id='field'>hello world</textarea>
                <script>
                  const field = document.getElementById('field');
                  field.focus();
                  field.setSelectionRange(5, 5);
                </script>
               </body></html>"#,
    )
    .await;

    ctx.process_async(json!({
        "id": 220,
        "method": "Input.dispatchKeyEvent",
        "params": {
            "type": "keyDown",
            "key": "Home",
            "code": "Home"
        }
    }))
    .await;
    ctx.expect_result(220, json!({}), None);

    assert_eq!(
        evaluate_string(
            &mut ctx,
            "JSON.stringify([document.getElementById('field').selectionStart, document.getElementById('field').selectionEnd])"
        ).await,
        "[0,0]"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn dispatch_key_event_end_moves_text_control_caret_to_end() {
    let mut ctx = TestContext::new();
    with_loaded_document(
        &mut ctx,
        r#"<html><body>
                <input id='field' value='hello world'>
                <script>
                  const field = document.getElementById('field');
                  field.focus();
                  field.setSelectionRange(1, 1);
                </script>
               </body></html>"#,
    )
    .await;

    ctx.process_async(json!({
        "id": 221,
        "method": "Input.dispatchKeyEvent",
        "params": {
            "type": "keyDown",
            "key": "End",
            "code": "End"
        }
    }))
    .await;
    ctx.expect_result(221, json!({}), None);

    assert_eq!(
        evaluate_string(
            &mut ctx,
            "JSON.stringify([document.getElementById('field').selectionStart, document.getElementById('field').selectionEnd, document.getElementById('field').value.length])"
        ).await,
        "[11,11,11]"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn dispatch_key_event_ctrl_a_selects_all_text_in_input() {
    let mut ctx = TestContext::new();
    with_loaded_document(
        &mut ctx,
        r#"<html><body>
                <input id='field' value='hello world'>
                <script>
                  const field = document.getElementById('field');
                  field.focus();
                  field.setSelectionRange(2, 4);
                </script>
               </body></html>"#,
    )
    .await;

    ctx.process_async(json!({
        "id": 222,
        "method": "Input.dispatchKeyEvent",
        "params": {
            "type": "keyDown",
            "key": "a",
            "code": "KeyA",
            "modifiers": 2
        }
    }))
    .await;
    ctx.expect_result(222, json!({}), None);

    assert_eq!(
        evaluate_string(
            &mut ctx,
            "JSON.stringify([document.getElementById('field').selectionStart, document.getElementById('field').selectionEnd, document.getElementById('field').value.length])"
        ).await,
        "[0,11,11]"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn dispatch_key_event_meta_a_selects_all_text_in_textarea() {
    let mut ctx = TestContext::new();
    with_loaded_document(
        &mut ctx,
        r#"<html><body>
                <textarea id='field'>hello world</textarea>
                <script>
                  const field = document.getElementById('field');
                  field.focus();
                  field.setSelectionRange(1, 1);
                </script>
               </body></html>"#,
    )
    .await;

    ctx.process_async(json!({
        "id": 223,
        "method": "Input.dispatchKeyEvent",
        "params": {
            "type": "keyDown",
            "key": "a",
            "code": "KeyA",
            "modifiers": 4
        }
    }))
    .await;
    ctx.expect_result(223, json!({}), None);

    assert_eq!(
        evaluate_string(
            &mut ctx,
            "JSON.stringify([document.getElementById('field').selectionStart, document.getElementById('field').selectionEnd, document.getElementById('field').value.length])"
        ).await,
        "[0,11,11]"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn dispatch_key_event_arrowdown_advances_single_select_and_emits_input_change() {
    let mut ctx = TestContext::new();
    with_loaded_document(
        &mut ctx,
        r#"<html><body>
                <select id='pick'>
                  <option value='a'>A</option>
                  <option value='b'>B</option>
                  <option value='c'>C</option>
                </select>
                <script>
                  const pick = document.getElementById('pick');
                  pick.focus();
                  window.__selectEvents = [];
                  ['keydown', 'input', 'change'].forEach((type) => {
                    pick.addEventListener(type, () => window.__selectEvents.push(type + ':' + pick.value));
                  });
                </script>
               </body></html>"#,
    ).await;

    ctx.process_async(json!({
        "id": 35,
        "method": "Input.dispatchKeyEvent",
        "params": {
            "type": "keyDown",
            "key": "ArrowDown",
            "code": "ArrowDown"
        }
    }))
    .await;
    ctx.expect_result(35, json!({}), None);

    assert_eq!(
        evaluate_string(&mut ctx, "document.getElementById('pick').value").await,
        "b"
    );
    assert_eq!(
        evaluate_string(&mut ctx, "JSON.stringify(window.__selectEvents)").await,
        r#"["keydown:a","input:b","change:b"]"#
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn dispatch_key_event_arrowup_moves_single_select_backward() {
    let mut ctx = TestContext::new();
    with_loaded_document(
        &mut ctx,
        r#"<html><body>
                <select id='pick'>
                  <option value='a'>A</option>
                  <option value='b' selected>B</option>
                  <option value='c'>C</option>
                </select>
                <script>
                  const pick = document.getElementById('pick');
                  pick.focus();
                </script>
               </body></html>"#,
    )
    .await;

    ctx.process_async(json!({
        "id": 36,
        "method": "Input.dispatchKeyEvent",
        "params": {
            "type": "keyDown",
            "key": "ArrowUp",
            "code": "ArrowUp"
        }
    }))
    .await;
    ctx.expect_result(36, json!({}), None);

    assert_eq!(
        evaluate_string(&mut ctx, "document.getElementById('pick').value").await,
        "a"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn dispatch_key_event_arrowdown_skips_disabled_select_options() {
    let mut ctx = TestContext::new();
    with_loaded_document(
        &mut ctx,
        r#"<html><body>
                <select id='pick'>
                  <option value='a' selected>A</option>
                  <option value='b' disabled>B</option>
                  <optgroup disabled>
                    <option value='c'>C</option>
                  </optgroup>
                  <option value='d'>D</option>
                </select>
                <script>
                  const pick = document.getElementById('pick');
                  pick.focus();
                  window.__selectSkipEvents = [];
                  ['input', 'change'].forEach((type) => {
                    pick.addEventListener(type, () => window.__selectSkipEvents.push(type + ':' + pick.value));
                  });
                </script>
               </body></html>"#,
    ).await;

    ctx.process_async(json!({
        "id": 44,
        "method": "Input.dispatchKeyEvent",
        "params": {
            "type": "keyDown",
            "key": "ArrowDown",
            "code": "ArrowDown"
        }
    }))
    .await;
    ctx.expect_result(44, json!({}), None);

    assert_eq!(
        evaluate_string(&mut ctx, "document.getElementById('pick').value").await,
        "d"
    );
    assert_eq!(
        evaluate_string(&mut ctx, "JSON.stringify(window.__selectSkipEvents)").await,
        r#"["input:d","change:d"]"#
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn dispatch_key_event_arrowup_skips_disabled_select_options() {
    let mut ctx = TestContext::new();
    with_loaded_document(
        &mut ctx,
        r#"<html><body>
                <select id='pick'>
                  <option value='a'>A</option>
                  <option value='b' disabled>B</option>
                  <optgroup disabled>
                    <option value='c'>C</option>
                  </optgroup>
                  <option value='d' selected>D</option>
                </select>
                <script>
                  const pick = document.getElementById('pick');
                  pick.focus();
                  window.__selectSkipBackEvents = [];
                  ['input', 'change'].forEach((type) => {
                    pick.addEventListener(type, () => window.__selectSkipBackEvents.push(type + ':' + pick.value));
                  });
                </script>
               </body></html>"#,
    ).await;

    ctx.process_async(json!({
        "id": 45,
        "method": "Input.dispatchKeyEvent",
        "params": {
            "type": "keyDown",
            "key": "ArrowUp",
            "code": "ArrowUp"
        }
    }))
    .await;
    ctx.expect_result(45, json!({}), None);

    assert_eq!(
        evaluate_string(&mut ctx, "document.getElementById('pick').value").await,
        "a"
    );
    assert_eq!(
        evaluate_string(&mut ctx, "JSON.stringify(window.__selectSkipBackEvents)").await,
        r#"["input:a","change:a"]"#
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn dispatch_key_event_arrowdown_advances_radio_group() {
    let mut ctx = TestContext::new();
    with_loaded_document(
        &mut ctx,
        r#"<html><body>
                <input id='one' type='radio' name='group' checked>
                <input id='two' type='radio' name='group'>
                <input id='three' type='radio' name='group'>
                <script>
                  const one = document.getElementById('one');
                  const two = document.getElementById('two');
                  one.focus();
                  window.__radioArrowEvents = [];
                  ['keydown'].forEach((type) => {
                    one.addEventListener(type, () => window.__radioArrowEvents.push(type + ':one'));
                  });
                  ['input', 'change'].forEach((type) => {
                    two.addEventListener(type, () => window.__radioArrowEvents.push(type + ':two'));
                  });
                </script>
               </body></html>"#,
    )
    .await;

    ctx.process_async(json!({
        "id": 37,
        "method": "Input.dispatchKeyEvent",
        "params": {
            "type": "keyDown",
            "key": "ArrowDown",
            "code": "ArrowDown"
        }
    }))
    .await;
    ctx.expect_result(37, json!({}), None);

    assert!(evaluate_bool(&mut ctx, "document.getElementById('two').checked").await);
    assert_eq!(
        evaluate_string(
            &mut ctx,
            "document.activeElement && document.activeElement.id"
        )
        .await,
        "two"
    );
    assert_eq!(
        evaluate_string(&mut ctx, "JSON.stringify(window.__radioArrowEvents)").await,
        r#"["keydown:one","input:two","change:two"]"#
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn dispatch_key_event_arrowup_wraps_radio_group_backward() {
    let mut ctx = TestContext::new();
    with_loaded_document(
        &mut ctx,
        r#"<html><body>
                <input id='one' type='radio' name='group'>
                <input id='two' type='radio' name='group'>
                <input id='three' type='radio' name='group' checked>
                <script>
                  const three = document.getElementById('three');
                  const two = document.getElementById('two');
                  three.focus();
                  window.__radioWrapEvents = [];
                  ['keydown'].forEach((type) => {
                    three.addEventListener(type, () => window.__radioWrapEvents.push(type + ':three'));
                  });
                  ['input', 'change'].forEach((type) => {
                    two.addEventListener(type, () => window.__radioWrapEvents.push(type + ':two'));
                  });
                </script>
               </body></html>"#,
    ).await;

    ctx.process_async(json!({
        "id": 38,
        "method": "Input.dispatchKeyEvent",
        "params": {
            "type": "keyDown",
            "key": "ArrowUp",
            "code": "ArrowUp"
        }
    }))
    .await;
    ctx.expect_result(38, json!({}), None);

    assert!(evaluate_bool(&mut ctx, "document.getElementById('two').checked").await);
    assert_eq!(
        evaluate_string(
            &mut ctx,
            "document.activeElement && document.activeElement.id"
        )
        .await,
        "two"
    );
    assert_eq!(
        evaluate_string(&mut ctx, "JSON.stringify(window.__radioWrapEvents)").await,
        r#"["keydown:three","input:two","change:two"]"#
    );
}
