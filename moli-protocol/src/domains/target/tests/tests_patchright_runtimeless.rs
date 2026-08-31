use super::*;

fn run_patchright_runtimeless_large_stack<F, Fut>(thread_name: &str, future_factory: F)
where
    F: FnOnce() -> Fut + Send + 'static,
    Fut: std::future::Future<Output = ()> + 'static,
{
    let result = std::thread::Builder::new()
        .name(thread_name.to_owned())
        .stack_size(32 * 1024 * 1024)
        .spawn(move || {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("large-stack patchright runtimeless test runtime should build")
                .block_on(future_factory());
        })
        .expect("large-stack patchright runtimeless test thread should spawn")
        .join();

    if let Err(payload) = result {
        std::panic::resume_unwind(payload);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn patchright_over_cdp_switching_back_to_first_context_keeps_older_page_runtime_and_utility_world_alive_without_runtime_enable()
 {
    let mut ctx = TestContext::new();
    let first =
        create_attached_page_session_without_runtime_enable_async(&mut ctx, 23910, 23911, 23912)
            .await;

    ctx.process_async(json!({
        "id": 23913,
        "method": "Page.addScriptToEvaluateOnNewDocument",
        "sessionId": first.session_id,
        "params": {
            "source": "globalThis.__lm_reactivated_utility_preload = 'first-preload';",
            "worldName": "utility"
        }
    }))
    .await;
    let preload = take_response_by_id(&mut ctx, 23913);
    assert_eq!(preload["sessionId"], json!(first.session_id));
    assert!(preload["result"]["identifier"].is_string());

    ctx.process_async(json!({
        "id": 23914,
        "method": "Runtime.addBinding",
        "sessionId": first.session_id,
        "params": {
            "name": "reactivatedUtilityBinding",
            "executionContextName": "utility"
        }
    }))
    .await;
    let add_binding = take_response_by_id(&mut ctx, 23914);
    assert_eq!(add_binding["result"], json!({}));
    assert!(
        !ctx.sent.iter().any(|message| {
            message["method"] == json!("Runtime.executionContextCreated")
                || message["method"] == json!("Runtime.executionContextsCleared")
        }),
        "Patchright-style setup should stay off Runtime.enable surfaces: {:?}",
        ctx.sent
    );
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 23915,
        "method": "Page.navigate",
        "sessionId": first.session_id,
        "params": {
            "url": "data:text/html,<body><div id=page>first-page</div></body>"
        }
    }))
    .await;
    let first_navigation = take_response_by_id(&mut ctx, 23915);
    assert_eq!(first_navigation["sessionId"], json!(first.session_id));
    assert!(
        ctx.sent.iter().all(|message| {
            message.get("error").is_none()
                && message["method"] != json!("Runtime.executionContextCreated")
                && message["method"] != json!("Runtime.executionContextsCleared")
        }),
        "unexpected protocol/runtime event during first navigation: {:?}",
        ctx.sent
    );
    ctx.take_all();

    let second =
        create_attached_page_session_without_runtime_enable_async(&mut ctx, 23916, 23917, 23918)
            .await;

    ctx.process_async(json!({
        "id": 23919,
        "method": "Page.navigate",
        "sessionId": second.session_id,
        "params": {
            "url": "data:text/html,<body><div id=page>second-page</div></body>"
        }
    }))
    .await;
    let second_navigation = take_response_by_id(&mut ctx, 23919);
    assert_eq!(second_navigation["sessionId"], json!(second.session_id));
    assert!(
        ctx.sent.iter().all(|message| {
            message.get("error").is_none()
                && message["method"] != json!("Runtime.executionContextCreated")
                && message["method"] != json!("Runtime.executionContextsCleared")
        }),
        "unexpected protocol/runtime event during second navigation: {:?}",
        ctx.sent
    );
    ctx.take_all();

    ctx.process_async(json!({
        "id": 23920,
        "method": "Target.attachToTarget",
        "params": { "targetId": first.target_id, "flatten": true }
    }))
    .await;
    let reattached_session_id = take_response_by_id(&mut ctx, 23920)["result"]["sessionId"]
        .as_str()
        .expect("reattached first session id")
        .to_owned();
    assert_ne!(reattached_session_id, first.session_id);
    ctx.take_first_matching("first target reattached", |message| {
        message["method"] == json!("Target.attachedToTarget")
            && message["params"]["sessionId"] == json!(reattached_session_id)
    });
    assert_eq!(
        ctx.conn.browser_context.as_ref().map(|bc| bc.id.as_str()),
        Some(first.browser_context_id.as_str()),
        "reattaching the first target should switch the active browser context back"
    );
    ctx.process_async(json!({
        "id": 23921,
        "method": "Runtime.evaluate",
        "sessionId": first.session_id,
        "params": {
            "expression": "document.querySelector('#page').textContent"
        }
    }))
    .await;
    let first_main_world_eval = take_response_by_id(&mut ctx, 23921);
    assert_eq!(
        first_main_world_eval["result"]["result"]["value"],
        json!("first-page")
    );

    ctx.process_async(json!({
        "id": 23922,
        "method": "Page.createIsolatedWorld",
        "sessionId": first.session_id,
        "params": {
            "frameId": first.target_id,
            "worldName": "utility"
        }
    }))
    .await;
    let first_utility_context =
        take_response_by_id(&mut ctx, 23922)["result"]["executionContextId"]
            .as_i64()
            .expect("reactivated first utility context id");
    ctx.take_all();

    ctx.process_async(json!({
        "id": 23923,
        "method": "Runtime.evaluate",
        "sessionId": first.session_id,
        "params": {
            "contextId": first_utility_context,
            "expression": "reactivatedUtilityBinding('payload-reactivated-first'); JSON.stringify([typeof globalThis.reactivatedUtilityBinding, globalThis.__lm_reactivated_utility_preload])"
        }
    })).await;
    let first_utility_eval = take_response_by_id(&mut ctx, 23923);
    assert_eq!(
        first_utility_eval["result"]["result"]["value"],
        json!("[\"function\",\"first-preload\"]")
    );
    let reactivated_binding_called = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Runtime.bindingCalled")
                && message["params"]["name"] == json!("reactivatedUtilityBinding")
        })
        .cloned()
        .expect("reactivated first utility world should emit bindingCalled");
    assert_eq!(
        reactivated_binding_called["params"]["executionContextId"],
        json!(first_utility_context)
    );
    assert_eq!(
        reactivated_binding_called["params"]["payload"],
        json!("payload-reactivated-first")
    );

    let first_context = ctx
        .conn
        .browser_contexts()
        .find(|bc| bc.id == first.browser_context_id)
        .expect("first browser context should still exist");
    assert_eq!(
        first_context.active_target_id(),
        Some(first.target_id.as_str())
    );
    assert_eq!(
        first_context.active_session_id(),
        Some(first.session_id.as_str())
    );
    assert_eq!(
        first_context
            .active_target
            .owner_state
            .document_start_scripts
            .len(),
        1
    );
    assert!(
        first_context.devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
            .runtime_bindings
            .iter()
            .any(|binding| {
                binding.name == "reactivatedUtilityBinding"
                    && binding.execution_context_name.as_deref() == Some("utility")
            }),
        "first browser context should retain its utility-world binding definition after being reactivated"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn patchright_over_cdp_reactivated_first_context_cleanup_updates_current_and_future_utility_worlds_without_runtime_enable()
 {
    let mut ctx = TestContext::new();
    let first =
        create_attached_page_session_without_runtime_enable_async(&mut ctx, 23924, 23925, 23926)
            .await;

    ctx.process_async(json!({
        "id": 23927,
        "method": "Runtime.addBinding",
        "sessionId": first.session_id,
        "params": {
            "name": "reactivationCleanupBinding",
            "executionContextName": "utility"
        }
    }))
    .await;
    let add_binding = take_response_by_id(&mut ctx, 23927);
    assert_eq!(add_binding["result"], json!({}));
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 23928,
        "method": "Page.navigate",
        "sessionId": first.session_id,
        "params": {
            "url": "data:text/html,<body><div id=page>cleanup-first</div></body>"
        }
    }))
    .await;
    let first_navigation = take_response_by_id(&mut ctx, 23928);
    assert_eq!(first_navigation["sessionId"], json!(first.session_id));
    ctx.take_all();

    ctx.process_async(json!({
        "id": 23929,
        "method": "Page.createIsolatedWorld",
        "sessionId": first.session_id,
        "params": {
            "frameId": first.target_id,
            "worldName": "utility"
        }
    }))
    .await;
    let first_utility_context =
        take_response_by_id(&mut ctx, 23929)["result"]["executionContextId"]
            .as_i64()
            .expect("first utility context id");
    ctx.take_all();

    ctx.process_async(json!({
        "id": 23930,
        "method": "Page.addScriptToEvaluateOnNewDocument",
        "sessionId": first.session_id,
        "params": {
            "source": "globalThis.__lm_reactivated_cleanup_marker = 'current-world-kept';",
            "worldName": "utility",
            "runImmediately": true
        }
    }))
    .await;
    let preload_identifier = take_response_by_id(&mut ctx, 23930)["result"]["identifier"]
        .as_str()
        .expect("preload identifier")
        .to_owned();

    ctx.process_async(json!({
            "id": 23931,
            "method": "Runtime.evaluate",
            "sessionId": first.session_id,
            "params": {
                "contextId": first_utility_context,
                "expression": "JSON.stringify([typeof globalThis.reactivationCleanupBinding, globalThis.__lm_reactivated_cleanup_marker])"
            }
        })).await;
    let seeded_current_world = take_response_by_id(&mut ctx, 23931);
    assert_eq!(
        seeded_current_world["result"]["result"]["value"],
        json!("[\"function\",\"current-world-kept\"]")
    );

    let second =
        create_attached_page_session_without_runtime_enable_async(&mut ctx, 23932, 23933, 23934)
            .await;
    ctx.process_async(json!({
        "id": 23935,
        "method": "Page.navigate",
        "sessionId": second.session_id,
        "params": {
            "url": "data:text/html,<body><div id=page>cleanup-second</div></body>"
        }
    }))
    .await;
    let second_navigation = take_response_by_id(&mut ctx, 23935);
    assert_eq!(second_navigation["sessionId"], json!(second.session_id));
    ctx.take_all();

    ctx.process_async(json!({
        "id": 23936,
        "method": "Target.attachToTarget",
        "params": { "targetId": first.target_id, "flatten": true }
    }))
    .await;
    let reattached_session_id = take_response_by_id(&mut ctx, 23936)["result"]["sessionId"]
        .as_str()
        .expect("reattached first session id")
        .to_owned();
    assert_ne!(reattached_session_id, first.session_id);
    ctx.take_first_matching("first target reattached", |message| {
        message["method"] == json!("Target.attachedToTarget")
            && message["params"]["sessionId"] == json!(reattached_session_id)
    });

    ctx.process_async(json!({
        "id": 23937,
        "method": "Runtime.removeBinding",
        "sessionId": first.session_id,
        "params": { "name": "reactivationCleanupBinding" }
    }))
    .await;
    let remove_binding = take_response_by_id(&mut ctx, 23937);
    assert_eq!(remove_binding["result"], json!({}));

    ctx.process_async(json!({
        "id": 23938,
        "method": "Page.removeScriptToEvaluateOnNewDocument",
        "sessionId": first.session_id,
        "params": { "identifier": preload_identifier }
    }))
    .await;
    let remove_preload = take_response_by_id(&mut ctx, 23938);
    assert_eq!(remove_preload["result"], json!({}));

    ctx.process_async(json!({
        "id": 23939,
        "method": "Runtime.evaluate",
        "sessionId": first.session_id,
        "params": {
            "contextId": first_utility_context,
            "expression": "JSON.stringify([typeof globalThis.reactivationCleanupBinding, globalThis.__lm_reactivated_cleanup_marker])"
        }
    })).await;
    let current_world_after_cleanup = take_response_by_id(&mut ctx, 23939);
    assert_eq!(
        current_world_after_cleanup["result"]["result"]["value"],
        json!("[\"undefined\",\"current-world-kept\"]")
    );
    assert!(
        ctx.sent.iter().all(|message| {
            message["method"] != json!("Runtime.bindingCalled")
                || message["params"]["name"] != json!("reactivationCleanupBinding")
        }),
        "removed binding should not fire after reactivated current-world cleanup"
    );
    ctx.take_all();

    ctx.process_async(json!({
        "id": 23940,
        "method": "Page.navigate",
        "sessionId": first.session_id,
        "params": {
            "url": "data:text/html,<body><div id=page>cleanup-reactivated</div></body>"
        }
    }))
    .await;
    let renavigation = take_response_by_id(&mut ctx, 23940);
    assert_eq!(renavigation["sessionId"], json!(first.session_id));
    ctx.take_all();

    ctx.process_async(json!({
        "id": 23941,
        "method": "Page.createIsolatedWorld",
        "sessionId": first.session_id,
        "params": {
            "frameId": first.target_id,
            "worldName": "utility"
        }
    }))
    .await;
    let second_utility_context =
        take_response_by_id(&mut ctx, 23941)["result"]["executionContextId"]
            .as_i64()
            .expect("reactivated utility context id");
    ctx.take_all();

    ctx.process_async(json!({
        "id": 23942,
        "method": "Runtime.evaluate",
        "sessionId": first.session_id,
        "params": {
            "contextId": second_utility_context,
            "expression": "JSON.stringify([typeof globalThis.reactivationCleanupBinding, globalThis.__lm_reactivated_cleanup_marker ?? null])"
        }
    })).await;
    let fresh_world_after_cleanup = take_response_by_id(&mut ctx, 23942);
    assert_eq!(
        fresh_world_after_cleanup["result"]["result"]["value"],
        json!("[\"undefined\",null]")
    );
    assert!(
        ctx.sent.iter().all(|message| {
            message["method"] != json!("Runtime.bindingCalled")
                || message["params"]["name"] != json!("reactivationCleanupBinding")
        }),
        "removed binding should stay absent in future utility worlds"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn patchright_over_cdp_crpage_cleanup_state_persists_when_auto_attach_reattaches_existing_target_with_mixed_binding_kinds_without_runtime_enable()
 {
    run_patchright_runtimeless_large_stack(
        "patchright-runtimeless-auto-reattach-existing-target",
        || async {
            let mut ctx = TestContext::new();
            let attached = create_attached_page_session_without_runtime_enable_async(
                &mut ctx, 35194, 35195, 35196,
            )
            .await;

            ctx.process_async(json!({
        "id": 35197,
        "method": "Page.navigate",
        "sessionId": attached.session_id,
        "params": {
            "url": "data:text/html,<body><div id='main-handle-b'>main-handle-b</div><div id='utility-handle-b'>utility-handle-b</div></body>"
        }
    })).await;
            let navigation = take_response_by_id(&mut ctx, 35197);
            assert_eq!(navigation["sessionId"], json!(attached.session_id));
            ctx.take_all();

            ctx.process_async(json!({
                "id": 35198,
                "method": "Page.createIsolatedWorld",
                "sessionId": attached.session_id,
                "params": {
                    "frameId": attached.target_id,
                    "worldName": "utility"
                }
            }))
            .await;
            let initial_utility_context =
                take_response_by_id(&mut ctx, 35198)["result"]["executionContextId"]
                    .as_i64()
                    .expect("initial utility context id");
            ctx.take_all();

            let custom_wrapper_a_source = patchright_page_binding_wrapper_source(
                "customBindingA",
                "__lm_custom_binding_a_deliver",
                None,
                false,
            );
            let custom_wrapper_b_source = patchright_page_binding_wrapper_source(
                "customBindingB",
                "__lm_custom_binding_b_deliver",
                None,
                false,
            );
            let custom_handle_wrapper_a_source = patchright_page_binding_wrapper_source(
                "customHandleBindingA",
                "__lm_custom_handle_binding_a_deliver",
                Some("__lm_custom_handle_binding_a_take"),
                true,
            );
            let custom_handle_wrapper_b_source = patchright_page_binding_wrapper_source(
                "customHandleBindingB",
                "__lm_custom_handle_binding_b_deliver",
                Some("__lm_custom_handle_binding_b_take"),
                true,
            );
            let retained_wrapper_source = patchright_page_binding_wrapper_source(
                "__pw_keptBinding",
                "__lm_pw_kept_binding_deliver",
                None,
                false,
            );
            let retained_handle_wrapper_source = patchright_page_binding_wrapper_source(
                "__pw_keptHandleBinding",
                "__lm_pw_kept_handle_binding_deliver",
                Some("__lm_pw_kept_handle_binding_take"),
                true,
            );

            for (id, binding_name, source) in [
                (
                    35199_u64,
                    "customBindingA",
                    custom_wrapper_a_source.as_str(),
                ),
                (
                    35203_u64,
                    "customBindingB",
                    custom_wrapper_b_source.as_str(),
                ),
                (
                    35207_u64,
                    "customHandleBindingA",
                    custom_handle_wrapper_a_source.as_str(),
                ),
                (
                    35211_u64,
                    "customHandleBindingB",
                    custom_handle_wrapper_b_source.as_str(),
                ),
                (
                    35215_u64,
                    "__pw_keptBinding",
                    retained_wrapper_source.as_str(),
                ),
                (
                    35219_u64,
                    "__pw_keptHandleBinding",
                    retained_handle_wrapper_source.as_str(),
                ),
            ] {
                install_patchright_crpage_binding_in_existing_worlds_async(
                    &mut ctx,
                    &attached.session_id,
                    initial_utility_context,
                    id,
                    id + 1,
                    id + 2,
                    id + 3,
                    binding_name,
                    source,
                )
                .await;
            }

            for (id, binding_name) in [
                (35223_u64, "customBindingA"),
                (35224_u64, "customHandleBindingA"),
            ] {
                ctx.process_async(json!({
                    "id": id,
                    "method": "Runtime.removeBinding",
                    "sessionId": attached.session_id,
                    "params": { "name": binding_name }
                }))
                .await;
                assert_eq!(take_response_by_id(&mut ctx, id)["result"], json!({}));
            }

            ctx.process_async(json!({
                "id": 35225,
                "method": "Target.detachFromTarget",
                "params": {
                    "targetId": attached.target_id,
                    "sessionId": attached.session_id
                }
            }))
            .await;
            ctx.expect_result(35225, json!({}), None);
            ctx.expect_event(
                "Target.detachedFromTarget",
                Some(&json!({
                    "targetId": attached.target_id,
                    "sessionId": attached.session_id,
                })),
            );
            assert_eq!(
                ctx.conn
                    .browser_contexts()
                    .find(|bc| bc.id == attached.browser_context_id)
                    .and_then(|bc| bc.active_session_id()),
                None
            );

            ctx.process_async(json!({
                "id": 35226,
                "method": "Target.setAutoAttach",
                "params": {
                    "autoAttach": true,
                    "waitForDebuggerOnStart": false
                }
            }))
            .await;
            ctx.expect_result(35226, json!({}), None);
            let auto_attach_event = ctx
                .take_first_matching("reattached target attachedToTarget", |message| {
                    message["method"] == json!("Target.attachedToTarget")
                });
            assert_eq!(
                auto_attach_event["params"]["targetInfo"]["targetId"],
                json!(attached.target_id)
            );
            let auto_attached_session_id = auto_attach_event["params"]["sessionId"]
                .as_str()
                .expect("auto-attached session id")
                .to_owned();
            assert_ne!(auto_attached_session_id, attached.session_id);
            ctx.take_all();

            ctx.process_async(json!({
        "id": 35227,
        "method": "Runtime.evaluate",
        "sessionId": auto_attached_session_id,
        "params": {
            "expression": "JSON.stringify([typeof globalThis.customBindingA, typeof globalThis.customBindingB, typeof globalThis.customHandleBindingA, typeof globalThis.customHandleBindingB, typeof globalThis.__pw_keptBinding, typeof globalThis.__pw_keptHandleBinding])"
        }
    })).await;
            let main_state = take_response_by_id(&mut ctx, 35227);
            assert_eq!(
                main_state["result"]["result"]["value"],
                json!(
                    "[\"undefined\",\"function\",\"undefined\",\"function\",\"function\",\"function\"]"
                )
            );

            ctx.process_async(json!({
                "id": 35228,
                "method": "Page.createIsolatedWorld",
                "sessionId": auto_attached_session_id,
                "params": {
                    "frameId": attached.target_id,
                    "worldName": "utility"
                }
            }))
            .await;
            let reattached_utility_context =
                take_response_by_id(&mut ctx, 35228)["result"]["executionContextId"]
                    .as_i64()
                    .expect("reattached utility context id");
            ctx.take_all();

            for (id, binding_name) in [
                (35229_u64, "customBindingB"),
                (35230_u64, "customHandleBindingB"),
                (35231_u64, "__pw_keptBinding"),
                (35232_u64, "__pw_keptHandleBinding"),
            ] {
                ctx.process_async(json!({
            "id": id,
            "method": "Runtime.addBinding",
            "sessionId": auto_attached_session_id,
            "params": { "name": binding_name, "executionContextId": reattached_utility_context }
        }))
        .await;
                assert_eq!(take_response_by_id(&mut ctx, id)["result"], json!({}));
            }

            for (id, source, expected_type) in [
                (35233_u64, custom_wrapper_a_source.as_str(), "undefined"),
                (35234_u64, custom_wrapper_b_source.as_str(), "function"),
                (
                    35235_u64,
                    custom_handle_wrapper_a_source.as_str(),
                    "undefined",
                ),
                (
                    35236_u64,
                    custom_handle_wrapper_b_source.as_str(),
                    "function",
                ),
                (35237_u64, retained_wrapper_source.as_str(), "function"),
                (
                    35238_u64,
                    retained_handle_wrapper_source.as_str(),
                    "function",
                ),
            ] {
                ctx.process_async(json!({
                    "id": id,
                    "method": "Runtime.evaluate",
                    "sessionId": auto_attached_session_id,
                    "params": {
                        "contextId": reattached_utility_context,
                        "expression": source,
                        "awaitPromise": true
                    }
                }))
                .await;
                let replayed = take_response_by_id(&mut ctx, id);
                assert_eq!(replayed["result"]["result"]["value"], json!(expected_type));
            }

            ctx.process_async(json!({
        "id": 35239,
        "method": "Runtime.evaluate",
        "sessionId": auto_attached_session_id,
        "params": {
            "contextId": reattached_utility_context,
            "expression": "JSON.stringify([typeof globalThis.customBindingA, typeof globalThis.customBindingB, typeof globalThis.customHandleBindingA, typeof globalThis.customHandleBindingB, typeof globalThis.__pw_keptBinding, typeof globalThis.__pw_keptHandleBinding])"
        }
    })).await;
            let utility_state = take_response_by_id(&mut ctx, 35239);
            assert_eq!(
                utility_state["result"]["result"]["value"],
                json!(
                    "[\"undefined\",\"function\",\"undefined\",\"function\",\"function\",\"function\"]"
                )
            );

            ctx.process_async(json!({
        "id": 35240,
        "method": "Runtime.evaluate",
        "sessionId": auto_attached_session_id,
        "params": {
            "expression": "globalThis.__lm_auto_reattach_custom_b = customBindingB({ source: 'after-auto-attach-custom-b', nested: { count: 1, values: ['a', 2, true] } }); 'scheduled-custom-b'",
            "awaitPromise": true
        }
    })).await;
            let scheduled_custom = take_response_by_id(&mut ctx, 35240);
            assert!(
                scheduled_custom["result"]["result"]["value"]
                    .as_str()
                    .expect("scheduled custom value")
                    .starts_with("scheduled-")
            );
            let custom_binding_called = ctx
                .sent
                .iter()
                .rev()
                .find(|message| {
                    message["method"] == json!("Runtime.bindingCalled")
                        && message["sessionId"] == json!(auto_attached_session_id)
                        && message["params"]["name"] == json!("customBindingB")
                })
                .cloned()
                .expect("custom binding should emit Runtime.bindingCalled after auto-attach");
            let custom_payload = custom_binding_called["params"]["payload"]
                .as_str()
                .expect("custom binding payload should be string");
            let custom_payload: serde_json::Value = serde_json::from_str(custom_payload)
                .expect("custom binding payload should be valid json");
            assert_eq!(custom_payload["name"], json!("customBindingB"));
            assert_eq!(custom_payload["seq"], json!(1));
            ctx.sent.clear();

            ctx.process_async(json!({
        "id": 35241,
        "method": "Runtime.evaluate",
        "sessionId": auto_attached_session_id,
        "params": {
            "expression": "globalThis.__lm_custom_binding_b_deliver({ name: 'customBindingB', seq: 1, result: 'after-auto-attach-custom-b-ok' }); 'delivered'",
            "awaitPromise": true
        }
    })).await;
            let delivered_custom = take_response_by_id(&mut ctx, 35241);
            assert_eq!(
                delivered_custom["result"]["result"]["value"],
                json!("delivered")
            );

            ctx.process_async(json!({
                "id": 35242,
                "method": "Runtime.evaluate",
                "sessionId": auto_attached_session_id,
                "params": {
                    "expression": "globalThis.__lm_auto_reattach_custom_b",
                    "awaitPromise": true
                }
            }))
            .await;
            let resolved_custom = take_response_by_id(&mut ctx, 35242);
            assert_eq!(
                resolved_custom["result"]["result"]["value"],
                json!("after-auto-attach-custom-b-ok")
            );

            ctx.process_async(json!({
        "id": 35243,
        "method": "Runtime.evaluate",
        "sessionId": auto_attached_session_id,
        "params": {
            "contextId": reattached_utility_context,
            "expression": "globalThis.__lm_auto_reattach_pw_handle = __pw_keptHandleBinding(document.getElementById('utility-handle-b')); 'scheduled-pw-handle'",
            "awaitPromise": true
        }
    })).await;
            let scheduled_handle = take_response_by_id(&mut ctx, 35243);
            assert!(
                scheduled_handle["result"]["result"]["value"]
                    .as_str()
                    .expect("scheduled handle value")
                    .starts_with("scheduled-")
            );
            let handle_binding_called = ctx
                .sent
                .iter()
                .rev()
                .find(|message| {
                    message["method"] == json!("Runtime.bindingCalled")
                        && message["sessionId"] == json!(auto_attached_session_id)
                        && message["params"]["name"] == json!("__pw_keptHandleBinding")
                        && message["params"]["executionContextId"]
                            == json!(reattached_utility_context)
                })
                .cloned()
                .expect(
                    "retained handle binding should emit Runtime.bindingCalled after auto-attach",
                );
            let handle_payload = handle_binding_called["params"]["payload"]
                .as_str()
                .expect("handle binding payload should be string");
            let handle_payload: serde_json::Value = serde_json::from_str(handle_payload)
                .expect("handle binding payload should be valid json");
            let handle_seq = handle_payload["seq"]
                .as_i64()
                .expect("handle payload seq should be integer");
            assert_eq!(handle_seq, 1);
            ctx.sent.clear();

            ctx.process_async(json!({
        "id": 35244,
        "method": "Runtime.evaluate",
        "sessionId": auto_attached_session_id,
        "params": {
            "contextId": reattached_utility_context,
            "expression": format!(
                "(() => {{ const handle = globalThis.__lm_pw_kept_handle_binding_take({{ name: '__pw_keptHandleBinding', seq: {handle_seq} }}); return JSON.stringify([handle.id, handle.textContent, typeof globalThis.__lm_pw_kept_handle_binding_take({{ name: '__pw_keptHandleBinding', seq: {handle_seq} }})]); }})()"
            )
        }
    })).await;
            let taken_handle = take_response_by_id(&mut ctx, 35244);
            assert_eq!(
                taken_handle["result"]["result"]["value"],
                json!("[\"utility-handle-b\",\"utility-handle-b\",\"undefined\"]")
            );

            ctx.process_async(json!({
        "id": 35245,
        "method": "Runtime.evaluate",
        "sessionId": auto_attached_session_id,
        "params": {
            "contextId": reattached_utility_context,
            "expression": format!(
                "globalThis.__lm_pw_kept_handle_binding_deliver({{ name: '__pw_keptHandleBinding', seq: {handle_seq}, result: 'after-auto-attach-pw-handle-ok' }}); 'delivered'"
            ),
            "awaitPromise": true
        }
    })).await;
            let delivered_handle = take_response_by_id(&mut ctx, 35245);
            assert_eq!(
                delivered_handle["result"]["result"]["value"],
                json!("delivered")
            );

            ctx.process_async(json!({
                "id": 35246,
                "method": "Runtime.evaluate",
                "sessionId": auto_attached_session_id,
                "params": {
                    "contextId": reattached_utility_context,
                    "expression": "globalThis.__lm_auto_reattach_pw_handle",
                    "awaitPromise": true
                }
            }))
            .await;
            let resolved_handle = take_response_by_id(&mut ctx, 35246);
            assert_eq!(
                resolved_handle["result"]["result"]["value"],
                json!("after-auto-attach-pw-handle-ok")
            );

            ctx.process_async(json!({
        "id": 35247,
        "method": "Runtime.evaluate",
        "sessionId": auto_attached_session_id,
        "params": {
            "expression": "globalThis.__lm_auto_reattach_custom_b_reject = customBindingB({ source: 'after-auto-attach-custom-b-reject', nested: { count: 2, values: ['b', 3, false] } }).then(() => 'unexpected-success', error => `rejected:${error}`); 'scheduled-custom-b-reject'",
            "awaitPromise": true
        }
    })).await;
            let scheduled_custom_reject = take_response_by_id(&mut ctx, 35247);
            assert!(
                scheduled_custom_reject["result"]["result"]["value"]
                    .as_str()
                    .expect("scheduled custom reject value")
                    .starts_with("scheduled-")
            );
            let custom_reject_binding_called = ctx
        .sent
        .iter()
        .rev()
        .find(|message| {
            message["method"] == json!("Runtime.bindingCalled")
                && message["sessionId"] == json!(auto_attached_session_id)
                && message["params"]["name"] == json!("customBindingB")
        })
        .cloned()
        .expect("custom binding should emit Runtime.bindingCalled for rejection after auto-attach");
            let custom_reject_payload = custom_reject_binding_called["params"]["payload"]
                .as_str()
                .expect("custom reject binding payload should be string");
            let custom_reject_payload: serde_json::Value =
                serde_json::from_str(custom_reject_payload)
                    .expect("custom reject binding payload should be valid json");
            assert_eq!(custom_reject_payload["name"], json!("customBindingB"));
            assert_eq!(custom_reject_payload["seq"], json!(2));
            ctx.sent.clear();

            ctx.process_async(json!({
        "id": 35248,
        "method": "Runtime.evaluate",
        "sessionId": auto_attached_session_id,
        "params": {
            "expression": "globalThis.__lm_custom_binding_b_deliver({ name: 'customBindingB', seq: 2, error: 'after-auto-attach-custom-b-error' }); 'delivered'",
            "awaitPromise": true
        }
    })).await;
            let delivered_custom_reject = take_response_by_id(&mut ctx, 35248);
            assert_eq!(
                delivered_custom_reject["result"]["result"]["value"],
                json!("delivered")
            );

            ctx.process_async(json!({
                "id": 35249,
                "method": "Runtime.evaluate",
                "sessionId": auto_attached_session_id,
                "params": {
                    "expression": "globalThis.__lm_auto_reattach_custom_b_reject",
                    "awaitPromise": true
                }
            }))
            .await;
            let rejected_custom = take_response_by_id(&mut ctx, 35249);
            assert_eq!(
                rejected_custom["result"]["result"]["value"],
                json!("rejected:after-auto-attach-custom-b-error")
            );

            ctx.process_async(json!({
        "id": 35250,
        "method": "Runtime.evaluate",
        "sessionId": auto_attached_session_id,
        "params": {
            "contextId": reattached_utility_context,
            "expression": "globalThis.__lm_auto_reattach_pw_handle_reject = __pw_keptHandleBinding(document.getElementById('utility-handle-b')).then(() => 'unexpected-success', error => `rejected:${error}`); 'scheduled-pw-handle-reject'",
            "awaitPromise": true
        }
    })).await;
            let scheduled_handle_reject = take_response_by_id(&mut ctx, 35250);
            assert!(
                scheduled_handle_reject["result"]["result"]["value"]
                    .as_str()
                    .expect("scheduled handle reject value")
                    .starts_with("scheduled-")
            );
            let handle_reject_binding_called = ctx
        .sent
        .iter()
        .rev()
        .find(|message| {
            message["method"] == json!("Runtime.bindingCalled")
                && message["sessionId"] == json!(auto_attached_session_id)
                && message["params"]["name"] == json!("__pw_keptHandleBinding")
                && message["params"]["executionContextId"] == json!(reattached_utility_context)
        })
        .cloned()
        .expect("retained handle binding should emit Runtime.bindingCalled for rejection after auto-attach");
            let handle_reject_payload = handle_reject_binding_called["params"]["payload"]
                .as_str()
                .expect("handle reject binding payload should be string");
            let handle_reject_payload: serde_json::Value =
                serde_json::from_str(handle_reject_payload)
                    .expect("handle reject binding payload should be valid json");
            let handle_reject_seq = handle_reject_payload["seq"]
                .as_i64()
                .expect("handle reject payload seq should be integer");
            assert_eq!(handle_reject_seq, 2);
            ctx.sent.clear();

            ctx.process_async(json!({
        "id": 35251,
        "method": "Runtime.evaluate",
        "sessionId": auto_attached_session_id,
        "params": {
            "contextId": reattached_utility_context,
            "expression": format!(
                "(() => {{ const handle = globalThis.__lm_pw_kept_handle_binding_take({{ name: '__pw_keptHandleBinding', seq: {handle_reject_seq} }}); return JSON.stringify([handle.id, handle.textContent, typeof globalThis.__lm_pw_kept_handle_binding_take({{ name: '__pw_keptHandleBinding', seq: {handle_reject_seq} }})]); }})()"
            )
        }
    })).await;
            let taken_handle_reject = take_response_by_id(&mut ctx, 35251);
            assert_eq!(
                taken_handle_reject["result"]["result"]["value"],
                json!("[\"utility-handle-b\",\"utility-handle-b\",\"undefined\"]")
            );

            ctx.process_async(json!({
        "id": 35252,
        "method": "Runtime.evaluate",
        "sessionId": auto_attached_session_id,
        "params": {
            "contextId": reattached_utility_context,
            "expression": format!(
                "globalThis.__lm_pw_kept_handle_binding_deliver({{ name: '__pw_keptHandleBinding', seq: {handle_reject_seq}, error: 'after-auto-attach-pw-handle-error' }}); 'delivered'"
            ),
            "awaitPromise": true
        }
    })).await;
            let delivered_handle_reject = take_response_by_id(&mut ctx, 35252);
            assert_eq!(
                delivered_handle_reject["result"]["result"]["value"],
                json!("delivered")
            );

            ctx.process_async(json!({
                "id": 35253,
                "method": "Runtime.evaluate",
                "sessionId": auto_attached_session_id,
                "params": {
                    "contextId": reattached_utility_context,
                    "expression": "globalThis.__lm_auto_reattach_pw_handle_reject",
                    "awaitPromise": true
                }
            }))
            .await;
            let rejected_handle = take_response_by_id(&mut ctx, 35253);
            assert_eq!(
                rejected_handle["result"]["result"]["value"],
                json!("rejected:after-auto-attach-pw-handle-error")
            );
        },
    );
}
