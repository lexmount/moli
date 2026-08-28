use super::*;

#[tokio::test(flavor = "multi_thread")]
async fn patchright_over_cdp_page_binding_remove_binding_deactivates_current_wrapper_after_runtime_enable()
 {
    let mut ctx = TestContext::new();
    let attached = create_attached_page_session_async(&mut ctx, 2542, 2543, 2544, 2545, 2546).await;

    ctx.process_async(json!({
        "id": 2547,
        "method": "Page.navigate",
        "sessionId": attached.session_id,
        "params": {
            "url": "data:text/html,<body>page-binding-remove-current-wrapper</body>"
        }
    }))
    .await;
    let navigation = take_response_by_id(&mut ctx, 2547);
    assert_eq!(navigation["sessionId"], json!(attached.session_id));
    ctx.take_all();

    ctx.process_async(json!({
        "id": 2548,
        "method": "Page.createIsolatedWorld",
        "sessionId": attached.session_id,
        "params": {
            "frameId": attached.target_id,
            "worldName": "utility"
        }
    }))
    .await;
    let utility_context_id = take_response_by_id(&mut ctx, 2548)["result"]["executionContextId"]
        .as_i64()
        .expect("utility context id");
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 2549,
        "method": "Runtime.addBinding",
        "sessionId": attached.session_id,
        "params": {
            "name": "patchedRemovedBinding",
            "executionContextId": utility_context_id
        }
    }))
    .await;
    let add_binding = take_response_by_id(&mut ctx, 2549);
    assert_eq!(add_binding["result"], json!({}));

    ctx.process_async(json!({
            "id": 2550,
            "method": "Runtime.evaluate",
            "sessionId": attached.session_id,
            "params": {
                "contextId": utility_context_id,
                "expression": r#"
                    (() => {
                        function addPageBinding(bindingName) {
                            const binding = globalThis[bindingName];
                            globalThis[bindingName] = (...args) => {
                                const me = globalThis[bindingName];
                                let callbacks = me.callbacks;
                                if (!callbacks) {
                                    callbacks = new Map();
                                    me.callbacks = callbacks;
                                }
                                const seq = (me.lastSeq || 0) + 1;
                                me.lastSeq = seq;
                                const payload = { name: bindingName, seq, serializedArgs: args };
                                const promise = new Promise((resolve, reject) => callbacks.set(seq, { resolve, reject }));
                                binding(JSON.stringify(payload));
                                return promise;
                            };
                        }
                        addPageBinding('patchedRemovedBinding');
                        return typeof globalThis.patchedRemovedBinding;
                    })()
                "#
            }
        })).await;
    let install_wrapper = take_response_by_id(&mut ctx, 2550);
    assert_eq!(
        install_wrapper["result"]["result"]["value"],
        json!("function")
    );

    ctx.process_async(json!({
        "id": 2551,
        "method": "Runtime.removeBinding",
        "sessionId": attached.session_id,
        "params": {
            "name": "patchedRemovedBinding"
        }
    }))
    .await;
    let remove_binding = take_response_by_id(&mut ctx, 2551);
    assert_eq!(remove_binding["result"], json!({}));

    ctx.process_async(json!({
        "id": 2552,
        "method": "Runtime.evaluate",
        "sessionId": attached.session_id,
        "params": {
            "contextId": utility_context_id,
            "expression": "typeof globalThis.patchedRemovedBinding"
        }
    }))
    .await;
    let removed = take_response_by_id(&mut ctx, 2552);
    assert_eq!(removed["result"]["result"]["value"], json!("function"));

    ctx.sent.clear();
    ctx.process_async(json!({
            "id": 2553,
            "method": "Runtime.evaluate",
            "sessionId": attached.session_id,
            "params": {
                "contextId": utility_context_id,
                "expression": "globalThis.patchedRemovedBinding('unexpected'); typeof globalThis.patchedRemovedBinding"
            }
        })).await;
    let guarded = take_response_by_id(&mut ctx, 2553);
    assert_eq!(guarded["result"]["result"]["value"], json!("function"));
    assert!(
        !ctx.sent.iter().any(|message| {
            message["method"] == json!("Runtime.bindingCalled")
                && message["params"]["name"] == json!("patchedRemovedBinding")
        }),
        "removed page-binding wrapper should remain inert in the current utility world"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn patchright_over_cdp_crpage_expose_and_remove_binding_sequence_rehydrates_existing_contexts_without_runtime_enable()
 {
    let mut ctx = TestContext::new();
    let attached =
        create_attached_page_session_without_runtime_enable_async(&mut ctx, 2554, 2555, 2556).await;

    ctx.process_async(json!({
        "id": 2557,
        "method": "Page.navigate",
        "sessionId": attached.session_id,
        "params": {
            "url": "data:text/html,<body><div id='page'>crpage-sequence</div></body>"
        }
    }))
    .await;
    let navigation = take_response_by_id(&mut ctx, 2557);
    assert_eq!(navigation["sessionId"], json!(attached.session_id));
    assert!(
        !ctx.sent.iter().any(|message| {
            message["method"] == json!("Runtime.executionContextCreated")
                || message["method"] == json!("Runtime.executionContextsCleared")
        }),
        "Patchright-style navigation should stay off Runtime.enable surfaces: {:?}",
        ctx.sent
    );
    ctx.take_all();

    ctx.process_async(json!({
        "id": 2558,
        "method": "Page.createIsolatedWorld",
        "sessionId": attached.session_id,
        "params": {
            "frameId": attached.target_id,
            "worldName": "utility"
        }
    }))
    .await;
    let utility_context_id = take_response_by_id(&mut ctx, 2558)["result"]["executionContextId"]
        .as_i64()
        .expect("utility context id");
    assert!(
        !ctx.sent.iter().any(|message| {
            message["method"] == json!("Runtime.executionContextCreated")
                || message["method"] == json!("Runtime.executionContextsCleared")
        }),
        "utility world creation should stay off Runtime.enable surfaces: {:?}",
        ctx.sent
    );
    ctx.take_all();

    ctx.process_async(json!({
        "id": 2559,
        "method": "Runtime.addBinding",
        "sessionId": attached.session_id,
        "params": {
            "name": "patchrightExposeSequenceBinding"
        }
    }))
    .await;
    let add_main_binding = take_response_by_id(&mut ctx, 2559);
    assert_eq!(add_main_binding["result"], json!({}));

    ctx.process_async(json!({
        "id": 2560,
        "method": "Runtime.addBinding",
        "sessionId": attached.session_id,
        "params": {
            "name": "patchrightExposeSequenceBinding",
            "executionContextId": utility_context_id
        }
    }))
    .await;
    let add_utility_binding = take_response_by_id(&mut ctx, 2560);
    assert_eq!(add_utility_binding["result"], json!({}));

    let wrapper_source = r#"
            (() => {
                function addPageBinding(bindingName) {
                    const binding = globalThis[bindingName];
                    globalThis[bindingName] = (...args) => {
                        const me = globalThis[bindingName];
                        let callbacks = me.callbacks;
                        if (!callbacks) {
                            callbacks = new Map();
                            me.callbacks = callbacks;
                        }
                        const seq = (me.lastSeq || 0) + 1;
                        me.lastSeq = seq;
                        const payload = { name: bindingName, seq, serializedArgs: args };
                        const promise = new Promise((resolve, reject) => callbacks.set(seq, { resolve, reject }));
                        binding(JSON.stringify(payload));
                        return promise;
                    };
                }
                function deliverBindingResult(arg) {
                    const callbacks = globalThis[arg.name].callbacks;
                    if ('error' in arg)
                        callbacks.get(arg.seq).reject(arg.error);
                    else
                        callbacks.get(arg.seq).resolve(arg.result);
                    callbacks.delete(arg.seq);
                }
                addPageBinding('patchrightExposeSequenceBinding');
                globalThis.__lm_patchright_expose_sequence_deliver = deliverBindingResult;
                return typeof globalThis.patchrightExposeSequenceBinding;
            })()
        "#;

    ctx.process_async(json!({
        "id": 2561,
        "method": "Runtime.evaluate",
        "sessionId": attached.session_id,
        "params": {
            "expression": wrapper_source,
            "awaitPromise": true
        }
    }))
    .await;
    let install_main_wrapper = take_response_by_id(&mut ctx, 2561);
    assert_eq!(
        install_main_wrapper["result"]["result"]["value"],
        json!("function")
    );

    ctx.process_async(json!({
        "id": 2562,
        "method": "Runtime.evaluate",
        "sessionId": attached.session_id,
        "params": {
            "contextId": utility_context_id,
            "expression": wrapper_source,
            "awaitPromise": true
        }
    }))
    .await;
    let install_utility_wrapper = take_response_by_id(&mut ctx, 2562);
    assert_eq!(
        install_utility_wrapper["result"]["result"]["value"],
        json!("function")
    );

    let mut main_context_id = 0_i64;
    let mut main_seq = 0_i64;
    let mut utility_seq = 0_i64;
    for (id, context_id, expression, serialized_arg, seq_out, main_context_out) in [
        (
            2563_u64,
            None,
            "globalThis.__lm_patchright_expose_sequence_main = patchrightExposeSequenceBinding('from-main'); 'scheduled-main'",
            "from-main",
            &mut main_seq,
            Some(&mut main_context_id),
        ),
        (
            2564_u64,
            Some(utility_context_id),
            "globalThis.__lm_patchright_expose_sequence_utility = patchrightExposeSequenceBinding('from-utility'); 'scheduled-utility'",
            "from-utility",
            &mut utility_seq,
            None,
        ),
    ] {
        let mut params = json!({
            "expression": expression,
            "awaitPromise": true
        });
        if let Some(context_id) = context_id {
            params["contextId"] = json!(context_id);
        }
        ctx.process_async(json!({
            "id": id,
            "method": "Runtime.evaluate",
            "sessionId": attached.session_id,
            "params": params
        }))
        .await;
        let scheduled = take_response_by_id(&mut ctx, id);
        assert!(
            scheduled["result"]["result"]["value"]
                .as_str()
                .expect("scheduled binding wrapper value")
                .starts_with("scheduled-")
        );
        let binding_called = ctx
            .sent
            .iter()
            .find(|message| {
                message["method"] == json!("Runtime.bindingCalled")
                    && message["params"]["name"] == json!("patchrightExposeSequenceBinding")
            })
            .cloned()
            .expect("binding wrapper should emit Runtime.bindingCalled");
        let execution_context_id = binding_called["params"]["executionContextId"]
            .as_i64()
            .expect("execution context id");
        if let Some(main_context_out) = main_context_out {
            *main_context_out = execution_context_id;
        } else {
            assert_eq!(execution_context_id, utility_context_id);
        }
        let payload = binding_called["params"]["payload"]
            .as_str()
            .expect("binding payload should be a json string");
        let payload: serde_json::Value =
            serde_json::from_str(payload).expect("binding payload should be valid json");
        assert_eq!(payload["name"], json!("patchrightExposeSequenceBinding"));
        assert_eq!(payload["serializedArgs"], json!([serialized_arg]));
        *seq_out = payload["seq"]
            .as_i64()
            .expect("binding payload seq should be an integer");
        assert_eq!(*seq_out, 1);
        ctx.sent.clear();
    }
    assert_ne!(main_context_id, utility_context_id);

    for (id, context_id, seq, result, promise_name) in [
        (
            2565_u64,
            None,
            main_seq,
            "resolved-main",
            "__lm_patchright_expose_sequence_main",
        ),
        (
            2566_u64,
            Some(utility_context_id),
            utility_seq,
            "resolved-utility",
            "__lm_patchright_expose_sequence_utility",
        ),
    ] {
        let mut deliver_params = json!({
            "expression": format!(
                "globalThis.__lm_patchright_expose_sequence_deliver({{ name: 'patchrightExposeSequenceBinding', seq: {seq}, result: '{result}' }}); 'delivered'"
            ),
            "awaitPromise": true
        });
        if let Some(context_id) = context_id {
            deliver_params["contextId"] = json!(context_id);
        }
        ctx.process_async(json!({
            "id": id,
            "method": "Runtime.evaluate",
            "sessionId": attached.session_id,
            "params": deliver_params
        }))
        .await;
        let delivered = take_response_by_id(&mut ctx, id);
        assert_eq!(delivered["result"]["result"]["value"], json!("delivered"));

        let mut promise_params = json!({
            "expression": format!("globalThis.{promise_name}"),
            "awaitPromise": true
        });
        if let Some(context_id) = context_id {
            promise_params["contextId"] = json!(context_id);
        }
        ctx.process_async(json!({
            "id": id + 10,
            "method": "Runtime.evaluate",
            "sessionId": attached.session_id,
            "params": promise_params
        }))
        .await;
        let resolved = take_response_by_id(&mut ctx, id + 10);
        assert_eq!(resolved["result"]["result"]["value"], json!(result));
    }

    ctx.process_async(json!({
        "id": 2567,
        "method": "Runtime.removeBinding",
        "sessionId": attached.session_id,
        "params": {
            "name": "patchrightExposeSequenceBinding"
        }
    }))
    .await;
    let remove_binding = take_response_by_id(&mut ctx, 2567);
    assert_eq!(remove_binding["result"], json!({}));

    for (id, context_id) in [(2568_u64, None), (2569_u64, Some(utility_context_id))] {
        let mut params = json!({
            "expression": "typeof globalThis.patchrightExposeSequenceBinding"
        });
        if let Some(context_id) = context_id {
            params["contextId"] = json!(context_id);
        }
        ctx.process_async(json!({
            "id": id,
            "method": "Runtime.evaluate",
            "sessionId": attached.session_id,
            "params": params
        }))
        .await;
        let removed = take_response_by_id(&mut ctx, id);
        assert_eq!(removed["result"]["result"]["value"], json!("undefined"));
    }

    ctx.process_async(json!({
        "id": 2570,
        "method": "Page.navigate",
        "sessionId": attached.session_id,
        "params": {
            "url": "data:text/html,<body><div id='page'>after-remove</div></body>"
        }
    }))
    .await;
    let second_navigation = take_response_by_id(&mut ctx, 2570);
    assert_eq!(second_navigation["sessionId"], json!(attached.session_id));
    assert!(
        !ctx.sent.iter().any(|message| {
            message["method"] == json!("Runtime.executionContextCreated")
                || message["method"] == json!("Runtime.executionContextsCleared")
        }),
        "post-remove navigation should stay off Runtime.enable surfaces: {:?}",
        ctx.sent
    );
    ctx.take_all();

    ctx.process_async(json!({
        "id": 2571,
        "method": "Page.createIsolatedWorld",
        "sessionId": attached.session_id,
        "params": {
            "frameId": attached.target_id,
            "worldName": "utility"
        }
    }))
    .await;
    let next_utility_context_id =
        take_response_by_id(&mut ctx, 2571)["result"]["executionContextId"]
            .as_i64()
            .expect("utility context id after remove");
    ctx.take_all();

    for (id, context_id) in [(2572_u64, None), (2573_u64, Some(next_utility_context_id))] {
        let mut params = json!({
            "expression": "typeof globalThis.patchrightExposeSequenceBinding"
        });
        if let Some(context_id) = context_id {
            params["contextId"] = json!(context_id);
        }
        ctx.process_async(json!({
            "id": id,
            "method": "Runtime.evaluate",
            "sessionId": attached.session_id,
            "params": params
        }))
        .await;
        let absent = take_response_by_id(&mut ctx, id);
        assert_eq!(absent["result"]["result"]["value"], json!("undefined"));
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn patchright_over_cdp_crpage_reexpose_binding_source_is_idempotent_for_existing_worlds_without_runtime_enable()
 {
    let mut ctx = TestContext::new();
    let attached =
        create_attached_page_session_without_runtime_enable_async(&mut ctx, 2574, 2575, 2576).await;

    ctx.process_async(json!({
        "id": 2577,
        "method": "Page.navigate",
        "sessionId": attached.session_id,
        "params": {
            "url": "data:text/html,<body><div id='page'>crpage-idempotent-reexpose</div></body>"
        }
    }))
    .await;
    let navigation = take_response_by_id(&mut ctx, 2577);
    assert_eq!(navigation["sessionId"], json!(attached.session_id));
    ctx.take_all();

    ctx.process_async(json!({
        "id": 2578,
        "method": "Page.createIsolatedWorld",
        "sessionId": attached.session_id,
        "params": {
            "frameId": attached.target_id,
            "worldName": "utility"
        }
    }))
    .await;
    let utility_context_id = take_response_by_id(&mut ctx, 2578)["result"]["executionContextId"]
        .as_i64()
        .expect("utility context id");
    ctx.take_all();

    ctx.process_async(json!({
        "id": 2579,
        "method": "Runtime.addBinding",
        "sessionId": attached.session_id,
        "params": {
            "name": "patchrightIdempotentBinding"
        }
    }))
    .await;
    assert_eq!(take_response_by_id(&mut ctx, 2579)["result"], json!({}));

    ctx.process_async(json!({
        "id": 2580,
        "method": "Runtime.addBinding",
        "sessionId": attached.session_id,
        "params": {
            "name": "patchrightIdempotentBinding",
            "executionContextId": utility_context_id
        }
    }))
    .await;
    assert_eq!(take_response_by_id(&mut ctx, 2580)["result"], json!({}));

    let wrapper_source = patchright_page_binding_wrapper_source(
        "patchrightIdempotentBinding",
        "__lm_patchright_idempotent_deliver",
        None,
        false,
    );
    for (id, context_id) in [(2581_u64, None), (2582_u64, Some(utility_context_id))] {
        let mut params = json!({
            "expression": wrapper_source,
            "awaitPromise": true
        });
        if let Some(context_id) = context_id {
            params["contextId"] = json!(context_id);
        }
        ctx.process_async(json!({
            "id": id,
            "method": "Runtime.evaluate",
            "sessionId": attached.session_id,
            "params": params
        }))
        .await;
        let installed = take_response_by_id(&mut ctx, id);
        assert_eq!(installed["result"]["result"]["value"], json!("function"));
    }

    let mut first_main_seq = 0_i64;
    let mut first_utility_seq = 0_i64;
    for (id, context_id, expression, serialized_arg, seq_out) in [
        (
            2583_u64,
            None,
            "globalThis.__lm_idempotent_main_first = patchrightIdempotentBinding('main-first'); 'scheduled-main-first'",
            "main-first",
            &mut first_main_seq,
        ),
        (
            2584_u64,
            Some(utility_context_id),
            "globalThis.__lm_idempotent_utility_first = patchrightIdempotentBinding('utility-first'); 'scheduled-utility-first'",
            "utility-first",
            &mut first_utility_seq,
        ),
    ] {
        let mut params = json!({
            "expression": expression,
            "awaitPromise": true
        });
        if let Some(context_id) = context_id {
            params["contextId"] = json!(context_id);
        }
        ctx.process_async(json!({
            "id": id,
            "method": "Runtime.evaluate",
            "sessionId": attached.session_id,
            "params": params
        }))
        .await;
        let scheduled = take_response_by_id(&mut ctx, id);
        assert!(
            scheduled["result"]["result"]["value"]
                .as_str()
                .expect("scheduled binding wrapper value")
                .starts_with("scheduled-")
        );
        let binding_called = ctx
            .sent
            .iter()
            .find(|message| {
                message["method"] == json!("Runtime.bindingCalled")
                    && message["params"]["name"] == json!("patchrightIdempotentBinding")
            })
            .cloned()
            .expect("binding wrapper should emit Runtime.bindingCalled");
        let payload = binding_called["params"]["payload"]
            .as_str()
            .expect("binding payload should be a json string");
        let payload: serde_json::Value =
            serde_json::from_str(payload).expect("binding payload should be valid json");
        assert_eq!(payload["name"], json!("patchrightIdempotentBinding"));
        assert_eq!(payload["serializedArgs"], json!([serialized_arg]));
        *seq_out = payload["seq"]
            .as_i64()
            .expect("binding payload seq should be an integer");
        assert_eq!(*seq_out, 1);
        ctx.sent.clear();
    }

    for (id, context_id) in [(2585_u64, None), (2586_u64, Some(utility_context_id))] {
        let mut params = json!({
            "expression": wrapper_source,
            "awaitPromise": true
        });
        if let Some(context_id) = context_id {
            params["contextId"] = json!(context_id);
        }
        ctx.process_async(json!({
            "id": id,
            "method": "Runtime.evaluate",
            "sessionId": attached.session_id,
            "params": params
        }))
        .await;
        let reinstalled = take_response_by_id(&mut ctx, id);
        assert_eq!(reinstalled["result"]["result"]["value"], json!("function"));
    }

    let mut second_main_seq = 0_i64;
    let mut second_utility_seq = 0_i64;
    for (id, context_id, expression, serialized_arg, seq_out) in [
        (
            2587_u64,
            None,
            "globalThis.__lm_idempotent_main_second = patchrightIdempotentBinding('main-second'); 'scheduled-main-second'",
            "main-second",
            &mut second_main_seq,
        ),
        (
            2588_u64,
            Some(utility_context_id),
            "globalThis.__lm_idempotent_utility_second = patchrightIdempotentBinding('utility-second'); 'scheduled-utility-second'",
            "utility-second",
            &mut second_utility_seq,
        ),
    ] {
        let mut params = json!({
            "expression": expression,
            "awaitPromise": true
        });
        if let Some(context_id) = context_id {
            params["contextId"] = json!(context_id);
        }
        ctx.process_async(json!({
            "id": id,
            "method": "Runtime.evaluate",
            "sessionId": attached.session_id,
            "params": params
        }))
        .await;
        let scheduled = take_response_by_id(&mut ctx, id);
        assert!(
            scheduled["result"]["result"]["value"]
                .as_str()
                .expect("scheduled binding wrapper value")
                .starts_with("scheduled-")
        );
        let binding_called = ctx
            .sent
            .iter()
            .find(|message| {
                message["method"] == json!("Runtime.bindingCalled")
                    && message["params"]["name"] == json!("patchrightIdempotentBinding")
            })
            .cloned()
            .expect("binding wrapper should emit Runtime.bindingCalled");
        let payload = binding_called["params"]["payload"]
            .as_str()
            .expect("binding payload should be a json string");
        let payload: serde_json::Value =
            serde_json::from_str(payload).expect("binding payload should be valid json");
        assert_eq!(payload["name"], json!("patchrightIdempotentBinding"));
        assert_eq!(payload["serializedArgs"], json!([serialized_arg]));
        *seq_out = payload["seq"]
            .as_i64()
            .expect("binding payload seq should be an integer");
        assert_eq!(*seq_out, 2);
        ctx.sent.clear();
    }

    for (id, context_id, seq, result, promise_name) in [
        (
            2589_u64,
            None,
            second_main_seq,
            "resolved-main-second",
            "__lm_idempotent_main_second",
        ),
        (
            2590_u64,
            Some(utility_context_id),
            second_utility_seq,
            "resolved-utility-second",
            "__lm_idempotent_utility_second",
        ),
        (
            2591_u64,
            None,
            first_main_seq,
            "resolved-main-first",
            "__lm_idempotent_main_first",
        ),
        (
            2592_u64,
            Some(utility_context_id),
            first_utility_seq,
            "resolved-utility-first",
            "__lm_idempotent_utility_first",
        ),
    ] {
        let mut deliver_params = json!({
            "expression": format!(
                "globalThis.__lm_patchright_idempotent_deliver({{ name: 'patchrightIdempotentBinding', seq: {seq}, result: '{result}' }}); 'delivered'"
            ),
            "awaitPromise": true
        });
        if let Some(context_id) = context_id {
            deliver_params["contextId"] = json!(context_id);
        }
        ctx.process_async(json!({
            "id": id,
            "method": "Runtime.evaluate",
            "sessionId": attached.session_id,
            "params": deliver_params
        }))
        .await;
        let delivered = take_response_by_id(&mut ctx, id);
        assert_eq!(delivered["result"]["result"]["value"], json!("delivered"));

        let mut promise_params = json!({
            "expression": format!("globalThis.{promise_name}"),
            "awaitPromise": true
        });
        if let Some(context_id) = context_id {
            promise_params["contextId"] = json!(context_id);
        }
        ctx.process_async(json!({
            "id": id + 10,
            "method": "Runtime.evaluate",
            "sessionId": attached.session_id,
            "params": promise_params
        }))
        .await;
        let resolved = take_response_by_id(&mut ctx, id + 10);
        assert_eq!(resolved["result"]["result"]["value"], json!(result));
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn patchright_over_cdp_main_and_utility_init_scripts_see_bindings_on_first_materialization_without_runtime_enable()
 {
    let mut ctx = TestContext::new();
    let attached =
        create_attached_page_session_without_runtime_enable_async(&mut ctx, 26650, 26651, 26652)
            .await;

    ctx.process_async(json!({
            "id": 26653,
            "method": "Page.addScriptToEvaluateOnNewDocument",
            "sessionId": attached.session_id,
            "params": {
                "source": r#"
                    globalThis.__lm_main_init_binding_type = typeof globalThis.patchrightMainInitBinding;
                    if (typeof globalThis.patchrightMainInitBinding === 'function')
                        globalThis.patchrightMainInitBinding('from-main-init');
                "#
            }
        })).await;
    let main_preload = take_response_by_id(&mut ctx, 26653);
    assert!(main_preload["result"]["identifier"].is_string());

    ctx.process_async(json!({
        "id": 26654,
        "method": "Runtime.addBinding",
        "sessionId": attached.session_id,
        "params": {
            "name": "patchrightMainInitBinding"
        }
    }))
    .await;
    let add_main_binding = take_response_by_id(&mut ctx, 26654);
    assert_eq!(add_main_binding["result"], json!({}));

    ctx.process_async(json!({
            "id": 26655,
            "method": "Page.addScriptToEvaluateOnNewDocument",
            "sessionId": attached.session_id,
            "params": {
                "source": r#"
                    globalThis.__lm_utility_init_binding_type = typeof globalThis.patchrightUtilityInitBinding;
                    if (typeof globalThis.patchrightUtilityInitBinding === 'function')
                        globalThis.patchrightUtilityInitBinding('from-utility-init');
                "#,
                "worldName": "utility"
            }
        })).await;
    let utility_preload = take_response_by_id(&mut ctx, 26655);
    assert!(utility_preload["result"]["identifier"].is_string());

    ctx.process_async(json!({
        "id": 26656,
        "method": "Runtime.addBinding",
        "sessionId": attached.session_id,
        "params": {
            "name": "patchrightUtilityInitBinding",
            "executionContextName": "utility"
        }
    }))
    .await;
    let add_utility_binding = take_response_by_id(&mut ctx, 26656);
    assert_eq!(add_utility_binding["result"], json!({}));
    assert!(
        !ctx.sent.iter().any(|message| {
            message["method"] == json!("Runtime.executionContextCreated")
                || message["method"] == json!("Runtime.executionContextsCleared")
        }),
        "patchright-style setup should stay off Runtime.enable surfaces: {:?}",
        ctx.sent
    );
    ctx.sent.clear();

    ctx.process_and_wait_for_response_async(json!({
        "id": 26657,
        "method": "Page.navigate",
        "sessionId": attached.session_id,
        "params": {
            "url": "data:text/html,<body><div id='page'>binding-order</div></body>"
        }
    }))
    .await;
    let navigation = take_response_by_id(&mut ctx, 26657);
    assert_eq!(navigation["sessionId"], json!(attached.session_id));
    let main_binding_called = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Runtime.bindingCalled")
                && message["params"]["name"] == json!("patchrightMainInitBinding")
        })
        .cloned()
        .expect("main-world init script should see the binding on first navigation");
    assert_eq!(
        main_binding_called["params"]["payload"],
        json!("from-main-init")
    );
    assert!(
        !ctx.sent.iter().any(|message| {
            message["method"] == json!("Runtime.executionContextCreated")
                || message["method"] == json!("Runtime.executionContextsCleared")
        }),
        "main-world init script should not require Runtime.enable context events: {:?}",
        ctx.sent
    );
    let utility_binding_called_during_navigation = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Runtime.bindingCalled")
                && message["params"]["name"] == json!("patchrightUtilityInitBinding")
        })
        .cloned();
    ctx.sent.clear();

    ctx.process_async(json!({
            "id": 26658,
            "method": "Runtime.evaluate",
            "sessionId": attached.session_id,
            "params": {
                "expression": "JSON.stringify([globalThis.__lm_main_init_binding_type, typeof globalThis.patchrightMainInitBinding])"
            }
        })).await;
    let main_state = take_response_by_id(&mut ctx, 26658);
    assert_eq!(
        main_state["result"]["result"]["value"],
        json!("[\"function\",\"function\"]")
    );

    ctx.process_async(json!({
        "id": 26659,
        "method": "Page.createIsolatedWorld",
        "sessionId": attached.session_id,
        "params": {
            "frameId": attached.target_id,
            "worldName": "utility"
        }
    }))
    .await;
    let utility_context_id = take_response_by_id(&mut ctx, 26659)["result"]["executionContextId"]
        .as_i64()
        .expect("utility context id");
    let utility_binding_called = utility_binding_called_during_navigation.or_else(|| {
            ctx.sent
                .iter()
                .find(|message| {
                    message["method"] == json!("Runtime.bindingCalled")
                        && message["params"]["name"] == json!("patchrightUtilityInitBinding")
                })
                .cloned()
        })
        .expect(
            "utility-world init script should see the binding when the world first materializes on the current page",
        );
    assert_eq!(
        utility_binding_called["params"]["executionContextId"],
        json!(utility_context_id)
    );
    assert_eq!(
        utility_binding_called["params"]["payload"],
        json!("from-utility-init")
    );
    assert!(
        !ctx.sent.iter().any(|message| {
            message["method"] == json!("Runtime.executionContextCreated")
                || message["method"] == json!("Runtime.executionContextsCleared")
        }),
        "utility world materialization should stay off Runtime.enable context events: {:?}",
        ctx.sent
    );
    ctx.sent.clear();

    ctx.process_async(json!({
            "id": 26660,
            "method": "Runtime.evaluate",
            "sessionId": attached.session_id,
            "params": {
                "contextId": utility_context_id,
                "expression": "JSON.stringify([globalThis.__lm_utility_init_binding_type, typeof globalThis.patchrightUtilityInitBinding])"
            }
        })).await;
    let utility_state = take_response_by_id(&mut ctx, 26660);
    assert_eq!(
        utility_state["result"]["result"]["value"],
        json!("[\"function\",\"function\"]")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn patchright_over_cdp_auto_attach_sweep_init_scripts_see_same_named_bindings_on_first_materialization_without_runtime_enable()
 {
    let mut ctx = TestContext::new();
    let first =
        create_attached_page_session_without_runtime_enable_async(&mut ctx, 26661, 26662, 26663)
            .await;
    let second =
        create_attached_page_session_without_runtime_enable_async(&mut ctx, 26664, 26665, 26666)
            .await;

    for (id, target_id, session_id) in [
        (
            26667_u64,
            first.target_id.as_str(),
            first.session_id.as_str(),
        ),
        (
            26668_u64,
            second.target_id.as_str(),
            second.session_id.as_str(),
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
        "id": 26669,
        "method": "Target.setAutoAttach",
        "params": {
            "autoAttach": true,
            "waitForDebuggerOnStart": false
        }
    }))
    .await;
    ctx.expect_result(26669, json!({}), None);
    let events = ctx.take_all();
    let attached_events = events
        .iter()
        .filter(|event| event["method"] == json!("Target.attachedToTarget"))
        .cloned()
        .collect::<Vec<_>>();
    assert_eq!(
        attached_events.len(),
        2,
        "auto-attach sweep should attach both targets"
    );
    let first_auto_session = attached_events
        .iter()
        .find(|event| event["params"]["targetInfo"]["targetId"] == json!(first.target_id))
        .and_then(|event| event["params"]["sessionId"].as_str())
        .expect("first auto-attached session id")
        .to_owned();
    let second_auto_session = attached_events
        .iter()
        .find(|event| event["params"]["targetInfo"]["targetId"] == json!(second.target_id))
        .and_then(|event| event["params"]["sessionId"].as_str())
        .expect("second auto-attached session id")
        .to_owned();

    for (id, session_id, main_payload, utility_payload) in [
        (
            26670_u64,
            first_auto_session.as_str(),
            "from-first-main-init",
            "from-first-utility-init",
        ),
        (
            26674_u64,
            second_auto_session.as_str(),
            "from-second-main-init",
            "from-second-utility-init",
        ),
    ] {
        ctx.process_async(json!({
                "id": id,
                "method": "Page.addScriptToEvaluateOnNewDocument",
                "sessionId": session_id,
                "params": {
                    "source": format!(
                        "globalThis.__lm_shared_main_init_binding_type = typeof globalThis.sharedPatchrightMainInitBinding; if (typeof globalThis.sharedPatchrightMainInitBinding === 'function') globalThis.sharedPatchrightMainInitBinding('{main_payload}');"
                    )
                }
            })).await;
        let main_preload = take_response_by_id(&mut ctx, id);
        assert!(main_preload["result"]["identifier"].is_string());

        ctx.process_async(json!({
            "id": id + 1,
            "method": "Runtime.addBinding",
            "sessionId": session_id,
            "params": {
                "name": "sharedPatchrightMainInitBinding"
            }
        }))
        .await;
        let add_main_binding = take_response_by_id(&mut ctx, id + 1);
        assert_eq!(add_main_binding["result"], json!({}));

        ctx.process_async(json!({
                "id": id + 2,
                "method": "Page.addScriptToEvaluateOnNewDocument",
                "sessionId": session_id,
                "params": {
                    "source": format!(
                        "globalThis.__lm_shared_utility_init_binding_type = typeof globalThis.sharedPatchrightUtilityInitBinding; if (typeof globalThis.sharedPatchrightUtilityInitBinding === 'function') globalThis.sharedPatchrightUtilityInitBinding('{utility_payload}');"
                    ),
                    "worldName": "utility"
                }
            })).await;
        let utility_preload = take_response_by_id(&mut ctx, id + 2);
        assert!(utility_preload["result"]["identifier"].is_string());

        ctx.process_async(json!({
            "id": id + 3,
            "method": "Runtime.addBinding",
            "sessionId": session_id,
            "params": {
                "name": "sharedPatchrightUtilityInitBinding",
                "executionContextName": "utility"
            }
        }))
        .await;
        let add_utility_binding = take_response_by_id(&mut ctx, id + 3);
        assert_eq!(add_utility_binding["result"], json!({}));
    }
    assert!(
        !ctx.sent.iter().any(|message| {
            message["method"] == json!("Runtime.executionContextCreated")
                || message["method"] == json!("Runtime.executionContextsCleared")
        }),
        "patchright-style setup should stay off Runtime.enable surfaces: {:?}",
        ctx.sent
    );
    ctx.sent.clear();

    let mut first_utility_binding_called_during_navigation = None::<Value>;
    let mut second_utility_binding_called_during_navigation = None::<Value>;
    for (id, session_id, payload, utility_binding_slot) in [
        (
            26678_u64,
            first_auto_session.as_str(),
            "from-first-main-init",
            &mut first_utility_binding_called_during_navigation,
        ),
        (
            26679_u64,
            second_auto_session.as_str(),
            "from-second-main-init",
            &mut second_utility_binding_called_during_navigation,
        ),
    ] {
        ctx.process_and_wait_for_response_async(json!({
            "id": id,
            "method": "Page.navigate",
            "sessionId": session_id,
            "params": {
                "url": format!("data:text/html,<body><div id='page'>{payload}</div></body>")
            }
        }))
        .await;
        let navigation = take_response_by_id(&mut ctx, id);
        assert_eq!(navigation["sessionId"], json!(session_id));
        let main_binding_called = ctx
            .sent
            .iter()
            .find(|message| {
                message["method"] == json!("Runtime.bindingCalled")
                    && message["sessionId"] == json!(session_id)
                    && message["params"]["name"] == json!("sharedPatchrightMainInitBinding")
            })
            .cloned()
            .expect("main-world init script should see the binding on first navigation");
        assert_eq!(main_binding_called["params"]["payload"], json!(payload));
        *utility_binding_slot = ctx
            .sent
            .iter()
            .find(|message| {
                message["method"] == json!("Runtime.bindingCalled")
                    && message["sessionId"] == json!(session_id)
                    && message["params"]["name"] == json!("sharedPatchrightUtilityInitBinding")
            })
            .cloned();
        assert!(
            !ctx.sent.iter().any(|message| {
                message["method"] == json!("Runtime.executionContextCreated")
                    || message["method"] == json!("Runtime.executionContextsCleared")
            }),
            "navigation should stay off Runtime.enable context events: {:?}",
            ctx.sent
        );
        ctx.sent.clear();

        ctx.process_async(json!({
                "id": id + 10,
                "method": "Runtime.evaluate",
                "sessionId": session_id,
                "params": {
                    "expression": "JSON.stringify([globalThis.__lm_shared_main_init_binding_type, typeof globalThis.sharedPatchrightMainInitBinding])"
                }
            })).await;
        let main_state = take_response_by_id(&mut ctx, id + 10);
        assert_eq!(
            main_state["result"]["result"]["value"],
            json!("[\"function\",\"function\"]")
        );
    }

    let mut first_utility_context = 0_i64;
    let mut second_utility_context = 0_i64;
    for (
        id,
        session_id,
        target_id,
        payload,
        utility_binding_called_during_navigation,
        utility_context_slot,
    ) in [
        (
            26690_u64,
            first_auto_session.as_str(),
            first.target_id.as_str(),
            "from-first-utility-init",
            first_utility_binding_called_during_navigation,
            &mut first_utility_context,
        ),
        (
            26691_u64,
            second_auto_session.as_str(),
            second.target_id.as_str(),
            "from-second-utility-init",
            second_utility_binding_called_during_navigation,
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
        *utility_context_slot = take_response_by_id(&mut ctx, id)["result"]["executionContextId"]
            .as_i64()
            .expect("utility context id");
        let utility_binding_called = utility_binding_called_during_navigation.or_else(|| {
                ctx.sent
                    .iter()
                    .find(|message| {
                        message["method"] == json!("Runtime.bindingCalled")
                            && message["sessionId"] == json!(session_id)
                            && message["params"]["name"] == json!("sharedPatchrightUtilityInitBinding")
                    })
                    .cloned()
            })
            .expect("utility-world init script should see the binding when the world first materializes");
        assert_eq!(
            utility_binding_called["params"]["executionContextId"],
            json!(*utility_context_slot)
        );
        assert_eq!(utility_binding_called["params"]["payload"], json!(payload));
        assert!(
            !ctx.sent.iter().any(|message| {
                message["method"] == json!("Runtime.executionContextCreated")
                    || message["method"] == json!("Runtime.executionContextsCleared")
            }),
            "utility world materialization should stay off Runtime.enable context events: {:?}",
            ctx.sent
        );
        ctx.sent.clear();
    }

    for (id, session_id, utility_context_id) in [
        (
            26700_u64,
            first_auto_session.as_str(),
            first_utility_context,
        ),
        (
            26701_u64,
            second_auto_session.as_str(),
            second_utility_context,
        ),
    ] {
        ctx.process_async(json!({
                "id": id,
                "method": "Runtime.evaluate",
                "sessionId": session_id,
                "params": {
                    "contextId": utility_context_id,
                    "expression": "JSON.stringify([globalThis.__lm_shared_utility_init_binding_type, typeof globalThis.sharedPatchrightUtilityInitBinding])"
                }
            })).await;
        let utility_state = take_response_by_id(&mut ctx, id);
        assert_eq!(
            utility_state["result"]["result"]["value"],
            json!("[\"function\",\"function\"]")
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn patchright_over_cdp_auto_attach_sweep_crpage_expose_binding_sequence_keeps_same_name_isolated_by_execution_context()
 {
    let mut ctx = TestContext::new();
    let first =
        create_attached_page_session_without_runtime_enable_async(&mut ctx, 2574, 2575, 2576).await;
    let second =
        create_attached_page_session_without_runtime_enable_async(&mut ctx, 2577, 2578, 2579).await;

    for (id, session_id, label) in [
        (2580_u64, first.session_id.as_str(), "first-crpage-sequence"),
        (
            2581_u64,
            second.session_id.as_str(),
            "second-crpage-sequence",
        ),
    ] {
        ctx.process_async(json!({
            "id": id,
            "method": "Page.navigate",
            "sessionId": session_id,
            "params": {
                "url": format!("data:text/html,<body><div id='page'>{label}</div></body>")
            }
        }))
        .await;
        let navigation = take_response_by_id(&mut ctx, id);
        assert_eq!(navigation["sessionId"], json!(session_id));
        assert!(
            !ctx.sent.iter().any(|message| {
                message["method"] == json!("Runtime.executionContextCreated")
                    || message["method"] == json!("Runtime.executionContextsCleared")
            }),
            "Patchright-style navigation should stay off Runtime.enable surfaces: {:?}",
            ctx.sent
        );
        ctx.take_all();
    }

    for (id, target_id, session_id) in [
        (
            2582_u64,
            first.target_id.as_str(),
            first.session_id.as_str(),
        ),
        (
            2583_u64,
            second.target_id.as_str(),
            second.session_id.as_str(),
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
        "id": 2584,
        "method": "Target.setAutoAttach",
        "params": {
            "autoAttach": true,
            "waitForDebuggerOnStart": false
        }
    }))
    .await;
    ctx.expect_result(2584, json!({}), None);
    let events = ctx.take_all();
    let attached_events = events
        .iter()
        .filter(|event| event["method"] == json!("Target.attachedToTarget"))
        .cloned()
        .collect::<Vec<_>>();
    assert_eq!(
        attached_events.len(),
        2,
        "auto-attach sweep should attach both targets"
    );
    let first_auto_session = attached_events
        .iter()
        .find(|event| event["params"]["targetInfo"]["targetId"] == json!(first.target_id))
        .and_then(|event| event["params"]["sessionId"].as_str())
        .expect("first auto-attached session id")
        .to_owned();
    let second_auto_session = attached_events
        .iter()
        .find(|event| event["params"]["targetInfo"]["targetId"] == json!(second.target_id))
        .and_then(|event| event["params"]["sessionId"].as_str())
        .expect("second auto-attached session id")
        .to_owned();

    let mut first_utility_context = 0_i64;
    let mut second_utility_context = 0_i64;
    for (id, session_id, target_id, utility_context) in [
        (
            2585_u64,
            first_auto_session.as_str(),
            first.target_id.as_str(),
            &mut first_utility_context,
        ),
        (
            2586_u64,
            second_auto_session.as_str(),
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
        *utility_context = take_response_by_id(&mut ctx, id)["result"]["executionContextId"]
            .as_i64()
            .expect("utility context id after auto-attach");
        ctx.take_all();
    }

    let wrapper_source = patchright_page_binding_wrapper_source(
        "sharedCrPageSequenceBinding",
        "__lm_shared_crpage_sequence_deliver",
        None,
        false,
    );
    for (id, session_id, utility_context) in [
        (2587_u64, first_auto_session.as_str(), first_utility_context),
        (
            2591_u64,
            second_auto_session.as_str(),
            second_utility_context,
        ),
    ] {
        install_patchright_crpage_binding_in_existing_worlds_async(
            &mut ctx,
            session_id,
            utility_context,
            id,
            id + 1,
            id + 2,
            id + 3,
            "sharedCrPageSequenceBinding",
            &wrapper_source,
        )
        .await;
    }

    let mut first_main_seq = 0_i64;
    let mut first_main_context = 0_i64;
    let mut first_utility_seq = 0_i64;
    let mut second_main_seq = 0_i64;
    let mut second_main_context = 0_i64;
    let mut second_utility_seq = 0_i64;
    for (id, session_id, context_id, expression, serialized_arg, seq_out, main_context_out) in [
        (
            2595_u64,
            first_auto_session.as_str(),
            None,
            "globalThis.__lm_first_crpage_main = sharedCrPageSequenceBinding('from-first-main'); 'scheduled-first-main'",
            "from-first-main",
            &mut first_main_seq,
            Some(&mut first_main_context),
        ),
        (
            2596_u64,
            first_auto_session.as_str(),
            Some(first_utility_context),
            "globalThis.__lm_first_crpage_utility = sharedCrPageSequenceBinding('from-first-utility'); 'scheduled-first-utility'",
            "from-first-utility",
            &mut first_utility_seq,
            None,
        ),
        (
            2597_u64,
            second_auto_session.as_str(),
            None,
            "globalThis.__lm_second_crpage_main = sharedCrPageSequenceBinding('from-second-main'); 'scheduled-second-main'",
            "from-second-main",
            &mut second_main_seq,
            Some(&mut second_main_context),
        ),
        (
            2598_u64,
            second_auto_session.as_str(),
            Some(second_utility_context),
            "globalThis.__lm_second_crpage_utility = sharedCrPageSequenceBinding('from-second-utility'); 'scheduled-second-utility'",
            "from-second-utility",
            &mut second_utility_seq,
            None,
        ),
    ] {
        let mut params = json!({
            "expression": expression,
            "awaitPromise": true
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
        let scheduled = take_response_by_id(&mut ctx, id);
        assert!(
            scheduled["result"]["result"]["value"]
                .as_str()
                .expect("scheduled binding wrapper value")
                .starts_with("scheduled-")
        );
        let binding_called = ctx
            .sent
            .iter()
            .find(|message| {
                message["method"] == json!("Runtime.bindingCalled")
                    && message["params"]["name"] == json!("sharedCrPageSequenceBinding")
            })
            .cloned()
            .expect("binding wrapper should emit Runtime.bindingCalled");
        let execution_context_id = binding_called["params"]["executionContextId"]
            .as_i64()
            .expect("execution context id");
        if let Some(main_context_out) = main_context_out {
            *main_context_out = execution_context_id;
        } else {
            assert_eq!(
                execution_context_id,
                context_id.expect("utility context id")
            );
        }
        let payload = binding_called["params"]["payload"]
            .as_str()
            .expect("binding payload should be a json string");
        let payload: serde_json::Value =
            serde_json::from_str(payload).expect("binding payload should be valid json");
        assert_eq!(payload["name"], json!("sharedCrPageSequenceBinding"));
        assert_eq!(payload["serializedArgs"], json!([serialized_arg]));
        *seq_out = payload["seq"]
            .as_i64()
            .expect("binding payload seq should be an integer");
        assert_eq!(*seq_out, 1);
        ctx.sent.clear();
    }

    assert_ne!(first_main_context, first_utility_context);
    assert_ne!(second_main_context, second_utility_context);

    for (id, session_id, context_id, seq, result, promise_name) in [
        (
            2599_u64,
            second_auto_session.as_str(),
            Some(second_utility_context),
            second_utility_seq,
            "second-utility-resolved",
            "__lm_second_crpage_utility",
        ),
        (
            2600_u64,
            first_auto_session.as_str(),
            None,
            first_main_seq,
            "first-main-resolved",
            "__lm_first_crpage_main",
        ),
        (
            2601_u64,
            first_auto_session.as_str(),
            Some(first_utility_context),
            first_utility_seq,
            "first-utility-resolved",
            "__lm_first_crpage_utility",
        ),
        (
            2602_u64,
            second_auto_session.as_str(),
            None,
            second_main_seq,
            "second-main-resolved",
            "__lm_second_crpage_main",
        ),
    ] {
        let mut deliver_params = json!({
            "expression": format!(
                "globalThis.__lm_shared_crpage_sequence_deliver({{ name: 'sharedCrPageSequenceBinding', seq: {seq}, result: '{result}' }}); 'delivered'"
            ),
            "awaitPromise": true
        });
        if let Some(context_id) = context_id {
            deliver_params["contextId"] = json!(context_id);
        }
        ctx.process_async(json!({
            "id": id,
            "method": "Runtime.evaluate",
            "sessionId": session_id,
            "params": deliver_params
        }))
        .await;
        let delivered = take_response_by_id(&mut ctx, id);
        assert_eq!(delivered["result"]["result"]["value"], json!("delivered"));

        let mut promise_params = json!({
            "expression": format!("globalThis.{promise_name}"),
            "awaitPromise": true
        });
        if let Some(context_id) = context_id {
            promise_params["contextId"] = json!(context_id);
        }
        ctx.process_async(json!({
            "id": id + 10,
            "method": "Runtime.evaluate",
            "sessionId": session_id,
            "params": promise_params
        }))
        .await;
        let resolved = take_response_by_id(&mut ctx, id + 10);
        assert_eq!(resolved["result"]["result"]["value"], json!(result));
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn patchright_over_cdp_auto_attach_sweep_crpage_expose_binding_sequence_keeps_serialized_object_args_isolated_by_execution_context()
 {
    let mut ctx = TestContext::new();
    let first =
        create_attached_page_session_without_runtime_enable_async(&mut ctx, 2603, 2604, 2605).await;
    let second =
        create_attached_page_session_without_runtime_enable_async(&mut ctx, 2606, 2607, 2608).await;

    for (id, session_id, label) in [
        (
            2609_u64,
            first.session_id.as_str(),
            "first-crpage-object-sequence",
        ),
        (
            2610_u64,
            second.session_id.as_str(),
            "second-crpage-object-sequence",
        ),
    ] {
        ctx.process_async(json!({
            "id": id,
            "method": "Page.navigate",
            "sessionId": session_id,
            "params": {
                "url": format!("data:text/html,<body><div id='page'>{label}</div></body>")
            }
        }))
        .await;
        let navigation = take_response_by_id(&mut ctx, id);
        assert_eq!(navigation["sessionId"], json!(session_id));
        assert!(
            !ctx.sent.iter().any(|message| {
                message["method"] == json!("Runtime.executionContextCreated")
                    || message["method"] == json!("Runtime.executionContextsCleared")
            }),
            "Patchright-style navigation should stay off Runtime.enable surfaces: {:?}",
            ctx.sent
        );
        ctx.take_all();
    }

    for (id, target_id, session_id) in [
        (
            2611_u64,
            first.target_id.as_str(),
            first.session_id.as_str(),
        ),
        (
            2612_u64,
            second.target_id.as_str(),
            second.session_id.as_str(),
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
        "id": 2613,
        "method": "Target.setAutoAttach",
        "params": {
            "autoAttach": true,
            "waitForDebuggerOnStart": false
        }
    }))
    .await;
    ctx.expect_result(2613, json!({}), None);
    let events = ctx.take_all();
    let attached_events = events
        .iter()
        .filter(|event| event["method"] == json!("Target.attachedToTarget"))
        .cloned()
        .collect::<Vec<_>>();
    assert_eq!(
        attached_events.len(),
        2,
        "auto-attach sweep should attach both targets"
    );
    let first_auto_session = attached_events
        .iter()
        .find(|event| event["params"]["targetInfo"]["targetId"] == json!(first.target_id))
        .and_then(|event| event["params"]["sessionId"].as_str())
        .expect("first auto-attached session id")
        .to_owned();
    let second_auto_session = attached_events
        .iter()
        .find(|event| event["params"]["targetInfo"]["targetId"] == json!(second.target_id))
        .and_then(|event| event["params"]["sessionId"].as_str())
        .expect("second auto-attached session id")
        .to_owned();

    let mut first_utility_context = 0_i64;
    let mut second_utility_context = 0_i64;
    for (id, session_id, target_id, utility_context) in [
        (
            2614_u64,
            first_auto_session.as_str(),
            first.target_id.as_str(),
            &mut first_utility_context,
        ),
        (
            2615_u64,
            second_auto_session.as_str(),
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
        *utility_context = take_response_by_id(&mut ctx, id)["result"]["executionContextId"]
            .as_i64()
            .expect("utility context id after auto-attach");
        ctx.take_all();
    }

    let wrapper_source = patchright_page_binding_wrapper_source(
        "sharedCrPageObjectSequenceBinding",
        "__lm_shared_crpage_object_sequence_deliver",
        None,
        false,
    );
    for (id, session_id, utility_context) in [
        (2616_u64, first_auto_session.as_str(), first_utility_context),
        (
            2620_u64,
            second_auto_session.as_str(),
            second_utility_context,
        ),
    ] {
        install_patchright_crpage_binding_in_existing_worlds_async(
            &mut ctx,
            session_id,
            utility_context,
            id,
            id + 1,
            id + 2,
            id + 3,
            "sharedCrPageObjectSequenceBinding",
            &wrapper_source,
        )
        .await;
    }

    let mut first_main_seq = 0_i64;
    let mut first_main_context = 0_i64;
    let mut first_utility_seq = 0_i64;
    let mut second_main_seq = 0_i64;
    let mut second_main_context = 0_i64;
    let mut second_utility_seq = 0_i64;
    for (id, session_id, context_id, expression, serialized_arg, seq_out, main_context_out) in [
        (
            2624_u64,
            first_auto_session.as_str(),
            None,
            "globalThis.__lm_first_crpage_object_main = sharedCrPageObjectSequenceBinding({ source: 'first-main', nested: { count: 1, values: ['a', 2, true] } }); 'scheduled-first-main'",
            json!([{
                "source": "first-main",
                "nested": { "count": 1, "values": ["a", 2, true] }
            }]),
            &mut first_main_seq,
            Some(&mut first_main_context),
        ),
        (
            2625_u64,
            first_auto_session.as_str(),
            Some(first_utility_context),
            "globalThis.__lm_first_crpage_object_utility = sharedCrPageObjectSequenceBinding({ source: 'first-utility', nested: { count: 2, values: ['b', 3, false] } }); 'scheduled-first-utility'",
            json!([{
                "source": "first-utility",
                "nested": { "count": 2, "values": ["b", 3, false] }
            }]),
            &mut first_utility_seq,
            None,
        ),
        (
            2626_u64,
            second_auto_session.as_str(),
            None,
            "globalThis.__lm_second_crpage_object_main = sharedCrPageObjectSequenceBinding({ source: 'second-main', nested: { count: 3, values: ['c', 4, true] } }); 'scheduled-second-main'",
            json!([{
                "source": "second-main",
                "nested": { "count": 3, "values": ["c", 4, true] }
            }]),
            &mut second_main_seq,
            Some(&mut second_main_context),
        ),
        (
            2627_u64,
            second_auto_session.as_str(),
            Some(second_utility_context),
            "globalThis.__lm_second_crpage_object_utility = sharedCrPageObjectSequenceBinding({ source: 'second-utility', nested: { count: 4, values: ['d', 5, false] } }); 'scheduled-second-utility'",
            json!([{
                "source": "second-utility",
                "nested": { "count": 4, "values": ["d", 5, false] }
            }]),
            &mut second_utility_seq,
            None,
        ),
    ] {
        let mut params = json!({
            "expression": expression,
            "awaitPromise": true
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
        let scheduled = take_response_by_id(&mut ctx, id);
        assert!(
            scheduled["result"]["result"]["value"]
                .as_str()
                .expect("scheduled binding wrapper value")
                .starts_with("scheduled-")
        );
        let binding_called = ctx
            .sent
            .iter()
            .find(|message| {
                message["method"] == json!("Runtime.bindingCalled")
                    && message["params"]["name"] == json!("sharedCrPageObjectSequenceBinding")
            })
            .cloned()
            .expect("binding wrapper should emit Runtime.bindingCalled");
        let execution_context_id = binding_called["params"]["executionContextId"]
            .as_i64()
            .expect("execution context id");
        if let Some(main_context_out) = main_context_out {
            *main_context_out = execution_context_id;
        } else {
            assert_eq!(
                execution_context_id,
                context_id.expect("utility context id")
            );
        }
        let payload = binding_called["params"]["payload"]
            .as_str()
            .expect("binding payload should be a json string");
        let payload: serde_json::Value =
            serde_json::from_str(payload).expect("binding payload should be valid json");
        assert_eq!(payload["name"], json!("sharedCrPageObjectSequenceBinding"));
        assert_eq!(payload["serializedArgs"], serialized_arg);
        *seq_out = payload["seq"]
            .as_i64()
            .expect("binding payload seq should be an integer");
        assert_eq!(*seq_out, 1);
        ctx.sent.clear();
    }

    assert_ne!(first_main_context, first_utility_context);
    assert_ne!(second_main_context, second_utility_context);

    for (id, session_id, context_id, seq, result, promise_name) in [
        (
            2628_u64,
            second_auto_session.as_str(),
            Some(second_utility_context),
            second_utility_seq,
            "second-utility-object-resolved",
            "__lm_second_crpage_object_utility",
        ),
        (
            2629_u64,
            first_auto_session.as_str(),
            None,
            first_main_seq,
            "first-main-object-resolved",
            "__lm_first_crpage_object_main",
        ),
        (
            2630_u64,
            first_auto_session.as_str(),
            Some(first_utility_context),
            first_utility_seq,
            "first-utility-object-resolved",
            "__lm_first_crpage_object_utility",
        ),
        (
            2631_u64,
            second_auto_session.as_str(),
            None,
            second_main_seq,
            "second-main-object-resolved",
            "__lm_second_crpage_object_main",
        ),
    ] {
        let mut deliver_params = json!({
            "expression": format!(
                "globalThis.__lm_shared_crpage_object_sequence_deliver({{ name: 'sharedCrPageObjectSequenceBinding', seq: {seq}, result: '{result}' }}); 'delivered'"
            ),
            "awaitPromise": true
        });
        if let Some(context_id) = context_id {
            deliver_params["contextId"] = json!(context_id);
        }
        ctx.process_async(json!({
            "id": id,
            "method": "Runtime.evaluate",
            "sessionId": session_id,
            "params": deliver_params
        }))
        .await;
        let delivered = take_response_by_id(&mut ctx, id);
        assert_eq!(delivered["result"]["result"]["value"], json!("delivered"));

        let mut promise_params = json!({
            "expression": format!("globalThis.{promise_name}"),
            "awaitPromise": true
        });
        if let Some(context_id) = context_id {
            promise_params["contextId"] = json!(context_id);
        }
        ctx.process_async(json!({
            "id": id + 10,
            "method": "Runtime.evaluate",
            "sessionId": session_id,
            "params": promise_params
        }))
        .await;
        let resolved = take_response_by_id(&mut ctx, id + 10);
        assert_eq!(resolved["result"]["result"]["value"], json!(result));
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn patchright_over_cdp_auto_attach_sweep_crpage_handle_binding_sequence_keeps_same_name_isolated_by_execution_context()
 {
    let mut ctx = TestContext::new();
    let first =
        create_attached_page_session_without_runtime_enable_async(&mut ctx, 2700, 2701, 2702).await;
    let second =
        create_attached_page_session_without_runtime_enable_async(&mut ctx, 2703, 2704, 2705).await;

    for (id, session_id, html) in [
        (
            2706_u64,
            first.session_id.as_str(),
            "<body><div id='first-main-handle'>first-main</div><div id='first-utility-handle'>first-utility</div></body>",
        ),
        (
            2707_u64,
            second.session_id.as_str(),
            "<body><div id='second-main-handle'>second-main</div><div id='second-utility-handle'>second-utility</div></body>",
        ),
    ] {
        ctx.process_async(json!({
            "id": id,
            "method": "Page.navigate",
            "sessionId": session_id,
            "params": {
                "url": format!("data:text/html,{html}")
            }
        }))
        .await;
        let navigation = take_response_by_id(&mut ctx, id);
        assert_eq!(navigation["sessionId"], json!(session_id));
        ctx.take_all();
    }

    for (id, target_id, session_id) in [
        (
            2708_u64,
            first.target_id.as_str(),
            first.session_id.as_str(),
        ),
        (
            2709_u64,
            second.target_id.as_str(),
            second.session_id.as_str(),
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
        "id": 2710,
        "method": "Target.setAutoAttach",
        "params": {
            "autoAttach": true,
            "waitForDebuggerOnStart": false
        }
    }))
    .await;
    ctx.expect_result(2710, json!({}), None);
    let events = ctx.take_all();
    let attached_events = events
        .iter()
        .filter(|event| event["method"] == json!("Target.attachedToTarget"))
        .cloned()
        .collect::<Vec<_>>();
    assert_eq!(
        attached_events.len(),
        2,
        "auto-attach sweep should attach both targets"
    );
    let first_auto_session = attached_events
        .iter()
        .find(|event| event["params"]["targetInfo"]["targetId"] == json!(first.target_id))
        .and_then(|event| event["params"]["sessionId"].as_str())
        .expect("first auto-attached session id")
        .to_owned();
    let second_auto_session = attached_events
        .iter()
        .find(|event| event["params"]["targetInfo"]["targetId"] == json!(second.target_id))
        .and_then(|event| event["params"]["sessionId"].as_str())
        .expect("second auto-attached session id")
        .to_owned();

    let mut first_utility_context = 0_i64;
    let mut second_utility_context = 0_i64;
    for (id, session_id, target_id, utility_context) in [
        (
            2711_u64,
            first_auto_session.as_str(),
            first.target_id.as_str(),
            &mut first_utility_context,
        ),
        (
            2712_u64,
            second_auto_session.as_str(),
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
        *utility_context = take_response_by_id(&mut ctx, id)["result"]["executionContextId"]
            .as_i64()
            .expect("utility context id after auto-attach");
        ctx.take_all();
    }

    let wrapper_source = patchright_page_binding_wrapper_source(
        "sharedCrPageHandleSequenceBinding",
        "__lm_shared_crpage_handle_sequence_deliver",
        Some("__lm_shared_crpage_take_handle"),
        true,
    );
    for (id, session_id, utility_context) in [
        (2713_u64, first_auto_session.as_str(), first_utility_context),
        (
            2717_u64,
            second_auto_session.as_str(),
            second_utility_context,
        ),
    ] {
        ctx.process_async(json!({
            "id": id,
            "method": "Runtime.addBinding",
            "sessionId": session_id,
            "params": {
                "name": "sharedCrPageHandleSequenceBinding"
            }
        }))
        .await;
        let add_main_binding = take_response_by_id(&mut ctx, id);
        assert_eq!(add_main_binding["result"], json!({}));

        ctx.process_async(json!({
            "id": id + 1,
            "method": "Runtime.addBinding",
            "sessionId": session_id,
            "params": {
                "name": "sharedCrPageHandleSequenceBinding",
                "executionContextId": utility_context
            }
        }))
        .await;
        let add_utility_binding = take_response_by_id(&mut ctx, id + 1);
        assert_eq!(add_utility_binding["result"], json!({}));

        ctx.process_async(json!({
            "id": id + 2,
            "method": "Runtime.evaluate",
            "sessionId": session_id,
            "params": {
                "expression": wrapper_source,
                "awaitPromise": true
            }
        }))
        .await;
        let install_main_wrapper = take_response_by_id(&mut ctx, id + 2);
        assert_eq!(
            install_main_wrapper["result"]["result"]["value"],
            json!("function")
        );

        ctx.process_async(json!({
            "id": id + 3,
            "method": "Runtime.evaluate",
            "sessionId": session_id,
            "params": {
                "contextId": utility_context,
                "expression": wrapper_source,
                "awaitPromise": true
            }
        }))
        .await;
        let install_utility_wrapper = take_response_by_id(&mut ctx, id + 3);
        assert_eq!(
            install_utility_wrapper["result"]["result"]["value"],
            json!("function")
        );
    }

    let mut first_main_seq = 0_i64;
    let mut first_main_context = 0_i64;
    let mut first_utility_seq = 0_i64;
    let mut second_main_seq = 0_i64;
    let mut second_main_context = 0_i64;
    let mut second_utility_seq = 0_i64;
    for (
        id,
        session_id,
        context_id,
        expression,
        handle_id,
        handle_text,
        seq_out,
        main_context_out,
    ) in [
        (
            2721_u64,
            first_auto_session.as_str(),
            None,
            "globalThis.__lm_first_crpage_handle_main = sharedCrPageHandleSequenceBinding(document.getElementById('first-main-handle')); 'scheduled-first-main'",
            "first-main-handle",
            "first-main",
            &mut first_main_seq,
            Some(&mut first_main_context),
        ),
        (
            2722_u64,
            first_auto_session.as_str(),
            Some(first_utility_context),
            "globalThis.__lm_first_crpage_handle_utility = sharedCrPageHandleSequenceBinding(document.getElementById('first-utility-handle')); 'scheduled-first-utility'",
            "first-utility-handle",
            "first-utility",
            &mut first_utility_seq,
            None,
        ),
        (
            2723_u64,
            second_auto_session.as_str(),
            None,
            "globalThis.__lm_second_crpage_handle_main = sharedCrPageHandleSequenceBinding(document.getElementById('second-main-handle')); 'scheduled-second-main'",
            "second-main-handle",
            "second-main",
            &mut second_main_seq,
            Some(&mut second_main_context),
        ),
        (
            2724_u64,
            second_auto_session.as_str(),
            Some(second_utility_context),
            "globalThis.__lm_second_crpage_handle_utility = sharedCrPageHandleSequenceBinding(document.getElementById('second-utility-handle')); 'scheduled-second-utility'",
            "second-utility-handle",
            "second-utility",
            &mut second_utility_seq,
            None,
        ),
    ] {
        let mut params = json!({
            "expression": expression,
            "awaitPromise": true
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
        let scheduled = take_response_by_id(&mut ctx, id);
        assert!(
            scheduled["result"]["result"]["value"]
                .as_str()
                .expect("scheduled binding wrapper value")
                .starts_with("scheduled-")
        );
        let binding_called = ctx
            .sent
            .iter()
            .find(|message| {
                message["method"] == json!("Runtime.bindingCalled")
                    && message["params"]["name"] == json!("sharedCrPageHandleSequenceBinding")
            })
            .cloned()
            .expect("handle binding wrapper should emit Runtime.bindingCalled");
        let execution_context_id = binding_called["params"]["executionContextId"]
            .as_i64()
            .expect("execution context id");
        if let Some(main_context_out) = main_context_out {
            *main_context_out = execution_context_id;
        } else {
            assert_eq!(
                execution_context_id,
                context_id.expect("utility context id")
            );
        }
        let payload = binding_called["params"]["payload"]
            .as_str()
            .expect("binding payload should be a json string");
        let payload: serde_json::Value =
            serde_json::from_str(payload).expect("binding payload should be valid json");
        assert_eq!(payload["name"], json!("sharedCrPageHandleSequenceBinding"));
        assert!(
            !payload
                .as_object()
                .expect("payload object")
                .contains_key("serializedArgs")
        );
        *seq_out = payload["seq"]
            .as_i64()
            .expect("binding payload seq should be an integer");
        assert_eq!(*seq_out, 1);
        ctx.sent.clear();

        let mut take_params = json!({
            "expression": format!(
                "(() => {{ const handle = globalThis.__lm_shared_crpage_take_handle({{ name: 'sharedCrPageHandleSequenceBinding', seq: {} }}); return JSON.stringify([handle.id, handle.textContent, typeof globalThis.__lm_shared_crpage_take_handle({{ name: 'sharedCrPageHandleSequenceBinding', seq: {} }})]); }})()",
                *seq_out, *seq_out
            ),
            "awaitPromise": true
        });
        if let Some(context_id) = context_id {
            take_params["contextId"] = json!(context_id);
        }
        ctx.process_async(json!({
            "id": id + 10,
            "method": "Runtime.evaluate",
            "sessionId": session_id,
            "params": take_params
        }))
        .await;
        let taken = take_response_by_id(&mut ctx, id + 10);
        assert_eq!(
            taken["result"]["result"]["value"],
            json!(format!("[\"{handle_id}\",\"{handle_text}\",\"undefined\"]"))
        );
    }

    assert_ne!(first_main_context, first_utility_context);
    assert_ne!(second_main_context, second_utility_context);

    for (id, session_id, context_id, seq, result, promise_name) in [
        (
            2730_u64,
            second_auto_session.as_str(),
            Some(second_utility_context),
            second_utility_seq,
            "second-utility-handle-resolved",
            "__lm_second_crpage_handle_utility",
        ),
        (
            2731_u64,
            first_auto_session.as_str(),
            None,
            first_main_seq,
            "first-main-handle-resolved",
            "__lm_first_crpage_handle_main",
        ),
        (
            2732_u64,
            first_auto_session.as_str(),
            Some(first_utility_context),
            first_utility_seq,
            "first-utility-handle-resolved",
            "__lm_first_crpage_handle_utility",
        ),
        (
            2733_u64,
            second_auto_session.as_str(),
            None,
            second_main_seq,
            "second-main-handle-resolved",
            "__lm_second_crpage_handle_main",
        ),
    ] {
        let mut deliver_params = json!({
            "expression": format!(
                "globalThis.__lm_shared_crpage_handle_sequence_deliver({{ name: 'sharedCrPageHandleSequenceBinding', seq: {seq}, result: '{result}' }}); 'delivered'"
            ),
            "awaitPromise": true
        });
        if let Some(context_id) = context_id {
            deliver_params["contextId"] = json!(context_id);
        }
        ctx.process_async(json!({
            "id": id,
            "method": "Runtime.evaluate",
            "sessionId": session_id,
            "params": deliver_params
        }))
        .await;
        let delivered = take_response_by_id(&mut ctx, id);
        assert_eq!(delivered["result"]["result"]["value"], json!("delivered"));

        let mut promise_params = json!({
            "expression": format!("globalThis.{promise_name}"),
            "awaitPromise": true
        });
        if let Some(context_id) = context_id {
            promise_params["contextId"] = json!(context_id);
        }
        ctx.process_async(json!({
            "id": id + 10,
            "method": "Runtime.evaluate",
            "sessionId": session_id,
            "params": promise_params
        }))
        .await;
        let resolved = take_response_by_id(&mut ctx, id + 10);
        assert_eq!(resolved["result"]["result"]["value"], json!(result));
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn patchright_over_cdp_auto_attach_sweep_crpage_remove_then_reexpose_binding_rehydrates_only_cleaned_context()
 {
    let mut ctx = TestContext::new();
    let first =
        create_attached_page_session_without_runtime_enable_async(&mut ctx, 2740, 2741, 2742).await;
    let second =
        create_attached_page_session_without_runtime_enable_async(&mut ctx, 2743, 2744, 2745).await;

    for (id, session_id, label) in [
        (
            2746_u64,
            first.session_id.as_str(),
            "first-crpage-rehydrate",
        ),
        (
            2747_u64,
            second.session_id.as_str(),
            "second-crpage-rehydrate",
        ),
    ] {
        ctx.process_async(json!({
            "id": id,
            "method": "Page.navigate",
            "sessionId": session_id,
            "params": {
                "url": format!("data:text/html,<body><div id='page'>{label}</div></body>")
            }
        }))
        .await;
        let navigation = take_response_by_id(&mut ctx, id);
        assert_eq!(navigation["sessionId"], json!(session_id));
        ctx.take_all();
    }

    for (id, target_id, session_id) in [
        (
            2748_u64,
            first.target_id.as_str(),
            first.session_id.as_str(),
        ),
        (
            2749_u64,
            second.target_id.as_str(),
            second.session_id.as_str(),
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
        "id": 2750,
        "method": "Target.setAutoAttach",
        "params": {
            "autoAttach": true,
            "waitForDebuggerOnStart": false
        }
    }))
    .await;
    ctx.expect_result(2750, json!({}), None);
    let events = ctx.take_all();
    let attached_events = events
        .iter()
        .filter(|event| event["method"] == json!("Target.attachedToTarget"))
        .cloned()
        .collect::<Vec<_>>();
    assert_eq!(
        attached_events.len(),
        2,
        "auto-attach sweep should attach both targets"
    );
    let first_auto_session = attached_events
        .iter()
        .find(|event| event["params"]["targetInfo"]["targetId"] == json!(first.target_id))
        .and_then(|event| event["params"]["sessionId"].as_str())
        .expect("first auto-attached session id")
        .to_owned();
    let second_auto_session = attached_events
        .iter()
        .find(|event| event["params"]["targetInfo"]["targetId"] == json!(second.target_id))
        .and_then(|event| event["params"]["sessionId"].as_str())
        .expect("second auto-attached session id")
        .to_owned();

    let mut first_utility_context = 0_i64;
    let mut second_utility_context = 0_i64;
    for (id, session_id, target_id, utility_context) in [
        (
            2751_u64,
            first_auto_session.as_str(),
            first.target_id.as_str(),
            &mut first_utility_context,
        ),
        (
            2752_u64,
            second_auto_session.as_str(),
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
        *utility_context = take_response_by_id(&mut ctx, id)["result"]["executionContextId"]
            .as_i64()
            .expect("utility context id after auto-attach");
        ctx.take_all();
    }

    let wrapper_source = patchright_page_binding_wrapper_source(
        "sharedCrPageRehydrateBinding",
        "__lm_shared_crpage_rehydrate_deliver",
        None,
        false,
    );
    for (id, session_id, utility_context) in [
        (2753_u64, first_auto_session.as_str(), first_utility_context),
        (
            2757_u64,
            second_auto_session.as_str(),
            second_utility_context,
        ),
    ] {
        ctx.process_async(json!({
            "id": id,
            "method": "Runtime.addBinding",
            "sessionId": session_id,
            "params": {
                "name": "sharedCrPageRehydrateBinding"
            }
        }))
        .await;
        assert_eq!(take_response_by_id(&mut ctx, id)["result"], json!({}));

        ctx.process_async(json!({
            "id": id + 1,
            "method": "Runtime.addBinding",
            "sessionId": session_id,
            "params": {
                "name": "sharedCrPageRehydrateBinding",
                "executionContextId": utility_context
            }
        }))
        .await;
        assert_eq!(take_response_by_id(&mut ctx, id + 1)["result"], json!({}));

        for offset in [2_u64, 3_u64] {
            let mut params = json!({
                "expression": wrapper_source,
                "awaitPromise": true
            });
            if offset == 3 {
                params["contextId"] = json!(utility_context);
            }
            ctx.process_async(json!({
                "id": id + offset,
                "method": "Runtime.evaluate",
                "sessionId": session_id,
                "params": params
            }))
            .await;
            let installed = take_response_by_id(&mut ctx, id + offset);
            assert_eq!(installed["result"]["result"]["value"], json!("function"));
        }
    }

    ctx.process_async(json!({
        "id": 2761,
        "method": "Runtime.removeBinding",
        "sessionId": first_auto_session,
        "params": {
            "name": "sharedCrPageRehydrateBinding"
        }
    }))
    .await;
    assert_eq!(take_response_by_id(&mut ctx, 2761)["result"], json!({}));

    for (id, session_id, context_id, expected_type) in [
        (2762_u64, first_auto_session.as_str(), None, "undefined"),
        (
            2763_u64,
            first_auto_session.as_str(),
            Some(first_utility_context),
            "undefined",
        ),
        (2764_u64, second_auto_session.as_str(), None, "function"),
        (
            2765_u64,
            second_auto_session.as_str(),
            Some(second_utility_context),
            "function",
        ),
    ] {
        let mut params = json!({
            "expression": "typeof globalThis.sharedCrPageRehydrateBinding"
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
        let state = take_response_by_id(&mut ctx, id);
        assert_eq!(state["result"]["result"]["value"], json!(expected_type));
    }

    for (id, context_id) in [(2766_u64, None), (2767_u64, Some(first_utility_context))] {
        ctx.process_async(json!({
            "id": id,
            "method": "Runtime.addBinding",
            "sessionId": first_auto_session,
            "params": if let Some(context_id) = context_id {
                json!({
                    "name": "sharedCrPageRehydrateBinding",
                    "executionContextId": context_id
                })
            } else {
                json!({
                    "name": "sharedCrPageRehydrateBinding"
                })
            }
        }))
        .await;
        assert_eq!(take_response_by_id(&mut ctx, id)["result"], json!({}));
    }

    for (id, context_id) in [(2768_u64, None), (2769_u64, Some(first_utility_context))] {
        let mut params = json!({
            "expression": wrapper_source,
            "awaitPromise": true
        });
        if let Some(context_id) = context_id {
            params["contextId"] = json!(context_id);
        }
        ctx.process_async(json!({
            "id": id,
            "method": "Runtime.evaluate",
            "sessionId": first_auto_session,
            "params": params
        }))
        .await;
        let installed = take_response_by_id(&mut ctx, id);
        assert_eq!(installed["result"]["result"]["value"], json!("function"));
    }

    let mut first_rehydrated_main_seq = 0_i64;
    let mut first_rehydrated_utility_seq = 0_i64;
    let mut second_kept_main_seq = 0_i64;
    let mut second_kept_utility_seq = 0_i64;
    let mut second_main_context = 0_i64;
    for (id, session_id, context_id, expression, serialized_arg, seq_out, main_context_out) in [
        (
            2770_u64,
            first_auto_session.as_str(),
            None,
            "globalThis.__lm_first_rehydrated_main = sharedCrPageRehydrateBinding('first-main-rehydrated'); 'scheduled-first-main'",
            "first-main-rehydrated",
            &mut first_rehydrated_main_seq,
            None,
        ),
        (
            2771_u64,
            first_auto_session.as_str(),
            Some(first_utility_context),
            "globalThis.__lm_first_rehydrated_utility = sharedCrPageRehydrateBinding('first-utility-rehydrated'); 'scheduled-first-utility'",
            "first-utility-rehydrated",
            &mut first_rehydrated_utility_seq,
            None,
        ),
        (
            2772_u64,
            second_auto_session.as_str(),
            None,
            "globalThis.__lm_second_kept_main = sharedCrPageRehydrateBinding('second-main-kept'); 'scheduled-second-main'",
            "second-main-kept",
            &mut second_kept_main_seq,
            Some(&mut second_main_context),
        ),
        (
            2773_u64,
            second_auto_session.as_str(),
            Some(second_utility_context),
            "globalThis.__lm_second_kept_utility = sharedCrPageRehydrateBinding('second-utility-kept'); 'scheduled-second-utility'",
            "second-utility-kept",
            &mut second_kept_utility_seq,
            None,
        ),
    ] {
        let mut params = json!({
            "expression": expression,
            "awaitPromise": true
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
        let scheduled = take_response_by_id(&mut ctx, id);
        assert!(
            scheduled["result"]["result"]["value"]
                .as_str()
                .expect("scheduled binding wrapper value")
                .starts_with("scheduled-")
        );
        let binding_called = ctx
            .sent
            .iter()
            .find(|message| {
                message["method"] == json!("Runtime.bindingCalled")
                    && message["params"]["name"] == json!("sharedCrPageRehydrateBinding")
            })
            .cloned()
            .expect("binding wrapper should emit Runtime.bindingCalled");
        let execution_context_id = binding_called["params"]["executionContextId"]
            .as_i64()
            .expect("execution context id");
        if let Some(main_context_out) = main_context_out {
            *main_context_out = execution_context_id;
        } else if let Some(context_id) = context_id {
            assert_eq!(execution_context_id, context_id);
        }
        let payload = binding_called["params"]["payload"]
            .as_str()
            .expect("binding payload should be a json string");
        let payload: serde_json::Value =
            serde_json::from_str(payload).expect("binding payload should be valid json");
        assert_eq!(payload["name"], json!("sharedCrPageRehydrateBinding"));
        assert_eq!(payload["serializedArgs"], json!([serialized_arg]));
        *seq_out = payload["seq"]
            .as_i64()
            .expect("binding payload seq should be an integer");
        ctx.sent.clear();
    }

    assert_eq!(first_rehydrated_main_seq, 1);
    assert_eq!(first_rehydrated_utility_seq, 1);
    assert_eq!(second_kept_main_seq, 1);
    assert_eq!(second_kept_utility_seq, 1);

    for (id, session_id, context_id, seq, result, promise_name) in [
        (
            2774_u64,
            first_auto_session.as_str(),
            None,
            first_rehydrated_main_seq,
            "first-main-rehydrated-resolved",
            "__lm_first_rehydrated_main",
        ),
        (
            2775_u64,
            first_auto_session.as_str(),
            Some(first_utility_context),
            first_rehydrated_utility_seq,
            "first-utility-rehydrated-resolved",
            "__lm_first_rehydrated_utility",
        ),
        (
            2776_u64,
            second_auto_session.as_str(),
            None,
            second_kept_main_seq,
            "second-main-kept-resolved",
            "__lm_second_kept_main",
        ),
        (
            2777_u64,
            second_auto_session.as_str(),
            Some(second_utility_context),
            second_kept_utility_seq,
            "second-utility-kept-resolved",
            "__lm_second_kept_utility",
        ),
    ] {
        let mut deliver_params = json!({
            "expression": format!(
                "globalThis.__lm_shared_crpage_rehydrate_deliver({{ name: 'sharedCrPageRehydrateBinding', seq: {seq}, result: '{result}' }}); 'delivered'"
            ),
            "awaitPromise": true
        });
        if let Some(context_id) = context_id {
            deliver_params["contextId"] = json!(context_id);
        }
        ctx.process_async(json!({
            "id": id,
            "method": "Runtime.evaluate",
            "sessionId": session_id,
            "params": deliver_params
        }))
        .await;
        let delivered = take_response_by_id(&mut ctx, id);
        assert_eq!(delivered["result"]["result"]["value"], json!("delivered"));

        let mut promise_params = json!({
            "expression": format!("globalThis.{promise_name}"),
            "awaitPromise": true
        });
        if let Some(context_id) = context_id {
            promise_params["contextId"] = json!(context_id);
        }
        ctx.process_async(json!({
            "id": id + 10,
            "method": "Runtime.evaluate",
            "sessionId": session_id,
            "params": promise_params
        }))
        .await;
        let resolved = take_response_by_id(&mut ctx, id + 10);
        assert_eq!(resolved["result"]["result"]["value"], json!(result));
    }

    assert_ne!(second_main_context, second_utility_context);
}

#[tokio::test(flavor = "multi_thread")]
async fn patchright_over_cdp_auto_attach_sweep_crpage_remove_then_reexpose_handle_binding_rehydrates_only_cleaned_context()
 {
    super::patchright_8mb_stack(
        "patchright-crpage-remove-reexpose-handle",
        || async {
    let mut ctx = TestContext::new();
    let first =
        create_attached_page_session_without_runtime_enable_async(&mut ctx, 2853, 2854, 2855).await;
    let second =
        create_attached_page_session_without_runtime_enable_async(&mut ctx, 2856, 2857, 2858).await;

    for (id, session_id, html) in [
        (
            2859_u64,
            first.session_id.as_str(),
            "<body><div id='first-rehydrate-handle'>first-rehydrate</div></body>",
        ),
        (
            2860_u64,
            second.session_id.as_str(),
            "<body><div id='second-kept-handle'>second-kept</div></body>",
        ),
    ] {
        ctx.process_async(json!({
            "id": id,
            "method": "Page.navigate",
            "sessionId": session_id,
            "params": {
                "url": format!("data:text/html,{html}")
            }
        }))
        .await;
        let navigation = take_response_by_id(&mut ctx, id);
        assert_eq!(navigation["sessionId"], json!(session_id));
        ctx.take_all();
    }

    for (id, target_id, session_id) in [
        (
            2861_u64,
            first.target_id.as_str(),
            first.session_id.as_str(),
        ),
        (
            2862_u64,
            second.target_id.as_str(),
            second.session_id.as_str(),
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
        "id": 2863,
        "method": "Target.setAutoAttach",
        "params": {
            "autoAttach": true,
            "waitForDebuggerOnStart": false
        }
    }))
    .await;
    ctx.expect_result(2863, json!({}), None);
    let events = ctx.take_all();
    let attached_events = events
        .iter()
        .filter(|event| event["method"] == json!("Target.attachedToTarget"))
        .cloned()
        .collect::<Vec<_>>();
    assert_eq!(
        attached_events.len(),
        2,
        "auto-attach sweep should attach both targets"
    );
    let first_auto_session = attached_events
        .iter()
        .find(|event| event["params"]["targetInfo"]["targetId"] == json!(first.target_id))
        .and_then(|event| event["params"]["sessionId"].as_str())
        .expect("first auto-attached session id")
        .to_owned();
    let second_auto_session = attached_events
        .iter()
        .find(|event| event["params"]["targetInfo"]["targetId"] == json!(second.target_id))
        .and_then(|event| event["params"]["sessionId"].as_str())
        .expect("second auto-attached session id")
        .to_owned();

    let mut first_utility_context = 0_i64;
    let mut second_utility_context = 0_i64;
    for (id, session_id, target_id, utility_context) in [
        (
            2864_u64,
            first_auto_session.as_str(),
            first.target_id.as_str(),
            &mut first_utility_context,
        ),
        (
            2865_u64,
            second_auto_session.as_str(),
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
        *utility_context = take_response_by_id(&mut ctx, id)["result"]["executionContextId"]
            .as_i64()
            .expect("utility context id after auto-attach");
        ctx.take_all();
    }

    let wrapper_source = patchright_page_binding_wrapper_source(
        "sharedCrPageHandleRehydrateBinding",
        "__lm_shared_crpage_handle_rehydrate_deliver",
        Some("__lm_shared_crpage_handle_rehydrate_take"),
        true,
    );
    for (id, session_id, utility_context) in [
        (2866_u64, first_auto_session.as_str(), first_utility_context),
        (
            2868_u64,
            second_auto_session.as_str(),
            second_utility_context,
        ),
    ] {
        ctx.process_async(json!({
            "id": id,
            "method": "Runtime.addBinding",
            "sessionId": session_id,
            "params": {
                "name": "sharedCrPageHandleRehydrateBinding",
                "executionContextId": utility_context
            }
        }))
        .await;
        assert_eq!(take_response_by_id(&mut ctx, id)["result"], json!({}));

        ctx.process_async(json!({
            "id": id + 1,
            "method": "Runtime.evaluate",
            "sessionId": session_id,
            "params": {
                "contextId": utility_context,
                "expression": wrapper_source,
                "awaitPromise": true
            }
        }))
        .await;
        let installed = take_response_by_id(&mut ctx, id + 1);
        assert_eq!(installed["result"]["result"]["value"], json!("function"));
    }

    ctx.process_async(json!({
        "id": 2870,
        "method": "Runtime.removeBinding",
        "sessionId": first_auto_session,
        "params": {
            "name": "sharedCrPageHandleRehydrateBinding"
        }
    }))
    .await;
    assert_eq!(take_response_by_id(&mut ctx, 2870)["result"], json!({}));

    for (id, session_id, utility_context, expected_type) in [
        (
            2871_u64,
            first_auto_session.as_str(),
            first_utility_context,
            "undefined",
        ),
        (
            2872_u64,
            second_auto_session.as_str(),
            second_utility_context,
            "function",
        ),
    ] {
        ctx.process_async(json!({
            "id": id,
            "method": "Runtime.evaluate",
            "sessionId": session_id,
            "params": {
                "contextId": utility_context,
                "expression": "typeof globalThis.sharedCrPageHandleRehydrateBinding"
            }
        }))
        .await;
        let state = take_response_by_id(&mut ctx, id);
        assert_eq!(state["result"]["result"]["value"], json!(expected_type));
    }

    ctx.process_async(json!({
        "id": 2873,
        "method": "Runtime.addBinding",
        "sessionId": first_auto_session,
        "params": {
            "name": "sharedCrPageHandleRehydrateBinding",
            "executionContextId": first_utility_context
        }
    }))
    .await;
    assert_eq!(take_response_by_id(&mut ctx, 2873)["result"], json!({}));

    ctx.process_async(json!({
        "id": 2874,
        "method": "Runtime.evaluate",
        "sessionId": first_auto_session,
        "params": {
            "contextId": first_utility_context,
            "expression": wrapper_source,
            "awaitPromise": true
        }
    }))
    .await;
    let reinstalled = take_response_by_id(&mut ctx, 2874);
    assert_eq!(reinstalled["result"]["result"]["value"], json!("function"));

    let mut first_seq = None;
    for (id, session_id, utility_context, expression, handle_id, expected_text, seq_out) in [
        (
            2875_u64,
            first_auto_session.as_str(),
            first_utility_context,
            "globalThis.__lm_first_rehydrated_handle = sharedCrPageHandleRehydrateBinding(document.getElementById('first-rehydrate-handle')); 'scheduled-first-rehydrated'",
            "first-rehydrate-handle",
            "first-rehydrate",
            true,
        ),
        (
            2876_u64,
            second_auto_session.as_str(),
            second_utility_context,
            "globalThis.__lm_second_kept_handle = sharedCrPageHandleRehydrateBinding(document.getElementById('second-kept-handle')); 'scheduled-second-kept'",
            "second-kept-handle",
            "second-kept",
            false,
        ),
    ] {
        ctx.process_async(json!({
            "id": id,
            "method": "Runtime.evaluate",
            "sessionId": session_id,
            "params": {
                "contextId": utility_context,
                "expression": expression,
                "awaitPromise": true
            }
        }))
        .await;
        let scheduled = take_response_by_id(&mut ctx, id);
        assert!(
            scheduled["result"]["result"]["value"]
                .as_str()
                .expect("scheduled handle wrapper value")
                .starts_with("scheduled-")
        );
        let binding_called = ctx
            .sent
            .iter()
            .find(|message| {
                message["method"] == json!("Runtime.bindingCalled")
                    && message["params"]["name"] == json!("sharedCrPageHandleRehydrateBinding")
                    && message["params"]["executionContextId"] == json!(utility_context)
            })
            .cloned()
            .expect("handle wrapper should emit Runtime.bindingCalled");
        let payload = binding_called["params"]["payload"]
            .as_str()
            .expect("binding payload should be string");
        let payload: serde_json::Value =
            serde_json::from_str(payload).expect("binding payload should be valid json");
        let seq = payload["seq"]
            .as_i64()
            .expect("binding seq should be integer");
        assert_eq!(seq, 1);
        assert_eq!(payload["name"], json!("sharedCrPageHandleRehydrateBinding"));
        ctx.sent.clear();

        ctx.process_async(json!({
                "id": id + 10,
                "method": "Runtime.evaluate",
                "sessionId": session_id,
                "params": {
                    "contextId": utility_context,
                    "expression": format!(
                        "(() => {{ const handle = globalThis.__lm_shared_crpage_handle_rehydrate_take({{ name: 'sharedCrPageHandleRehydrateBinding', seq: {seq} }}); return JSON.stringify([handle.id, handle.textContent, typeof globalThis.__lm_shared_crpage_handle_rehydrate_take({{ name: 'sharedCrPageHandleRehydrateBinding', seq: {seq} }})]); }})()"
                    )
                }
            })).await;
        let taken = take_response_by_id(&mut ctx, id + 10);
        assert_eq!(
            taken["result"]["result"]["value"],
            json!(format!(
                "[\"{handle_id}\",\"{expected_text}\",\"undefined\"]"
            ))
        );

        ctx.process_async(json!({
                "id": id + 20,
                "method": "Runtime.evaluate",
                "sessionId": session_id,
                "params": {
                    "contextId": utility_context,
                    "expression": format!(
                        "globalThis.__lm_shared_crpage_handle_rehydrate_deliver({{ name: 'sharedCrPageHandleRehydrateBinding', seq: {seq}, result: '{expected_text}-resolved' }}); 'delivered'"
                    ),
                    "awaitPromise": true
                }
            })).await;
        let delivered = take_response_by_id(&mut ctx, id + 20);
        assert_eq!(delivered["result"]["result"]["value"], json!("delivered"));

        if seq_out {
            first_seq = Some(seq);
        }
    }

    assert_eq!(first_seq, Some(1));

    ctx.process_async(json!({
        "id": 2897,
        "method": "Runtime.evaluate",
        "sessionId": first_auto_session,
        "params": {
            "contextId": first_utility_context,
            "expression": "globalThis.__lm_first_rehydrated_handle",
            "awaitPromise": true
        }
    }))
    .await;
    let first_resolved = take_response_by_id(&mut ctx, 2897);
    assert_eq!(
        first_resolved["result"]["result"]["value"],
        json!("first-rehydrate-resolved")
    );

    ctx.process_async(json!({
        "id": 2898,
        "method": "Runtime.evaluate",
        "sessionId": second_auto_session,
        "params": {
            "contextId": second_utility_context,
            "expression": "globalThis.__lm_second_kept_handle",
            "awaitPromise": true
        }
    }))
    .await;
    let second_resolved = take_response_by_id(&mut ctx, 2898);
    assert_eq!(
        second_resolved["result"]["result"]["value"],
        json!("second-kept-resolved")
    );
        },
    )
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn patchright_over_cdp_auto_attach_sweep_crpage_reexpose_source_is_idempotent_and_isolated_per_browser_context()
 {
    let mut ctx = TestContext::new();
    let first =
        create_attached_page_session_without_runtime_enable_async(&mut ctx, 2788, 2789, 2790).await;
    let second =
        create_attached_page_session_without_runtime_enable_async(&mut ctx, 2791, 2792, 2793).await;

    for (id, session_id, label) in [
        (
            2794_u64,
            first.session_id.as_str(),
            "first-crpage-idempotent",
        ),
        (
            2795_u64,
            second.session_id.as_str(),
            "second-crpage-idempotent",
        ),
    ] {
        ctx.process_async(json!({
            "id": id,
            "method": "Page.navigate",
            "sessionId": session_id,
            "params": {
                "url": format!("data:text/html,<body><div id='page'>{label}</div></body>")
            }
        }))
        .await;
        let navigation = take_response_by_id(&mut ctx, id);
        assert_eq!(navigation["sessionId"], json!(session_id));
        ctx.take_all();
    }

    for (id, target_id, session_id) in [
        (
            2796_u64,
            first.target_id.as_str(),
            first.session_id.as_str(),
        ),
        (
            2797_u64,
            second.target_id.as_str(),
            second.session_id.as_str(),
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
        "id": 2798,
        "method": "Target.setAutoAttach",
        "params": {
            "autoAttach": true,
            "waitForDebuggerOnStart": false
        }
    }))
    .await;
    ctx.expect_result(2798, json!({}), None);
    let events = ctx.take_all();
    let attached_events = events
        .iter()
        .filter(|event| event["method"] == json!("Target.attachedToTarget"))
        .cloned()
        .collect::<Vec<_>>();
    assert_eq!(
        attached_events.len(),
        2,
        "auto-attach sweep should attach both targets"
    );
    let first_auto_session = attached_events
        .iter()
        .find(|event| event["params"]["targetInfo"]["targetId"] == json!(first.target_id))
        .and_then(|event| event["params"]["sessionId"].as_str())
        .expect("first auto-attached session id")
        .to_owned();
    let second_auto_session = attached_events
        .iter()
        .find(|event| event["params"]["targetInfo"]["targetId"] == json!(second.target_id))
        .and_then(|event| event["params"]["sessionId"].as_str())
        .expect("second auto-attached session id")
        .to_owned();

    let mut first_utility_context = 0_i64;
    let mut second_utility_context = 0_i64;
    for (id, session_id, target_id, utility_context) in [
        (
            2799_u64,
            first_auto_session.as_str(),
            first.target_id.as_str(),
            &mut first_utility_context,
        ),
        (
            2800_u64,
            second_auto_session.as_str(),
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
        *utility_context = take_response_by_id(&mut ctx, id)["result"]["executionContextId"]
            .as_i64()
            .expect("utility context id after auto-attach");
        ctx.take_all();
    }

    let wrapper_source = patchright_page_binding_wrapper_source(
        "sharedCrPageIdempotentFanoutBinding",
        "__lm_shared_crpage_idempotent_fanout_deliver",
        None,
        false,
    );
    for (id, session_id, utility_context) in [
        (2801_u64, first_auto_session.as_str(), first_utility_context),
        (
            2805_u64,
            second_auto_session.as_str(),
            second_utility_context,
        ),
    ] {
        ctx.process_async(json!({
            "id": id,
            "method": "Runtime.addBinding",
            "sessionId": session_id,
            "params": {
                "name": "sharedCrPageIdempotentFanoutBinding"
            }
        }))
        .await;
        assert_eq!(take_response_by_id(&mut ctx, id)["result"], json!({}));

        ctx.process_async(json!({
            "id": id + 1,
            "method": "Runtime.addBinding",
            "sessionId": session_id,
            "params": {
                "name": "sharedCrPageIdempotentFanoutBinding",
                "executionContextId": utility_context
            }
        }))
        .await;
        assert_eq!(take_response_by_id(&mut ctx, id + 1)["result"], json!({}));

        for offset in [2_u64, 3_u64] {
            let mut params = json!({
                "expression": wrapper_source,
                "awaitPromise": true
            });
            if offset == 3 {
                params["contextId"] = json!(utility_context);
            }
            ctx.process_async(json!({
                "id": id + offset,
                "method": "Runtime.evaluate",
                "sessionId": session_id,
                "params": params
            }))
            .await;
            let installed = take_response_by_id(&mut ctx, id + offset);
            assert_eq!(installed["result"]["result"]["value"], json!("function"));
        }
    }

    let mut first_main_seq_1 = 0_i64;
    let mut first_utility_seq_1 = 0_i64;
    for (id, context_id, expression, serialized_arg, seq_out) in [
        (
            2809_u64,
            None,
            "globalThis.__lm_first_idempotent_main_1 = sharedCrPageIdempotentFanoutBinding('first-main-1'); 'scheduled-first-main-1'",
            "first-main-1",
            &mut first_main_seq_1,
        ),
        (
            2810_u64,
            Some(first_utility_context),
            "globalThis.__lm_first_idempotent_utility_1 = sharedCrPageIdempotentFanoutBinding('first-utility-1'); 'scheduled-first-utility-1'",
            "first-utility-1",
            &mut first_utility_seq_1,
        ),
    ] {
        let mut params = json!({
            "expression": expression,
            "awaitPromise": true
        });
        if let Some(context_id) = context_id {
            params["contextId"] = json!(context_id);
        }
        ctx.process_async(json!({
            "id": id,
            "method": "Runtime.evaluate",
            "sessionId": first_auto_session,
            "params": params
        }))
        .await;
        let scheduled = take_response_by_id(&mut ctx, id);
        assert!(
            scheduled["result"]["result"]["value"]
                .as_str()
                .expect("scheduled binding wrapper value")
                .starts_with("scheduled-")
        );
        let binding_called = ctx
            .sent
            .iter()
            .find(|message| {
                message["method"] == json!("Runtime.bindingCalled")
                    && message["params"]["name"] == json!("sharedCrPageIdempotentFanoutBinding")
            })
            .cloned()
            .expect("binding wrapper should emit Runtime.bindingCalled");
        let payload = binding_called["params"]["payload"]
            .as_str()
            .expect("binding payload should be a json string");
        let payload: serde_json::Value =
            serde_json::from_str(payload).expect("binding payload should be valid json");
        assert_eq!(payload["serializedArgs"], json!([serialized_arg]));
        *seq_out = payload["seq"]
            .as_i64()
            .expect("binding seq should be integer");
        assert_eq!(*seq_out, 1);
        ctx.sent.clear();
    }

    for (id, context_id) in [(2811_u64, None), (2812_u64, Some(first_utility_context))] {
        let mut params = json!({
            "expression": wrapper_source,
            "awaitPromise": true
        });
        if let Some(context_id) = context_id {
            params["contextId"] = json!(context_id);
        }
        ctx.process_async(json!({
            "id": id,
            "method": "Runtime.evaluate",
            "sessionId": first_auto_session,
            "params": params
        }))
        .await;
        let reinstalled = take_response_by_id(&mut ctx, id);
        assert_eq!(reinstalled["result"]["result"]["value"], json!("function"));
    }

    let mut first_main_seq_2 = 0_i64;
    let mut first_utility_seq_2 = 0_i64;
    let mut second_main_seq_1 = 0_i64;
    let mut second_main_context = 0_i64;
    let mut second_utility_seq_1 = 0_i64;
    for (id, session_id, context_id, expression, serialized_arg, seq_out, capture_main_context) in [
        (
            2813_u64,
            first_auto_session.as_str(),
            None,
            "globalThis.__lm_first_idempotent_main_2 = sharedCrPageIdempotentFanoutBinding('first-main-2'); 'scheduled-first-main-2'",
            "first-main-2",
            &mut first_main_seq_2,
            false,
        ),
        (
            2814_u64,
            first_auto_session.as_str(),
            Some(first_utility_context),
            "globalThis.__lm_first_idempotent_utility_2 = sharedCrPageIdempotentFanoutBinding('first-utility-2'); 'scheduled-first-utility-2'",
            "first-utility-2",
            &mut first_utility_seq_2,
            false,
        ),
        (
            2815_u64,
            second_auto_session.as_str(),
            None,
            "globalThis.__lm_second_idempotent_main_1 = sharedCrPageIdempotentFanoutBinding('second-main-1'); 'scheduled-second-main-1'",
            "second-main-1",
            &mut second_main_seq_1,
            true,
        ),
        (
            2816_u64,
            second_auto_session.as_str(),
            Some(second_utility_context),
            "globalThis.__lm_second_idempotent_utility_1 = sharedCrPageIdempotentFanoutBinding('second-utility-1'); 'scheduled-second-utility-1'",
            "second-utility-1",
            &mut second_utility_seq_1,
            false,
        ),
    ] {
        let mut params = json!({
            "expression": expression,
            "awaitPromise": true
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
        let scheduled = take_response_by_id(&mut ctx, id);
        assert!(
            scheduled["result"]["result"]["value"]
                .as_str()
                .expect("scheduled binding wrapper value")
                .starts_with("scheduled-")
        );
        let binding_called = ctx
            .sent
            .iter()
            .find(|message| {
                message["method"] == json!("Runtime.bindingCalled")
                    && message["params"]["name"] == json!("sharedCrPageIdempotentFanoutBinding")
            })
            .cloned()
            .expect("binding wrapper should emit Runtime.bindingCalled");
        if capture_main_context {
            second_main_context = binding_called["params"]["executionContextId"]
                .as_i64()
                .expect("second main execution context");
        }
        let payload = binding_called["params"]["payload"]
            .as_str()
            .expect("binding payload should be a json string");
        let payload: serde_json::Value =
            serde_json::from_str(payload).expect("binding payload should be valid json");
        assert_eq!(payload["serializedArgs"], json!([serialized_arg]));
        *seq_out = payload["seq"]
            .as_i64()
            .expect("binding seq should be integer");
        ctx.sent.clear();
    }

    assert_eq!(first_main_seq_2, 2);
    assert_eq!(first_utility_seq_2, 2);
    assert_eq!(second_main_seq_1, 1);
    assert_eq!(second_utility_seq_1, 1);
    assert_ne!(second_main_context, second_utility_context);

    for (id, session_id, context_id, seq, result, promise_name) in [
        (
            2817_u64,
            first_auto_session.as_str(),
            None,
            first_main_seq_2,
            "resolved-first-main-2",
            "__lm_first_idempotent_main_2",
        ),
        (
            2818_u64,
            first_auto_session.as_str(),
            Some(first_utility_context),
            first_utility_seq_2,
            "resolved-first-utility-2",
            "__lm_first_idempotent_utility_2",
        ),
        (
            2819_u64,
            second_auto_session.as_str(),
            None,
            second_main_seq_1,
            "resolved-second-main-1",
            "__lm_second_idempotent_main_1",
        ),
        (
            2820_u64,
            second_auto_session.as_str(),
            Some(second_utility_context),
            second_utility_seq_1,
            "resolved-second-utility-1",
            "__lm_second_idempotent_utility_1",
        ),
        (
            2821_u64,
            first_auto_session.as_str(),
            None,
            first_main_seq_1,
            "resolved-first-main-1",
            "__lm_first_idempotent_main_1",
        ),
        (
            2822_u64,
            first_auto_session.as_str(),
            Some(first_utility_context),
            first_utility_seq_1,
            "resolved-first-utility-1",
            "__lm_first_idempotent_utility_1",
        ),
    ] {
        let mut deliver_params = json!({
            "expression": format!(
                "globalThis.__lm_shared_crpage_idempotent_fanout_deliver({{ name: 'sharedCrPageIdempotentFanoutBinding', seq: {seq}, result: '{result}' }}); 'delivered'"
            ),
            "awaitPromise": true
        });
        if let Some(context_id) = context_id {
            deliver_params["contextId"] = json!(context_id);
        }
        ctx.process_async(json!({
            "id": id,
            "method": "Runtime.evaluate",
            "sessionId": session_id,
            "params": deliver_params
        }))
        .await;
        let delivered = take_response_by_id(&mut ctx, id);
        assert_eq!(delivered["result"]["result"]["value"], json!("delivered"));

        let mut promise_params = json!({
            "expression": format!("globalThis.{promise_name}"),
            "awaitPromise": true
        });
        if let Some(context_id) = context_id {
            promise_params["contextId"] = json!(context_id);
        }
        ctx.process_async(json!({
            "id": id + 10,
            "method": "Runtime.evaluate",
            "sessionId": session_id,
            "params": promise_params
        }))
        .await;
        let resolved = take_response_by_id(&mut ctx, id + 10);
        assert_eq!(resolved["result"]["result"]["value"], json!(result));
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn patchright_over_cdp_auto_attach_sweep_crpage_handle_reexpose_source_is_idempotent_and_isolated_per_browser_context()
 {
    super::patchright_8mb_stack(
        "patchright-crpage-handle-reexpose-idempotent",
        || async {
    let mut ctx = TestContext::new();
    let first =
        create_attached_page_session_without_runtime_enable_async(&mut ctx, 2823, 2824, 2825).await;
    let second =
        create_attached_page_session_without_runtime_enable_async(&mut ctx, 2826, 2827, 2828).await;

    for (id, session_id, html) in [
        (
            2829_u64,
            first.session_id.as_str(),
            "<body><div id='first-handle-1'>first-1</div><div id='first-handle-2'>first-2</div></body>",
        ),
        (
            2830_u64,
            second.session_id.as_str(),
            "<body><div id='second-handle-1'>second-1</div></body>",
        ),
    ] {
        ctx.process_async(json!({
            "id": id,
            "method": "Page.navigate",
            "sessionId": session_id,
            "params": {
                "url": format!("data:text/html,{html}")
            }
        }))
        .await;
        let navigation = take_response_by_id(&mut ctx, id);
        assert_eq!(navigation["sessionId"], json!(session_id));
        ctx.take_all();
    }

    for (id, target_id, session_id) in [
        (
            2831_u64,
            first.target_id.as_str(),
            first.session_id.as_str(),
        ),
        (
            2832_u64,
            second.target_id.as_str(),
            second.session_id.as_str(),
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
        "id": 2833,
        "method": "Target.setAutoAttach",
        "params": {
            "autoAttach": true,
            "waitForDebuggerOnStart": false
        }
    }))
    .await;
    ctx.expect_result(2833, json!({}), None);
    let events = ctx.take_all();
    let attached_events = events
        .iter()
        .filter(|event| event["method"] == json!("Target.attachedToTarget"))
        .cloned()
        .collect::<Vec<_>>();
    assert_eq!(
        attached_events.len(),
        2,
        "auto-attach sweep should attach both targets"
    );
    let first_auto_session = attached_events
        .iter()
        .find(|event| event["params"]["targetInfo"]["targetId"] == json!(first.target_id))
        .and_then(|event| event["params"]["sessionId"].as_str())
        .expect("first auto-attached session id")
        .to_owned();
    let second_auto_session = attached_events
        .iter()
        .find(|event| event["params"]["targetInfo"]["targetId"] == json!(second.target_id))
        .and_then(|event| event["params"]["sessionId"].as_str())
        .expect("second auto-attached session id")
        .to_owned();

    let mut first_utility_context = 0_i64;
    let mut second_utility_context = 0_i64;
    for (id, session_id, target_id, utility_context) in [
        (
            2834_u64,
            first_auto_session.as_str(),
            first.target_id.as_str(),
            &mut first_utility_context,
        ),
        (
            2835_u64,
            second_auto_session.as_str(),
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
        *utility_context = take_response_by_id(&mut ctx, id)["result"]["executionContextId"]
            .as_i64()
            .expect("utility context id after auto-attach");
        ctx.take_all();
    }

    let wrapper_source = patchright_page_binding_wrapper_source(
        "sharedCrPageHandleIdempotentFanoutBinding",
        "__lm_shared_crpage_handle_idempotent_fanout_deliver",
        Some("__lm_shared_crpage_handle_idempotent_fanout_take"),
        true,
    );
    for (id, session_id, utility_context) in [
        (2836_u64, first_auto_session.as_str(), first_utility_context),
        (
            2839_u64,
            second_auto_session.as_str(),
            second_utility_context,
        ),
    ] {
        ctx.process_async(json!({
            "id": id,
            "method": "Runtime.addBinding",
            "sessionId": session_id,
            "params": {
                "name": "sharedCrPageHandleIdempotentFanoutBinding",
                "executionContextId": utility_context
            }
        }))
        .await;
        assert_eq!(take_response_by_id(&mut ctx, id)["result"], json!({}));

        ctx.process_async(json!({
            "id": id + 1,
            "method": "Runtime.evaluate",
            "sessionId": session_id,
            "params": {
                "contextId": utility_context,
                "expression": wrapper_source,
                "awaitPromise": true
            }
        }))
        .await;
        let installed = take_response_by_id(&mut ctx, id + 1);
        assert_eq!(installed["result"]["result"]["value"], json!("function"));
    }

    ctx.process_async(json!({
            "id": 2842,
            "method": "Runtime.evaluate",
            "sessionId": first_auto_session,
            "params": {
                "contextId": first_utility_context,
                "expression": "globalThis.__lm_first_handle_promise_1 = sharedCrPageHandleIdempotentFanoutBinding(document.getElementById('first-handle-1')); 'scheduled-first-1'",
                "awaitPromise": true
            }
        })).await;
    let first_scheduled_1 = take_response_by_id(&mut ctx, 2842);
    assert_eq!(
        first_scheduled_1["result"]["result"]["value"],
        json!("scheduled-first-1")
    );
    let first_binding_called_1 = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Runtime.bindingCalled")
                && message["params"]["name"] == json!("sharedCrPageHandleIdempotentFanoutBinding")
                && message["params"]["executionContextId"] == json!(first_utility_context)
        })
        .cloned()
        .expect("first utility handle binding should emit Runtime.bindingCalled");
    let first_payload_1 = first_binding_called_1["params"]["payload"]
        .as_str()
        .expect("binding payload should be string");
    let first_payload_1: serde_json::Value =
        serde_json::from_str(first_payload_1).expect("binding payload should be valid json");
    let first_seq_1 = first_payload_1["seq"]
        .as_i64()
        .expect("first seq should be integer");
    assert_eq!(first_seq_1, 1);
    ctx.sent.clear();

    ctx.process_async(json!({
            "id": 2843,
            "method": "Runtime.evaluate",
            "sessionId": first_auto_session,
            "params": {
                "contextId": first_utility_context,
                "expression": format!("(() => {{ const handle = globalThis.__lm_shared_crpage_handle_idempotent_fanout_take({{ name: 'sharedCrPageHandleIdempotentFanoutBinding', seq: {first_seq_1} }}); return JSON.stringify([handle.id, handle.textContent, typeof globalThis.__lm_shared_crpage_handle_idempotent_fanout_take({{ name: 'sharedCrPageHandleIdempotentFanoutBinding', seq: {first_seq_1} }})]); }})()")
            }
        })).await;
    let first_taken_1 = take_response_by_id(&mut ctx, 2843);
    assert_eq!(
        first_taken_1["result"]["result"]["value"],
        json!("[\"first-handle-1\",\"first-1\",\"undefined\"]")
    );

    ctx.process_async(json!({
        "id": 2844,
        "method": "Runtime.evaluate",
        "sessionId": first_auto_session,
        "params": {
            "contextId": first_utility_context,
            "expression": wrapper_source,
            "awaitPromise": true
        }
    }))
    .await;
    let first_reinstalled = take_response_by_id(&mut ctx, 2844);
    assert_eq!(
        first_reinstalled["result"]["result"]["value"],
        json!("function")
    );

    ctx.process_async(json!({
            "id": 2845,
            "method": "Runtime.evaluate",
            "sessionId": first_auto_session,
            "params": {
                "contextId": first_utility_context,
                "expression": "globalThis.__lm_first_handle_promise_2 = sharedCrPageHandleIdempotentFanoutBinding(document.getElementById('first-handle-2')); 'scheduled-first-2'",
                "awaitPromise": true
            }
        })).await;
    let first_scheduled_2 = take_response_by_id(&mut ctx, 2845);
    assert_eq!(
        first_scheduled_2["result"]["result"]["value"],
        json!("scheduled-first-2")
    );
    let first_binding_called_2 = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Runtime.bindingCalled")
                && message["params"]["name"] == json!("sharedCrPageHandleIdempotentFanoutBinding")
                && message["params"]["executionContextId"] == json!(first_utility_context)
        })
        .cloned()
        .expect("reexposed first utility handle binding should emit Runtime.bindingCalled");
    let first_payload_2 = first_binding_called_2["params"]["payload"]
        .as_str()
        .expect("binding payload should be string");
    let first_payload_2: serde_json::Value =
        serde_json::from_str(first_payload_2).expect("binding payload should be valid json");
    let first_seq_2 = first_payload_2["seq"]
        .as_i64()
        .expect("second seq should be integer");
    assert_eq!(first_seq_2, 2);
    ctx.sent.clear();

    ctx.process_async(json!({
            "id": 2846,
            "method": "Runtime.evaluate",
            "sessionId": first_auto_session,
            "params": {
                "contextId": first_utility_context,
                "expression": format!("(() => {{ const handle = globalThis.__lm_shared_crpage_handle_idempotent_fanout_take({{ name: 'sharedCrPageHandleIdempotentFanoutBinding', seq: {first_seq_2} }}); return JSON.stringify([handle.id, handle.textContent, typeof globalThis.__lm_shared_crpage_handle_idempotent_fanout_take({{ name: 'sharedCrPageHandleIdempotentFanoutBinding', seq: {first_seq_2} }})]); }})()")
            }
        })).await;
    let first_taken_2 = take_response_by_id(&mut ctx, 2846);
    assert_eq!(
        first_taken_2["result"]["result"]["value"],
        json!("[\"first-handle-2\",\"first-2\",\"undefined\"]")
    );

    ctx.process_async(json!({
            "id": 2847,
            "method": "Runtime.evaluate",
            "sessionId": second_auto_session,
            "params": {
                "contextId": second_utility_context,
                "expression": "globalThis.__lm_second_handle_promise_1 = sharedCrPageHandleIdempotentFanoutBinding(document.getElementById('second-handle-1')); 'scheduled-second-1'",
                "awaitPromise": true
            }
        })).await;
    let second_scheduled_1 = take_response_by_id(&mut ctx, 2847);
    assert_eq!(
        second_scheduled_1["result"]["result"]["value"],
        json!("scheduled-second-1")
    );
    let second_binding_called_1 = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Runtime.bindingCalled")
                && message["params"]["name"] == json!("sharedCrPageHandleIdempotentFanoutBinding")
                && message["params"]["executionContextId"] == json!(second_utility_context)
        })
        .cloned()
        .expect("second utility handle binding should emit Runtime.bindingCalled");
    let second_payload_1 = second_binding_called_1["params"]["payload"]
        .as_str()
        .expect("binding payload should be string");
    let second_payload_1: serde_json::Value =
        serde_json::from_str(second_payload_1).expect("binding payload should be valid json");
    let second_seq_1 = second_payload_1["seq"]
        .as_i64()
        .expect("second context seq should be integer");
    assert_eq!(second_seq_1, 1);
    ctx.sent.clear();

    ctx.process_async(json!({
            "id": 2848,
            "method": "Runtime.evaluate",
            "sessionId": second_auto_session,
            "params": {
                "contextId": second_utility_context,
                "expression": format!("(() => {{ const handle = globalThis.__lm_shared_crpage_handle_idempotent_fanout_take({{ name: 'sharedCrPageHandleIdempotentFanoutBinding', seq: {second_seq_1} }}); return JSON.stringify([handle.id, handle.textContent, typeof globalThis.__lm_shared_crpage_handle_idempotent_fanout_take({{ name: 'sharedCrPageHandleIdempotentFanoutBinding', seq: {second_seq_1} }})]); }})()")
            }
        })).await;
    let second_taken_1 = take_response_by_id(&mut ctx, 2848);
    assert_eq!(
        second_taken_1["result"]["result"]["value"],
        json!("[\"second-handle-1\",\"second-1\",\"undefined\"]")
    );

    for (id, session_id, context_id, seq, result, promise_name) in [
        (
            2849_u64,
            first_auto_session.as_str(),
            first_utility_context,
            first_seq_2,
            "resolved-first-2",
            "__lm_first_handle_promise_2",
        ),
        (
            2850_u64,
            second_auto_session.as_str(),
            second_utility_context,
            second_seq_1,
            "resolved-second-1",
            "__lm_second_handle_promise_1",
        ),
        (
            2851_u64,
            first_auto_session.as_str(),
            first_utility_context,
            first_seq_1,
            "resolved-first-1",
            "__lm_first_handle_promise_1",
        ),
    ] {
        ctx.process_async(json!({
                "id": id,
                "method": "Runtime.evaluate",
                "sessionId": session_id,
                "params": {
                    "contextId": context_id,
                    "expression": format!(
                        "globalThis.__lm_shared_crpage_handle_idempotent_fanout_deliver({{ name: 'sharedCrPageHandleIdempotentFanoutBinding', seq: {seq}, result: '{result}' }}); 'delivered'"
                    ),
                    "awaitPromise": true
                }
            })).await;
        let delivered = take_response_by_id(&mut ctx, id);
        assert_eq!(delivered["result"]["result"]["value"], json!("delivered"));

        ctx.process_async(json!({
            "id": id + 10,
            "method": "Runtime.evaluate",
            "sessionId": session_id,
            "params": {
                "contextId": context_id,
                "expression": format!("globalThis.{promise_name}"),
                "awaitPromise": true
            }
        }))
        .await;
        let resolved = take_response_by_id(&mut ctx, id + 10);
        assert_eq!(resolved["result"]["result"]["value"], json!(result));
    }
        },
    )
    .await;
}
