use super::*;

#[tokio::test(flavor = "multi_thread")]
async fn same_context_background_session_can_stage_its_own_script_execution_disabled_before_activation()
 {
    let mut ctx = TestContext::new();
    load_bc_with_titled_page_async(
        &mut ctx,
        "BID-9-PRE-SCRIPT-DISABLED",
        "TID-000000000PS",
        "<title>active</title><div id='ok'>active target</div>",
    )
    .await;
    ctx.with_fixture(|conn| {
        conn.browser_context
            .as_mut()
            .unwrap()
            .attach_active_session("SID-active")
    });

    ctx.process_async(json!({
        "id": 104194920,
        "method": "Target.setAutoAttach",
        "params": { "autoAttach": true, "waitForDebuggerOnStart": false }
    }))
    .await;
    ctx.expect_result(104194920, json!({}), None);

    ctx.process_async(json!({
        "id": 104194921,
        "method": "Target.createTarget",
        "params": {
            "background": true, "browserContextId": "BID-9-PRE-SCRIPT-DISABLED", "url": "about:blank#second"}
    }))
    .await;
    let created = ctx.take_one();
    assert_eq!(created["method"], "Target.targetCreated");
    let second_target_id = created["params"]["targetInfo"]["targetId"]
        .as_str()
        .expect("second target id")
        .to_owned();
    let attached = ctx.take_one();
    assert_eq!(attached["method"], "Target.attachedToTarget");
    let second_session_id = attached["params"]["sessionId"]
        .as_str()
        .expect("second target session id")
        .to_owned();
    ctx.expect_result(104194921, json!({ "targetId": second_target_id }), None);

    ctx.process_async(json!({
        "id": 104194922,
        "method": "Emulation.setScriptExecutionDisabled",
        "sessionId": second_session_id,
        "params": { "value": true }
    }))
    .await;
    ctx.expect_result(104194922, json!({}), Some(&second_session_id));

    {
        ctx.with_fixture(|conn| {
            let staged = {
                let active = {
                    conn.browser_context
                        .as_ref()
                        .expect("active browser context")
                };
                assert!(
                    !active
                        .active_page_target()
                        .effective_emulation_state
                        .script_execution_disabled
                );

                active
                    .background_target(&second_target_id)
                    .filter(|target| target.has_non_default_session_state())
                    .expect("second target should have staged background page session state")
            };
            assert!(staged.effective_emulation_state.script_execution_disabled);
        });
    }

    ctx.process_async(json!({
            "id": 104194923,
            "method": "Page.navigate",
            "sessionId": "SID-active",
            "params": {
                "url": "data:text/html,<body><script>document.body.dataset.inlineRan='yes'; globalThis.__inlineRan = true;</script>active</body>"
            }
        })).await;
    let _ = take_response_by_id(&mut ctx, 104194923);
    assert!(
        ctx.sent
            .iter()
            .all(|message| message.get("error").is_none()),
        "unexpected protocol error during active navigation: {:?}",
        ctx.sent
    );
    ctx.take_all();

    ctx.process_async(json!({
            "id": 104194924,
            "method": "Runtime.evaluate",
            "sessionId": "SID-active",
            "params": {
                "expression": "JSON.stringify({ inlineRan: !!globalThis.__inlineRan, dataset: document.body.dataset.inlineRan || null })"
            }
        })).await;
    let active_eval = take_response_by_id(&mut ctx, 104194924);
    let active_payload = active_eval["result"]["result"]["value"]
        .as_str()
        .expect("active payload should be string");
    let active_payload: serde_json::Value =
        serde_json::from_str(active_payload).expect("active payload should be valid json");
    assert_eq!(active_payload["inlineRan"], json!(true));
    assert_eq!(active_payload["dataset"], json!("yes"));

    ctx.process_async(json!({
        "id": 104194925,
        "method": "Target.closeTarget",
        "params": {"targetId": "TID-000000000PS"}
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 104194925);
    ctx.take_all();

    ctx.process_async(json!({
            "id": 104194926,
            "method": "Page.navigate",
            "sessionId": second_session_id,
            "params": {
                "url": "data:text/html,<body><script>document.body.dataset.inlineRan='yes'; globalThis.__inlineRan = true;</script>activated</body>"
            }
        })).await;
    let _ = take_response_by_id(&mut ctx, 104194926);
    assert!(
        ctx.sent
            .iter()
            .all(|message| message.get("error").is_none()),
        "unexpected protocol error during activated navigation: {:?}",
        ctx.sent
    );
    ctx.take_all();

    {
        ctx.with_fixture(|conn| {
            let activated = {
                conn.browser_context
                    .as_ref()
                    .expect("activated browser context")
            };
            assert!(
                activated
                    .active_page_target()
                    .effective_emulation_state
                    .script_execution_disabled
            );
        });
    }

    ctx.process_async(json!({
            "id": 104194927,
            "method": "Runtime.evaluate",
            "sessionId": second_session_id,
            "params": {
                "expression": "JSON.stringify({ inlineRan: !!globalThis.__inlineRan, dataset: document.body.dataset.inlineRan || null, runtimeEvalStillWorks: 1 + 1 })"
            }
        })).await;
    let activated_eval = take_response_by_id(&mut ctx, 104194927);
    let activated_payload = activated_eval["result"]["result"]["value"]
        .as_str()
        .expect("activated payload should be string");
    let activated_payload: serde_json::Value =
        serde_json::from_str(activated_payload).expect("activated payload should be valid json");
    assert_eq!(activated_payload["inlineRan"], json!(false));
    assert_eq!(activated_payload["dataset"], serde_json::Value::Null);
    assert_eq!(activated_payload["runtimeEvalStillWorks"], json!(2));
}

#[tokio::test(flavor = "multi_thread")]
async fn same_context_background_session_can_reenable_its_own_script_execution_before_activation() {
    let mut ctx = TestContext::new();
    load_bc_with_titled_page_async(
        &mut ctx,
        "BID-9-PRE-SCRIPT-REENABLE",
        "TID-000000000PSE",
        "<title>active</title><div id='ok'>active target</div>",
    )
    .await;
    ctx.with_fixture(|conn| {
        conn.browser_context
            .as_mut()
            .unwrap()
            .attach_active_session("SID-active")
    });

    ctx.process_async(json!({
        "id": 1041949270,
        "method": "Target.setAutoAttach",
        "params": { "autoAttach": true, "waitForDebuggerOnStart": false }
    }))
    .await;
    ctx.expect_result(1041949270, json!({}), None);

    ctx.process_async(json!({
        "id": 1041949271,
        "method": "Target.createTarget",
        "params": {
            "background": true, "browserContextId": "BID-9-PRE-SCRIPT-REENABLE", "url": "about:blank#second"}
    }))
    .await;
    let created = ctx.take_one();
    assert_eq!(created["method"], "Target.targetCreated");
    let second_target_id = created["params"]["targetInfo"]["targetId"]
        .as_str()
        .expect("second target id")
        .to_owned();
    let attached = ctx.take_one();
    assert_eq!(attached["method"], "Target.attachedToTarget");
    let second_session_id = attached["params"]["sessionId"]
        .as_str()
        .expect("second target session id")
        .to_owned();
    ctx.expect_result(1041949271, json!({ "targetId": second_target_id }), None);

    ctx.process_async(json!({
        "id": 1041949272,
        "method": "Emulation.setScriptExecutionDisabled",
        "sessionId": second_session_id,
        "params": { "value": true }
    }))
    .await;
    ctx.expect_result(1041949272, json!({}), Some(&second_session_id));

    ctx.process_async(json!({
        "id": 1041949273,
        "method": "Emulation.setScriptExecutionDisabled",
        "sessionId": second_session_id,
        "params": { "value": false }
    }))
    .await;
    ctx.expect_result(1041949273, json!({}), Some(&second_session_id));

    {
        ctx.with_fixture(|conn| {
            let active = {
                conn.browser_context
                    .as_ref()
                    .expect("active browser context")
            };
            assert!(
                !active
                    .active_page_target()
                    .effective_emulation_state
                    .script_execution_disabled
            );
            // A completed renderer call may retain its monotonic correlation
            // allocator in the background session; only the effective setting must
            // collapse back to the default.
            assert!(
                active
                    .background_target(&second_target_id)
                    .filter(|target| target.has_non_default_session_state())
                    .is_none_or(|state| {
                        !state.effective_emulation_state.script_execution_disabled
                    }),
                "script execution re-enable should clear the staged background setting: {:#?}",
                active
                    .background_target(&second_target_id)
                    .filter(|target| target.has_non_default_session_state())
            );
        });
    }

    ctx.process_async(json!({
            "id": 1041949274,
            "method": "Page.navigate",
            "sessionId": "SID-active",
            "params": {
                "url": "data:text/html,<body><script>document.body.dataset.inlineRan='yes'; globalThis.__inlineRan = true;</script>active</body>"
            }
        })).await;
    let _ = take_response_by_id(&mut ctx, 1041949274);
    assert!(
        ctx.sent
            .iter()
            .all(|message| message.get("error").is_none()),
        "unexpected protocol error during active navigation: {:?}",
        ctx.sent
    );
    ctx.take_all();

    ctx.process_async(json!({
            "id": 1041949275,
            "method": "Runtime.evaluate",
            "sessionId": "SID-active",
            "params": {
                "expression": "JSON.stringify({ inlineRan: !!globalThis.__inlineRan, dataset: document.body.dataset.inlineRan || null })"
            }
        })).await;
    let active_eval = take_response_by_id(&mut ctx, 1041949275);
    let active_payload = active_eval["result"]["result"]["value"]
        .as_str()
        .expect("active payload should be string");
    let active_payload: serde_json::Value =
        serde_json::from_str(active_payload).expect("active payload should be valid json");
    assert_eq!(active_payload["inlineRan"], json!(true));
    assert_eq!(active_payload["dataset"], json!("yes"));

    ctx.process_async(json!({
        "id": 1041949276,
        "method": "Target.closeTarget",
        "params": {"targetId": "TID-000000000PSE"}
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 1041949276);
    ctx.take_all();

    ctx.process_async(json!({
            "id": 1041949277,
            "method": "Page.navigate",
            "sessionId": second_session_id,
            "params": {
                "url": "data:text/html,<body><script>document.body.dataset.inlineRan='yes'; globalThis.__inlineRan = true;</script>activated</body>"
            }
        })).await;
    let _ = take_response_by_id(&mut ctx, 1041949277);
    assert!(
        ctx.sent
            .iter()
            .all(|message| message.get("error").is_none()),
        "unexpected protocol error during activated navigation: {:?}",
        ctx.sent
    );
    ctx.take_all();

    {
        ctx.with_fixture(|conn| {
            let activated = {
                conn.browser_context
                    .as_ref()
                    .expect("activated browser context")
            };
            assert!(
                !activated
                    .active_page_target()
                    .effective_emulation_state
                    .script_execution_disabled
            );
        });
    }

    ctx.process_async(json!({
            "id": 1041949278,
            "method": "Runtime.evaluate",
            "sessionId": second_session_id,
            "params": {
                "expression": "JSON.stringify({ inlineRan: !!globalThis.__inlineRan, dataset: document.body.dataset.inlineRan || null, runtimeEvalStillWorks: 1 + 1 })"
            }
        })).await;
    let activated_eval = take_response_by_id(&mut ctx, 1041949278);
    let activated_payload = activated_eval["result"]["result"]["value"]
        .as_str()
        .expect("activated payload should be string");
    let activated_payload: serde_json::Value =
        serde_json::from_str(activated_payload).expect("activated payload should be valid json");
    assert_eq!(activated_payload["inlineRan"], json!(true));
    assert_eq!(activated_payload["dataset"], json!("yes"));
    assert_eq!(activated_payload["runtimeEvalStillWorks"], json!(2));
}

#[tokio::test(flavor = "multi_thread")]
async fn same_context_background_session_can_stage_its_own_lifecycle_events_before_activation() {
    let mut ctx = TestContext::new();
    load_bc_with_titled_page_async(
        &mut ctx,
        "BID-9-PRE-LIFECYCLE",
        "TID-000000000PY",
        "<title>active</title><div id='ok'>active target</div>",
    )
    .await;
    ctx.with_fixture(|conn| {
        conn.browser_context
            .as_mut()
            .unwrap()
            .attach_active_session("SID-active")
    });

    ctx.process_async(json!({
        "id": 104194930,
        "method": "Target.setAutoAttach",
        "params": { "autoAttach": true, "waitForDebuggerOnStart": false }
    }))
    .await;
    ctx.expect_result(104194930, json!({}), None);

    ctx.process_async(json!({
        "id": 104194931,
        "method": "Target.createTarget",
        "params": {
            "background": true, "browserContextId": "BID-9-PRE-LIFECYCLE", "url": "about:blank#second"}
    }))
    .await;
    let created = ctx.take_one();
    assert_eq!(created["method"], "Target.targetCreated");
    let second_target_id = created["params"]["targetInfo"]["targetId"]
        .as_str()
        .expect("second target id")
        .to_owned();
    let attached = ctx.take_one();
    assert_eq!(attached["method"], "Target.attachedToTarget");
    let second_session_id = attached["params"]["sessionId"]
        .as_str()
        .expect("second target session id")
        .to_owned();
    ctx.expect_result(104194931, json!({ "targetId": second_target_id }), None);

    ctx.process_async(json!({
        "id": 104194932,
        "method": "Page.enable",
        "sessionId": second_session_id
    }))
    .await;
    ctx.expect_result(104194932, json!({}), Some(&second_session_id));

    ctx.process_async(json!({
        "id": 104194933,
        "method": "Page.setLifecycleEventsEnabled",
        "sessionId": second_session_id,
        "params": { "enabled": true }
    }))
    .await;
    ctx.expect_result(104194933, json!({}), Some(&second_session_id));

    {
        ctx.with_fixture(|conn| {
            let staged = {
                let active = {
                    conn.browser_context
                        .as_ref()
                        .expect("active browser context")
                };
                assert!(
                    !active.active_page_target().devtools_sessions
                        [moli_page_types::DevToolsSessionKey::Primary]
                        .page_session_state
                        .page_lifecycle_events
                );

                active
                    .background_target(&second_target_id)
                    .filter(|target| target.has_non_default_session_state())
                    .expect("second target should have staged background page session state")
            };
            assert!(
                staged.devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
                    .page_session_state
                    .page_lifecycle_events
            );
        });
    }

    ctx.process_async(json!({
        "id": 104194934,
        "method": "Page.navigate",
        "sessionId": "SID-active",
        "params": {
            "url": "data:text/html,<title>page-a</title><div id='ok'>page a</div>"
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 104194934);
    assert!(
        ctx.sent
            .iter()
            .all(|message| message.get("error").is_none()),
        "unexpected protocol error during active navigation: {:?}",
        ctx.sent
    );
    assert!(
        !ctx.sent
            .iter()
            .any(|message| message["method"] == json!("Page.lifecycleEvent")
                && message["sessionId"] == json!("SID-active")),
        "active target should not emit lifecycle events before activation: {:?}",
        ctx.sent
    );
    ctx.take_all();

    ctx.process_async(json!({
        "id": 104194935,
        "method": "Target.closeTarget",
        "params": {"targetId": "TID-000000000PY"}
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 104194935);
    ctx.take_all();

    ctx.process_async(json!({
        "id": 104194936,
        "method": "Page.navigate",
        "sessionId": second_session_id,
        "params": {
            "url": "data:text/html,<title>page-b</title><div id='ok'>page b</div>"
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 104194936);
    crate::testing::wait_until_scheduler_message(
        &mut ctx,
        "activated target networkIdle lifecycle event",
        |message| {
            message["method"] == json!("Page.lifecycleEvent")
                && message["sessionId"] == json!(second_session_id)
                && message["params"]["frameId"] == json!(second_target_id)
                && message["params"]["name"] == json!("networkIdle")
        },
    )
    .await;
    assert!(
        ctx.sent
            .iter()
            .all(|message| message.get("error").is_none()),
        "unexpected protocol error during activated navigation: {:?}",
        ctx.sent
    );

    let lifecycle_events = ctx
        .sent
        .iter()
        .filter(|message| {
            message["method"] == json!("Page.lifecycleEvent")
                && message["sessionId"] == json!(second_session_id)
                && message["params"]["frameId"] == json!(second_target_id)
        })
        .map(|message| {
            message["params"]["name"]
                .as_str()
                .expect("lifecycle name should be string")
                .to_owned()
        })
        .collect::<Vec<_>>();
    assert_eq!(
        lifecycle_events,
        vec![
            "init".to_owned(),
            "DOMContentLoaded".to_owned(),
            "load".to_owned(),
            "networkAlmostIdle".to_owned(),
            "networkIdle".to_owned(),
        ]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn same_context_background_session_can_disable_its_own_lifecycle_events_before_activation() {
    let mut ctx = TestContext::new();
    load_bc_with_titled_page_async(
        &mut ctx,
        "BID-9-PRE-LIFECYCLE-DISABLE",
        "TID-000000000PYD",
        "<title>active</title><div id='ok'>active target</div>",
    )
    .await;
    ctx.with_fixture(|conn| {
        conn.browser_context
            .as_mut()
            .unwrap()
            .attach_active_session("SID-active")
    });

    ctx.process_async(json!({
        "id": 1041949320,
        "method": "Target.setAutoAttach",
        "params": { "autoAttach": true, "waitForDebuggerOnStart": false }
    }))
    .await;
    ctx.expect_result(1041949320, json!({}), None);

    ctx.process_async(json!({
        "id": 1041949321,
        "method": "Target.createTarget",
        "params": {
            "background": true, "browserContextId": "BID-9-PRE-LIFECYCLE-DISABLE", "url": "about:blank#second"}
    }))
    .await;
    let created = ctx.take_one();
    assert_eq!(created["method"], "Target.targetCreated");
    let second_target_id = created["params"]["targetInfo"]["targetId"]
        .as_str()
        .expect("second target id")
        .to_owned();
    let attached = ctx.take_one();
    assert_eq!(attached["method"], "Target.attachedToTarget");
    let second_session_id = attached["params"]["sessionId"]
        .as_str()
        .expect("second target session id")
        .to_owned();
    ctx.expect_result(1041949321, json!({ "targetId": second_target_id }), None);

    ctx.process_async(json!({
        "id": 1041949322,
        "method": "Page.setLifecycleEventsEnabled",
        "sessionId": second_session_id,
        "params": { "enabled": true }
    }))
    .await;
    ctx.expect_result(1041949322, json!({}), Some(&second_session_id));

    ctx.process_async(json!({
        "id": 1041949323,
        "method": "Page.setLifecycleEventsEnabled",
        "sessionId": second_session_id,
        "params": { "enabled": false }
    }))
    .await;
    ctx.expect_result(1041949323, json!({}), Some(&second_session_id));

    {
        ctx.with_fixture(|conn| {
            let active = {
                conn.browser_context
                    .as_ref()
                    .expect("active browser context")
            };
            assert!(
                !active.active_page_target().devtools_sessions
                    [moli_page_types::DevToolsSessionKey::Primary]
                    .page_session_state
                    .page_lifecycle_events
            );
            assert!(
                active
                    .background_target(&second_target_id)
                    .filter(|target| target.has_non_default_session_state())
                    .is_none(),
                "lifecycle disable should collapse staged background state back to default"
            );
        });
    }

    ctx.process_async(json!({
        "id": 1041949324,
        "method": "Page.navigate",
        "sessionId": "SID-active",
        "params": {
            "url": "data:text/html,<title>page-a</title><div id='ok'>page a</div>"
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 1041949324);
    assert!(
        ctx.sent
            .iter()
            .all(|message| message.get("error").is_none()),
        "unexpected protocol error during active navigation: {:?}",
        ctx.sent
    );
    assert!(
        !ctx.sent
            .iter()
            .any(|message| message["method"] == json!("Page.lifecycleEvent")
                && message["sessionId"] == json!("SID-active")),
        "active target should not emit lifecycle events before activation: {:?}",
        ctx.sent
    );
    ctx.take_all();

    ctx.process_async(json!({
        "id": 1041949325,
        "method": "Target.closeTarget",
        "params": {"targetId": "TID-000000000PYD"}
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 1041949325);
    ctx.take_all();

    ctx.process_async(json!({
        "id": 1041949326,
        "method": "Page.navigate",
        "sessionId": second_session_id,
        "params": {
            "url": "data:text/html,<title>page-b</title><div id='ok'>page b</div>"
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 1041949326);
    assert!(
        ctx.sent
            .iter()
            .all(|message| message.get("error").is_none()),
        "unexpected protocol error during activated navigation: {:?}",
        ctx.sent
    );
    assert!(
        !ctx.sent.iter().any(|message| {
            message["method"] == json!("Page.lifecycleEvent")
                && message["sessionId"] == json!(second_session_id)
                && message["params"]["frameId"] == json!(second_target_id)
        }),
        "disabled lifecycle events should not emit on first activated navigation: {:?}",
        ctx.sent
    );
}
