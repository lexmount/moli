use super::*;

#[tokio::test(flavor = "multi_thread")]
async fn patchright_over_cdp_page_binding_handle_round_trip_resolves_without_runtime_enable() {
    let mut ctx = TestContext::new();

    ctx.process_async(json!({
        "id": 312,
        "method": "Target.createBrowserContext"
    }))
    .await;
    let browser_context_id = take_response_by_id(&mut ctx, 312)["result"]["browserContextId"]
        .as_str()
        .expect("browser context id")
        .to_owned();

    ctx.process_async(json!({
        "id": 313,
        "method": "Target.createTarget",
        "params": {
            "browserContextId": browser_context_id,
            "url": "about:blank"
        }
    }))
    .await;
    let target_id = take_response_by_id(&mut ctx, 313)["result"]["targetId"]
        .as_str()
        .expect("target id")
        .to_owned();
    ctx.expect_event(
        "Target.targetCreated",
        Some(&json!({
            "targetInfo": {
                "targetId": target_id,
                "browserContextId": browser_context_id,
            }
        })),
    );

    ctx.process_async(json!({
        "id": 314,
        "method": "Target.attachToTarget",
        "params": {
            "targetId": target_id,
            "flatten": true
        }
    }))
    .await;
    let session_id = ctx.with_fixture(|conn| {
        conn.browser_context
            .as_ref()
            .and_then(|bc| bc.active_session_id_owned())
            .expect("session id should exist")
    });
    ctx.expect_result(314, json!({ "sessionId": session_id }), None);
    ctx.expect_event(
        "Target.attachedToTarget",
        Some(&json!({
            "sessionId": session_id,
            "targetInfo": {
                "targetId": target_id,
                "browserContextId": browser_context_id,
            }
        })),
    );

    ctx.process_async(json!({
        "id": 315,
        "method": "Page.navigate",
        "sessionId": session_id,
        "params": {
            "url": "data:text/html,<body><div id='handle-me'>node</div></body>"
        }
    }))
    .await;
    let navigation = take_response_by_id(&mut ctx, 315);
    assert_eq!(navigation["sessionId"], json!(session_id));
    ctx.take_all();

    ctx.process_async(json!({
        "id": 316,
        "method": "Page.createIsolatedWorld",
        "sessionId": session_id,
        "params": {
            "frameId": target_id,
            "worldName": "utility"
        }
    }))
    .await;
    let utility_context_id = take_response_by_id(&mut ctx, 316)["result"]["executionContextId"]
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
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 317,
        "method": "Runtime.addBinding",
        "sessionId": session_id,
        "params": {
            "name": "patchedHandleBinding",
            "executionContextId": utility_context_id
        }
    }))
    .await;
    let add_binding = take_response_by_id(&mut ctx, 317);
    assert_eq!(add_binding["result"], json!({}));

    ctx.process_async(json!({
            "id": 318,
            "method": "Runtime.evaluate",
            "sessionId": session_id,
            "params": {
                "contextId": utility_context_id,
                "expression": r#"
                    (() => {
                        function addHandleBinding(bindingName) {
                            const binding = globalThis[bindingName];
                            globalThis[bindingName] = (...args) => {
                                const me = globalThis[bindingName];
                                let callbacks = me.callbacks;
                                if (!callbacks) {
                                    callbacks = new Map();
                                    me.callbacks = callbacks;
                                }
                                let handles = me.handles;
                                if (!handles) {
                                    handles = new Map();
                                    me.handles = handles;
                                }
                                const seq = (me.lastSeq || 0) + 1;
                                me.lastSeq = seq;
                                handles.set(seq, args[0]);
                                const promise = new Promise((resolve, reject) => callbacks.set(seq, { resolve, reject }));
                                binding(JSON.stringify({ name: bindingName, seq }));
                                return promise;
                            };
                        }
                        function takeBindingHandle(arg) {
                            const handles = globalThis[arg.name].handles;
                            const handle = handles.get(arg.seq);
                            handles.delete(arg.seq);
                            return handle;
                        }
                        function deliverBindingResult(arg) {
                            const callbacks = globalThis[arg.name].callbacks;
                            if ('error' in arg)
                                callbacks.get(arg.seq).reject(arg.error);
                            else
                                callbacks.get(arg.seq).resolve(arg.result);
                            callbacks.delete(arg.seq);
                        }
                        addHandleBinding('patchedHandleBinding');
                        globalThis.__lm_takeHandleBindingHandle = takeBindingHandle;
                        globalThis.__lm_deliverHandleBindingResult = deliverBindingResult;
                        return typeof globalThis.patchedHandleBinding;
                    })()
                "#
            }
        })).await;
    let install_wrapper = take_response_by_id(&mut ctx, 318);
    assert_eq!(
        install_wrapper["result"]["result"]["value"],
        json!("function")
    );

    ctx.process_async(json!({
            "id": 319,
            "method": "Runtime.evaluate",
            "sessionId": session_id,
            "params": {
                "contextId": utility_context_id,
                "expression": "globalThis.__lm_handlePromise = patchedHandleBinding(document.getElementById('handle-me')); 'scheduled'"
            }
        })).await;
    let scheduled = take_response_by_id(&mut ctx, 319);
    assert_eq!(scheduled["result"]["result"]["value"], json!("scheduled"));
    let binding_called = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Runtime.bindingCalled")
                && message["params"]["name"] == json!("patchedHandleBinding")
        })
        .cloned()
        .expect("handle binding wrapper should emit Runtime.bindingCalled");
    assert_eq!(
        binding_called["params"]["executionContextId"],
        json!(utility_context_id)
    );
    let binding_payload = binding_called["params"]["payload"]
        .as_str()
        .expect("binding payload should be a json string");
    let binding_payload: serde_json::Value =
        serde_json::from_str(binding_payload).expect("binding payload should be valid json");
    assert_eq!(
        binding_payload,
        json!({
            "name": "patchedHandleBinding",
            "seq": binding_payload["seq"],
        })
    );
    let seq = binding_payload["seq"]
        .as_i64()
        .expect("binding payload seq should be an integer");
    ctx.sent.clear();

    ctx.process_async(json!({
            "id": 320,
            "method": "Runtime.evaluate",
            "sessionId": session_id,
            "params": {
                "contextId": utility_context_id,
                "expression": format!("(() => {{ const handle = globalThis.__lm_takeHandleBindingHandle({{ name: 'patchedHandleBinding', seq: {seq} }}); return JSON.stringify([handle.id, handle.tagName, typeof globalThis.__lm_takeHandleBindingHandle({{ name: 'patchedHandleBinding', seq: {seq} }})]); }})()")
            }
        })).await;
    let taken_handle = take_response_by_id(&mut ctx, 320);
    assert_eq!(
        taken_handle["result"]["result"]["value"],
        json!("[\"handle-me\",\"DIV\",\"undefined\"]")
    );

    ctx.process_async(json!({
            "id": 321,
            "method": "Runtime.evaluate",
            "sessionId": session_id,
            "params": {
                "contextId": utility_context_id,
                "expression": format!("globalThis.__lm_deliverHandleBindingResult({{ name: 'patchedHandleBinding', seq: {seq}, result: 'handle-ok' }}); 'delivered'")
            }
        })).await;
    let delivered = take_response_by_id(&mut ctx, 321);
    assert_eq!(delivered["result"]["result"]["value"], json!("delivered"));

    ctx.process_async(json!({
        "id": 322,
        "method": "Runtime.evaluate",
        "sessionId": session_id,
        "params": {
            "contextId": utility_context_id,
            "expression": "globalThis.__lm_handlePromise",
            "awaitPromise": true
        }
    }))
    .await;
    let resolved = take_response_by_id(&mut ctx, 322);
    assert_eq!(resolved["result"]["result"]["value"], json!("handle-ok"));
    assert!(
        !ctx.sent.iter().any(|message| {
            message["method"] == json!("Runtime.executionContextCreated")
                || message["method"] == json!("Runtime.executionContextsCleared")
        }),
        "handle binding round trip should stay off Runtime.enable surfaces: {:?}",
        ctx.sent
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn patchright_over_cdp_page_binding_handle_wrapper_rejects_extra_arguments_without_runtime_enable()
 {
    let mut ctx = TestContext::new();
    let session =
        create_attached_page_session_without_runtime_enable_async(&mut ctx, 3331, 3332, 3333).await;

    ctx.process_async(json!({
            "id": 3334,
            "method": "Page.navigate",
            "sessionId": session.session_id,
            "params": {
                "url": "data:text/html,<body><div id='handle-me'>node</div><span id='extra'>extra</span></body>"
            }
        })).await;
    let navigation = take_response_by_id(&mut ctx, 3334);
    assert_eq!(navigation["sessionId"], json!(session.session_id));
    ctx.take_all();

    ctx.process_async(json!({
        "id": 3335,
        "method": "Page.createIsolatedWorld",
        "sessionId": session.session_id,
        "params": {
            "frameId": session.target_id,
            "worldName": "utility"
        }
    }))
    .await;
    let utility_context_id = take_response_by_id(&mut ctx, 3335)["result"]["executionContextId"]
        .as_i64()
        .expect("utility context id");
    ctx.take_all();

    ctx.process_async(json!({
        "id": 3336,
        "method": "Runtime.addBinding",
        "sessionId": session.session_id,
        "params": {
            "name": "patchedHandleBinding",
            "executionContextId": utility_context_id
        }
    }))
    .await;
    assert_eq!(take_response_by_id(&mut ctx, 3336)["result"], json!({}));

    let wrapper_source = patchright_page_binding_wrapper_source(
        "patchedHandleBinding",
        "__lm_handle_binding_deliver",
        Some("__lm_take_handle_binding_handle"),
        true,
    );
    ctx.process_async(json!({
        "id": 3337,
        "method": "Runtime.evaluate",
        "sessionId": session.session_id,
        "params": {
            "contextId": utility_context_id,
            "expression": wrapper_source,
            "awaitPromise": true
        }
    }))
    .await;
    assert_eq!(
        take_response_by_id(&mut ctx, 3337)["result"]["result"]["value"],
        json!("function")
    );

    ctx.process_async(json!({
            "id": 3338,
            "method": "Runtime.evaluate",
            "sessionId": session.session_id,
            "params": {
                "contextId": utility_context_id,
                "expression": "patchedHandleBinding(document.getElementById('handle-me'), document.getElementById('extra'))"
            }
        })).await;
    let invalid_call = take_response_by_id(&mut ctx, 3338);
    let exception_text = [
        invalid_call["result"]["exceptionDetails"]["text"]
            .as_str()
            .unwrap_or_default(),
        invalid_call["result"]["exceptionDetails"]["exception"]["description"]
            .as_str()
            .unwrap_or_default(),
    ]
    .join("\n");
    assert!(
        exception_text.contains("exposeBindingHandle supports a single argument, 2 received"),
        "unexpected exception text: {exception_text}"
    );
    assert!(
        !ctx.sent
            .iter()
            .any(|message| message["method"] == json!("Runtime.bindingCalled")),
        "invalid needsHandle call should not emit Runtime.bindingCalled: {:?}",
        ctx.sent
    );
    ctx.sent.clear();

    ctx.process_async(json!({
            "id": 3339,
            "method": "Runtime.evaluate",
            "sessionId": session.session_id,
            "params": {
                "contextId": utility_context_id,
                "expression": "globalThis.__lm_handlePromise = patchedHandleBinding(document.getElementById('handle-me')); 'scheduled'",
                "awaitPromise": true
            }
        })).await;
    let scheduled = take_response_by_id(&mut ctx, 3339);
    assert_eq!(scheduled["result"]["result"]["value"], json!("scheduled"));

    let binding_called = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Runtime.bindingCalled")
                && message["params"]["name"] == json!("patchedHandleBinding")
        })
        .cloned()
        .expect("valid needsHandle call should emit Runtime.bindingCalled");
    assert_eq!(
        binding_called["params"]["executionContextId"],
        json!(utility_context_id)
    );
    let binding_payload = binding_called["params"]["payload"]
        .as_str()
        .expect("binding payload should be a json string");
    let binding_payload: serde_json::Value =
        serde_json::from_str(binding_payload).expect("binding payload should be valid json");
    assert_eq!(binding_payload["name"], json!("patchedHandleBinding"));
    assert_eq!(binding_payload["seq"], json!(1));
    ctx.sent.clear();

    ctx.process_async(json!({
            "id": 3340,
            "method": "Runtime.evaluate",
            "sessionId": session.session_id,
            "params": {
                "contextId": utility_context_id,
                "expression": "(() => { const handle = globalThis.__lm_take_handle_binding_handle({ name: 'patchedHandleBinding', seq: 1 }); return JSON.stringify([handle.id, handle.tagName, typeof globalThis.__lm_take_handle_binding_handle({ name: 'patchedHandleBinding', seq: 1 })]); })()"
            }
        })).await;
    let taken_handle = take_response_by_id(&mut ctx, 3340);
    assert_eq!(
        taken_handle["result"]["result"]["value"],
        json!("[\"handle-me\",\"DIV\",\"undefined\"]")
    );

    ctx.process_async(json!({
            "id": 3341,
            "method": "Runtime.evaluate",
            "sessionId": session.session_id,
            "params": {
                "contextId": utility_context_id,
                "expression": "globalThis.__lm_handle_binding_deliver({ name: 'patchedHandleBinding', seq: 1, result: 'handle-ok' }); 'delivered'",
                "awaitPromise": true
            }
        })).await;
    let delivered = take_response_by_id(&mut ctx, 3341);
    assert_eq!(delivered["result"]["result"]["value"], json!("delivered"));

    ctx.process_async(json!({
        "id": 3342,
        "method": "Runtime.evaluate",
        "sessionId": session.session_id,
        "params": {
            "contextId": utility_context_id,
            "expression": "globalThis.__lm_handlePromise",
            "awaitPromise": true
        }
    }))
    .await;
    let resolved = take_response_by_id(&mut ctx, 3342);
    assert_eq!(resolved["result"]["result"]["value"], json!("handle-ok"));
}

#[tokio::test(flavor = "multi_thread")]
async fn patchright_over_cdp_page_binding_handle_wrapper_allows_trailing_undefined_argument_without_runtime_enable()
 {
    let mut ctx = TestContext::new();
    let session =
        create_attached_page_session_without_runtime_enable_async(&mut ctx, 3343, 3344, 3345).await;

    ctx.process_async(json!({
        "id": 3346,
        "method": "Page.navigate",
        "sessionId": session.session_id,
        "params": {
            "url": "data:text/html,<body><div id='handle-me'>node</div></body>"
        }
    }))
    .await;
    let navigation = take_response_by_id(&mut ctx, 3346);
    assert_eq!(navigation["sessionId"], json!(session.session_id));
    ctx.take_all();

    ctx.process_async(json!({
        "id": 3347,
        "method": "Page.createIsolatedWorld",
        "sessionId": session.session_id,
        "params": {
            "frameId": session.target_id,
            "worldName": "utility"
        }
    }))
    .await;
    let utility_context_id = take_response_by_id(&mut ctx, 3347)["result"]["executionContextId"]
        .as_i64()
        .expect("utility context id");
    ctx.take_all();

    ctx.process_async(json!({
        "id": 3348,
        "method": "Runtime.addBinding",
        "sessionId": session.session_id,
        "params": {
            "name": "patchedHandleBinding",
            "executionContextId": utility_context_id
        }
    }))
    .await;
    assert_eq!(take_response_by_id(&mut ctx, 3348)["result"], json!({}));

    let wrapper_source = patchright_page_binding_wrapper_source(
        "patchedHandleBinding",
        "__lm_handle_binding_deliver",
        Some("__lm_take_handle_binding_handle"),
        true,
    );
    ctx.process_async(json!({
        "id": 3349,
        "method": "Runtime.evaluate",
        "sessionId": session.session_id,
        "params": {
            "contextId": utility_context_id,
            "expression": wrapper_source,
            "awaitPromise": true
        }
    }))
    .await;
    assert_eq!(
        take_response_by_id(&mut ctx, 3349)["result"]["result"]["value"],
        json!("function")
    );

    ctx.process_async(json!({
            "id": 3350,
            "method": "Runtime.evaluate",
            "sessionId": session.session_id,
            "params": {
                "contextId": utility_context_id,
                "expression": "globalThis.__lm_handlePromise = patchedHandleBinding(document.getElementById('handle-me'), undefined); 'scheduled'",
                "awaitPromise": true
            }
        })).await;
    let scheduled = take_response_by_id(&mut ctx, 3350);
    assert_eq!(scheduled["result"]["result"]["value"], json!("scheduled"));

    let binding_called = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Runtime.bindingCalled")
                && message["params"]["name"] == json!("patchedHandleBinding")
        })
        .cloned()
        .expect("needsHandle wrapper should allow trailing undefined argument");
    assert_eq!(
        binding_called["params"]["executionContextId"],
        json!(utility_context_id)
    );
    let binding_payload = binding_called["params"]["payload"]
        .as_str()
        .expect("binding payload should be a json string");
    let binding_payload: serde_json::Value =
        serde_json::from_str(binding_payload).expect("binding payload should be valid json");
    assert_eq!(
        binding_payload,
        json!({
            "name": "patchedHandleBinding",
            "seq": 1,
        })
    );
    ctx.sent.clear();

    ctx.process_async(json!({
            "id": 3351,
            "method": "Runtime.evaluate",
            "sessionId": session.session_id,
            "params": {
                "contextId": utility_context_id,
                "expression": "(() => { const handle = globalThis.__lm_take_handle_binding_handle({ name: 'patchedHandleBinding', seq: 1 }); return JSON.stringify([handle.id, handle.tagName, typeof globalThis.__lm_take_handle_binding_handle({ name: 'patchedHandleBinding', seq: 1 })]); })()"
            }
        })).await;
    let taken_handle = take_response_by_id(&mut ctx, 3351);
    assert_eq!(
        taken_handle["result"]["result"]["value"],
        json!("[\"handle-me\",\"DIV\",\"undefined\"]")
    );

    ctx.process_async(json!({
            "id": 3352,
            "method": "Runtime.evaluate",
            "sessionId": session.session_id,
            "params": {
                "contextId": utility_context_id,
                "expression": "globalThis.__lm_handle_binding_deliver({ name: 'patchedHandleBinding', seq: 1, result: 'handle-ok' }); 'delivered'",
                "awaitPromise": true
            }
        })).await;
    let delivered = take_response_by_id(&mut ctx, 3352);
    assert_eq!(delivered["result"]["result"]["value"], json!("delivered"));

    ctx.process_async(json!({
        "id": 3353,
        "method": "Runtime.evaluate",
        "sessionId": session.session_id,
        "params": {
            "contextId": utility_context_id,
            "expression": "globalThis.__lm_handlePromise",
            "awaitPromise": true
        }
    }))
    .await;
    let resolved = take_response_by_id(&mut ctx, 3353);
    assert_eq!(resolved["result"]["result"]["value"], json!("handle-ok"));
}

#[tokio::test(flavor = "multi_thread")]
async fn patchright_over_cdp_dual_world_handle_binding_argument_error_stays_execution_context_local_without_runtime_enable()
 {
    let mut ctx = TestContext::new();
    let session =
        create_attached_page_session_without_runtime_enable_async(&mut ctx, 3354, 3355, 3356).await;

    ctx.process_async(json!({
            "id": 3357,
            "method": "Page.navigate",
            "sessionId": session.session_id,
            "params": {
                "url": "data:text/html,<body><div id='main-handle'>main</div><div id='utility-handle'>utility</div><span id='extra'>extra</span></body>"
            }
        })).await;
    let navigation = take_response_by_id(&mut ctx, 3357);
    assert_eq!(navigation["sessionId"], json!(session.session_id));
    ctx.take_all();

    ctx.process_async(json!({
        "id": 3358,
        "method": "Page.createIsolatedWorld",
        "sessionId": session.session_id,
        "params": {
            "frameId": session.target_id,
            "worldName": "utility"
        }
    }))
    .await;
    let utility_context_id = take_response_by_id(&mut ctx, 3358)["result"]["executionContextId"]
        .as_i64()
        .expect("utility context id");
    ctx.take_all();

    ctx.process_async(json!({
        "id": 3359,
        "method": "Runtime.addBinding",
        "sessionId": session.session_id,
        "params": {
            "name": "patchedHandleBinding"
        }
    }))
    .await;
    assert_eq!(take_response_by_id(&mut ctx, 3359)["result"], json!({}));

    ctx.process_async(json!({
        "id": 3360,
        "method": "Runtime.addBinding",
        "sessionId": session.session_id,
        "params": {
            "name": "patchedHandleBinding",
            "executionContextId": utility_context_id
        }
    }))
    .await;
    assert_eq!(take_response_by_id(&mut ctx, 3360)["result"], json!({}));

    let wrapper_source = patchright_page_binding_wrapper_source(
        "patchedHandleBinding",
        "__lm_handle_binding_deliver",
        Some("__lm_take_handle_binding_handle"),
        true,
    );
    ctx.process_async(json!({
        "id": 3361,
        "method": "Runtime.evaluate",
        "sessionId": session.session_id,
        "params": {
            "expression": &wrapper_source,
            "awaitPromise": true
        }
    }))
    .await;
    assert_eq!(
        take_response_by_id(&mut ctx, 3361)["result"]["result"]["value"],
        json!("function")
    );

    ctx.process_async(json!({
        "id": 3362,
        "method": "Runtime.evaluate",
        "sessionId": session.session_id,
        "params": {
            "contextId": utility_context_id,
            "expression": &wrapper_source,
            "awaitPromise": true
        }
    }))
    .await;
    assert_eq!(
        take_response_by_id(&mut ctx, 3362)["result"]["result"]["value"],
        json!("function")
    );

    ctx.process_async(json!({
            "id": 3363,
            "method": "Runtime.evaluate",
            "sessionId": session.session_id,
            "params": {
                "contextId": utility_context_id,
                "expression": "patchedHandleBinding(document.getElementById('utility-handle'), document.getElementById('extra'))"
            }
        })).await;
    let invalid_call = take_response_by_id(&mut ctx, 3363);
    let exception_text = [
        invalid_call["result"]["exceptionDetails"]["text"]
            .as_str()
            .unwrap_or_default(),
        invalid_call["result"]["exceptionDetails"]["exception"]["description"]
            .as_str()
            .unwrap_or_default(),
    ]
    .join("\n");
    assert!(
        exception_text.contains("exposeBindingHandle supports a single argument, 2 received"),
        "unexpected exception text: {exception_text}"
    );
    assert!(
        !ctx.sent
            .iter()
            .any(|message| message["method"] == json!("Runtime.bindingCalled")),
        "invalid utility-world needsHandle call should not emit Runtime.bindingCalled: {:?}",
        ctx.sent
    );
    ctx.sent.clear();

    ctx.process_async(json!({
            "id": 3364,
            "method": "Runtime.evaluate",
            "sessionId": session.session_id,
            "params": {
                "expression": "globalThis.__lm_main_handle_promise = patchedHandleBinding(document.getElementById('main-handle')); 'scheduled-main'",
                "awaitPromise": true
            }
        })).await;
    let main_scheduled = take_response_by_id(&mut ctx, 3364);
    assert_eq!(
        main_scheduled["result"]["result"]["value"],
        json!("scheduled-main")
    );
    let main_binding_called = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Runtime.bindingCalled")
                && message["params"]["name"] == json!("patchedHandleBinding")
        })
        .cloned()
        .expect("main-world call should emit Runtime.bindingCalled");
    assert_ne!(
        main_binding_called["params"]["executionContextId"],
        json!(utility_context_id)
    );
    let main_payload = main_binding_called["params"]["payload"]
        .as_str()
        .expect("main binding payload should be a json string");
    let main_payload: serde_json::Value =
        serde_json::from_str(main_payload).expect("main binding payload should be valid json");
    assert_eq!(
        main_payload,
        json!({
            "name": "patchedHandleBinding",
            "seq": 1,
        })
    );
    ctx.sent.clear();

    ctx.process_async(json!({
            "id": 3365,
            "method": "Runtime.evaluate",
            "sessionId": session.session_id,
            "params": {
                "contextId": utility_context_id,
                "expression": "globalThis.__lm_utility_handle_promise = patchedHandleBinding(document.getElementById('utility-handle'), undefined); 'scheduled-utility'",
                "awaitPromise": true
            }
        })).await;
    let utility_scheduled = take_response_by_id(&mut ctx, 3365);
    assert_eq!(
        utility_scheduled["result"]["result"]["value"],
        json!("scheduled-utility")
    );
    let utility_binding_called = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Runtime.bindingCalled")
                && message["params"]["name"] == json!("patchedHandleBinding")
                && message["params"]["executionContextId"] == json!(utility_context_id)
        })
        .cloned()
        .expect("utility-world valid call should emit Runtime.bindingCalled");
    let utility_payload = utility_binding_called["params"]["payload"]
        .as_str()
        .expect("utility binding payload should be a json string");
    let utility_payload: serde_json::Value = serde_json::from_str(utility_payload)
        .expect("utility binding payload should be valid json");
    assert_eq!(
        utility_payload,
        json!({
            "name": "patchedHandleBinding",
            "seq": 1,
        })
    );
    ctx.sent.clear();

    ctx.process_async(json!({
            "id": 3366,
            "method": "Runtime.evaluate",
            "sessionId": session.session_id,
            "params": {
                "expression": "(() => { const handle = globalThis.__lm_take_handle_binding_handle({ name: 'patchedHandleBinding', seq: 1 }); return JSON.stringify([handle.id, handle.textContent, typeof globalThis.__lm_take_handle_binding_handle({ name: 'patchedHandleBinding', seq: 1 })]); })()"
            }
        })).await;
    let main_taken_handle = take_response_by_id(&mut ctx, 3366);
    assert_eq!(
        main_taken_handle["result"]["result"]["value"],
        json!("[\"main-handle\",\"main\",\"undefined\"]")
    );

    ctx.process_async(json!({
            "id": 3367,
            "method": "Runtime.evaluate",
            "sessionId": session.session_id,
            "params": {
                "contextId": utility_context_id,
                "expression": "(() => { const handle = globalThis.__lm_take_handle_binding_handle({ name: 'patchedHandleBinding', seq: 1 }); return JSON.stringify([handle.id, handle.textContent, typeof globalThis.__lm_take_handle_binding_handle({ name: 'patchedHandleBinding', seq: 1 })]); })()"
            }
        })).await;
    let utility_taken_handle = take_response_by_id(&mut ctx, 3367);
    assert_eq!(
        utility_taken_handle["result"]["result"]["value"],
        json!("[\"utility-handle\",\"utility\",\"undefined\"]")
    );

    for (id, context_id, result, promise_name) in [
        (3368_u64, None, "main-handle-ok", "__lm_main_handle_promise"),
        (
            3369_u64,
            Some(utility_context_id),
            "utility-handle-ok",
            "__lm_utility_handle_promise",
        ),
    ] {
        let mut deliver_params = json!({
            "expression": format!(
                "globalThis.__lm_handle_binding_deliver({{ name: 'patchedHandleBinding', seq: 1, result: '{result}' }}); 'delivered'"
            ),
            "awaitPromise": true
        });
        if let Some(context_id) = context_id {
            deliver_params["contextId"] = json!(context_id);
        }
        ctx.process_async(json!({
            "id": id,
            "method": "Runtime.evaluate",
            "sessionId": session.session_id,
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
            "sessionId": session.session_id,
            "params": promise_params
        }))
        .await;
        let resolved = take_response_by_id(&mut ctx, id + 10);
        assert_eq!(resolved["result"]["result"]["value"], json!(result));
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn patchright_over_cdp_remove_binding_only_clears_matching_name_in_current_main_and_utility_worlds_without_runtime_enable()
 {
    let mut ctx = TestContext::new();
    let session =
        create_attached_page_session_without_runtime_enable_async(&mut ctx, 3370, 3371, 3372).await;

    ctx.process_async(json!({
            "id": 3373,
            "method": "Page.navigate",
            "sessionId": session.session_id,
            "params": {
                "url": "data:text/html,<body><div id='main-node'>main</div><div id='utility-node'>utility</div></body>"
            }
        })).await;
    let navigation = take_response_by_id(&mut ctx, 3373);
    assert_eq!(navigation["sessionId"], json!(session.session_id));
    ctx.take_all();

    ctx.process_async(json!({
        "id": 3374,
        "method": "Page.createIsolatedWorld",
        "sessionId": session.session_id,
        "params": {
            "frameId": session.target_id,
            "worldName": "utility"
        }
    }))
    .await;
    let utility_context_id = take_response_by_id(&mut ctx, 3374)["result"]["executionContextId"]
        .as_i64()
        .expect("utility context id");
    ctx.take_all();

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
    let retained_wrapper_source = patchright_page_binding_wrapper_source(
        "__pw_keptBinding",
        "__lm_pw_kept_binding_deliver",
        None,
        false,
    );

    for (id, binding_name) in [
        (3375_u64, "customBindingA"),
        (3377_u64, "customBindingB"),
        (3379_u64, "__pw_keptBinding"),
    ] {
        ctx.process_async(json!({
            "id": id,
            "method": "Runtime.addBinding",
            "sessionId": session.session_id,
            "params": {
                "name": binding_name
            }
        }))
        .await;
        assert_eq!(take_response_by_id(&mut ctx, id)["result"], json!({}));

        ctx.process_async(json!({
            "id": id + 1,
            "method": "Runtime.addBinding",
            "sessionId": session.session_id,
            "params": {
                "name": binding_name,
                "executionContextId": utility_context_id
            }
        }))
        .await;
        assert_eq!(take_response_by_id(&mut ctx, id + 1)["result"], json!({}));
    }

    for (id, source, context_id) in [
        (3381_u64, custom_a_wrapper_source.as_str(), None),
        (3382_u64, custom_b_wrapper_source.as_str(), None),
        (3383_u64, retained_wrapper_source.as_str(), None),
        (
            3384_u64,
            custom_a_wrapper_source.as_str(),
            Some(utility_context_id),
        ),
        (
            3385_u64,
            custom_b_wrapper_source.as_str(),
            Some(utility_context_id),
        ),
        (
            3386_u64,
            retained_wrapper_source.as_str(),
            Some(utility_context_id),
        ),
    ] {
        let mut params = json!({
            "expression": source,
            "awaitPromise": true
        });
        if let Some(context_id) = context_id {
            params["contextId"] = json!(context_id);
        }
        ctx.process_async(json!({
            "id": id,
            "method": "Runtime.evaluate",
            "sessionId": session.session_id,
            "params": params
        }))
        .await;
        let installed = take_response_by_id(&mut ctx, id);
        assert_eq!(installed["result"]["result"]["value"], json!("function"));
    }

    ctx.process_async(json!({
        "id": 3387,
        "method": "Runtime.removeBinding",
        "sessionId": session.session_id,
        "params": {
            "name": "customBindingA"
        }
    }))
    .await;
    assert_eq!(take_response_by_id(&mut ctx, 3387)["result"], json!({}));

    for (id, context_id) in [(3388_u64, None), (3389_u64, Some(utility_context_id))] {
        let mut params = json!({
            "expression": "JSON.stringify([typeof globalThis.customBindingA, typeof globalThis.customBindingB, typeof globalThis.__pw_keptBinding])"
        });
        if let Some(context_id) = context_id {
            params["contextId"] = json!(context_id);
        }
        ctx.process_async(json!({
            "id": id,
            "method": "Runtime.evaluate",
            "sessionId": session.session_id,
            "params": params
        }))
        .await;
        let state = take_response_by_id(&mut ctx, id);
        assert_eq!(
            state["result"]["result"]["value"],
            json!("[\"undefined\",\"function\",\"function\"]")
        );
    }

    for (
        id,
        context_id,
        expression,
        binding_name,
        expected_arg,
        deliver_expression,
        promise_name,
        expected_result,
    ) in [
        (
            3390_u64,
            None,
            "globalThis.__lm_custom_b_main = customBindingB('main-b'); 'scheduled-main-b'",
            "customBindingB",
            "main-b",
            "globalThis.__lm_custom_binding_b_deliver({ name: 'customBindingB', seq: 1, result: 'main-b-ok' }); 'delivered'",
            "__lm_custom_b_main",
            "main-b-ok",
        ),
        (
            3391_u64,
            Some(utility_context_id),
            "globalThis.__lm_pw_kept_utility = __pw_keptBinding('utility-kept'); 'scheduled-utility-kept'",
            "__pw_keptBinding",
            "utility-kept",
            "globalThis.__lm_pw_kept_binding_deliver({ name: '__pw_keptBinding', seq: 1, result: 'utility-kept-ok' }); 'delivered'",
            "__lm_pw_kept_utility",
            "utility-kept-ok",
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
            "sessionId": session.session_id,
            "params": params
        }))
        .await;
        let scheduled = take_response_by_id(&mut ctx, id);
        assert!(
            scheduled["result"]["result"]["value"]
                .as_str()
                .expect("scheduled value")
                .starts_with("scheduled-")
        );

        let binding_called = ctx
            .sent
            .iter()
            .find(|message| {
                message["method"] == json!("Runtime.bindingCalled")
                    && message["params"]["name"] == json!(binding_name)
                    && match context_id {
                        Some(context_id) => {
                            message["params"]["executionContextId"] == json!(context_id)
                        }
                        None => true,
                    }
            })
            .cloned()
            .expect("remaining binding should still emit Runtime.bindingCalled");
        let payload = binding_called["params"]["payload"]
            .as_str()
            .expect("binding payload should be a json string");
        let payload: serde_json::Value =
            serde_json::from_str(payload).expect("binding payload should be valid json");
        assert_eq!(payload["name"], json!(binding_name));
        assert_eq!(payload["serializedArgs"], json!([expected_arg]));
        assert_eq!(payload["seq"], json!(1));
        ctx.sent.clear();

        let mut deliver_params = json!({
            "expression": deliver_expression,
            "awaitPromise": true
        });
        if let Some(context_id) = context_id {
            deliver_params["contextId"] = json!(context_id);
        }
        ctx.process_async(json!({
            "id": id + 10,
            "method": "Runtime.evaluate",
            "sessionId": session.session_id,
            "params": deliver_params
        }))
        .await;
        let delivered = take_response_by_id(&mut ctx, id + 10);
        assert_eq!(delivered["result"]["result"]["value"], json!("delivered"));

        let mut promise_params = json!({
            "expression": format!("globalThis.{promise_name}"),
            "awaitPromise": true
        });
        if let Some(context_id) = context_id {
            promise_params["contextId"] = json!(context_id);
        }
        ctx.process_async(json!({
            "id": id + 20,
            "method": "Runtime.evaluate",
            "sessionId": session.session_id,
            "params": promise_params
        }))
        .await;
        let resolved = take_response_by_id(&mut ctx, id + 20);
        assert_eq!(
            resolved["result"]["result"]["value"],
            json!(expected_result)
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn patchright_over_cdp_crpage_cleanup_replay_only_clears_matching_name_after_navigation_without_runtime_enable()
 {
    super::patchright_8mb_stack(
        "patchright-handle-cleanup-clear-matching-after-navigation",
        || async {
            run_patchright_over_cdp_crpage_cleanup_replay_only_clears_matching_name_after_navigation_without_runtime_enable()
                .await;
        },
    )
    .await;
}

async fn run_patchright_over_cdp_crpage_cleanup_replay_only_clears_matching_name_after_navigation_without_runtime_enable()
 {
    let mut ctx = TestContext::new();
    let session =
        create_attached_page_session_without_runtime_enable_async(&mut ctx, 34100, 34101, 34102)
            .await;

    ctx.process_async(json!({
            "id": 34103,
            "method": "Page.navigate",
            "sessionId": session.session_id,
            "params": {
                "url": "data:text/html,<body><div id='main-node'>main</div><div id='utility-node'>utility</div></body>"
            }
        })).await;
    let navigation = take_response_by_id(&mut ctx, 34103);
    assert_eq!(navigation["sessionId"], json!(session.session_id));
    ctx.take_all();

    ctx.process_async(json!({
        "id": 34104,
        "method": "Page.createIsolatedWorld",
        "sessionId": session.session_id,
        "params": {
            "frameId": session.target_id,
            "worldName": "utility"
        }
    }))
    .await;
    let initial_utility_context_id =
        take_response_by_id(&mut ctx, 34104)["result"]["executionContextId"]
            .as_i64()
            .expect("initial utility context id");
    ctx.take_all();

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
    let retained_wrapper_source = patchright_page_binding_wrapper_source(
        "__pw_keptBinding",
        "__lm_pw_kept_binding_deliver",
        None,
        false,
    );

    install_patchright_crpage_binding_in_existing_worlds_async(
        &mut ctx,
        &session.session_id,
        initial_utility_context_id,
        34105,
        34106,
        34107,
        34108,
        "customBindingA",
        &custom_a_wrapper_source,
    )
    .await;
    install_patchright_crpage_binding_in_existing_worlds_async(
        &mut ctx,
        &session.session_id,
        initial_utility_context_id,
        34109,
        34110,
        34111,
        34112,
        "customBindingB",
        &custom_b_wrapper_source,
    )
    .await;
    install_patchright_crpage_binding_in_existing_worlds_async(
        &mut ctx,
        &session.session_id,
        initial_utility_context_id,
        34113,
        34114,
        34115,
        34116,
        "__pw_keptBinding",
        &retained_wrapper_source,
    )
    .await;

    for (id, source, world_name) in [
        (34117_u64, custom_a_wrapper_source.as_str(), None),
        (34118_u64, custom_b_wrapper_source.as_str(), None),
        (34119_u64, retained_wrapper_source.as_str(), None),
        (34120_u64, custom_a_wrapper_source.as_str(), Some("utility")),
        (34121_u64, custom_b_wrapper_source.as_str(), Some("utility")),
        (34122_u64, retained_wrapper_source.as_str(), Some("utility")),
    ] {
        let mut params = json!({
            "source": source,
            "runImmediately": true
        });
        if let Some(world_name) = world_name {
            params["worldName"] = json!(world_name);
        }
        ctx.process_async(json!({
            "id": id,
            "method": "Page.addScriptToEvaluateOnNewDocument",
            "sessionId": session.session_id,
            "params": params
        }))
        .await;
        assert!(
            take_response_by_id(&mut ctx, id)["result"]["identifier"]
                .as_str()
                .is_some()
        );
    }

    ctx.process_async(json!({
        "id": 34123,
        "method": "Runtime.removeBinding",
        "sessionId": session.session_id,
        "params": {
            "name": "customBindingA"
        }
    }))
    .await;
    assert_eq!(take_response_by_id(&mut ctx, 34123)["result"], json!({}));

    ctx.process_async(json!({
        "id": 34124,
        "method": "Page.navigate",
        "sessionId": session.session_id,
        "params": {
            "url": "data:text/html,<body><div id='page'>replay</div></body>"
        }
    }))
    .await;
    let replay_navigation = take_response_by_id(&mut ctx, 34124);
    assert_eq!(replay_navigation["sessionId"], json!(session.session_id));
    ctx.take_all();

    for (id, source, expected_type) in [
        (34125_u64, custom_a_wrapper_source.as_str(), "undefined"),
        (34126_u64, custom_b_wrapper_source.as_str(), "function"),
        (34127_u64, retained_wrapper_source.as_str(), "function"),
    ] {
        ctx.process_async(json!({
            "id": id,
            "method": "Runtime.evaluate",
            "sessionId": session.session_id,
            "params": {
                "expression": source,
                "awaitPromise": true
            }
        }))
        .await;
        let replayed = take_response_by_id(&mut ctx, id);
        assert_eq!(replayed["result"]["result"]["value"], json!(expected_type));
    }

    ctx.process_async(json!({
            "id": 34128,
            "method": "Runtime.evaluate",
            "sessionId": session.session_id,
            "params": {
                "expression": "JSON.stringify([typeof globalThis.customBindingA, typeof globalThis.customBindingB, typeof globalThis.__pw_keptBinding])"
            }
        })).await;
    let main_state = take_response_by_id(&mut ctx, 34128);
    assert_eq!(
        main_state["result"]["result"]["value"],
        json!("[\"undefined\",\"function\",\"function\"]")
    );

    ctx.process_async(json!({
        "id": 34129,
        "method": "Page.createIsolatedWorld",
        "sessionId": session.session_id,
        "params": {
            "frameId": session.target_id,
            "worldName": "utility"
        }
    }))
    .await;
    let replay_utility_context_id =
        take_response_by_id(&mut ctx, 34129)["result"]["executionContextId"]
            .as_i64()
            .expect("replay utility context id");
    ctx.take_all();

    for (id, binding_name) in [
        (34130_u64, "customBindingB"),
        (34131_u64, "__pw_keptBinding"),
    ] {
        ctx.process_async(json!({
            "id": id,
            "method": "Runtime.addBinding",
            "sessionId": session.session_id,
            "params": {
                "name": binding_name,
                "executionContextId": replay_utility_context_id
            }
        }))
        .await;
        assert_eq!(take_response_by_id(&mut ctx, id)["result"], json!({}));
    }

    for (id, source, expected_type) in [
        (34132_u64, custom_a_wrapper_source.as_str(), "undefined"),
        (34133_u64, custom_b_wrapper_source.as_str(), "function"),
        (34134_u64, retained_wrapper_source.as_str(), "function"),
    ] {
        ctx.process_async(json!({
            "id": id,
            "method": "Runtime.evaluate",
            "sessionId": session.session_id,
            "params": {
                "contextId": replay_utility_context_id,
                "expression": source,
                "awaitPromise": true
            }
        }))
        .await;
        let replayed = take_response_by_id(&mut ctx, id);
        assert_eq!(replayed["result"]["result"]["value"], json!(expected_type));
    }

    ctx.process_async(json!({
            "id": 34135,
            "method": "Runtime.evaluate",
            "sessionId": session.session_id,
            "params": {
                "contextId": replay_utility_context_id,
                "expression": "JSON.stringify([typeof globalThis.customBindingA, typeof globalThis.customBindingB, typeof globalThis.__pw_keptBinding])"
            }
        })).await;
    let utility_state = take_response_by_id(&mut ctx, 34135);
    assert_eq!(
        utility_state["result"]["result"]["value"],
        json!("[\"undefined\",\"function\",\"function\"]")
    );

    for (
        id,
        context_id,
        expression,
        binding_name,
        expected_arg,
        deliver_expression,
        promise_name,
        expected_result,
    ) in [
        (
            34136_u64,
            None,
            "globalThis.__lm_replay_main_custom_b = customBindingB({ source: 'replay-main-b', nested: { count: 1, values: ['a', 2, true] } }); 'scheduled-main-b'",
            "customBindingB",
            json!([{
                "source": "replay-main-b",
                "nested": { "count": 1, "values": ["a", 2, true] }
            }]),
            "globalThis.__lm_custom_binding_b_deliver({ name: 'customBindingB', seq: 1, result: 'replay-main-b-ok' }); 'delivered'",
            "__lm_replay_main_custom_b",
            "replay-main-b-ok",
        ),
        (
            34137_u64,
            Some(replay_utility_context_id),
            "globalThis.__lm_replay_utility_pw_kept = __pw_keptBinding({ source: 'replay-utility-kept', nested: { count: 2, values: ['b', 3, false] } }); 'scheduled-utility-kept'",
            "__pw_keptBinding",
            json!([{
                "source": "replay-utility-kept",
                "nested": { "count": 2, "values": ["b", 3, false] }
            }]),
            "globalThis.__lm_pw_kept_binding_deliver({ name: '__pw_keptBinding', seq: 1, result: 'replay-utility-kept-ok' }); 'delivered'",
            "__lm_replay_utility_pw_kept",
            "replay-utility-kept-ok",
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
            "sessionId": session.session_id,
            "params": params
        }))
        .await;
        let scheduled = take_response_by_id(&mut ctx, id);
        assert!(
            scheduled["result"]["result"]["value"]
                .as_str()
                .expect("scheduled value")
                .starts_with("scheduled-")
        );

        let binding_called = ctx
            .sent
            .iter()
            .find(|message| {
                message["method"] == json!("Runtime.bindingCalled")
                    && message["params"]["name"] == json!(binding_name)
                    && match context_id {
                        Some(context_id) => {
                            message["params"]["executionContextId"] == json!(context_id)
                        }
                        None => true,
                    }
            })
            .cloned()
            .expect("remaining replay binding should still emit Runtime.bindingCalled");
        let payload = binding_called["params"]["payload"]
            .as_str()
            .expect("binding payload should be a json string");
        let payload: serde_json::Value =
            serde_json::from_str(payload).expect("binding payload should be valid json");
        assert_eq!(payload["name"], json!(binding_name));
        assert_eq!(payload["serializedArgs"], expected_arg);
        assert_eq!(payload["seq"], json!(1));
        ctx.sent.clear();

        let mut deliver_params = json!({
            "expression": deliver_expression,
            "awaitPromise": true
        });
        if let Some(context_id) = context_id {
            deliver_params["contextId"] = json!(context_id);
        }
        ctx.process_async(json!({
            "id": id + 10,
            "method": "Runtime.evaluate",
            "sessionId": session.session_id,
            "params": deliver_params
        }))
        .await;
        let delivered = take_response_by_id(&mut ctx, id + 10);
        assert_eq!(delivered["result"]["result"]["value"], json!("delivered"));

        let mut promise_params = json!({
            "expression": format!("globalThis.{promise_name}"),
            "awaitPromise": true
        });
        if let Some(context_id) = context_id {
            promise_params["contextId"] = json!(context_id);
        }
        ctx.process_async(json!({
            "id": id + 20,
            "method": "Runtime.evaluate",
            "sessionId": session.session_id,
            "params": promise_params
        }))
        .await;
        let resolved = take_response_by_id(&mut ctx, id + 20);
        assert_eq!(
            resolved["result"]["result"]["value"],
            json!(expected_result)
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn patchright_over_cdp_auto_attach_sweep_crpage_cleanup_replay_only_clears_matching_name_in_cleaned_context()
 {
    super::patchright_8mb_stack(
        "patchright-handle-cleanup-clear-matching",
        || async {
            run_patchright_over_cdp_auto_attach_sweep_crpage_cleanup_replay_only_clears_matching_name_in_cleaned_context()
                .await;
        },
    )
    .await;
}

async fn run_patchright_over_cdp_auto_attach_sweep_crpage_cleanup_replay_only_clears_matching_name_in_cleaned_context()
 {
    let mut ctx = TestContext::new();
    let first =
        create_attached_page_session_without_runtime_enable_async(&mut ctx, 34150, 34151, 34152)
            .await;
    let second =
        create_attached_page_session_without_runtime_enable_async(&mut ctx, 34153, 34154, 34155)
            .await;

    for (id, session_id, html) in [
        (
            34156_u64,
            first.session_id.as_str(),
            "<body><div id='first-main'>first-main</div><div id='first-utility'>first-utility</div></body>",
        ),
        (
            34157_u64,
            second.session_id.as_str(),
            "<body><div id='second-main'>second-main</div><div id='second-utility'>second-utility</div></body>",
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
            34158_u64,
            first.target_id.as_str(),
            first.session_id.as_str(),
        ),
        (
            34159_u64,
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
        "id": 34160,
        "method": "Target.setAutoAttach",
        "params": {
            "autoAttach": true,
            "waitForDebuggerOnStart": false
        }
    }))
    .await;
    ctx.expect_result(34160, json!({}), None);
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
            34161_u64,
            first_auto_session.as_str(),
            first.target_id.as_str(),
            &mut first_utility_context,
        ),
        (
            34162_u64,
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
    let retained_wrapper_source = patchright_page_binding_wrapper_source(
        "__pw_keptBinding",
        "__lm_pw_kept_binding_deliver",
        None,
        false,
    );

    for (id, session_id, utility_context) in [
        (
            34163_u64,
            first_auto_session.as_str(),
            first_utility_context,
        ),
        (
            34175_u64,
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
            "customBindingA",
            &custom_a_wrapper_source,
        )
        .await;
        install_patchright_crpage_binding_in_existing_worlds_async(
            &mut ctx,
            session_id,
            utility_context,
            id + 4,
            id + 5,
            id + 6,
            id + 7,
            "customBindingB",
            &custom_b_wrapper_source,
        )
        .await;
        install_patchright_crpage_binding_in_existing_worlds_async(
            &mut ctx,
            session_id,
            utility_context,
            id + 8,
            id + 9,
            id + 10,
            id + 11,
            "__pw_keptBinding",
            &retained_wrapper_source,
        )
        .await;
    }

    for (id, source, world_name, session_id) in [
        (
            34187_u64,
            custom_a_wrapper_source.as_str(),
            None,
            first_auto_session.as_str(),
        ),
        (
            34188_u64,
            custom_b_wrapper_source.as_str(),
            None,
            first_auto_session.as_str(),
        ),
        (
            34189_u64,
            retained_wrapper_source.as_str(),
            None,
            first_auto_session.as_str(),
        ),
        (
            34190_u64,
            custom_a_wrapper_source.as_str(),
            Some("utility"),
            first_auto_session.as_str(),
        ),
        (
            34191_u64,
            custom_b_wrapper_source.as_str(),
            Some("utility"),
            first_auto_session.as_str(),
        ),
        (
            34192_u64,
            retained_wrapper_source.as_str(),
            Some("utility"),
            first_auto_session.as_str(),
        ),
        (
            34193_u64,
            custom_a_wrapper_source.as_str(),
            None,
            second_auto_session.as_str(),
        ),
        (
            34194_u64,
            custom_b_wrapper_source.as_str(),
            None,
            second_auto_session.as_str(),
        ),
        (
            34195_u64,
            retained_wrapper_source.as_str(),
            None,
            second_auto_session.as_str(),
        ),
        (
            34196_u64,
            custom_a_wrapper_source.as_str(),
            Some("utility"),
            second_auto_session.as_str(),
        ),
        (
            34197_u64,
            custom_b_wrapper_source.as_str(),
            Some("utility"),
            second_auto_session.as_str(),
        ),
        (
            34198_u64,
            retained_wrapper_source.as_str(),
            Some("utility"),
            second_auto_session.as_str(),
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
            "id": id,
            "method": "Page.addScriptToEvaluateOnNewDocument",
            "sessionId": session_id,
            "params": params
        }))
        .await;
        assert!(
            take_response_by_id(&mut ctx, id)["result"]["identifier"]
                .as_str()
                .is_some()
        );
    }

    ctx.process_async(json!({
        "id": 34199,
        "method": "Runtime.removeBinding",
        "sessionId": first_auto_session,
        "params": {
            "name": "customBindingA"
        }
    }))
    .await;
    assert_eq!(take_response_by_id(&mut ctx, 34199)["result"], json!({}));

    for (id, session_id, label) in [
        (34200_u64, first_auto_session.as_str(), "first-replay"),
        (34201_u64, second_auto_session.as_str(), "second-replay"),
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

    for (id, session_id, source, expected_type) in [
        (
            34202_u64,
            first_auto_session.as_str(),
            custom_a_wrapper_source.as_str(),
            "undefined",
        ),
        (
            34203_u64,
            first_auto_session.as_str(),
            custom_b_wrapper_source.as_str(),
            "function",
        ),
        (
            34204_u64,
            first_auto_session.as_str(),
            retained_wrapper_source.as_str(),
            "function",
        ),
        (
            34205_u64,
            second_auto_session.as_str(),
            custom_a_wrapper_source.as_str(),
            "function",
        ),
        (
            34206_u64,
            second_auto_session.as_str(),
            custom_b_wrapper_source.as_str(),
            "function",
        ),
        (
            34207_u64,
            second_auto_session.as_str(),
            retained_wrapper_source.as_str(),
            "function",
        ),
    ] {
        ctx.process_async(json!({
            "id": id,
            "method": "Runtime.evaluate",
            "sessionId": session_id,
            "params": {
                "expression": source,
                "awaitPromise": true
            }
        }))
        .await;
        let replayed = take_response_by_id(&mut ctx, id);
        assert_eq!(replayed["result"]["result"]["value"], json!(expected_type));
    }

    for (id, session_id, expected_state) in [
        (
            34208_u64,
            first_auto_session.as_str(),
            json!("[\"undefined\",\"function\",\"function\"]"),
        ),
        (
            34209_u64,
            second_auto_session.as_str(),
            json!("[\"function\",\"function\",\"function\"]"),
        ),
    ] {
        ctx.process_async(json!({
                "id": id,
                "method": "Runtime.evaluate",
                "sessionId": session_id,
                "params": {
                    "expression": "JSON.stringify([typeof globalThis.customBindingA, typeof globalThis.customBindingB, typeof globalThis.__pw_keptBinding])"
                }
            })).await;
        let state = take_response_by_id(&mut ctx, id);
        assert_eq!(state["result"]["result"]["value"], expected_state);
    }

    let mut first_replay_utility_context = 0_i64;
    let mut second_replay_utility_context = 0_i64;
    for (id, session_id, target_id, utility_context) in [
        (
            34210_u64,
            first_auto_session.as_str(),
            first.target_id.as_str(),
            &mut first_replay_utility_context,
        ),
        (
            34211_u64,
            second_auto_session.as_str(),
            second.target_id.as_str(),
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
        *utility_context = take_response_by_id(&mut ctx, id)["result"]["executionContextId"]
            .as_i64()
            .expect("replay utility context id");
        ctx.take_all();
    }

    for (id, session_id, utility_context, names) in [
        (
            34212_u64,
            first_auto_session.as_str(),
            first_replay_utility_context,
            vec!["customBindingB", "__pw_keptBinding"],
        ),
        (
            34214_u64,
            second_auto_session.as_str(),
            second_replay_utility_context,
            vec!["customBindingA", "customBindingB", "__pw_keptBinding"],
        ),
    ] {
        for (offset, binding_name) in names.iter().enumerate() {
            ctx.process_async(json!({
                "id": id + offset as u64,
                "method": "Runtime.addBinding",
                "sessionId": session_id,
                "params": {
                    "name": binding_name,
                    "executionContextId": utility_context
                }
            }))
            .await;
            assert_eq!(
                take_response_by_id(&mut ctx, id + offset as u64)["result"],
                json!({})
            );
        }
    }

    for (id, session_id, utility_context, source, expected_type) in [
        (
            34217_u64,
            first_auto_session.as_str(),
            first_replay_utility_context,
            custom_a_wrapper_source.as_str(),
            "undefined",
        ),
        (
            34218_u64,
            first_auto_session.as_str(),
            first_replay_utility_context,
            custom_b_wrapper_source.as_str(),
            "function",
        ),
        (
            34219_u64,
            first_auto_session.as_str(),
            first_replay_utility_context,
            retained_wrapper_source.as_str(),
            "function",
        ),
        (
            34220_u64,
            second_auto_session.as_str(),
            second_replay_utility_context,
            custom_a_wrapper_source.as_str(),
            "function",
        ),
        (
            34221_u64,
            second_auto_session.as_str(),
            second_replay_utility_context,
            custom_b_wrapper_source.as_str(),
            "function",
        ),
        (
            34222_u64,
            second_auto_session.as_str(),
            second_replay_utility_context,
            retained_wrapper_source.as_str(),
            "function",
        ),
    ] {
        ctx.process_async(json!({
            "id": id,
            "method": "Runtime.evaluate",
            "sessionId": session_id,
            "params": {
                "contextId": utility_context,
                "expression": source,
                "awaitPromise": true
            }
        }))
        .await;
        let replayed = take_response_by_id(&mut ctx, id);
        assert_eq!(replayed["result"]["result"]["value"], json!(expected_type));
    }

    for (id, session_id, utility_context, expected_state) in [
        (
            34223_u64,
            first_auto_session.as_str(),
            first_replay_utility_context,
            json!("[\"undefined\",\"function\",\"function\"]"),
        ),
        (
            34224_u64,
            second_auto_session.as_str(),
            second_replay_utility_context,
            json!("[\"function\",\"function\",\"function\"]"),
        ),
    ] {
        ctx.process_async(json!({
                "id": id,
                "method": "Runtime.evaluate",
                "sessionId": session_id,
                "params": {
                    "contextId": utility_context,
                    "expression": "JSON.stringify([typeof globalThis.customBindingA, typeof globalThis.customBindingB, typeof globalThis.__pw_keptBinding])"
                }
            })).await;
        let state = take_response_by_id(&mut ctx, id);
        assert_eq!(state["result"]["result"]["value"], expected_state);
    }

    for (
        id,
        session_id,
        context_id,
        expression,
        binding_name,
        expected_arg,
        deliver_expression,
        promise_name,
        expected_result,
    ) in [
        (
            34225_u64,
            first_auto_session.as_str(),
            None,
            "globalThis.__lm_first_replay_pw = __pw_keptBinding({ source: 'first-replay-main-kept', nested: { count: 1, values: ['a', 2, true] } }); 'scheduled-first-main-kept'",
            "__pw_keptBinding",
            json!([{
                "source": "first-replay-main-kept",
                "nested": { "count": 1, "values": ["a", 2, true] }
            }]),
            "globalThis.__lm_pw_kept_binding_deliver({ name: '__pw_keptBinding', seq: 1, result: 'first-replay-main-kept-ok' }); 'delivered'",
            "__lm_first_replay_pw",
            "first-replay-main-kept-ok",
        ),
        (
            34226_u64,
            first_auto_session.as_str(),
            Some(first_replay_utility_context),
            "globalThis.__lm_first_replay_custom_b = customBindingB({ source: 'first-replay-utility-b', nested: { count: 2, values: ['b', 3, false] } }); 'scheduled-first-utility-b'",
            "customBindingB",
            json!([{
                "source": "first-replay-utility-b",
                "nested": { "count": 2, "values": ["b", 3, false] }
            }]),
            "globalThis.__lm_custom_binding_b_deliver({ name: 'customBindingB', seq: 1, result: 'first-replay-utility-b-ok' }); 'delivered'",
            "__lm_first_replay_custom_b",
            "first-replay-utility-b-ok",
        ),
        (
            34227_u64,
            second_auto_session.as_str(),
            None,
            "globalThis.__lm_second_replay_custom_a = customBindingA({ source: 'second-replay-main-a', nested: { count: 3, values: ['c', 4, true] } }); 'scheduled-second-main-a'",
            "customBindingA",
            json!([{
                "source": "second-replay-main-a",
                "nested": { "count": 3, "values": ["c", 4, true] }
            }]),
            "globalThis.__lm_custom_binding_a_deliver({ name: 'customBindingA', seq: 1, result: 'second-replay-main-a-ok' }); 'delivered'",
            "__lm_second_replay_custom_a",
            "second-replay-main-a-ok",
        ),
        (
            34228_u64,
            second_auto_session.as_str(),
            Some(second_replay_utility_context),
            "globalThis.__lm_second_replay_custom_b = customBindingB({ source: 'second-replay-utility-b', nested: { count: 4, values: ['d', 5, false] } }); 'scheduled-second-utility-b'",
            "customBindingB",
            json!([{
                "source": "second-replay-utility-b",
                "nested": { "count": 4, "values": ["d", 5, false] }
            }]),
            "globalThis.__lm_custom_binding_b_deliver({ name: 'customBindingB', seq: 1, result: 'second-replay-utility-b-ok' }); 'delivered'",
            "__lm_second_replay_custom_b",
            "second-replay-utility-b-ok",
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
                .expect("scheduled value")
                .starts_with("scheduled-")
        );

        let binding_called = ctx
            .sent
            .iter()
            .rev()
            .find(|message| {
                message["method"] == json!("Runtime.bindingCalled")
                    && message["sessionId"] == json!(session_id)
                    && message["params"]["name"] == json!(binding_name)
                    && match context_id {
                        Some(context_id) => {
                            message["params"]["executionContextId"] == json!(context_id)
                        }
                        None => true,
                    }
            })
            .cloned()
            .expect("replayed binding should emit Runtime.bindingCalled");
        let payload = binding_called["params"]["payload"]
            .as_str()
            .expect("binding payload should be a json string");
        let payload: serde_json::Value = serde_json::from_str(payload).unwrap_or_else(|error| {
                panic!(
                    "binding payload should be valid json: {error}; payload={payload:?}; event={binding_called:?}"
                )
            });
        assert_eq!(payload["name"], json!(binding_name));
        assert_eq!(payload["serializedArgs"], expected_arg);
        assert_eq!(payload["seq"], json!(1));
        ctx.sent.clear();

        let mut deliver_params = json!({
            "expression": deliver_expression,
            "awaitPromise": true
        });
        if let Some(context_id) = context_id {
            deliver_params["contextId"] = json!(context_id);
        }
        ctx.process_async(json!({
            "id": id + 10,
            "method": "Runtime.evaluate",
            "sessionId": session_id,
            "params": deliver_params
        }))
        .await;
        let delivered = take_response_by_id(&mut ctx, id + 10);
        assert_eq!(delivered["result"]["result"]["value"], json!("delivered"));

        let mut promise_params = json!({
            "expression": format!("globalThis.{promise_name}"),
            "awaitPromise": true
        });
        if let Some(context_id) = context_id {
            promise_params["contextId"] = json!(context_id);
        }
        ctx.process_async(json!({
            "id": id + 20,
            "method": "Runtime.evaluate",
            "sessionId": session_id,
            "params": promise_params
        }))
        .await;
        let resolved = take_response_by_id(&mut ctx, id + 20);
        assert_eq!(
            resolved["result"]["result"]["value"],
            json!(expected_result)
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn patchright_over_cdp_auto_attach_sweep_crpage_cleanup_replay_removes_all_custom_bindings_and_retains_pw_only_in_cleaned_context()
 {
    super::patchright_8mb_stack(
        "patchright-handle-cleanup-remove-custom",
        || async {
            run_patchright_over_cdp_auto_attach_sweep_crpage_cleanup_replay_removes_all_custom_bindings_and_retains_pw_only_in_cleaned_context()
                .await;
        },
    )
    .await;
}

async fn run_patchright_over_cdp_auto_attach_sweep_crpage_cleanup_replay_removes_all_custom_bindings_and_retains_pw_only_in_cleaned_context()
 {
    let mut ctx = TestContext::new();
    let first =
        create_attached_page_session_without_runtime_enable_async(&mut ctx, 34700, 34701, 34702)
            .await;
    let second =
        create_attached_page_session_without_runtime_enable_async(&mut ctx, 34703, 34704, 34705)
            .await;

    for (id, session_id, html) in [
        (
            34706_u64,
            first.session_id.as_str(),
            "<body><div id='first-main'>first-main</div><div id='first-utility'>first-utility</div></body>",
        ),
        (
            34707_u64,
            second.session_id.as_str(),
            "<body><div id='second-main'>second-main</div><div id='second-utility'>second-utility</div></body>",
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
            34708_u64,
            first.target_id.as_str(),
            first.session_id.as_str(),
        ),
        (
            34709_u64,
            second.target_id.as_str(),
            second.session_id.as_str(),
        ),
    ] {
        ctx.process_async(json!({
            "id": id,
            "method": "Target.detachFromTarget",
            "params": { "targetId": target_id, "sessionId": session_id }
        }))
        .await;
        ctx.expect_result(id, json!({}), None);
        ctx.expect_event(
            "Target.detachedFromTarget",
            Some(&json!({ "targetId": target_id, "sessionId": session_id })),
        );
    }

    ctx.process_async(json!({
        "id": 34710,
        "method": "Target.setAutoAttach",
        "params": { "autoAttach": true, "waitForDebuggerOnStart": false }
    }))
    .await;
    ctx.expect_result(34710, json!({}), None);
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
            34711_u64,
            first_auto_session.as_str(),
            first.target_id.as_str(),
            &mut first_utility_context,
        ),
        (
            34712_u64,
            second_auto_session.as_str(),
            second.target_id.as_str(),
            &mut second_utility_context,
        ),
    ] {
        ctx.process_async(json!({
            "id": id,
            "method": "Page.createIsolatedWorld",
            "sessionId": session_id,
            "params": { "frameId": target_id, "worldName": "utility" }
        }))
        .await;
        *utility_context = take_response_by_id(&mut ctx, id)["result"]["executionContextId"]
            .as_i64()
            .expect("utility context id after auto-attach");
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
    let retained_wrapper_source = patchright_page_binding_wrapper_source(
        "__pw_keptBinding",
        "__lm_pw_kept_binding_deliver",
        None,
        false,
    );

    for (id, session_id, utility_context) in [
        (
            34713_u64,
            first_auto_session.as_str(),
            first_utility_context,
        ),
        (
            34725_u64,
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
            "customBindingA",
            &custom_a_wrapper_source,
        )
        .await;
        install_patchright_crpage_binding_in_existing_worlds_async(
            &mut ctx,
            session_id,
            utility_context,
            id + 4,
            id + 5,
            id + 6,
            id + 7,
            "customBindingB",
            &custom_b_wrapper_source,
        )
        .await;
        install_patchright_crpage_binding_in_existing_worlds_async(
            &mut ctx,
            session_id,
            utility_context,
            id + 8,
            id + 9,
            id + 10,
            id + 11,
            "__pw_keptBinding",
            &retained_wrapper_source,
        )
        .await;
    }

    for (id, source, world_name, session_id) in [
        (
            34737_u64,
            custom_a_wrapper_source.as_str(),
            None,
            first_auto_session.as_str(),
        ),
        (
            34738_u64,
            custom_b_wrapper_source.as_str(),
            None,
            first_auto_session.as_str(),
        ),
        (
            34739_u64,
            retained_wrapper_source.as_str(),
            None,
            first_auto_session.as_str(),
        ),
        (
            34740_u64,
            custom_a_wrapper_source.as_str(),
            Some("utility"),
            first_auto_session.as_str(),
        ),
        (
            34741_u64,
            custom_b_wrapper_source.as_str(),
            Some("utility"),
            first_auto_session.as_str(),
        ),
        (
            34742_u64,
            retained_wrapper_source.as_str(),
            Some("utility"),
            first_auto_session.as_str(),
        ),
        (
            34743_u64,
            custom_a_wrapper_source.as_str(),
            None,
            second_auto_session.as_str(),
        ),
        (
            34744_u64,
            custom_b_wrapper_source.as_str(),
            None,
            second_auto_session.as_str(),
        ),
        (
            34745_u64,
            retained_wrapper_source.as_str(),
            None,
            second_auto_session.as_str(),
        ),
        (
            34746_u64,
            custom_a_wrapper_source.as_str(),
            Some("utility"),
            second_auto_session.as_str(),
        ),
        (
            34747_u64,
            custom_b_wrapper_source.as_str(),
            Some("utility"),
            second_auto_session.as_str(),
        ),
        (
            34748_u64,
            retained_wrapper_source.as_str(),
            Some("utility"),
            second_auto_session.as_str(),
        ),
    ] {
        let mut params = json!({ "source": source, "runImmediately": true });
        if let Some(world_name) = world_name {
            params["worldName"] = json!(world_name);
        }
        ctx.process_async(json!({
            "id": id,
            "method": "Page.addScriptToEvaluateOnNewDocument",
            "sessionId": session_id,
            "params": params
        }))
        .await;
        assert!(
            take_response_by_id(&mut ctx, id)["result"]["identifier"]
                .as_str()
                .is_some()
        );
    }

    for (id, binding_name) in [(34749_u64, "customBindingA"), (34750_u64, "customBindingB")] {
        ctx.process_async(json!({
            "id": id,
            "method": "Runtime.removeBinding",
            "sessionId": first_auto_session,
            "params": { "name": binding_name }
        }))
        .await;
        assert_eq!(take_response_by_id(&mut ctx, id)["result"], json!({}));
    }

    for (id, session_id, label) in [
        (34751_u64, first_auto_session.as_str(), "first-replay"),
        (34752_u64, second_auto_session.as_str(), "second-replay"),
    ] {
        ctx.process_async(json!({
            "id": id,
            "method": "Page.navigate",
            "sessionId": session_id,
            "params": { "url": format!("data:text/html,<body><div id='page'>{label}</div></body>") }
        }))
        .await;
        let navigation = take_response_by_id(&mut ctx, id);
        assert_eq!(navigation["sessionId"], json!(session_id));
        ctx.take_all();
    }

    for (id, session_id, source, expected_type) in [
        (
            34753_u64,
            first_auto_session.as_str(),
            custom_a_wrapper_source.as_str(),
            "undefined",
        ),
        (
            34754_u64,
            first_auto_session.as_str(),
            custom_b_wrapper_source.as_str(),
            "undefined",
        ),
        (
            34755_u64,
            first_auto_session.as_str(),
            retained_wrapper_source.as_str(),
            "function",
        ),
        (
            34756_u64,
            second_auto_session.as_str(),
            custom_a_wrapper_source.as_str(),
            "function",
        ),
        (
            34757_u64,
            second_auto_session.as_str(),
            custom_b_wrapper_source.as_str(),
            "function",
        ),
        (
            34758_u64,
            second_auto_session.as_str(),
            retained_wrapper_source.as_str(),
            "function",
        ),
    ] {
        ctx.process_async(json!({
            "id": id,
            "method": "Runtime.evaluate",
            "sessionId": session_id,
            "params": { "expression": source, "awaitPromise": true }
        }))
        .await;
        let replayed = take_response_by_id(&mut ctx, id);
        assert_eq!(replayed["result"]["result"]["value"], json!(expected_type));
    }

    for (id, session_id, expected_state) in [
        (
            34759_u64,
            first_auto_session.as_str(),
            json!("[\"undefined\",\"undefined\",\"function\"]"),
        ),
        (
            34760_u64,
            second_auto_session.as_str(),
            json!("[\"function\",\"function\",\"function\"]"),
        ),
    ] {
        ctx.process_async(json!({
                "id": id,
                "method": "Runtime.evaluate",
                "sessionId": session_id,
                "params": {
                    "expression": "JSON.stringify([typeof globalThis.customBindingA, typeof globalThis.customBindingB, typeof globalThis.__pw_keptBinding])"
                }
            })).await;
        let state = take_response_by_id(&mut ctx, id);
        assert_eq!(state["result"]["result"]["value"], expected_state);
    }

    let mut first_replay_utility_context = 0_i64;
    let mut second_replay_utility_context = 0_i64;
    for (id, session_id, target_id, utility_context) in [
        (
            34761_u64,
            first_auto_session.as_str(),
            first.target_id.as_str(),
            &mut first_replay_utility_context,
        ),
        (
            34762_u64,
            second_auto_session.as_str(),
            second.target_id.as_str(),
            &mut second_replay_utility_context,
        ),
    ] {
        ctx.process_async(json!({
            "id": id,
            "method": "Page.createIsolatedWorld",
            "sessionId": session_id,
            "params": { "frameId": target_id, "worldName": "utility" }
        }))
        .await;
        *utility_context = take_response_by_id(&mut ctx, id)["result"]["executionContextId"]
            .as_i64()
            .expect("replay utility context id");
        ctx.take_all();
    }

    for (id, session_id, utility_context, names) in [
        (
            34763_u64,
            first_auto_session.as_str(),
            first_replay_utility_context,
            vec!["__pw_keptBinding"],
        ),
        (
            34764_u64,
            second_auto_session.as_str(),
            second_replay_utility_context,
            vec!["customBindingA", "customBindingB", "__pw_keptBinding"],
        ),
    ] {
        for (offset, binding_name) in names.iter().enumerate() {
            ctx.process_async(json!({
                "id": id + offset as u64,
                "method": "Runtime.addBinding",
                "sessionId": session_id,
                "params": { "name": binding_name, "executionContextId": utility_context }
            }))
            .await;
            assert_eq!(
                take_response_by_id(&mut ctx, id + offset as u64)["result"],
                json!({})
            );
        }
    }

    for (id, session_id, utility_context, source, expected_type) in [
        (
            34767_u64,
            first_auto_session.as_str(),
            first_replay_utility_context,
            custom_a_wrapper_source.as_str(),
            "undefined",
        ),
        (
            34768_u64,
            first_auto_session.as_str(),
            first_replay_utility_context,
            custom_b_wrapper_source.as_str(),
            "undefined",
        ),
        (
            34769_u64,
            first_auto_session.as_str(),
            first_replay_utility_context,
            retained_wrapper_source.as_str(),
            "function",
        ),
        (
            34770_u64,
            second_auto_session.as_str(),
            second_replay_utility_context,
            custom_a_wrapper_source.as_str(),
            "function",
        ),
        (
            34771_u64,
            second_auto_session.as_str(),
            second_replay_utility_context,
            custom_b_wrapper_source.as_str(),
            "function",
        ),
        (
            34772_u64,
            second_auto_session.as_str(),
            second_replay_utility_context,
            retained_wrapper_source.as_str(),
            "function",
        ),
    ] {
        ctx.process_async(json!({
            "id": id,
            "method": "Runtime.evaluate",
            "sessionId": session_id,
            "params": { "contextId": utility_context, "expression": source, "awaitPromise": true }
        }))
        .await;
        let replayed = take_response_by_id(&mut ctx, id);
        assert_eq!(replayed["result"]["result"]["value"], json!(expected_type));
    }

    for (id, session_id, utility_context, expected_state) in [
        (
            34773_u64,
            first_auto_session.as_str(),
            first_replay_utility_context,
            json!("[\"undefined\",\"undefined\",\"function\"]"),
        ),
        (
            34774_u64,
            second_auto_session.as_str(),
            second_replay_utility_context,
            json!("[\"function\",\"function\",\"function\"]"),
        ),
    ] {
        ctx.process_async(json!({
                "id": id,
                "method": "Runtime.evaluate",
                "sessionId": session_id,
                "params": {
                    "contextId": utility_context,
                    "expression": "JSON.stringify([typeof globalThis.customBindingA, typeof globalThis.customBindingB, typeof globalThis.__pw_keptBinding])"
                }
            })).await;
        let state = take_response_by_id(&mut ctx, id);
        assert_eq!(state["result"]["result"]["value"], expected_state);
    }

    for (
        id,
        session_id,
        context_id,
        expression,
        binding_name,
        expected_arg,
        deliver_expression,
        promise_name,
        expected_result,
    ) in [
        (
            34775_u64,
            first_auto_session.as_str(),
            Some(first_replay_utility_context),
            "globalThis.__lm_first_replay_pw_kept = __pw_keptBinding({ source: 'first-replay-kept', nested: { count: 1, values: ['a', 2, true] } }); 'scheduled-first-kept'",
            "__pw_keptBinding",
            json!([{ "source": "first-replay-kept", "nested": { "count": 1, "values": ["a", 2, true] } }]),
            "globalThis.__lm_pw_kept_binding_deliver({ name: '__pw_keptBinding', seq: 1, result: 'first-replay-kept-ok' }); 'delivered'",
            "__lm_first_replay_pw_kept",
            "first-replay-kept-ok",
        ),
        (
            34776_u64,
            second_auto_session.as_str(),
            None,
            "globalThis.__lm_second_replay_custom_a = customBindingA({ source: 'second-replay-a', nested: { count: 2, values: ['b', 3, false] } }); 'scheduled-second-a'",
            "customBindingA",
            json!([{ "source": "second-replay-a", "nested": { "count": 2, "values": ["b", 3, false] } }]),
            "globalThis.__lm_custom_binding_a_deliver({ name: 'customBindingA', seq: 1, result: 'second-replay-a-ok' }); 'delivered'",
            "__lm_second_replay_custom_a",
            "second-replay-a-ok",
        ),
    ] {
        let mut params = json!({ "expression": expression, "awaitPromise": true });
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
                .expect("scheduled value")
                .starts_with("scheduled-")
        );

        let binding_called = ctx
            .sent
            .iter()
            .find(|message| {
                message["method"] == json!("Runtime.bindingCalled")
                    && message["params"]["name"] == json!(binding_name)
                    && match context_id {
                        Some(context_id) => {
                            message["params"]["executionContextId"] == json!(context_id)
                        }
                        None => true,
                    }
            })
            .cloned()
            .expect("remaining replay binding should still emit Runtime.bindingCalled");
        let payload = binding_called["params"]["payload"]
            .as_str()
            .expect("binding payload should be a json string");
        let payload: serde_json::Value =
            serde_json::from_str(payload).expect("binding payload should be valid json");
        assert_eq!(payload["name"], json!(binding_name));
        assert_eq!(payload["serializedArgs"], expected_arg);
        assert_eq!(payload["seq"], json!(1));
        ctx.sent.clear();

        let mut deliver_params = json!({ "expression": deliver_expression, "awaitPromise": true });
        if let Some(context_id) = context_id {
            deliver_params["contextId"] = json!(context_id);
        }
        ctx.process_async(json!({
            "id": id + 10,
            "method": "Runtime.evaluate",
            "sessionId": session_id,
            "params": deliver_params
        }))
        .await;
        let delivered = take_response_by_id(&mut ctx, id + 10);
        assert_eq!(delivered["result"]["result"]["value"], json!("delivered"));

        let mut promise_params = json!({
            "expression": format!("globalThis.{promise_name}"),
            "awaitPromise": true
        });
        if let Some(context_id) = context_id {
            promise_params["contextId"] = json!(context_id);
        }
        ctx.process_async(json!({
            "id": id + 20,
            "method": "Runtime.evaluate",
            "sessionId": session_id,
            "params": promise_params
        }))
        .await;
        let resolved = take_response_by_id(&mut ctx, id + 20);
        assert_eq!(
            resolved["result"]["result"]["value"],
            json!(expected_result)
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn patchright_over_cdp_page_binding_handle_round_trip_rejects_without_runtime_enable() {
    let mut ctx = TestContext::new();

    ctx.process_async(json!({
        "id": 323,
        "method": "Target.createBrowserContext"
    }))
    .await;
    let browser_context_id = take_response_by_id(&mut ctx, 323)["result"]["browserContextId"]
        .as_str()
        .expect("browser context id")
        .to_owned();

    ctx.process_async(json!({
        "id": 324,
        "method": "Target.createTarget",
        "params": {
            "browserContextId": browser_context_id,
            "url": "about:blank"
        }
    }))
    .await;
    let target_id = take_response_by_id(&mut ctx, 324)["result"]["targetId"]
        .as_str()
        .expect("target id")
        .to_owned();
    ctx.expect_event(
        "Target.targetCreated",
        Some(&json!({
            "targetInfo": {
                "targetId": target_id,
                "browserContextId": browser_context_id,
            }
        })),
    );

    ctx.process_async(json!({
        "id": 325,
        "method": "Target.attachToTarget",
        "params": {
            "targetId": target_id,
            "flatten": true
        }
    }))
    .await;
    let session_id = ctx.with_fixture(|conn| {
        conn.browser_context
            .as_ref()
            .and_then(|bc| bc.active_session_id_owned())
            .expect("session id should exist")
    });
    ctx.expect_result(325, json!({ "sessionId": session_id }), None);
    ctx.expect_event(
        "Target.attachedToTarget",
        Some(&json!({
            "sessionId": session_id,
            "targetInfo": {
                "targetId": target_id,
                "browserContextId": browser_context_id,
            }
        })),
    );

    ctx.process_async(json!({
        "id": 326,
        "method": "Page.navigate",
        "sessionId": session_id,
        "params": {
            "url": "data:text/html,<body><div id='reject-handle'>node</div></body>"
        }
    }))
    .await;
    let navigation = take_response_by_id(&mut ctx, 326);
    assert_eq!(navigation["sessionId"], json!(session_id));
    ctx.take_all();

    ctx.process_async(json!({
        "id": 327,
        "method": "Page.createIsolatedWorld",
        "sessionId": session_id,
        "params": {
            "frameId": target_id,
            "worldName": "utility"
        }
    }))
    .await;
    let utility_context_id = take_response_by_id(&mut ctx, 327)["result"]["executionContextId"]
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
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 328,
        "method": "Runtime.addBinding",
        "sessionId": session_id,
        "params": {
            "name": "patchedRejectingHandleBinding",
            "executionContextId": utility_context_id
        }
    }))
    .await;
    let add_binding = take_response_by_id(&mut ctx, 328);
    assert_eq!(add_binding["result"], json!({}));

    ctx.process_async(json!({
            "id": 329,
            "method": "Runtime.evaluate",
            "sessionId": session_id,
            "params": {
                "contextId": utility_context_id,
                "expression": r#"
                    (() => {
                        function addHandleBinding(bindingName) {
                            const binding = globalThis[bindingName];
                            globalThis[bindingName] = (...args) => {
                                const me = globalThis[bindingName];
                                let callbacks = me.callbacks;
                                if (!callbacks) {
                                    callbacks = new Map();
                                    me.callbacks = callbacks;
                                }
                                let handles = me.handles;
                                if (!handles) {
                                    handles = new Map();
                                    me.handles = handles;
                                }
                                const seq = (me.lastSeq || 0) + 1;
                                me.lastSeq = seq;
                                handles.set(seq, args[0]);
                                const promise = new Promise((resolve, reject) => callbacks.set(seq, { resolve, reject }));
                                binding(JSON.stringify({ name: bindingName, seq }));
                                return promise;
                            };
                        }
                        function takeBindingHandle(arg) {
                            const handles = globalThis[arg.name].handles;
                            const handle = handles.get(arg.seq);
                            handles.delete(arg.seq);
                            return handle;
                        }
                        function deliverBindingResult(arg) {
                            const callbacks = globalThis[arg.name].callbacks;
                            if ('error' in arg)
                                callbacks.get(arg.seq).reject(arg.error);
                            else
                                callbacks.get(arg.seq).resolve(arg.result);
                            callbacks.delete(arg.seq);
                        }
                        addHandleBinding('patchedRejectingHandleBinding');
                        globalThis.__lm_takeRejectingHandleBindingHandle = takeBindingHandle;
                        globalThis.__lm_deliverRejectingHandleBindingResult = deliverBindingResult;
                        return typeof globalThis.patchedRejectingHandleBinding;
                    })()
                "#
            }
        })).await;
    let install_wrapper = take_response_by_id(&mut ctx, 329);
    assert_eq!(
        install_wrapper["result"]["result"]["value"],
        json!("function")
    );

    ctx.process_async(json!({
            "id": 330,
            "method": "Runtime.evaluate",
            "sessionId": session_id,
            "params": {
                "contextId": utility_context_id,
                "expression": "globalThis.__lm_handlePromise = patchedRejectingHandleBinding(document.getElementById('reject-handle')).then(() => 'unexpected-success', error => `rejected:${error}`); 'scheduled'"
            }
        })).await;
    let scheduled = take_response_by_id(&mut ctx, 330);
    assert_eq!(scheduled["result"]["result"]["value"], json!("scheduled"));
    let binding_called = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Runtime.bindingCalled")
                && message["params"]["name"] == json!("patchedRejectingHandleBinding")
        })
        .cloned()
        .expect("handle binding wrapper should emit Runtime.bindingCalled");
    assert_eq!(
        binding_called["params"]["executionContextId"],
        json!(utility_context_id)
    );
    let binding_payload = binding_called["params"]["payload"]
        .as_str()
        .expect("binding payload should be a json string");
    let binding_payload: serde_json::Value =
        serde_json::from_str(binding_payload).expect("binding payload should be valid json");
    assert_eq!(
        binding_payload,
        json!({
            "name": "patchedRejectingHandleBinding",
            "seq": binding_payload["seq"],
        })
    );
    let seq = binding_payload["seq"]
        .as_i64()
        .expect("binding payload seq should be an integer");
    ctx.sent.clear();

    ctx.process_async(json!({
            "id": 331,
            "method": "Runtime.evaluate",
            "sessionId": session_id,
            "params": {
                "contextId": utility_context_id,
                "expression": format!("(() => {{ const handle = globalThis.__lm_takeRejectingHandleBindingHandle({{ name: 'patchedRejectingHandleBinding', seq: {seq} }}); return JSON.stringify([handle.id, handle.tagName, typeof globalThis.__lm_takeRejectingHandleBindingHandle({{ name: 'patchedRejectingHandleBinding', seq: {seq} }})]); }})()")
            }
        })).await;
    let taken_handle = take_response_by_id(&mut ctx, 331);
    assert_eq!(
        taken_handle["result"]["result"]["value"],
        json!("[\"reject-handle\",\"DIV\",\"undefined\"]")
    );

    ctx.process_async(json!({
            "id": 332,
            "method": "Runtime.evaluate",
            "sessionId": session_id,
            "params": {
                "contextId": utility_context_id,
                "expression": format!("globalThis.__lm_deliverRejectingHandleBindingResult({{ name: 'patchedRejectingHandleBinding', seq: {seq}, error: 'handle-rejected' }}); 'delivered'")
            }
        })).await;
    let delivered = take_response_by_id(&mut ctx, 332);
    assert_eq!(delivered["result"]["result"]["value"], json!("delivered"));

    ctx.process_async(json!({
        "id": 333,
        "method": "Runtime.evaluate",
        "sessionId": session_id,
        "params": {
            "contextId": utility_context_id,
            "expression": "globalThis.__lm_handlePromise",
            "awaitPromise": true
        }
    }))
    .await;
    let rejected = take_response_by_id(&mut ctx, 333);
    assert_eq!(
        rejected["result"]["result"]["value"],
        json!("rejected:handle-rejected")
    );
    assert!(
        !ctx.sent.iter().any(|message| {
            message["method"] == json!("Runtime.executionContextCreated")
                || message["method"] == json!("Runtime.executionContextsCleared")
        }),
        "handle binding rejection round trip should stay off Runtime.enable surfaces: {:?}",
        ctx.sent
    );
}
