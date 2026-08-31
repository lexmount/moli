use super::*;

#[tokio::test(flavor = "multi_thread")]
async fn patchright_over_cdp_auto_attach_sweep_replacement_targets_keep_thin_mixed_name_handle_cleanup_isolated_per_browser_context_without_runtime_enable()
 {
    patchright_replacement_targets_large_stack(|| async {
    let mut ctx = TestContext::new();
    let first =
        create_attached_page_session_without_runtime_enable_async(&mut ctx, 36501, 36502, 36503)
            .await;
    let second =
        create_attached_page_session_without_runtime_enable_async(&mut ctx, 36504, 36505, 36506)
            .await;

    for (id, session_id, html) in [
        (
            36507_u64,
            first.session_id.as_str(),
            "<body><div id='utility-handle-a'>thin-first-mixed-name-handle</div></body>",
        ),
        (
            36508_u64,
            second.session_id.as_str(),
            "<body><div id='utility-handle-a'>thin-second-mixed-name-handle</div></body>",
        ),
    ] {
        ctx.process_async(json!({
            "id": id,
            "method": "Page.navigate",
            "sessionId": session_id,
            "params": { "url": format!("data:text/html,{html}") }
        }))
        .await;
        let navigation = take_response_by_id(&mut ctx, id);
        assert_eq!(navigation["sessionId"], json!(session_id));
        ctx.take_all();
    }

    let mut first_utility_context = 0_i64;
    let mut second_utility_context = 0_i64;
    for (id, session_id, target_id, out_context) in [
        (
            36509_u64,
            first.session_id.as_str(),
            first.target_id.as_str(),
            &mut first_utility_context,
        ),
        (
            36510_u64,
            second.session_id.as_str(),
            second.target_id.as_str(),
            &mut second_utility_context,
        ),
    ] {
        ctx.process_async(json!({
            "id": id,
            "method": "Page.createIsolatedWorld",
            "sessionId": session_id,
            "params": {
                "frameId": target_id,
                "worldName": "utility"
            }
        }))
        .await;
        *out_context = take_response_by_id(&mut ctx, id)["result"]["executionContextId"]
            .as_i64()
            .expect("thin mixed-name handle initial utility context id");
        ctx.take_all();
    }

    let custom_a_wrapper_source = patchright_page_binding_wrapper_source(
        "customBindingA",
        "__lm_custom_binding_a_deliver",
        None,
        false,
    );
    let custom_b_wrapper_source = patchright_page_binding_wrapper_source(
        "customBindingB",
        "__lm_custom_binding_b_deliver",
        None,
        false,
    );
    let custom_handle_a_wrapper_source = patchright_page_binding_wrapper_source(
        "customHandleBindingA",
        "__lm_custom_handle_binding_a_deliver",
        Some("__lm_custom_handle_binding_a_take"),
        true,
    );
    let custom_handle_b_wrapper_source = patchright_page_binding_wrapper_source(
        "customHandleBindingB",
        "__lm_custom_handle_binding_b_deliver",
        Some("__lm_custom_handle_binding_b_take"),
        true,
    );
    let retained_handle_wrapper_source = patchright_page_binding_wrapper_source(
        "__pw_keptHandleBinding",
        "__lm_pw_kept_handle_binding_deliver",
        Some("__lm_pw_kept_handle_binding_take"),
        true,
    );

    for (session_id, utility_context_id, id_base) in [
        (first.session_id.as_str(), first_utility_context, 36511_u64),
        (
            second.session_id.as_str(),
            second_utility_context,
            36531_u64,
        ),
    ] {
        install_patchright_crpage_binding_in_existing_worlds_async(
            &mut ctx,
            session_id,
            utility_context_id,
            id_base,
            id_base + 1,
            id_base + 2,
            id_base + 3,
            "customBindingA",
            &custom_a_wrapper_source,
        )
        .await;
        install_patchright_crpage_binding_in_existing_worlds_async(
            &mut ctx,
            session_id,
            utility_context_id,
            id_base + 4,
            id_base + 5,
            id_base + 6,
            id_base + 7,
            "customBindingB",
            &custom_b_wrapper_source,
        )
        .await;
        install_patchright_crpage_binding_in_existing_worlds_async(
            &mut ctx,
            session_id,
            utility_context_id,
            id_base + 8,
            id_base + 9,
            id_base + 10,
            id_base + 11,
            "customHandleBindingA",
            &custom_handle_a_wrapper_source,
        )
        .await;
        install_patchright_crpage_binding_in_existing_worlds_async(
            &mut ctx,
            session_id,
            utility_context_id,
            id_base + 12,
            id_base + 13,
            id_base + 14,
            id_base + 15,
            "customHandleBindingB",
            &custom_handle_b_wrapper_source,
        )
        .await;
        install_patchright_crpage_binding_in_existing_worlds_async(
            &mut ctx,
            session_id,
            utility_context_id,
            id_base + 16,
            id_base + 17,
            id_base + 18,
            id_base + 19,
            "__pw_keptHandleBinding",
            &retained_handle_wrapper_source,
        )
        .await;
    }

    for (session_id, id_base) in [
        (first.session_id.as_str(), 36551_u64),
        (second.session_id.as_str(), 36561_u64),
    ] {
        for (source, world_name, offset) in [
            (custom_a_wrapper_source.as_str(), None, 0_u64),
            (custom_b_wrapper_source.as_str(), None, 1_u64),
            (custom_handle_a_wrapper_source.as_str(), None, 2_u64),
            (custom_handle_b_wrapper_source.as_str(), None, 3_u64),
            (retained_handle_wrapper_source.as_str(), None, 4_u64),
            (custom_a_wrapper_source.as_str(), Some("utility"), 5_u64),
            (custom_b_wrapper_source.as_str(), Some("utility"), 6_u64),
            (
                custom_handle_a_wrapper_source.as_str(),
                Some("utility"),
                7_u64,
            ),
            (
                custom_handle_b_wrapper_source.as_str(),
                Some("utility"),
                8_u64,
            ),
            (
                retained_handle_wrapper_source.as_str(),
                Some("utility"),
                9_u64,
            ),
        ] {
            let mut params = json!({
                "source": source,
                "runImmediately": true
            });
            if let Some(world_name) = world_name {
                params["worldName"] = json!(world_name);
            }
            ctx.process_async(json!({
                "id": id_base + offset,
                "method": "Page.addScriptToEvaluateOnNewDocument",
                "sessionId": session_id,
                "params": params
            }))
            .await;
            assert!(
                take_response_by_id(&mut ctx, id_base + offset)["result"]["identifier"]
                    .as_str()
                    .is_some()
            );
        }
    }

    for (id, name) in [
        (36571_u64, "customBindingA"),
        (36572_u64, "customHandleBindingA"),
    ] {
        ctx.process_async(json!({
            "id": id,
            "method": "Runtime.removeBinding",
            "sessionId": first.session_id,
            "params": { "name": name }
        }))
        .await;
        assert_eq!(take_response_by_id(&mut ctx, id)["result"], json!({}));
    }

    for (id, target_id) in [
        (36573_u64, first.target_id.as_str()),
        (36574_u64, second.target_id.as_str()),
    ] {
        ctx.process_async(json!({
            "id": id,
            "method": "Target.closeTarget",
            "params": { "targetId": target_id }
        }))
        .await;
        ctx.expect_result(id, json!({ "success": true }), None);
        ctx.take_all();
    }

    let first_replacement = attach_page_session_without_runtime_enable_in_existing_context_async(
        &mut ctx,
        &first.browser_context_id,
        36575,
        36576,
    )
    .await;
    let second_replacement = attach_page_session_without_runtime_enable_in_existing_context_async(
        &mut ctx,
        &second.browser_context_id,
        36577,
        36578,
    )
    .await;

    for (id, session_id, html) in [
        (
            36579_u64,
            first_replacement.session_id.as_str(),
            "<body><div id='utility-handle-a'>thin-first-mixed-name-handle-replacement</div></body>",
        ),
        (
            36580_u64,
            second_replacement.session_id.as_str(),
            "<body><div id='utility-handle-a'>thin-second-mixed-name-handle-replacement</div></body>",
        ),
    ] {
        ctx.process_async(json!({
            "id": id,
            "method": "Page.navigate",
            "sessionId": session_id,
            "params": { "url": format!("data:text/html,{html}") }
        }))
        .await;
        let navigation = take_response_by_id(&mut ctx, id);
        assert_eq!(navigation["sessionId"], json!(session_id));
        ctx.take_all();
    }

    for (id, target_id, session_id) in [
        (
            36581_u64,
            first_replacement.target_id.as_str(),
            first_replacement.session_id.as_str(),
        ),
        (
            36582_u64,
            second_replacement.target_id.as_str(),
            second_replacement.session_id.as_str(),
        ),
    ] {
        ctx.process_async(json!({
            "id": id,
            "method": "Target.detachFromTarget",
            "params": {
                "targetId": target_id,
                "sessionId": session_id
            }
        }))
        .await;
        ctx.expect_result(id, json!({}), None);
        ctx.expect_event(
            "Target.detachedFromTarget",
            Some(&json!({
                "targetId": target_id,
                "sessionId": session_id,
            })),
        );
    }

    ctx.process_async(json!({
        "id": 36583,
        "method": "Target.setAutoAttach",
        "params": {
            "autoAttach": true,
            "waitForDebuggerOnStart": false
        }
    }))
    .await;
    ctx.expect_result(36583, json!({}), None);
    let events = ctx.take_all();
    let attached_events = events
        .iter()
        .filter(|event| event["method"] == json!("Target.attachedToTarget"))
        .cloned()
        .collect::<Vec<_>>();
    assert_eq!(attached_events.len(), 2);
    let first_reauto_session = attached_events
        .iter()
        .find(|event| {
            event["params"]["targetInfo"]["targetId"] == json!(first_replacement.target_id)
        })
        .and_then(|event| event["params"]["sessionId"].as_str())
        .expect("thin mixed-name handle first replacement re-auto-attached session")
        .to_owned();
    let second_reauto_session = attached_events
        .iter()
        .find(|event| {
            event["params"]["targetInfo"]["targetId"] == json!(second_replacement.target_id)
        })
        .and_then(|event| event["params"]["sessionId"].as_str())
        .expect("thin mixed-name handle second replacement re-auto-attached session")
        .to_owned();

    let mut first_replay_utility_context = 0_i64;
    let mut second_replay_utility_context = 0_i64;
    for (id, session_id, target_id, out_context) in [
        (
            36584_u64,
            first_reauto_session.as_str(),
            first_replacement.target_id.as_str(),
            &mut first_replay_utility_context,
        ),
        (
            36585_u64,
            second_reauto_session.as_str(),
            second_replacement.target_id.as_str(),
            &mut second_replay_utility_context,
        ),
    ] {
        ctx.process_async(json!({
            "id": id,
            "method": "Page.createIsolatedWorld",
            "sessionId": session_id,
            "params": {
                "frameId": target_id,
                "worldName": "utility"
            }
        }))
        .await;
        *out_context = take_response_by_id(&mut ctx, id)["result"]["executionContextId"]
            .as_i64()
            .expect("thin mixed-name handle replay utility context id");
        ctx.take_all();
    }

    install_patchright_crpage_binding_in_existing_worlds_async(
        &mut ctx,
        first_reauto_session.as_str(),
        first_replay_utility_context,
        36586,
        36587,
        36588,
        36589,
        "customBindingB",
        &custom_b_wrapper_source,
    )
    .await;
    install_patchright_crpage_binding_in_existing_worlds_async(
        &mut ctx,
        first_reauto_session.as_str(),
        first_replay_utility_context,
        36590,
        36591,
        36592,
        36593,
        "customHandleBindingB",
        &custom_handle_b_wrapper_source,
    )
    .await;
    install_patchright_crpage_binding_in_existing_worlds_async(
        &mut ctx,
        first_reauto_session.as_str(),
        first_replay_utility_context,
        36594,
        36595,
        36596,
        36597,
        "__pw_keptHandleBinding",
        &retained_handle_wrapper_source,
    )
    .await;
    install_patchright_crpage_binding_in_existing_worlds_async(
        &mut ctx,
        second_reauto_session.as_str(),
        second_replay_utility_context,
        36598,
        36599,
        36600,
        36601,
        "customBindingA",
        &custom_a_wrapper_source,
    )
    .await;
    install_patchright_crpage_binding_in_existing_worlds_async(
        &mut ctx,
        second_reauto_session.as_str(),
        second_replay_utility_context,
        36602,
        36603,
        36604,
        36605,
        "customBindingB",
        &custom_b_wrapper_source,
    )
    .await;
    install_patchright_crpage_binding_in_existing_worlds_async(
        &mut ctx,
        second_reauto_session.as_str(),
        second_replay_utility_context,
        36606,
        36607,
        36608,
        36609,
        "customHandleBindingA",
        &custom_handle_a_wrapper_source,
    )
    .await;
    install_patchright_crpage_binding_in_existing_worlds_async(
        &mut ctx,
        second_reauto_session.as_str(),
        second_replay_utility_context,
        36610,
        36611,
        36612,
        36613,
        "customHandleBindingB",
        &custom_handle_b_wrapper_source,
    )
    .await;
    install_patchright_crpage_binding_in_existing_worlds_async(
        &mut ctx,
        second_reauto_session.as_str(),
        second_replay_utility_context,
        36614,
        36615,
        36616,
        36617,
        "__pw_keptHandleBinding",
        &retained_handle_wrapper_source,
    )
    .await;

    for (id, session_id, context_id, expected_state) in [
        (
            36618_u64,
            first_reauto_session.as_str(),
            None::<i64>,
            json!("[\"undefined\",\"function\",\"undefined\",\"function\",\"function\"]"),
        ),
        (
            36619_u64,
            first_reauto_session.as_str(),
            Some(first_replay_utility_context),
            json!("[\"undefined\",\"function\",\"undefined\",\"function\",\"function\"]"),
        ),
        (
            36620_u64,
            second_reauto_session.as_str(),
            None::<i64>,
            json!("[\"function\",\"function\",\"function\",\"function\",\"function\"]"),
        ),
        (
            36621_u64,
            second_reauto_session.as_str(),
            Some(second_replay_utility_context),
            json!("[\"function\",\"function\",\"function\",\"function\",\"function\"]"),
        ),
    ] {
        let mut params = json!({
            "expression": "JSON.stringify([typeof globalThis.customBindingA, typeof globalThis.customBindingB, typeof globalThis.customHandleBindingA, typeof globalThis.customHandleBindingB, typeof globalThis.__pw_keptHandleBinding])"
        });
        if let Some(context_id) = context_id {
            params["contextId"] = json!(context_id);
        }
        ctx.process_async(json!({
            "id": id,
            "method": "Runtime.evaluate",
            "sessionId": session_id,
            "params": params
        }))
        .await;
        assert_eq!(
            take_response_by_id(&mut ctx, id)["result"]["result"]["value"],
            expected_state
        );
    }

    ctx.process_async(json!({
            "id": 36622,
            "method": "Runtime.evaluate",
            "sessionId": second_reauto_session,
            "params": {
                "expression": "globalThis.__lm_thin_mixed_name_handle_second_custom_a = customBindingA({ source: 'thin-mixed-name-handle-second-custom-a', nested: { count: 38, values: ['mixed-name-handle', 39, false] } }).then(() => 'unexpected-success', error => `rejected:${error}`); 'scheduled-thin-mixed-name-handle-second-custom-a'",
                "awaitPromise": true
            }
        })).await;
    let scheduled_second_custom_a = take_response_by_id(&mut ctx, 36622);
    assert!(
        scheduled_second_custom_a["result"]["result"]["value"]
            .as_str()
            .expect("scheduled thin mixed-name handle second custom a")
            .starts_with("scheduled-")
    );
    let second_custom_a_called = ctx
        .sent
        .iter()
        .rev()
        .find(|message| {
            message["method"] == json!("Runtime.bindingCalled")
                && message["sessionId"] == json!(second_reauto_session)
                && message["params"]["name"] == json!("customBindingA")
        })
        .cloned()
        .expect("thin mixed-name handle second custom a bindingCalled");
    let second_custom_a_payload: serde_json::Value = serde_json::from_str(
        second_custom_a_called["params"]["payload"]
            .as_str()
            .expect("thin mixed-name handle second custom a payload string"),
    )
    .expect("thin mixed-name handle second custom a payload json");
    let second_custom_a_seq = second_custom_a_payload["seq"]
        .as_i64()
        .expect("thin mixed-name handle second custom a seq");
    assert_eq!(
        second_custom_a_payload["serializedArgs"],
        json!([{ "source": "thin-mixed-name-handle-second-custom-a", "nested": { "count": 38, "values": ["mixed-name-handle", 39, false] } }])
    );
    ctx.sent.clear();

    ctx.process_async(json!({
            "id": 36623,
            "method": "Runtime.evaluate",
            "sessionId": second_reauto_session,
            "params": {
                "expression": format!("globalThis.__lm_custom_binding_a_deliver({{ name: 'customBindingA', seq: {second_custom_a_seq}, error: 'thin-mixed-name-handle-second-custom-a-error' }}); 'delivered'"),
                "awaitPromise": true
            }
        })).await;
    assert_eq!(
        take_response_by_id(&mut ctx, 36623)["result"]["result"]["value"],
        json!("delivered")
    );

    ctx.process_async(json!({
        "id": 36624,
        "method": "Runtime.evaluate",
        "sessionId": second_reauto_session,
        "params": {
            "expression": "globalThis.__lm_thin_mixed_name_handle_second_custom_a",
            "awaitPromise": true
        }
    }))
    .await;
    assert_eq!(
        take_response_by_id(&mut ctx, 36624)["result"]["result"]["value"],
        json!("rejected:thin-mixed-name-handle-second-custom-a-error")
    );

    ctx.process_async(json!({
            "id": 36625,
            "method": "Runtime.evaluate",
            "sessionId": first_reauto_session,
            "params": {
                "contextId": first_replay_utility_context,
                "expression": "globalThis.__lm_thin_mixed_name_handle_first_pw = __pw_keptHandleBinding(document.getElementById('utility-handle-a')); 'scheduled-thin-mixed-name-handle-first-pw'",
                "awaitPromise": true
            }
        })).await;
    let scheduled_first_pw_handle = take_response_by_id(&mut ctx, 36625);
    assert!(
        scheduled_first_pw_handle["result"]["result"]["value"]
            .as_str()
            .expect("scheduled thin mixed-name handle first pw")
            .starts_with("scheduled-")
    );
    let first_pw_handle_called = ctx
        .sent
        .iter()
        .rev()
        .find(|message| {
            message["method"] == json!("Runtime.bindingCalled")
                && message["sessionId"] == json!(first_reauto_session)
                && message["params"]["name"] == json!("__pw_keptHandleBinding")
                && message["params"]["executionContextId"] == json!(first_replay_utility_context)
        })
        .cloned()
        .expect("thin mixed-name handle first pw bindingCalled");
    let first_pw_handle_payload: serde_json::Value = serde_json::from_str(
        first_pw_handle_called["params"]["payload"]
            .as_str()
            .expect("thin mixed-name handle first pw payload string"),
    )
    .expect("thin mixed-name handle first pw payload json");
    let first_pw_handle_seq = first_pw_handle_payload["seq"]
        .as_i64()
        .expect("thin mixed-name handle first pw seq");
    ctx.sent.clear();

    ctx.process_async(json!({
            "id": 36626,
            "method": "Runtime.evaluate",
            "sessionId": first_reauto_session,
            "params": {
                "contextId": first_replay_utility_context,
                "expression": format!("(() => {{ const handle = globalThis.__lm_pw_kept_handle_binding_take({{ name: '__pw_keptHandleBinding', seq: {first_pw_handle_seq} }}); return JSON.stringify([handle.id, handle.textContent, typeof globalThis.__lm_pw_kept_handle_binding_take({{ name: '__pw_keptHandleBinding', seq: {first_pw_handle_seq} }})]); }})()")
            }
        })).await;
    assert_eq!(
        take_response_by_id(&mut ctx, 36626)["result"]["result"]["value"],
        json!("[\"utility-handle-a\",\"thin-first-mixed-name-handle-replacement\",\"undefined\"]")
    );

    ctx.process_async(json!({
            "id": 36627,
            "method": "Runtime.evaluate",
            "sessionId": first_reauto_session,
            "params": {
                "contextId": first_replay_utility_context,
                "expression": format!("globalThis.__lm_pw_kept_handle_binding_deliver({{ name: '__pw_keptHandleBinding', seq: {first_pw_handle_seq}, result: 'thin-mixed-name-handle-first-pw-ok' }}); 'delivered'"),
                "awaitPromise": true
            }
        })).await;
    assert_eq!(
        take_response_by_id(&mut ctx, 36627)["result"]["result"]["value"],
        json!("delivered")
    );

    ctx.process_async(json!({
        "id": 36628,
        "method": "Runtime.evaluate",
        "sessionId": first_reauto_session,
        "params": {
            "contextId": first_replay_utility_context,
            "expression": "globalThis.__lm_thin_mixed_name_handle_first_pw",
            "awaitPromise": true
        }
    }))
    .await;
    assert_eq!(
        take_response_by_id(&mut ctx, 36628)["result"]["result"]["value"],
        json!("thin-mixed-name-handle-first-pw-ok")
    );
    })
    .await;
}
