// Tests grouped by behavior. Shared fixtures live in the parent module.
use super::*;

#[tokio::test]
async fn runtime_evaluate_detached_document_xpath_returns_iterator_results() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(&mut ctx, "<html><body></body></html>").await;
    let _ = enable_runtime_and_take_execution_context_id_async(&mut ctx, 150).await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 151,
        "method": "Runtime.evaluate",
        "params": {
            "expression": "(() => { const doc = new DOMParser().parseFromString('<div id=\"a\"></div><section><div id=\"b\"></div></section>', 'text/html'); const result = doc.evaluate('//div', doc, null, XPathResult.ORDERED_NODE_ITERATOR_TYPE); return [result.resultType, result.iterateNext()?.id ?? null, result.iterateNext()?.id ?? null, result.iterateNext()]; })()",
            "returnByValue": true
        }
    }))
    .await;

    let response = take_response_by_id(&mut ctx, 151);
    assert_eq!(
        response["result"]["result"]["value"],
        json!([5, "a", "b", null])
    );
}

#[tokio::test]
async fn runtime_evaluate_live_document_xpath_returns_iterator_results() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(
        &mut ctx,
        "<html><body><div id=\"a\"></div><section><div id=\"b\"></div></section></body></html>",
    )
    .await;
    let _ = enable_runtime_and_take_execution_context_id_async(&mut ctx, 151).await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 152,
        "method": "Runtime.evaluate",
        "params": {
            "expression": "(() => { const result = document.evaluate('.//div', document.body, null, XPathResult.ORDERED_NODE_ITERATOR_TYPE); return [result.resultType, result.iterateNext()?.id ?? null, result.iterateNext()?.id ?? null, result.iterateNext()]; })()",
            "returnByValue": true
        }
    }))
    .await;

    let response = take_response_by_id(&mut ctx, 152);
    assert_eq!(
        response["result"]["result"]["value"],
        json!([5, "a", "b", null])
    );
}

#[tokio::test]
async fn runtime_evaluate_patchright_closed_shadow_root_xpath_engine_maps_back_to_live_nodes() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(&mut ctx, "<html><body></body></html>").await;
    let _ = enable_runtime_and_take_execution_context_id_async(&mut ctx, 152).await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 153,
        "method": "Runtime.evaluate",
        "params": {
            "expression": r#"(() => {
                const host = document.createElement('div');
                document.body.appendChild(host);
                const root = host.attachShadow({ mode: 'closed' });
                root.innerHTML = '<section><div id="a"></div><span><div id="b"></div></span></section>';

                const result = [];
                const parser = new DOMParser();
                function getAllChildElements(node) {
                    const elements = [];
                    const traverse = currentNode => {
                        if (currentNode.nodeType === Node.ELEMENT_NODE)
                            elements.push(currentNode);
                        currentNode.childNodes?.forEach(traverse);
                    };
                    if (node.nodeType === Node.DOCUMENT_FRAGMENT_NODE || node.nodeType === Node.ELEMENT_NODE)
                        traverse(node);
                    return elements;
                }

                const csrHTMLContent = root.innerHTML;
                const csrChildElements = getAllChildElements(root);
                const htmlDoc = parser.parseFromString(csrHTMLContent, 'text/html');
                const rootDiv = htmlDoc.body;
                const rootDivChildElements = getAllChildElements(rootDiv);
                const it = htmlDoc.evaluate('//div', htmlDoc, null, XPathResult.ORDERED_NODE_ITERATOR_TYPE);
                for (let node = it.iterateNext(); node; node = it.iterateNext()) {
                    const nodeIndex = rootDivChildElements.indexOf(node) - 1;
                    if (nodeIndex >= 0) {
                        const originalNode = csrChildElements[nodeIndex];
                        if (originalNode.nodeType === Node.ELEMENT_NODE)
                            result.push(originalNode.id);
                    }
                }
                return result;
            })()"#,
            "returnByValue": true
        }
    }))
    .await;

    let response = take_response_by_id(&mut ctx, 153);
    assert_eq!(response["result"]["result"]["value"], json!(["a", "b"]));
}

#[tokio::test]
async fn runtime_evaluate_dom_parser_query_apis_reuse_existing_detached_node_identity() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(&mut ctx, "<html><body></body></html>").await;
    let _ = enable_runtime_and_take_execution_context_id_async(&mut ctx, 154).await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 155,
        "method": "Runtime.evaluate",
        "params": {
            "expression": r#"(() => {
                const doc = new DOMParser().parseFromString('<div id="a"></div><section><div id="b"></div></section>', 'text/html');
                const bodyChildren = Array.from(doc.body.childNodes).filter(node => node.nodeType === Node.ELEMENT_NODE);
                const first = bodyChildren[0];
                const second = bodyChildren[1].childNodes[0];
                return [
                    first === doc.querySelector('#a'),
                    second === doc.querySelector('#b'),
                    second === doc.getElementById('b'),
                    doc.getElementsByTagName('div').item(1) === second,
                ];
            })()"#,
            "returnByValue": true
        }
    }))
    .await;

    let response = take_response_by_id(&mut ctx, 155);
    assert_eq!(
        response["result"]["result"]["value"],
        json!([true, true, true, true])
    );
}

#[tokio::test]
async fn registered_named_world_object_handles_remain_callable_after_navigation() {
    let mut ctx = TestContext::new();
    let (background_tx, mut background_rx) = tokio::sync::mpsc::unbounded_channel();
    ctx.with_fixture(|conn| conn.set_background_event_sender(background_tx));
    with_loaded_document_async(&mut ctx, "<html><body>before</body></html>").await;
    ctx.with_fixture(|conn| {
        let bc = {
            conn.browser_context
                .as_mut()
                .expect("browser context should exist")
        };
        bc.set_active_target_id("TID-1");
        bc.attach_active_session("SID-1");
        bc.active_page_target_mut().devtools_sessions
            [moli_page_types::DevToolsSessionKey::Primary]
            .runtime_session_state
            .runtime_frontend_enabled = true;
    });

    let _ = create_isolated_world_async(&mut ctx, 506, "utility").await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 5061,
        "method": "Page.addScriptToEvaluateOnNewDocument",
        "sessionId": "SID-1",
        "params": {
            "source": "",
            "worldName": "utility"
        }
    }))
    .await;
    assert!(
        take_response_by_id(&mut ctx, 5061)["result"]["identifier"]
            .as_str()
            .is_some()
    );
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 507,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": "data:text/html,<body>after</body>" }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 507);
    let isolated_context_id = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Runtime.executionContextCreated")
                && message["params"]["context"]["name"] == json!("utility")
        })
        .and_then(|message| message["params"]["context"]["id"].as_i64())
        .expect("navigation should replay the isolated utility world");
    ctx.sent.clear();
    while background_rx.try_recv().is_ok() {}

    ctx.process_async(json!({
        "id": 508,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": "({ answer: 42 })",
            "contextId": isolated_context_id
        }
    }))
    .await;
    let object_id = take_response_by_id(&mut ctx, 508)["result"]["result"]["objectId"]
        .as_str()
        .expect("isolated evaluation should return an object handle")
        .to_owned();

    ctx.process_async(json!({
        "id": 509,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": "({ bonus: 1 })",
            "contextId": isolated_context_id
        }
    }))
    .await;
    let argument_object_id = take_response_by_id(&mut ctx, 509)["result"]["result"]["objectId"]
        .as_str()
        .expect("isolated evaluation should return an argument object handle")
        .to_owned();

    ctx.process_async(json!({
        "id": 510,
        "method": "Runtime.callFunctionOn",
        "sessionId": "SID-1",
        "params": {
            "objectId": object_id,
            "functionDeclaration": "function(arg) { return Promise.resolve(this.answer + arg.bonus); }",
            "arguments": [{ "objectId": argument_object_id }],
            "returnByValue": true,
            "awaitPromise": true
        }
    }))
    .await;
    let mut other_context = crate::conn::BrowserContext::new("BID-2".into());
    other_context.set_active_target_id("TID-2");
    other_context.attach_active_session("SID-2");
    ctx.with_fixture(|conn| conn.push_inactive_browser_context_fixture_for_test(other_context));
    ctx.with_fixture(|conn| { assert!(
        conn.activate_browser_context_by_id("BID-2")
            ,
        "test setup should switch the active context away from the pending Runtime.callFunctionOn owner"
    ) });

    for _ in 0..64 {
        if ctx.sent.iter().any(|message| message["id"] == json!(510)) {
            break;
        }
        while let Ok(message) = background_rx.try_recv() {
            match message.take_runtime_inspector_response_ready() {
                Ok(response) => {
                    ctx.sent
                        .push(response.into_protocol_message_for_typed_runtime_route());
                }
                Err(message) => ctx.sent.push(message.into_protocol_message()),
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert!(
        ctx.sent.iter().any(|message| message["id"] == json!(510)),
        "expected deferred Runtime.callFunctionOn response; sent={:?}",
        ctx.sent
    );
    let result = take_response_by_id(&mut ctx, 510);
    assert_eq!(result["result"]["result"]["value"], json!(43));
}
