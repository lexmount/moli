use super::*;
use moli_core::page::RendererJavaScriptDialogCompletion;

fn dialog_for_test(
    frame_id: Option<&str>,
    dialog_type: &str,
    message: &str,
) -> RendererPendingJavaScriptDialog {
    renderer_dialog_for_test(frame_id, dialog_type, message, "", None)
}

fn page_owner_for_test(
    ctx: &mut TestContext,
    session_id: Option<&str>,
) -> crate::conn::TargetPageResidenceIdentity {
    let runtime_slot = ctx
        .conn
        .runtime_session_owner_slot_mut(session_id)
        .expect("target session should expose a runtime owner slot");
    if runtime_slot.page_attachment_id().is_none() {
        runtime_slot.replace_page_attachment_id_for_test();
    }
    ctx.conn
        .target_page_residence_identity_for_session(session_id)
        .expect("target session should expose a Page residence")
}

fn push_dialog_for_session(
    ctx: &mut TestContext,
    session_id: Option<&str>,
    dialog: RendererPendingJavaScriptDialog,
) {
    let page_owner = page_owner_for_test(ctx, session_id);
    let source_frame_id = match dialog.source() {
        moli_core::page::RendererJavaScriptDialogSource::ChildFrame { frame_id, .. } => {
            frame_id.clone()
        }
        moli_core::page::RendererJavaScriptDialogSource::RootFrame
        | moli_core::page::RendererJavaScriptDialogSource::LightweightPopup { .. } => ctx
            .conn
            .target_session_owner_frame_tree_identity(session_id)
            .map(|(frame_id, _, _, _)| frame_id)
            .or_else(|| page_owner.target_id().map(str::to_owned))
            .expect("test target should expose a root frame"),
    };
    let dialog = crate::conn::TargetJavaScriptDialog::new(page_owner, source_frame_id, dialog);
    ctx.conn
        .with_target_devtools_session_state_for_session_mut(session_id, |state| {
            state
                .page_session_state
                .javascript_dialog_state
                .push(dialog);
        })
        .expect("target session state should exist");
}

#[test]
fn retiring_page_scope_and_clearing_dialog_state_dismisses_installed_dialog() {
    let mut ctx = TestContext::new();
    load_bc_with_session(
        &mut ctx,
        "BID-dialog-clear",
        "TID-dialog-clear",
        "SID-dialog-clear",
        "about:blank",
    );
    let completion = RendererJavaScriptDialogCompletion::pending();
    push_dialog_for_session(
        &mut ctx,
        Some("SID-dialog-clear"),
        renderer_dialog_for_test(
            Some("FRAME-dialog-clear"),
            "prompt",
            "retire me",
            "default",
            Some(completion.clone()),
        ),
    );
    let retired_scope = ctx
        .conn
        .runtime_session_owner_slot(Some("SID-dialog-clear"))
        .expect("target Page runtime slot")
        .javascript_dialog_scope_observer();

    ctx.conn
        .runtime_session_owner_slot_mut(Some("SID-dialog-clear"))
        .expect("target Page runtime slot")
        .retire_javascript_dialog_scope();

    ctx.conn
        .with_target_devtools_session_state_for_session_mut(Some("SID-dialog-clear"), |state| {
            state.page_session_state.javascript_dialog_state.clear()
        })
        .expect("target page session state should clear");

    let state = &ctx
        .conn
        .target_page_session_state_for_session(Some("SID-dialog-clear"))
        .expect("target page session state")
        .javascript_dialog_state;
    assert!(state.is_empty());
    assert!(
        !ctx.conn
            .runtime_session_owner_slot(Some("SID-dialog-clear"))
            .expect("target Page runtime slot")
            .observes_javascript_dialog_scope(&retired_scope)
    );
    assert!(
        !completion.finish(true, "late input".to_owned()),
        "clearing the exact dialog residence must settle its completion once"
    );
    let result = completion.wait();
    assert!(!result.accepted);
    assert!(result.user_input.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn pending_javascript_dialogs_are_preserved_per_parked_target() {
    fn dialog(target_id: &str, message: &str) -> crate::conn::TargetJavaScriptDialog {
        target_dialog_for_test(
            crate::conn::TargetPageResidenceIdentity::new_for_test(
                "BID-1".to_owned(),
                Some(target_id.to_owned()),
                1,
            ),
            target_id,
            "alert",
            message,
            "",
            None,
        )
    }

    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-A", "SID-A", "about:blank");

    {
        let browser_context = ctx.conn.browser_context.as_mut().unwrap();
        browser_context.active_page_target_mut().devtools_sessions
            [moli_page_types::DevToolsSessionKey::Primary]
            .page_session_state
            .javascript_dialog_state
            .push(dialog("TID-A", "a"));
        browser_context.insert_page_target_host(PageTargetHost::new(
            "TID-B".to_owned(),
            Some("SID-B".to_owned()),
            crate::conn::TargetIdentityState::new(
                "about:blank".to_owned(),
                URL_BASE.to_owned(),
                "Secure".to_owned(),
            ),
            crate::conn::TargetPageSlot::empty_for_test_fixture(),
        ));
        assert!(
            browser_context
                .select_page_target_async("TID-B")
                .await
                .unwrap()
        );
        assert!(
            browser_context.active_page_target().devtools_sessions
                [moli_page_types::DevToolsSessionKey::Primary]
                .page_session_state
                .javascript_dialog_state
                .is_empty()
        );
        browser_context.active_page_target_mut().devtools_sessions
            [moli_page_types::DevToolsSessionKey::Primary]
            .page_session_state
            .javascript_dialog_state
            .push(dialog("TID-B", "b"));
    }

    {
        let browser_context = ctx.conn.browser_context.as_mut().unwrap();
        assert!(
            browser_context
                .select_page_target_async("TID-A")
                .await
                .unwrap()
        );
        assert_eq!(
            browser_context.active_page_target().devtools_sessions
                [moli_page_types::DevToolsSessionKey::Primary]
                .page_session_state
                .javascript_dialog_state
                .pending_dialogs(),
            &[dialog("TID-A", "a")]
        );
    }

    {
        let browser_context = ctx.conn.browser_context.as_mut().unwrap();
        assert!(
            browser_context
                .select_page_target_async("TID-B")
                .await
                .unwrap()
        );
        assert_eq!(
            browser_context.active_page_target().devtools_sessions
                [moli_page_types::DevToolsSessionKey::Primary]
                .page_session_state
                .javascript_dialog_state
                .pending_dialogs(),
            &[dialog("TID-B", "b")]
        );
    }
}
#[tokio::test(flavor = "multi_thread")]
async fn javascript_dialog_events_round_trip_through_page_domain() {
    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");
    let page = ctx
        .conn
        .load_page_via_runtime_async("data:text/html,<button id='b'>alert</button>")
        .await
        .expect("page should load");
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .active_page_target_mut()
        .runtime_slot
        .set_loaded_page_for_test(page);

    ctx.process_async(json!({
        "id": 2,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": "alert('smoke alert'); 'done'",
            "returnByValue": true
        }
    }))
    .await;

    let opening = ctx
        .take_all()
        .into_iter()
        .find(|message| message["method"] == json!("Page.javascriptDialogOpening"))
        .expect("alert should emit Page.javascriptDialogOpening");
    assert_eq!(opening["sessionId"], json!("SID-1"));
    assert_eq!(opening["params"]["type"], json!("alert"));
    assert_eq!(opening["params"]["message"], json!("smoke alert"));
    assert_eq!(opening["params"]["hasBrowserHandler"], json!(false));
    assert_eq!(opening["params"]["frameId"], json!("TID-1"));

    ctx.process_async(json!({
        "id": 3,
        "method": "Page.handleJavaScriptDialog",
        "sessionId": "SID-1",
        "params": { "accept": true }
    }))
    .await;
    let sent = ctx.take_all();
    assert!(
        sent.iter().any(|message| {
            message["method"] == json!("Page.javascriptDialogClosed")
                && message["params"]["result"] == json!(true)
                && message["params"]["frameId"] == json!("TID-1")
        }),
        "handleJavaScriptDialog should emit a closed event: {sent:?}"
    );
    assert!(
        sent.iter()
            .any(|message| message["id"] == json!(3) && message["result"] == json!({})),
        "handleJavaScriptDialog should resolve: {sent:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn document_open_preserves_dialog_order_and_retires_replaced_document_state() {
    let mut ctx = TestContext::new();
    load_bc_with_session(
        &mut ctx,
        "BID-dialog-open",
        "TID-dialog-open",
        "SID-dialog-open",
        "about:blank",
    );
    ctx.install_buffered_navigation_fixture_for_session_owner(
        url::Url::parse("https://dialog-replacement.example/").unwrap(),
        "<!doctype html><body>dialog replacement</body>".into(),
        Some("SID-dialog-open"),
    )
    .await;

    ctx.process_async(json!({
        "id": 39,
        "method": "Runtime.evaluate",
        "sessionId": "SID-dialog-open",
        "params": {
            "expression": "alert('retired dialog'); document.open(); document.write('<main>replacement</main>'); document.close(); 'done'",
            "returnByValue": true
        }
    }))
    .await;

    let sent = ctx.take_all();
    let opening_index = sent
        .iter()
        .position(|message| {
            message["method"] == json!("Page.javascriptDialogOpening")
                && message["params"]["message"] == json!("retired dialog")
        })
        .expect("the dialog produced before document.open() must remain observable");
    let response_index = sent
        .iter()
        .position(|message| message["id"] == json!(39))
        .expect("Runtime.evaluate response");
    assert!(
        opening_index < response_index,
        "concrete dialog output must retain its source order before the command response: {sent:?}"
    );
    assert!(
        ctx.conn
            .target_page_session_state_for_session(Some("SID-dialog-open"))
            .expect("replacement target page session state")
            .javascript_dialog_state
            .is_empty(),
        "the replacement Document must retire the already-emitted dialog state"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn get_javascript_dialog_text_peeks_without_closing_dialog() {
    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");
    let page_owner = page_owner_for_test(&mut ctx, Some("SID-1"));
    ctx.conn
        .with_target_devtools_session_state_for_session_mut(Some("SID-1"), |state| {
            state
                .page_session_state
                .javascript_dialog_state
                .push(target_dialog_for_test(
                    page_owner,
                    "TID-1",
                    "alert",
                    "classic alert",
                    "",
                    None,
                ));
        })
        .expect("target session state should exist");

    let context = crate::devtools_runtime::DevToolsCommandContext {
        protocol: crate::devtools_runtime::DevToolsProtocol::WebDriverClassic,
        session_id: Some(crate::devtools_runtime::DevToolsSessionId::from("SID-1")),
        target_id: Some(crate::devtools_runtime::DevToolsTargetId::from("TID-1")),
        browser_context_id: None,
    };
    let (result, _, _) = ctx
        .conn
        .execute_devtools_command_with_protocol_events(
            crate::devtools_runtime::DevToolsCommand::GetJavaScriptDialog(
                crate::devtools_runtime::DevToolsGetJavaScriptDialogCommand {
                    context: context.clone(),
                },
            ),
        )
        .await
        .into_parts_with_protocol_events();
    let crate::devtools_runtime::DevToolsCommandResult::JavaScriptDialog(result) =
        result.expect("get dialog text should resolve")
    else {
        panic!("get dialog text should return dialog result");
    };
    assert_eq!(result.message, "classic alert");
    assert_eq!(result.dialog_type, "alert");

    let (result, _, events) = ctx
        .conn
        .execute_devtools_command_with_protocol_events(
            crate::devtools_runtime::DevToolsCommand::HandleJavaScriptDialog(
                crate::devtools_runtime::DevToolsHandleJavaScriptDialogCommand {
                    context,
                    accept: true,
                    prompt_text: String::new(),
                },
            ),
        )
        .await
        .into_parts_with_protocol_events();
    assert!(matches!(
        result.expect("handle dialog should resolve"),
        crate::devtools_runtime::DevToolsCommandResult::Empty
    ));
    let closed_event = events
        .into_iter()
        .find(|event| {
            event.clone().into_parts().0["method"] == json!("Page.javascriptDialogClosed")
        })
        .expect("peeked dialog should emit Page.javascriptDialogClosed");
    assert!(
        closed_event.protocol_message().is_none(),
        "closed dialog event should stay typed until wire projection"
    );
    let (closed_message, closed_sidecar) = closed_event.into_parts();
    assert_eq!(closed_message["params"]["result"], json!(true));
    assert_eq!(closed_message["params"]["frameId"], json!("TID-1"));
    assert!(matches!(
        closed_sidecar,
        Some(crate::devtools_runtime::AutomationEvent::UserPromptClosed(
            _
        ))
    ));
}

#[tokio::test(flavor = "multi_thread")]
async fn set_javascript_dialog_prompt_text_is_used_when_accepting_prompt() {
    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");
    let page_owner = page_owner_for_test(&mut ctx, Some("SID-1"));
    ctx.conn
        .with_target_devtools_session_state_for_session_mut(Some("SID-1"), |state| {
            state
                .page_session_state
                .javascript_dialog_state
                .push(target_dialog_for_test(
                    page_owner, "TID-1", "prompt", "prompt?", "", None,
                ));
        })
        .expect("target session state should exist");

    let context = crate::devtools_runtime::DevToolsCommandContext {
        protocol: crate::devtools_runtime::DevToolsProtocol::WebDriverClassic,
        session_id: Some(crate::devtools_runtime::DevToolsSessionId::from("SID-1")),
        target_id: Some(crate::devtools_runtime::DevToolsTargetId::from("TID-1")),
        browser_context_id: None,
    };
    let (result, _, _) = ctx
        .conn
        .execute_devtools_command_with_protocol_events(
            crate::devtools_runtime::DevToolsCommand::SetJavaScriptDialogPromptText(
                crate::devtools_runtime::DevToolsSetJavaScriptDialogPromptTextCommand {
                    context: context.clone(),
                    prompt_text: "cheese".to_owned(),
                },
            ),
        )
        .await
        .into_parts_with_protocol_events();
    assert!(matches!(
        result.expect("set prompt text should resolve"),
        crate::devtools_runtime::DevToolsCommandResult::Empty
    ));

    let (result, _, events) = ctx
        .conn
        .execute_devtools_command_with_protocol_events(
            crate::devtools_runtime::DevToolsCommand::HandleJavaScriptDialog(
                crate::devtools_runtime::DevToolsHandleJavaScriptDialogCommand {
                    context,
                    accept: true,
                    prompt_text: String::new(),
                },
            ),
        )
        .await
        .into_parts_with_protocol_events();
    assert!(matches!(
        result.expect("accept prompt should resolve"),
        crate::devtools_runtime::DevToolsCommandResult::Empty
    ));
    let messages = events
        .into_iter()
        .map(|event| event.into_protocol_message())
        .collect::<Vec<_>>();
    assert!(
        messages.iter().any(|message| {
            message["method"] == json!("Page.javascriptDialogClosed")
                && message["params"]["result"] == json!(true)
                && message["params"]["userInput"] == json!("cheese")
        }),
        "stored prompt text should be consumed by accept: {messages:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn handle_javascript_dialog_finishes_renderer_completion() {
    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");
    let completion = RendererJavaScriptDialogCompletion::pending();
    push_dialog_for_session(
        &mut ctx,
        Some("SID-1"),
        renderer_dialog_for_test(
            Some("TID-1"),
            "prompt",
            "prompt?",
            "default",
            Some(completion.clone()),
        ),
    );

    ctx.process_async(json!({
        "id": 33,
        "method": "Page.handleJavaScriptDialog",
        "sessionId": "SID-1",
        "params": { "accept": true, "promptText": "typed" }
    }))
    .await;

    let result = completion.wait();
    assert!(result.accepted);
    assert_eq!(result.user_input, "typed");
    let sent = ctx.take_all();
    assert!(
        sent.iter().any(|message| {
            message["method"] == json!("Page.javascriptDialogClosed")
                && message["params"]["result"] == json!(true)
                && message["params"]["userInput"] == json!("typed")
        }),
        "handle should close protocol dialog while resuming renderer completion: {sent:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn handle_javascript_dialog_rejects_when_no_dialog_is_showing() {
    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");

    ctx.process_async(json!({
        "id": 34,
        "method": "Page.handleJavaScriptDialog",
        "sessionId": "SID-1",
        "params": { "accept": true }
    }))
    .await;

    let sent = ctx.take_all();
    assert!(
        sent.iter().any(|message| {
            message["id"] == json!(34)
                && message["error"]["code"] == json!(-32602)
                && message["error"]["message"] == json!("No dialog is showing")
        }),
        "handleJavaScriptDialog without a pending dialog should reject like Chromium: {sent:?}"
    );
    assert!(
        sent.iter()
            .all(|message| message["method"] != json!("Page.javascriptDialogClosed")),
        "no-dialog handling must not emit a closed event: {sent:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn javascript_dialog_pending_state_is_session_local_for_active_auxiliary_session() {
    let mut ctx = TestContext::new();
    load_bc_with_session(
        &mut ctx,
        "BID-dialog-aux",
        "TID-dialog-aux",
        "SID-primary",
        "about:blank",
    );
    let browser_context = ctx.conn.browser_context.as_mut().unwrap();
    assert!(
        browser_context.assign_auxiliary_session_to_target("TID-dialog-aux", "SID-aux".to_owned())
    );
    push_dialog_for_session(
        &mut ctx,
        Some("SID-aux"),
        dialog_for_test(Some("TID-dialog-aux"), "alert", "aux dialog"),
    );
    assert!(
        ctx.conn
            .target_page_session_state_for_session(Some("SID-primary"))
            .expect("primary page session state")
            .javascript_dialog_state
            .is_empty(),
        "primary session must not see auxiliary pending dialog"
    );

    ctx.process_async(json!({
        "id": 36,
        "method": "Page.handleJavaScriptDialog",
        "sessionId": "SID-primary",
        "params": { "accept": true }
    }))
    .await;
    let primary = take_response_by_id(&mut ctx, 36);
    assert_eq!(primary["sessionId"], json!("SID-primary"));
    assert_eq!(primary["error"]["code"], json!(-32602));
    assert_eq!(primary["error"]["message"], json!("No dialog is showing"));

    ctx.process_async(json!({
        "id": 37,
        "method": "Page.handleJavaScriptDialog",
        "sessionId": "SID-aux",
        "params": { "accept": true }
    }))
    .await;
    let closed = ctx
        .sent
        .iter()
        .find(|message| message["method"] == json!("Page.javascriptDialogClosed"))
        .expect("auxiliary handle should emit closed event");
    assert_eq!(closed["sessionId"], json!("SID-aux"));
    assert_eq!(closed["params"]["frameId"], json!("TID-dialog-aux"));
    assert_eq!(closed["params"]["result"], json!(true));
    let auxiliary = take_response_by_id(&mut ctx, 37);
    assert_eq!(auxiliary["sessionId"], json!("SID-aux"));
    assert_eq!(auxiliary["result"], json!({}));
    assert!(
        ctx.conn
            .target_page_session_state_for_session(Some("SID-aux"))
            .expect("auxiliary page session state")
            .javascript_dialog_state
            .is_empty(),
        "auxiliary handle should consume only its own dialog"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn handle_javascript_dialog_rejects_dialog_without_current_page_residence() {
    let mut ctx = TestContext::new();
    ctx.conn.browser_context = Some(BrowserContext::new_with_page_for_test(
        "BID-dialog-missing-frame",
        "TID-dialog-missing-frame",
    ));
    ctx.conn
        .with_target_devtools_session_state_for_session_mut(None, |state| {
            state
                .page_session_state
                .javascript_dialog_state
                .push(target_dialog_for_test(
                    crate::conn::TargetPageResidenceIdentity::new_for_test(
                        "BID-dialog-missing-frame".to_owned(),
                        Some("retired-target".to_owned()),
                        1,
                    ),
                    "retired-target",
                    "alert",
                    "stale dialog",
                    "",
                    None,
                ));
        })
        .expect("target session state should exist");

    ctx.process_async(json!({
        "id": 35,
        "method": "Page.handleJavaScriptDialog",
        "params": { "accept": true }
    }))
    .await;

    let sent = ctx.take_all();
    assert!(
        sent.iter().any(|message| {
            message["id"] == json!(35)
                && message["error"]["code"] == json!(-32602)
                && message["error"]["message"] == json!("No dialog is showing")
        }),
        "dialog without its exact Page residence should reject: {sent:?}"
    );
    assert!(
        sent.iter()
            .all(|message| message["method"] != json!("Page.javascriptDialogClosed")),
        "stale dialog must not emit a closed event for a replacement target: {sent:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn javascript_dialog_events_are_emitted_after_runtime_call_function_on() {
    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");
    let page = ctx
        .conn
        .load_page_via_runtime_async("data:text/html,<body>callFunctionOn</body>")
        .await
        .expect("page should load");
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .active_page_target_mut()
        .runtime_slot
        .set_loaded_page_for_test(page);

    ctx.process_async(json!({
        "id": 4,
        "method": "Runtime.enable",
        "sessionId": "SID-1"
    }))
    .await;
    let _ = ctx.take_all();

    ctx.process_async(json!({
        "id": 5,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": "({ run: function() { alert('callFunctionOn alert'); } })"
        }
    }))
    .await;
    let object_id = take_response_by_id(&mut ctx, 5)["result"]["result"]["objectId"]
        .as_str()
        .expect("Runtime.evaluate should return an object id")
        .to_owned();
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 6,
        "method": "Runtime.callFunctionOn",
        "sessionId": "SID-1",
        "params": {
            "objectId": object_id,
            "functionDeclaration": "function() { this.run(); }",
            "awaitPromise": true,
            "returnByValue": true,
            "userGesture": true
        }
    }))
    .await;

    let sent = ctx.take_all();
    assert!(
        sent.iter().any(|message| {
            message["method"] == json!("Page.javascriptDialogOpening")
                && message["params"]["message"] == json!("callFunctionOn alert")
        }),
        "Runtime.callFunctionOn should flush pending JavaScript dialogs: {sent:?}"
    );
}
#[tokio::test(flavor = "multi_thread")]
async fn javascript_dialog_events_are_emitted_from_playwright_utility_world_call_function_on() {
    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");
    let page = ctx
        .conn
        .load_page_via_runtime_async("data:text/html,<body>utility dialog</body>")
        .await
        .expect("page should load");
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .active_page_target_mut()
        .runtime_slot
        .set_loaded_page_for_test(page);

    ctx.process_async(json!({
        "id": 7,
        "method": "Runtime.enable",
        "sessionId": "SID-1"
    }))
    .await;
    let _ = ctx.take_all();

    ctx.process_async(json!({
        "id": 8,
        "method": "Page.createIsolatedWorld",
        "sessionId": "SID-1",
        "params": {
            "frameId": "TID-1",
            "worldName": "__playwright_utility_world__"
        }
    }))
    .await;
    let isolated_context_id = take_response_by_id(&mut ctx, 8)["result"]["executionContextId"]
        .as_i64()
        .expect("isolated context id");
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 9,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "contextId": isolated_context_id,
            "expression": "({ global: globalThis, evaluate(isFunction, returnByValue, expression, argCount, ...args) { let result = this.global.eval(expression); if (isFunction === true) { result = result(...args.slice(0, argCount)); } else if (isFunction !== false && typeof result === 'function') { result = result(...args.slice(0, argCount)); } return returnByValue ? result : result; } })"
        }
    }))
    .await;
    let object_id = take_response_by_id(&mut ctx, 9)["result"]["result"]["objectId"]
        .as_str()
        .expect("Runtime.evaluate should return a utility object id")
        .to_owned();
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 10,
        "method": "Runtime.callFunctionOn",
        "sessionId": "SID-1",
        "params": {
            "objectId": object_id.clone(),
            "functionDeclaration": "(utilityScript, ...args) => utilityScript.evaluate(...args)",
            "arguments": [
                { "objectId": object_id },
                { "value": true },
                { "value": true },
                { "value": "() => alert('utility alert')" },
                { "value": 0 }
            ],
            "returnByValue": true,
            "awaitPromise": true,
            "userGesture": true
        }
    }))
    .await;

    let sent = ctx.take_all();
    assert!(
        sent.iter().any(|message| {
            message["method"] == json!("Page.javascriptDialogOpening")
                && message["params"]["message"] == json!("utility alert")
        }),
        "Playwright-style Runtime.callFunctionOn should flush pending JavaScript dialogs: {sent:?}"
    );
}
#[tokio::test(flavor = "multi_thread")]
async fn javascript_dialog_events_are_emitted_from_playwright_serialized_utility_call_function_on()
{
    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");
    let page = ctx
        .conn
        .load_page_via_runtime_async("data:text/html,<body>serialized utility dialog</body>")
        .await
        .expect("page should load");
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .active_page_target_mut()
        .runtime_slot
        .set_loaded_page_for_test(page);

    ctx.process_async(json!({
        "id": 11,
        "method": "Runtime.enable",
        "sessionId": "SID-1"
    }))
    .await;
    let _ = ctx.take_all();

    ctx.process_async(json!({
        "id": 12,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "contextId": 1,
            "expression": "({ global: globalThis, evaluate(isFunction, returnByValue, expression, argCount, ...argsAndHandles) { const args = argsAndHandles.slice(0, argCount).map(value => value && value.v === 'null' ? null : value); let result = this.global.eval(expression); if (isFunction === true) { result = result(...args); } else if (isFunction === false) { result = result; } else if (typeof result === 'function') { result = result(...args); } return returnByValue ? undefined : result; } })"
        }
    }))
    .await;
    let object_id = take_response_by_id(&mut ctx, 12)["result"]["result"]["objectId"]
        .as_str()
        .expect("Runtime.evaluate should return a utility object id")
        .to_owned();
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 13,
        "method": "Runtime.callFunctionOn",
        "sessionId": "SID-1",
        "params": {
            "objectId": object_id.clone(),
            "functionDeclaration": "(utilityScript, ...args) => utilityScript.evaluate(...args)",
            "arguments": [
                { "objectId": object_id },
                {},
                { "value": true },
                { "value": "() => alert('serialized utility alert')" },
                { "value": 1 },
                { "value": { "v": "null" } }
            ],
            "returnByValue": true,
            "awaitPromise": true,
            "userGesture": true
        }
    }))
    .await;

    let sent = ctx.take_all();
    assert!(
        sent.iter().any(|message| {
            message["method"] == json!("Page.javascriptDialogOpening")
                && message["params"]["message"] == json!("serialized utility alert")
        }),
        "Playwright serialized Runtime.callFunctionOn should flush pending JavaScript dialogs: {sent:?}"
    );
}
