use super::*;

#[test]
fn command_dispatch_completes_parse_errors_without_legacy_fallback() {
    let mut conn = CdpConnection::new();
    let step = conn.start_command_dispatch("{");
    assert_eq!(
        complete_messages(step),
        vec![json!({
            "id": null,
            "error": {"code": -32700, "message": "Parse error"}
        })]
    );
}

#[test]
fn command_dispatch_completes_invalid_methods_without_legacy_fallback() {
    let mut conn = CdpConnection::new();
    let raw = serde_json::to_string(&json!({
        "id": 7,
        "method": "MalformedMethod",
        "sessionId": "SID-1"
    }))
    .unwrap();
    let step = conn.start_command_dispatch(&raw);
    assert_eq!(
        complete_messages(step),
        vec![json!({
            "id": 7,
            "error": {"code": -32600, "message": "Invalid method"},
            "sessionId": "SID-1"
        })]
    );
}

#[test]
fn command_dispatch_completes_startup_commands_without_legacy_fallback() {
    let mut conn = CdpConnection::new();
    let raw = serde_json::to_string(&json!({
        "id": 8,
        "method": "Page.getFrameTree",
        "sessionId": "STARTUP"
    }))
    .unwrap();
    let step = conn.start_command_dispatch(&raw);
    let messages = complete_messages(step);
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0]["id"], json!(8));
    assert_eq!(messages[0]["sessionId"], json!("STARTUP"));
    assert_eq!(
        messages[0]["result"]["frameTree"]["frame"]["id"],
        json!("TID-STARTUP")
    );
}

#[test]
fn command_dispatch_completes_unknown_domains_without_legacy_fallback() {
    let mut conn = CdpConnection::new();
    let mut bc = BrowserContext::new("BID-1".into());
    bc.set_active_target_id("TID-1");
    bc.attach_active_session("SID-1");
    conn.install_browser_context_fixture_for_test(bc);
    let raw = serde_json::to_string(&json!({
        "id": 9,
        "method": "Nope.command",
        "sessionId": "SID-1"
    }))
    .unwrap();
    let step = conn.start_command_dispatch(&raw);
    assert_eq!(
        complete_messages(step),
        vec![json!({
            "id": 9,
            "error": {"code": -32601, "message": "Unknown domain"},
            "sessionId": "SID-1"
        })]
    );
}

#[test]
fn command_dispatch_completes_console_log_and_inspector_owner_commands() {
    let mut conn = CdpConnection::new();
    conn.browser_context = Some(BrowserContext::new("BID-dispatch".to_owned()));

    for (id, method) in [
        (10, "Console.enable"),
        (11, "Log.enable"),
        (12, "Inspector.enable"),
    ] {
        let raw = serde_json::to_string(&json!({ "id": id, "method": method })).unwrap();
        let step = conn.start_command_dispatch(&raw);
        let messages = complete_messages(step);
        assert_eq!(
            messages[0],
            json!({ "id": id, "result": {} }),
            "{method} should complete through the command dispatch entry"
        );
    }
}

#[test]
fn command_dispatch_completes_browser_sync_commands() {
    let mut conn = CdpConnection::new();
    conn.publish_default_browser_target();
    for (id, method) in [
        (20, "Browser.getVersion"),
        (21, "Browser.getWindowForTarget"),
        (22, "Browser.setWindowBounds"),
        (23, "Browser.setDownloadBehavior"),
    ] {
        let params = match method {
            "Browser.setWindowBounds" => json!({
                "windowId": 1_923_710_101_i64,
                "bounds": { "windowState": "normal", "width": 800, "height": 600 }
            }),
            "Browser.setDownloadBehavior" => json!({
                "behavior": "allow",
                "downloadPath": "/tmp/moli-downloads"
            }),
            _ => json!({}),
        };
        let raw = serde_json::to_string(&json!({ "id": id, "method": method, "params": params }))
            .unwrap();
        let step = conn.start_command_dispatch(&raw);
        let messages = complete_messages(step);
        assert_eq!(messages.len(), 1, "{method} should emit one response");
        assert_eq!(messages[0]["id"], json!(id));
        assert!(
            messages[0].get("result").is_some(),
            "{method} should complete successfully: {:?}",
            messages[0]
        );
    }
}

#[test]
fn command_dispatch_completes_browser_owner_commands_without_legacy_fallback() {
    let mut conn = CdpConnection::new();
    for (id, method, params, expects_result) in [
        (
            24,
            "Browser.openDownloadAsStream",
            json!({ "guid": "missing-download-guid" }),
            false,
        ),
        (
            25,
            "Browser.setPermission",
            json!({
                "permission": { "name": "geolocation" },
                "setting": "denied"
            }),
            true,
        ),
        (
            26,
            "Browser.grantPermissions",
            json!({
                "permissions": [{ "name": "notifications" }]
            }),
            true,
        ),
        (27, "Browser.resetPermissions", json!({}), true),
    ] {
        let raw = serde_json::to_string(&json!({ "id": id, "method": method, "params": params }))
            .unwrap();
        let step = conn.start_command_dispatch(&raw);
        let messages = complete_messages(step);
        assert_eq!(messages.len(), 1, "{method} should emit one response");
        assert_eq!(messages[0]["id"], json!(id));
        if expects_result {
            assert!(
                messages[0].get("result").is_some(),
                "{method} should complete successfully: {:?}",
                messages[0]
            );
        } else {
            assert!(
                messages[0].get("error").is_some(),
                "{method} should complete with a protocol error: {:?}",
                messages[0]
            );
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn command_dispatch_completes_live_browser_permission_without_legacy_fallback() {
    let mut conn = CdpConnection::new();
    let mut browser_context = BrowserContext::new("BID-browser-permission-live".to_owned());
    browser_context.set_active_target_id("TID-browser-permission-live".to_owned());
    conn.install_browser_context_fixture_for_test(browser_context);
    let page = conn
        .load_page_via_runtime_async("data:text/html,<p>browser permission</p>")
        .await
        .expect("page should load");
    conn.browser_context
        .as_mut()
        .expect("browser context")
        .active_page_target_mut()
        .runtime_slot
        .set_loaded_page_for_test(page);

    let raw = serde_json::to_string(&json!({
        "id": 28,
        "method": "Browser.setPermission",
        "params": {
            "permission": { "name": "geolocation" },
            "setting": "denied"
        }
    }))
    .unwrap();
    let pending = match conn.start_command_dispatch(&raw) {
        CdpCommandTaskStep::Pending(pending) => pending,
        CdpCommandTaskStep::Complete(_) => {
            panic!("live Browser.setPermission should update the live page")
        }
    };
    let completed = pending.wait().await;
    let step = conn.complete_pending_command_dispatch(completed).await;
    assert_eq!(
        complete_messages(step),
        vec![json!({ "id": 28, "result": {} })]
    );
    assert_eq!(conn.permission_overrides.len(), 1);
}

#[tokio::test]
async fn command_dispatch_completes_target_startup_commands_without_legacy_fallback() {
    let mut conn = CdpConnection::new();
    let create_raw = serde_json::to_string(&json!({
        "id": 29,
        "method": "Target.createTarget",
        "params": { "url": "about:blank" }
    }))
    .unwrap();
    let create_step = conn.start_command_dispatch(&create_raw);
    let create_messages = match create_step {
        CdpCommandTaskStep::Pending(pending) => {
            complete_command_task_for_test(&mut conn, *pending).await
        }
        CdpCommandTaskStep::Complete(outcome) => outcome.into_parts().0,
    };
    assert_eq!(create_messages.len(), 1);
    assert_eq!(create_messages[0]["id"], json!(29));
    let target_id = create_messages[0]["result"]["targetId"]
        .as_str()
        .expect("createTarget should return targetId")
        .to_owned();
    let browser_context = conn.browser_context.as_ref().expect("browser context");
    assert_eq!(browser_context.active_target_id(), Some(target_id.as_str()));
    assert!(
        browser_context
            .active_page_target()
            .runtime_slot
            .has_loaded_page(),
        "Target.createTarget should complete target lifecycle initial document ensure"
    );

    let attach_raw = serde_json::to_string(&json!({
        "id": 30,
        "method": "Target.attachToTarget",
        "params": { "targetId": target_id }
    }))
    .unwrap();
    let attach_step = conn.start_command_dispatch(&attach_raw);
    let attach_messages = match attach_step {
        CdpCommandTaskStep::Pending(pending) => {
            complete_command_task_for_test(&mut conn, *pending).await
        }
        CdpCommandTaskStep::Complete(outcome) => outcome.into_parts().0,
    };
    assert_eq!(attach_messages.len(), 2);
    assert_eq!(
        attach_messages[0]["method"],
        json!("Target.attachedToTarget")
    );
    let attached_session_id = attach_messages[0]["params"]["sessionId"]
        .as_str()
        .expect("attachedToTarget should identify the new session");
    assert_eq!(attach_messages[1]["id"], json!(30));
    assert_eq!(
        attach_messages[1]["result"]["sessionId"],
        json!(attached_session_id)
    );
}

#[test]
fn command_dispatch_completes_network_sync_settings() {
    let mut conn = CdpConnection::new();
    conn.browser_context = Some(BrowserContext::new_with_page_for_test(
        "BID-network",
        "TID-network",
    ));

    for (id, method, params) in [
        (30, "Network.enable", json!({})),
        (31, "Network.disable", json!({})),
        (
            32,
            "Network.setCacheDisabled",
            json!({"cacheDisabled": true}),
        ),
        (
            33,
            "Network.setBypassServiceWorker",
            json!({"bypass": true}),
        ),
    ] {
        let raw = serde_json::to_string(&json!({ "id": id, "method": method, "params": params }))
            .unwrap();
        let step = conn.start_command_dispatch(&raw);
        assert_eq!(
            complete_messages(step),
            vec![json!({ "id": id, "result": {} })],
            "{method} should complete through the command dispatch entry"
        );
    }
}

#[test]
fn command_dispatch_completes_page_dialog_without_legacy_fallback() {
    let mut conn = CdpConnection::new();
    let raw = serde_json::to_string(&json!({
        "id": 40,
        "method": "Page.handleJavaScriptDialog",
        "params": {"accept": true}
    }))
    .unwrap();
    let step = conn.start_command_dispatch(&raw);
    assert_eq!(
        complete_messages(step),
        vec![json!({
            "id": 40,
            "error": {"code": -32602, "message": "No dialog is showing"}
        })]
    );
}

#[test]
fn command_dispatch_reports_no_document_for_default_page_screenshot() {
    let mut conn = crate::testing::real_layout_test_connection();
    conn.browser_context = Some(BrowserContext::new("BID-page-shot".to_owned()));
    let raw = serde_json::to_string(&json!({
        "id": 41,
        "method": "Page.captureScreenshot"
    }))
    .unwrap();
    let step = conn.start_command_dispatch(&raw);
    assert_eq!(
        complete_messages(step),
        vec![json!({
            "id": 41,
            "error": {
                "code": -32000,
                "message": "NoDocumentLoaded"
            }
        })]
    );
}

#[test]
fn command_dispatch_preserves_page_screenshot_unsupported_for_mock_layout() {
    let mut conn = CdpConnection::new_with_initial_storage_partition_and_runtime_config(
        crate::CdpInitialStoragePartition::memory(),
        moli_core::runtime::NavigationRuntimeConfig::new(
            moli_fetch::FetchConfig::default(),
            moli_core::OptionalResourceFetchMask::NONE,
            true,
            moli_core::LayoutPolicy::Mock,
        ),
    );
    conn.browser_context = Some(BrowserContext::new("BID-page-shot-mock".to_owned()));
    let raw = serde_json::to_string(&json!({
        "id": 42,
        "method": "Page.captureScreenshot"
    }))
    .unwrap();
    let step = conn.start_command_dispatch(&raw);
    assert_eq!(
        complete_messages(step),
        vec![json!({
            "id": 42,
            "error": {
                "code": -32000,
                "message": "Page.captureScreenshot is not supported: renderer layout is disabled; start Moli with --layout."
            }
        })]
    );
}

#[test]
fn command_dispatch_completes_additional_page_sync_commands_without_legacy_fallback() {
    let mut conn = CdpConnection::new();
    let mut browser_context = BrowserContext::new("BID-page-sync".to_owned());
    browser_context.set_active_target_id("TID-page-sync");
    conn.install_browser_context_fixture_for_test(browser_context);

    let download_raw = serde_json::to_string(&json!({
        "id": 411,
        "method": "Page.setDownloadBehavior",
        "params": { "behavior": "allow", "downloadPath": "/tmp/moli-downloads" }
    }))
    .unwrap();
    let step = conn.start_command_dispatch(&download_raw);
    assert_eq!(
        complete_messages(step),
        vec![json!({ "id": 411, "result": {} })]
    );
    let settings = conn
        .download_behavior
        .effective_for_browser_context(Some("BID-page-sync"));
    assert_eq!(settings.behavior, "allow");

    let metrics_raw = serde_json::to_string(&json!({
        "id": 412,
        "method": "Page.getLayoutMetrics"
    }))
    .unwrap();
    let step = conn.start_command_dispatch(&metrics_raw);
    let messages = complete_messages(step);
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0]["id"], json!(412));
    assert_eq!(messages[0]["error"]["code"], -32000);
    assert_eq!(messages[0]["error"]["message"], "NoDocumentLoaded");

    let print_raw = serde_json::to_string(&json!({
        "id": 413,
        "method": "Page.printToPDF"
    }))
    .unwrap();
    let step = conn.start_command_dispatch(&print_raw);
    assert_eq!(
        complete_messages(step),
        vec![json!({
            "id": 413,
            "error": {
                "code": -32000,
                "message": "Page.printToPDF is not supported: renderer layout is disabled; start Moli with --layout."
            }
        })]
    );
}

#[test]
fn command_dispatch_migrates_page_navigation_and_termination_without_legacy_fallback() {
    let mut conn = CdpConnection::new();
    let navigate_raw = serde_json::to_string(&json!({
        "id": 413,
        "method": "Page.navigate",
        "params": { "url": "data:text/html,navigate" }
    }))
    .unwrap();
    let _navigate_step = conn.start_command_dispatch(&navigate_raw);

    for (method, params) in [
        ("Page.navigateToHistoryEntry", json!({"entryId": 1})),
        ("Page.reload", json!({})),
    ] {
        let raw = serde_json::to_string(&json!({
            "id": 414,
            "method": method,
            "params": params
        }))
        .unwrap();
        let _step = conn.start_command_dispatch(&raw);
    }

    for method in ["Page.crash", "Page.stopLoading", "Page.close"] {
        let raw = serde_json::to_string(&json!({ "id": 414, "method": method })).unwrap();
        let _step = conn.start_command_dispatch(&raw);
    }
}

#[test]
fn command_dispatch_completes_page_create_isolated_world_errors_without_legacy_fallback() {
    let mut conn = CdpConnection::new();
    let raw = serde_json::to_string(&json!({
        "id": 415,
        "method": "Page.createIsolatedWorld",
        "params": {
            "frameId": "TID-1",
            "worldName": "utility"
        }
    }))
    .unwrap();
    let step = conn.start_command_dispatch(&raw);
    assert_eq!(
        complete_messages(step),
        vec![json!({
            "id": 415,
            "error": {
                "code": -31998,
                "message": "BrowserContextNotLoaded"
            }
        })]
    );
}

#[test]
fn command_dispatch_completes_dom_sync_and_error_commands_without_legacy_fallback() {
    let mut conn = CdpConnection::new();

    let enable_raw = serde_json::to_string(&json!({
        "id": 421,
        "method": "DOM.enable"
    }))
    .unwrap();
    let step = conn.start_command_dispatch(&enable_raw);
    assert_eq!(
        complete_messages(step),
        vec![json!({ "id": 421, "result": {} })]
    );

    let get_document_raw = serde_json::to_string(&json!({
        "id": 422,
        "method": "DOM.getDocument"
    }))
    .unwrap();
    let step = conn.start_command_dispatch(&get_document_raw);
    assert_eq!(
        complete_messages(step),
        vec![json!({
            "id": 422,
            "error": {
                "code": -31998,
                "message": "BrowserContextNotLoaded"
            }
        })]
    );

    let discard_raw = serde_json::to_string(&json!({
        "id": 423,
        "method": "DOM.discardSearchResults",
        "params": { "searchId": "missing" }
    }))
    .unwrap();
    let step = conn.start_command_dispatch(&discard_raw);
    assert_eq!(
        complete_messages(step),
        vec![json!({ "id": 423, "result": {} })]
    );

    conn.browser_context = Some(BrowserContext::new("BID-dom-sync".to_owned()));
    let unknown_raw = serde_json::to_string(&json!({
        "id": 424,
        "method": "DOM.noSuchMethod"
    }))
    .unwrap();
    let step = conn.start_command_dispatch(&unknown_raw);
    assert_eq!(
        complete_messages(step),
        vec![json!({
            "id": 424,
            "error": {
                "code": -32601,
                "message": "UnknownMethod"
            }
        })]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn command_dispatch_completes_live_page_preload_without_legacy_fallback() {
    let mut ctx = crate::testing::TestContext::new();
    let mut browser_context = BrowserContext::new("BID-page-preload-live".to_owned());
    browser_context.set_active_target_id("TID-page-preload-live".to_owned());
    browser_context.attach_active_session("SID-page-preload-live");
    ctx.conn
        .install_browser_context_fixture_for_test(browser_context);
    let page = ctx
        .conn
        .load_page_via_runtime_async("data:text/html,<p>preload</p>")
        .await
        .expect("page should load");
    ctx.conn
        .browser_context
        .as_mut()
        .expect("browser context")
        .active_page_target_mut()
        .runtime_slot
        .set_loaded_page_for_test(page);

    let add_raw = serde_json::to_string(&json!({
        "id": 42,
        "method": "Page.addScriptToEvaluateOnNewDocument",
        "sessionId": "SID-page-preload-live",
        "params": {
            "source": "globalThis.__dispatchPreload = true;",
            "worldName": "__dispatch_world"
        }
    }))
    .unwrap();
    let add_pending = match ctx.conn.start_command_dispatch(&add_raw) {
        CdpCommandTaskStep::Pending(pending) => pending,
        CdpCommandTaskStep::Complete(_) => {
            panic!("live Page.addScriptToEvaluateOnNewDocument should update the live page")
        }
    };
    let (add_messages, _) = ctx
        .complete_command_task_step_for_test(CdpCommandTaskStep::Pending(add_pending))
        .await;
    assert_eq!(add_messages.len(), 1);
    assert_eq!(add_messages[0]["id"], json!(42));
    assert_eq!(add_messages[0]["sessionId"], json!("SID-page-preload-live"));
    let identifier = add_messages[0]["result"]["identifier"]
        .as_str()
        .expect("preload identifier")
        .to_owned();
    assert!(
        ctx.conn
            .browser_context
            .as_ref()
            .expect("browser context")
            .active_page_target()
            .owner_state
            .document_start_scripts
            .iter()
            .any(|(stored_id, script)| {
                stored_id == &identifier
                    && script.source == "globalThis.__dispatchPreload = true;"
                    && script.world_name.as_deref() == Some("__dispatch_world")
            }),
        "preload should be persisted on the owner state"
    );

    let remove_raw = serde_json::to_string(&json!({
        "id": 43,
        "method": "Page.removeScriptToEvaluateOnNewDocument",
        "sessionId": "SID-page-preload-live",
        "params": { "identifier": identifier }
    }))
    .unwrap();
    let remove_pending = match ctx.conn.start_command_dispatch(&remove_raw) {
        CdpCommandTaskStep::Pending(pending) => pending,
        CdpCommandTaskStep::Complete(_) => {
            panic!("live Page.removeScriptToEvaluateOnNewDocument should update the live page")
        }
    };
    let (remove_messages, _) = ctx
        .complete_command_task_step_for_test(CdpCommandTaskStep::Pending(remove_pending))
        .await;
    assert_eq!(
        remove_messages,
        vec![json!({
            "id": 43,
            "sessionId": "SID-page-preload-live",
            "result": {}
        })]
    );
    assert!(
        ctx.conn
            .browser_context
            .as_ref()
            .expect("browser context")
            .active_page_target()
            .owner_state
            .document_start_scripts
            .is_empty(),
        "preload removal should clear persisted owner state"
    );

    let create_world_raw = serde_json::to_string(&json!({
        "id": 44,
        "method": "Page.createIsolatedWorld",
        "sessionId": "SID-page-preload-live",
        "params": {
            "frameId": "TID-page-preload-live",
            "worldName": "__dispatch_world"
        }
    }))
    .unwrap();
    let create_world_pending = match ctx.conn.start_command_dispatch(&create_world_raw) {
        CdpCommandTaskStep::Pending(pending) => pending,
        CdpCommandTaskStep::Complete(_) => {
            panic!("live Page.createIsolatedWorld should use explicit pending page dispatch")
        }
    };
    let (create_world_messages, _) = ctx
        .complete_command_task_step_for_test(CdpCommandTaskStep::Pending(create_world_pending))
        .await;
    assert_eq!(create_world_messages.len(), 1);
    assert_eq!(create_world_messages[0]["id"], json!(44));
    assert_eq!(
        create_world_messages[0]["sessionId"],
        json!("SID-page-preload-live")
    );
    assert!(
        create_world_messages[0]["result"]["executionContextId"]
            .as_i64()
            .is_some(),
        "createIsolatedWorld should return an execution context id"
    );
}

#[test]
fn command_dispatch_completes_target_sync_commands() {
    let mut conn = CdpConnection::new();

    for (id, method) in [
        (50, "Target.createBrowserContext"),
        (51, "Target.getBrowserContexts"),
        (52, "Target.getTargets"),
        (53, "Target.attachToBrowserTarget"),
        (54, "Target.getTargetInfo"),
        (55, "Target.setDiscoverTargets"),
    ] {
        let params = match method {
            "Target.setDiscoverTargets" => json!({"discover": true}),
            _ => json!({}),
        };
        let raw = serde_json::to_string(&json!({ "id": id, "method": method, "params": params }))
            .unwrap();
        let step = conn.start_command_dispatch(&raw);
        let messages = complete_messages(step);
        assert!(
            messages.iter().any(
                |message| message.get("id").and_then(Value::as_u64) == Some(id)
                    && message.get("result").is_some()
            ),
            "{method} should emit a successful command response: {messages:?}"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn command_dispatch_completes_target_activate_without_legacy_fallback() {
    let mut conn = CdpConnection::new();
    let mut browser_context = BrowserContext::new("BID-target-activate".to_owned());
    browser_context.set_active_target_id("TID-target-activate".to_owned());
    conn.install_browser_context_fixture_for_test(browser_context);

    let raw = serde_json::to_string(&json!({
        "id": 56,
        "method": "Target.activateTarget",
        "params": { "targetId": "TID-target-activate" }
    }))
    .unwrap();
    let pending = match conn.start_command_dispatch(&raw) {
        CdpCommandTaskStep::Pending(pending) => pending,
        CdpCommandTaskStep::Complete(_) => {
            panic!("Target.activateTarget should use the Target pending dispatcher")
        }
    };
    assert_eq!(
        complete_command_task_for_test(&mut conn, *pending).await,
        vec![json!({ "id": 56, "result": {} })]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn command_dispatch_completes_target_set_auto_attach_without_legacy_fallback() {
    let mut conn = CdpConnection::new();
    let mut browser_context = BrowserContext::new("BID-target-auto-attach".to_owned());
    browser_context.set_active_target_id("TID-target-auto-attach".to_owned());
    conn.install_browser_context_fixture_for_test(browser_context);

    let raw = serde_json::to_string(&json!({
        "id": 57,
        "method": "Target.setAutoAttach",
        "params": {
            "autoAttach": true,
            "waitForDebuggerOnStart": false
        }
    }))
    .unwrap();
    let pending = match conn.start_command_dispatch(&raw) {
        CdpCommandTaskStep::Pending(pending) => pending,
        CdpCommandTaskStep::Complete(_) => {
            panic!("Target.setAutoAttach should use the Target pending dispatcher")
        }
    };
    let messages = complete_command_task_for_test(&mut conn, *pending).await;
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0]["method"], json!("Target.attachedToTarget"));
    assert_eq!(messages[0]["params"]["waitingForDebugger"], json!(false));
    assert!(
        messages[0]["params"]["sessionId"].as_str().is_some(),
        "auto attach should assign a session"
    );
    assert_eq!(messages[1], json!({ "id": 57, "result": {} }));
}

#[tokio::test(flavor = "multi_thread")]
async fn command_dispatch_completes_page_bring_to_front_without_legacy_fallback() {
    let mut conn = CdpConnection::new();
    let mut browser_context = BrowserContext::new("BID-page-bring".to_owned());
    browser_context.set_active_target_id("TID-page-bring".to_owned());
    browser_context.attach_active_session("SID-page-bring".to_owned());
    conn.install_browser_context_fixture_for_test(browser_context);

    let raw = serde_json::to_string(&json!({
        "id": 5701,
        "method": "Page.bringToFront",
        "sessionId": "SID-page-bring"
    }))
    .unwrap();
    let pending = match conn.start_command_dispatch(&raw) {
        CdpCommandTaskStep::Pending(pending) => pending,
        CdpCommandTaskStep::Complete(_) => {
            panic!("Page.bringToFront should use the Page pending dispatcher")
        }
    };
    assert_eq!(
        complete_command_task_for_test(&mut conn, *pending).await,
        vec![json!({
            "id": 5701,
            "result": {},
            "sessionId": "SID-page-bring"
        })]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn command_dispatch_completes_target_detach_without_legacy_fallback() {
    let mut conn = CdpConnection::new();
    let mut browser_context = BrowserContext::new("BID-target-detach".to_owned());
    browser_context.set_active_target_id("TID-target-detach".to_owned());
    browser_context.attach_active_session("SID-target-detach".to_owned());
    conn.install_browser_context_fixture_for_test(browser_context);

    let raw = serde_json::to_string(&json!({
        "id": 58,
        "method": "Target.detachFromTarget",
        "params": {
            "targetId": "TID-target-detach",
            "sessionId": "SID-target-detach"
        }
    }))
    .unwrap();
    let pending = match conn.start_command_dispatch(&raw) {
        CdpCommandTaskStep::Pending(pending) => pending,
        CdpCommandTaskStep::Complete(_) => {
            panic!("Target.detachFromTarget should use the Target pending dispatcher")
        }
    };
    let messages = complete_command_task_for_test(&mut conn, *pending).await;
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0], json!({ "id": 58, "result": {} }));
    assert_eq!(messages[1]["method"], json!("Target.detachedFromTarget"));
    assert_eq!(
        messages[1]["params"],
        json!({
            "targetId": "TID-target-detach",
            "sessionId": "SID-target-detach"
        })
    );
    assert!(
        !conn
            .browser_context
            .as_ref()
            .expect("browser context should remain loaded")
            .has_active_session()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn command_dispatch_completes_target_close_without_legacy_fallback() {
    let mut conn = CdpConnection::new();
    let mut browser_context = BrowserContext::new("BID-target-close".to_owned());
    browser_context.set_active_target_id("TID-target-close".to_owned());
    browser_context.attach_active_session("SID-target-close".to_owned());
    conn.install_browser_context_fixture_for_test(browser_context);

    let raw = serde_json::to_string(&json!({
        "id": 59,
        "method": "Target.closeTarget",
        "params": { "targetId": "TID-target-close" }
    }))
    .unwrap();
    let pending = match conn.start_command_dispatch(&raw) {
        CdpCommandTaskStep::Pending(pending) => pending,
        CdpCommandTaskStep::Complete(_) => {
            panic!("Target.closeTarget should use the Target pending dispatcher")
        }
    };
    let messages = complete_command_task_for_test(&mut conn, *pending).await;
    assert_eq!(messages.len(), 3);
    assert_eq!(
        messages[0],
        json!({ "id": 59, "result": { "success": true } })
    );
    assert_eq!(messages[1]["method"], json!("Inspector.detached"));
    assert_eq!(messages[1]["sessionId"], json!("SID-target-close"));
    assert_eq!(
        messages[1]["params"],
        json!({ "reason": "Render process gone." })
    );
    assert_eq!(messages[2]["method"], json!("Target.detachedFromTarget"));
    assert_eq!(
        messages[2]["params"],
        json!({
            "targetId": "TID-target-close",
            "sessionId": "SID-target-close"
        })
    );
    assert!(
        conn.browser_context
            .as_ref()
            .expect("browser context should remain loaded")
            .active_target_identity()
            .is_none(),
        "closing the selected target should leave the context without an active target"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn command_dispatch_completes_target_dispose_browser_context_without_legacy_fallback() {
    let mut conn = CdpConnection::new();
    let mut browser_context = BrowserContext::new("BID-target-dispose".to_owned());
    browser_context.set_active_target_id("TID-target-dispose".to_owned());
    browser_context.attach_active_session("SID-target-dispose".to_owned());
    conn.install_browser_context_fixture_for_test(browser_context);

    let raw = serde_json::to_string(&json!({
        "id": 60,
        "method": "Target.disposeBrowserContext",
        "params": { "browserContextId": "BID-target-dispose" }
    }))
    .unwrap();
    let pending = match conn.start_command_dispatch(&raw) {
        CdpCommandTaskStep::Pending(pending) => pending,
        CdpCommandTaskStep::Complete(_) => {
            panic!("Target.disposeBrowserContext should use the Target pending dispatcher")
        }
    };
    let messages = complete_command_task_for_test(&mut conn, *pending).await;
    assert_eq!(messages.len(), 3);
    assert_eq!(messages[0], json!({ "id": 60, "result": {} }));
    assert_eq!(messages[1]["method"], json!("Inspector.detached"));
    assert_eq!(
        messages[1]["params"],
        json!({ "reason": "Render process gone." })
    );
    assert_eq!(messages[1]["sessionId"], json!("SID-target-dispose"));
    assert_eq!(messages[2]["method"], json!("Target.detachedFromTarget"));
    assert_eq!(
        messages[2]["params"],
        json!({
            "targetId": "TID-target-dispose",
            "sessionId": "SID-target-dispose"
        })
    );
    assert!(
        conn.browser_context.is_none(),
        "disposing the active browser context should remove it"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn command_dispatch_completes_target_send_message_without_legacy_fallback() {
    let mut conn = CdpConnection::new();
    let mut browser_context = BrowserContext::new("BID-target-send".to_owned());
    browser_context.set_active_target_id("TID-target-send".to_owned());
    browser_context.attach_active_session("SID-target-send".to_owned());
    conn.install_browser_context_fixture_for_test(browser_context);

    let nested = serde_json::to_string(&json!({
        "id": 6101,
        "method": "Target.getBrowserContexts"
    }))
    .unwrap();
    let raw = serde_json::to_string(&json!({
        "id": 61,
        "method": "Target.sendMessageToTarget",
        "params": {
            "message": nested,
            "sessionId": "SID-target-send"
        }
    }))
    .unwrap();
    let pending = match conn.start_command_dispatch(&raw) {
        CdpCommandTaskStep::Pending(pending) => pending,
        CdpCommandTaskStep::Complete(_) => {
            panic!("Target.sendMessageToTarget should use the Target pending dispatcher")
        }
    };
    let messages = complete_command_task_for_test(&mut conn, *pending).await;
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0], json!({ "id": 61, "result": {} }));
    assert_eq!(
        messages[1]["method"],
        json!("Target.receivedMessageFromTarget")
    );
    assert_eq!(messages[1]["params"]["sessionId"], "SID-target-send");
    let nested: Value = serde_json::from_str(
        messages[1]["params"]["message"]
            .as_str()
            .expect("nested message should be stringified"),
    )
    .expect("nested message should be valid JSON");
    assert_eq!(nested["id"], 6101);
    assert_eq!(
        nested["result"]["browserContextIds"],
        json!(["BID-target-send"])
    );
}

#[test]
fn command_dispatch_completes_cookie_read_commands() {
    let mut conn = CdpConnection::new();
    conn.browser_context = Some(BrowserContext::new("BID-cookie".to_owned()));

    for (id, method) in [
        (60, "Storage.getCookies"),
        (61, "Storage.clearCookies"),
        (62, "Storage.deleteCookies"),
        (63, "Storage.setCookies"),
        (64, "Network.getCookies"),
        (65, "Network.getAllCookies"),
        (66, "Network.clearBrowserCookies"),
        (67, "Network.setCookie"),
        (68, "Network.setCookies"),
    ] {
        let params = match method {
            "Storage.deleteCookies" | "Network.deleteCookies" => json!({"name": "missing"}),
            "Network.setCookie" => json!({
                "name": "network_sid",
                "value": "1",
                "url": "https://example.com/app"
            }),
            "Storage.setCookies" | "Network.setCookies" => json!({
                "cookies": [{
                    "name": "sid",
                    "value": "1",
                    "url": "https://example.com/app"
                }]
            }),
            _ => json!({}),
        };
        let raw = serde_json::to_string(&json!({ "id": id, "method": method, "params": params }))
            .unwrap();
        let step = conn.start_command_dispatch(&raw);
        let messages = complete_messages(step);
        assert_eq!(messages.len(), 1, "{method} should emit one response");
        assert_eq!(messages[0]["id"], json!(id));
        assert!(
            messages[0].get("result").is_some(),
            "{method} should complete successfully: {:?}",
            messages[0]
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn command_dispatch_completes_live_storage_set_cookies_without_legacy_fallback() {
    let mut conn = CdpConnection::new();
    let mut browser_context = BrowserContext::new("BID-storage-live".to_owned());
    browser_context.set_active_target_id("TID-storage-live".to_owned());
    conn.install_browser_context_fixture_for_test(browser_context);
    let page = conn
        .load_page_via_runtime_async("data:text/html,<p>storage</p>")
        .await
        .expect("page should load");
    conn.browser_context
        .as_mut()
        .expect("browser context")
        .active_page_target_mut()
        .runtime_slot
        .set_loaded_page_for_test(page);

    let raw = serde_json::to_string(&json!({
        "id": 67,
        "method": "Storage.setCookies",
        "params": {
            "browserContextId": "BID-storage-live",
            "cookies": [{
                "name": "sid",
                "value": "1",
                "url": "https://example.com/app"
            }]
        }
    }))
    .unwrap();
    let pending = match conn.start_command_dispatch(&raw) {
        CdpCommandTaskStep::Pending(pending) => pending,
        CdpCommandTaskStep::Complete(_) => {
            panic!("live Storage.setCookies should snapshot the page cookie owner")
        }
    };
    let completed = pending.wait().await;
    let step = conn.complete_pending_command_dispatch(completed).await;
    let messages = complete_messages(step);
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0]["id"], json!(67));
    assert_eq!(messages[0]["result"]["success"], json!(true));
    assert_eq!(
        messages[0]["result"]["cookieReports"][0]["status"]["kind"],
        json!("Accepted")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn command_dispatch_completes_live_network_extra_headers_without_legacy_fallback() {
    let mut conn = CdpConnection::new();
    let mut browser_context = BrowserContext::new("BID-network-live".to_owned());
    browser_context.set_active_target_id("TID-network-live".to_owned());
    conn.install_browser_context_fixture_for_test(browser_context);
    let page = conn
        .load_page_via_runtime_async("data:text/html,<p>network</p>")
        .await
        .expect("page should load");
    conn.browser_context
        .as_mut()
        .expect("browser context")
        .active_page_target_mut()
        .runtime_slot
        .set_loaded_page_for_test(page);

    let raw = serde_json::to_string(&json!({
        "id": 68,
        "method": "Network.setExtraHTTPHeaders",
        "params": { "headers": { "x-dispatch-test": "ok" } }
    }))
    .unwrap();
    let pending = match conn.start_command_dispatch(&raw) {
        CdpCommandTaskStep::Pending(pending) => pending,
        CdpCommandTaskStep::Complete(_) => {
            panic!("live Network.setExtraHTTPHeaders should update the live page")
        }
    };
    let completed = pending.wait().await;
    let step = conn.complete_pending_command_dispatch(completed).await;
    assert_eq!(
        complete_messages(step),
        vec![json!({ "id": 68, "result": {} })]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn command_dispatch_completes_live_network_blocked_urls_without_legacy_fallback() {
    let mut conn = CdpConnection::new();
    let mut browser_context = BrowserContext::new("BID-network-blocked-live".to_owned());
    browser_context.set_active_target_id("TID-network-blocked-live".to_owned());
    conn.install_browser_context_fixture_for_test(browser_context);
    let page = conn
        .load_page_via_runtime_async("data:text/html,<p>network blocked</p>")
        .await
        .expect("page should load");
    conn.browser_context
        .as_mut()
        .expect("browser context")
        .active_page_target_mut()
        .runtime_slot
        .set_loaded_page_for_test(page);

    let raw = serde_json::to_string(&json!({
        "id": 681,
        "method": "Network.setBlockedURLs",
        "params": { "urls": ["*://blocked-dispatch.test/*"] }
    }))
    .unwrap();
    let pending = match conn.start_command_dispatch(&raw) {
        CdpCommandTaskStep::Pending(pending) => pending,
        CdpCommandTaskStep::Complete(_) => {
            panic!("live Network.setBlockedURLs should update the live page")
        }
    };
    let completed = pending.wait().await;
    let step = conn.complete_pending_command_dispatch(completed).await;
    assert_eq!(
        complete_messages(step),
        vec![json!({ "id": 681, "result": {} })]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn command_dispatch_completes_live_network_set_cookie_without_legacy_fallback() {
    let mut conn = CdpConnection::new();
    let mut browser_context = BrowserContext::new("BID-network-cookie-live".to_owned());
    browser_context.set_active_target_id("TID-network-cookie-live".to_owned());
    conn.install_browser_context_fixture_for_test(browser_context);
    let page = conn
        .load_page_via_runtime_async("data:text/html,<p>network cookie</p>")
        .await
        .expect("page should load");
    conn.browser_context
        .as_mut()
        .expect("browser context")
        .active_page_target_mut()
        .runtime_slot
        .set_loaded_page_for_test(page);

    let raw = serde_json::to_string(&json!({
        "id": 69,
        "method": "Network.setCookie",
        "params": {
            "name": "network_sid",
            "value": "1",
            "url": "https://example.com/app"
        }
    }))
    .unwrap();
    let pending = match conn.start_command_dispatch(&raw) {
        CdpCommandTaskStep::Pending(pending) => pending,
        CdpCommandTaskStep::Complete(_) => {
            panic!("live Network.setCookie should snapshot the page cookie owner")
        }
    };
    let completed = pending.wait().await;
    let step = conn.complete_pending_command_dispatch(completed).await;
    let messages = complete_messages(step);
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0]["id"], json!(69));
    assert_eq!(messages[0]["result"]["success"], json!(true));
    assert_eq!(
        messages[0]["result"]["cookieReports"][0]["status"]["kind"],
        json!("Accepted")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn command_dispatch_completes_live_network_emulation_without_legacy_fallback() {
    let mut conn = CdpConnection::new();
    let mut browser_context = BrowserContext::new("BID-network-emulated-live".to_owned());
    browser_context.set_active_target_id("TID-network-emulated-live".to_owned());
    conn.install_browser_context_fixture_for_test(browser_context);
    let page = conn
        .load_page_via_runtime_async("data:text/html,<p>network emulated</p>")
        .await
        .expect("page should load");
    conn.browser_context
        .as_mut()
        .expect("browser context")
        .active_page_target_mut()
        .runtime_slot
        .set_loaded_page_for_test(page);

    let raw = serde_json::to_string(&json!({
        "id": 682,
        "method": "Network.emulateNetworkConditions",
        "params": {
            "offline": true,
            "latency": 0,
            "downloadThroughput": -1,
            "uploadThroughput": -1,
        }
    }))
    .unwrap();
    let pending = match conn.start_command_dispatch(&raw) {
        CdpCommandTaskStep::Pending(pending) => pending,
        CdpCommandTaskStep::Complete(_) => {
            panic!("live Network.emulateNetworkConditions should update the live page")
        }
    };
    let completed = pending.wait().await;
    let step = conn.complete_pending_command_dispatch(completed).await;
    assert_eq!(
        complete_messages(step),
        vec![json!({ "id": 682, "result": {} })]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn command_dispatch_completes_live_network_user_agent_without_legacy_fallback() {
    let mut conn = CdpConnection::new();
    let mut browser_context = BrowserContext::new("BID-network-ua-live".to_owned());
    browser_context.set_active_target_id("TID-network-ua-live".to_owned());
    conn.install_browser_context_fixture_for_test(browser_context);
    let page = conn
        .load_page_via_runtime_async("data:text/html,<p>network ua</p>")
        .await
        .expect("page should load");
    conn.browser_context
        .as_mut()
        .expect("browser context")
        .active_page_target_mut()
        .runtime_slot
        .set_loaded_page_for_test(page);

    let raw = serde_json::to_string(&json!({
        "id": 683,
        "method": "Network.setUserAgentOverride",
        "params": { "userAgent": "MoliDispatchNetworkUA/1.0" }
    }))
    .unwrap();
    let pending = match conn.start_command_dispatch(&raw) {
        CdpCommandTaskStep::Pending(pending) => pending,
        CdpCommandTaskStep::Complete(_) => {
            panic!("live Network.setUserAgentOverride should update the live page loader")
        }
    };
    let completed = pending.wait().await;
    let step = conn.complete_pending_command_dispatch(completed).await;
    assert_eq!(
        complete_messages(step),
        vec![json!({ "id": 683, "result": {} })]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn command_dispatch_completes_live_emulation_user_agent_without_legacy_fallback() {
    let mut conn = CdpConnection::new();
    let mut browser_context = BrowserContext::new("BID-emulation-ua-live".to_owned());
    browser_context.set_active_target_id("TID-emulation-ua-live".to_owned());
    conn.install_browser_context_fixture_for_test(browser_context);
    let page = conn
        .load_page_via_runtime_async("data:text/html,<p>emulation ua</p>")
        .await
        .expect("page should load");
    conn.browser_context
        .as_mut()
        .expect("browser context")
        .active_page_target_mut()
        .runtime_slot
        .set_loaded_page_for_test(page);

    let raw = serde_json::to_string(&json!({
        "id": 684,
        "method": "Emulation.setUserAgentOverride",
        "params": { "userAgent": "MoliDispatchEmulationUA/1.0" }
    }))
    .unwrap();
    let pending = match conn.start_command_dispatch(&raw) {
        CdpCommandTaskStep::Pending(pending) => pending,
        CdpCommandTaskStep::Complete(_) => {
            panic!("live Emulation.setUserAgentOverride should update the live page loader")
        }
    };
    let completed = pending.wait().await;
    let step = conn.complete_pending_command_dispatch(completed).await;
    assert_eq!(
        complete_messages(step),
        vec![json!({ "id": 684, "result": {} })]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn command_dispatch_completes_live_emulation_locale_without_legacy_fallback() {
    let mut conn = CdpConnection::new();
    let mut browser_context = BrowserContext::new("BID-emulation-locale-live".to_owned());
    browser_context.set_active_target_id("TID-emulation-locale-live".to_owned());
    conn.install_browser_context_fixture_for_test(browser_context);
    let page = conn
        .load_page_via_runtime_async("data:text/html,<p>emulation locale</p>")
        .await
        .expect("page should load");
    conn.browser_context
        .as_mut()
        .expect("browser context")
        .active_page_target_mut()
        .runtime_slot
        .set_loaded_page_for_test(page);

    let raw = serde_json::to_string(&json!({
        "id": 685,
        "method": "Emulation.setLocaleOverride",
        "params": { "locale": "fr-FR" }
    }))
    .unwrap();
    assert_eq!(
        complete_messages(conn.start_command_dispatch(&raw)),
        vec![json!({ "id": 685, "result": {} })]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn command_dispatch_completes_live_security_tls_without_legacy_fallback() {
    let mut conn = CdpConnection::new();
    let mut browser_context = BrowserContext::new("BID-security-tls-live".to_owned());
    browser_context.set_active_target_id("TID-security-tls-live".to_owned());
    conn.install_browser_context_fixture_for_test(browser_context);
    let page = conn
        .load_page_via_runtime_async("data:text/html,<p>security tls</p>")
        .await
        .expect("page should load");
    conn.browser_context
        .as_mut()
        .expect("browser context")
        .active_page_target_mut()
        .runtime_slot
        .set_loaded_page_for_test(page);

    let raw = serde_json::to_string(&json!({
        "id": 686,
        "method": "Security.setIgnoreCertificateErrors",
        "params": { "ignore": true }
    }))
    .unwrap();
    let pending = match conn.start_command_dispatch(&raw) {
        CdpCommandTaskStep::Pending(pending) => pending,
        CdpCommandTaskStep::Complete(_) => {
            panic!("live Security.setIgnoreCertificateErrors should update the live page loader")
        }
    };
    let completed = pending.wait().await;
    let step = conn.complete_pending_command_dispatch(completed).await;
    assert_eq!(
        complete_messages(step),
        vec![json!({ "id": 686, "result": {} })]
    );
    assert!(!conn.tls_verify_host());
}

#[tokio::test(flavor = "multi_thread")]
async fn command_dispatch_completes_live_fetch_enable_without_legacy_fallback() {
    let mut ctx = crate::testing::TestContext::new();
    let mut browser_context = BrowserContext::new("BID-fetch-live".to_owned());
    browser_context.set_active_target_id("TID-fetch-live".to_owned());
    ctx.conn
        .install_browser_context_fixture_for_test(browser_context);
    let page = ctx
        .conn
        .load_page_via_runtime_async("data:text/html,<p>fetch</p>")
        .await
        .expect("page should load");
    ctx.conn
        .browser_context
        .as_mut()
        .expect("browser context")
        .active_page_target_mut()
        .runtime_slot
        .set_loaded_page_for_test(page);

    let raw = serde_json::to_string(&json!({
        "id": 69,
        "method": "Fetch.enable",
        "params": {
            "patterns": [{ "urlPattern": "*", "requestStage": "Request" }]
        }
    }))
    .unwrap();
    let pending = match ctx.conn.start_command_dispatch(&raw) {
        CdpCommandTaskStep::Pending(pending) => pending,
        CdpCommandTaskStep::Complete(_) => {
            panic!("live Fetch.enable should update live page interception state")
        }
    };
    let completed = pending.wait().await;
    let step = ctx.conn.complete_pending_command_dispatch(completed).await;
    let (messages, _) = ctx.complete_command_task_step_for_test(step).await;
    assert_eq!(messages, vec![json!({ "id": 69, "result": {} })]);
}

#[tokio::test(flavor = "multi_thread")]
async fn command_dispatch_completes_live_fetch_disable_without_legacy_fallback() {
    let mut ctx = crate::testing::TestContext::new();
    let mut browser_context = BrowserContext::new("BID-fetch-disable-live".to_owned());
    browser_context.set_active_target_id("TID-fetch-disable-live".to_owned());
    browser_context
        .active_page_target_mut()
        .fetch_owner
        .configure(None, true, Vec::new());
    ctx.conn
        .install_browser_context_fixture_for_test(browser_context);
    let page = ctx
        .conn
        .load_page_via_runtime_async("data:text/html,<p>fetch disable</p>")
        .await
        .expect("page should load");
    ctx.conn
        .browser_context
        .as_mut()
        .expect("browser context")
        .active_page_target_mut()
        .runtime_slot
        .set_loaded_page_for_test(page);

    let raw = serde_json::to_string(&json!({
        "id": 6901,
        "method": "Fetch.disable"
    }))
    .unwrap();
    let pending = match ctx.conn.start_command_dispatch(&raw) {
        CdpCommandTaskStep::Pending(pending) => pending,
        CdpCommandTaskStep::Complete(_) => {
            panic!("live Fetch.disable should clear live page interception state")
        }
    };
    let completed = pending.wait().await;
    let step = ctx.conn.complete_pending_command_dispatch(completed).await;
    let (messages, _) = ctx.complete_command_task_step_for_test(step).await;
    assert_eq!(messages, vec![json!({ "id": 6901, "result": {} })]);
    assert!(
        !ctx.conn
            .browser_context
            .as_ref()
            .expect("browser context should remain loaded")
            .active_page_target()
            .fetch_owner
            .is_enabled()
    );
}

#[test]
fn command_dispatch_completes_fetch_fulfill_request_without_legacy_fallback() {
    let mut conn = CdpConnection::new();

    let raw = serde_json::to_string(&json!({
        "id": 6902,
        "method": "Fetch.fulfillRequest",
        "params": {
            "requestId": "INT-6902",
            "responseCode": 204
        }
    }))
    .unwrap();
    let step = conn.start_command_dispatch(&raw);
    assert_eq!(
        complete_messages(step),
        vec![json!({
            "id": 6902,
            "error": {
                "code": -31998,
                "message": "BrowserContextNotLoaded"
            }
        })]
    );
}

#[test]
fn command_dispatch_completes_fetch_fail_request_without_legacy_fallback() {
    let mut conn = CdpConnection::new();

    let raw = serde_json::to_string(&json!({
        "id": 6903,
        "method": "Fetch.failRequest",
        "params": {
            "requestId": "INT-6903",
            "errorReason": "Aborted"
        }
    }))
    .unwrap();
    let step = conn.start_command_dispatch(&raw);
    assert_eq!(
        complete_messages(step),
        vec![json!({
            "id": 6903,
            "error": {
                "code": -31998,
                "message": "BrowserContextNotLoaded"
            }
        })]
    );
}

#[test]
fn command_dispatch_completes_fetch_websocket_commands_without_legacy_fallback() {
    let mut conn = CdpConnection::new();

    for (id, method, params) in [
        (
            6904,
            "Fetch.dispatchWebSocketMessage",
            json!({
                "requestId": "missing-websocket",
                "opcode": "text",
                "data": "hello"
            }),
        ),
        (
            6905,
            "Fetch.closeWebSocket",
            json!({
                "requestId": "missing-websocket",
                "code": 1000,
                "reason": "done"
            }),
        ),
    ] {
        let raw = serde_json::to_string(&json!({
            "id": id,
            "method": method,
            "params": params
        }))
        .unwrap();
        let step = conn.start_command_dispatch(&raw);
        assert_eq!(
            complete_messages(step),
            vec![json!({
                "id": id,
                "error": {
                    "code": -32000,
                    "message": "RequestNotFound"
                }
            })]
        );
    }
}

#[test]
fn command_dispatch_completes_fetch_body_commands_without_legacy_fallback() {
    let mut conn = CdpConnection::new();

    for (id, method, request_id) in [
        (6906, "Fetch.getResponseBody", "INT-6906"),
        (6907, "Fetch.takeResponseBodyAsStream", "INT-6907"),
    ] {
        let raw = serde_json::to_string(&json!({
            "id": id,
            "method": method,
            "params": { "requestId": request_id }
        }))
        .unwrap();
        let step = conn.start_command_dispatch(&raw);
        assert_eq!(
            complete_messages(step),
            vec![json!({
                "id": id,
                "error": {
                    "code": -31998,
                    "message": "BrowserContextNotLoaded"
                }
            })]
        );
    }
}

#[test]
fn command_dispatch_completes_fetch_continue_commands_without_legacy_fallback() {
    let mut conn = CdpConnection::new();

    for (id, method, request_id, extra_params) in [
        (6908, "Fetch.continueRequest", "INT-6908", json!({})),
        (
            6909,
            "Fetch.continueWithAuth",
            "INT-6909",
            json!({
                "authChallengeResponse": { "response": "Default" }
            }),
        ),
        (6910, "Fetch.continueResponse", "INT-6910", json!({})),
    ] {
        let mut params = serde_json::Map::new();
        params.insert("requestId".to_owned(), json!(request_id));
        if let Some(extra) = extra_params.as_object() {
            params.extend(extra.clone());
        }
        let raw = serde_json::to_string(&json!({
            "id": id,
            "method": method,
            "params": params
        }))
        .unwrap();
        let step = conn.start_command_dispatch(&raw);
        assert_eq!(
            complete_messages(step),
            vec![json!({
                "id": id,
                "error": {
                    "code": -31998,
                    "message": "BrowserContextNotLoaded"
                }
            })]
        );
    }
}

#[test]
fn command_dispatch_completes_fetch_unknown_method_without_legacy_fallback() {
    let mut conn = CdpConnection::new();

    let raw = serde_json::to_string(&json!({
        "id": 6911,
        "method": "Fetch.noSuchMethod",
        "params": {}
    }))
    .unwrap();
    let step = conn.start_command_dispatch(&raw);
    assert_eq!(
        complete_messages(step),
        vec![json!({
            "id": 6911,
            "error": {
                "code": -32601,
                "message": "UnknownMethod"
            }
        })]
    );
}

#[test]
fn command_dispatch_completes_shim_domains_without_legacy_fallback() {
    let mut conn = CdpConnection::new();

    for (id, method) in [
        (70, "Audits.enable"),
        (71, "Audits.disable"),
        (72, "SystemInfo.getInfo"),
        (73, "SystemInfo.getProcessInfo"),
        (74, "WebAuthn.enable"),
        (75, "WebAuthn.disable"),
        (76, "WebMCP.enable"),
        (77, "WebMCP.disable"),
    ] {
        let raw = serde_json::to_string(&json!({ "id": id, "method": method })).unwrap();
        let step = conn.start_command_dispatch(&raw);
        let messages = complete_messages(step);
        assert_eq!(messages.len(), 1, "{method} should emit one response");
        assert_eq!(messages[0]["id"], json!(id));
        assert!(
            messages[0].get("result").is_some(),
            "{method} should complete successfully: {:?}",
            messages[0]
        );
    }
}

#[test]
fn command_dispatch_completes_shim_domain_unknown_methods_without_legacy_fallback() {
    let mut conn = CdpConnection::new();

    for (id, method) in [
        (80, "Audits.noSuchMethod"),
        (81, "SystemInfo.noSuchMethod"),
        (82, "WebAuthn.noSuchMethod"),
        (83, "WebMCP.noSuchMethod"),
    ] {
        let raw = serde_json::to_string(&json!({ "id": id, "method": method })).unwrap();
        let step = conn.start_command_dispatch(&raw);
        assert_eq!(
            complete_messages(step),
            vec![json!({
                "id": id,
                "error": {"code": -32601, "message": "UnknownMethod"}
            })],
            "{method} should return UnknownMethod through the command dispatch entry"
        );
    }
}

#[test]
fn command_dispatch_completes_additional_sync_domains_without_legacy_fallback() {
    let mut conn = CdpConnection::new();

    for (id, method, expects_result) in [
        (90, "DOMSnapshot.enable", true),
        (91, "DOMSnapshot.disable", true),
        (92, "DOMSnapshot.captureSnapshot", false),
        (93, "Security.enable", true),
        (94, "Security.disable", true),
        (95, "Security.handleCertificateError", true),
        (96, "Security.setOverrideCertificateErrors", true),
        (97, "Network.clearBrowserCache", false),
        (98, "Network.getResponseBody", false),
        (981, "Network.getRequestPostData", false),
        (99, "IO.close", false),
        (100, "IO.read", false),
        (1003, "Accessibility.enable", true),
        (1004, "Accessibility.disable", true),
        (1005, "CSS.enable", true),
        (1006, "CSS.disable", true),
        (1007, "Runtime.disable", true),
        (1008, "Runtime.discardConsoleEntries", true),
    ] {
        let params = match method {
            "IO.close" | "IO.read" => json!({ "handle": "missing-stream" }),
            "Network.getResponseBody" | "Network.getRequestPostData" => {
                json!({ "requestId": "missing-request" })
            }
            _ => json!({}),
        };
        let raw = serde_json::to_string(&json!({ "id": id, "method": method, "params": params }))
            .unwrap();
        let step = conn.start_command_dispatch(&raw);
        let messages = complete_messages(step);
        assert_eq!(messages.len(), 1, "{method} should emit one response");
        assert_eq!(messages[0]["id"], json!(id));
        if expects_result {
            assert!(
                messages[0].get("result").is_some(),
                "{method} should complete successfully: {:?}",
                messages[0]
            );
        } else {
            assert!(
                messages[0].get("error").is_some(),
                "{method} should complete with a protocol error: {:?}",
                messages[0]
            );
        }
    }
}

#[test]
fn command_dispatch_completes_input_owner_commands_without_legacy_fallback() {
    let mut conn = crate::testing::real_layout_test_connection();
    conn.browser_context = Some(BrowserContext::new_with_page_for_test(
        "BID-input",
        "TID-input",
    ));

    let raw = serde_json::to_string(&json!({
        "id": 1001,
        "method": "Input.setInterceptDrags",
        "params": { "enabled": true }
    }))
    .unwrap();
    let step = conn.start_command_dispatch(&raw);
    assert_eq!(
        complete_messages(step),
        vec![json!({
            "id": 1001,
            "error": {
                "code": -32000,
                "message": crate::domains::input::SET_INTERCEPT_DRAGS_UNSUPPORTED_MESSAGE
            }
        })]
    );
    assert!(
        conn.browser_context
            .as_ref()
            .is_some_and(|context| !context.active_page_target().input_intercept_drags_enabled)
    );

    let raw = serde_json::to_string(&json!({
        "id": 1002,
        "method": "Input.dispatchDragEvent",
        "params": {
            "type": "drop",
            "x": 0,
            "y": 0,
            "data": { "items": [], "files": [], "dragOperationsMask": 0 }
        }
    }))
    .unwrap();
    let step = conn.start_command_dispatch(&raw);
    assert_eq!(
        complete_messages(step),
        vec![json!({
            "id": 1002,
            "error": {
                "code": -32000,
                "message": "NoDocumentLoaded"
            }
        })]
    );
}

#[test]
fn command_dispatch_completes_additional_sync_domain_unknown_methods_without_legacy_fallback() {
    let mut conn = CdpConnection::new();

    for (id, method) in [
        (101, "Browser.noSuchMethod"),
        (102, "Target.noSuchMethod"),
        (103, "DOMSnapshot.noSuchMethod"),
        (104, "Security.noSuchMethod"),
        (105, "IO.noSuchMethod"),
        (106, "Network.noSuchMethod"),
        (107, "Emulation.noSuchMethod"),
        (108, "Performance.noSuchMethod"),
        (109, "Input.noSuchMethod"),
        (110, "Accessibility.noSuchMethod"),
        (111, "CSS.noSuchMethod"),
        (112, "Runtime.noSuchMethod"),
        (113, "Page.noSuchMethod"),
    ] {
        let raw = serde_json::to_string(&json!({ "id": id, "method": method })).unwrap();
        let step = conn.start_command_dispatch(&raw);
        assert_eq!(
            complete_messages(step),
            vec![json!({
                "id": id,
                "error": {"code": -32601, "message": "UnknownMethod"}
            })],
            "{method} should return UnknownMethod through the command dispatch entry"
        );
    }
}
