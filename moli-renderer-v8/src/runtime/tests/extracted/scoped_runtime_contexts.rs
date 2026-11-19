use super::*;

#[tokio::test(flavor = "multi_thread")]
async fn per_page_isolate_policy_rejects_cross_page_runtime_object_ids() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let first_url = url::Url::parse("https://example.test/shared-runtime-object-a").unwrap();
    let second_url = url::Url::parse("https://example.test/shared-runtime-object-b").unwrap();

    let (mut first_page, _, _, _creation_artifacts, first_download) = runtime
        .create_html_page_from_response(
            first_url.clone(),
            first_url,
            None,
            false,
            0,
            200,
            vec![("content-type".to_owned(), b"text/html".to_vec())],
            &loader,
            crate::RendererWebStorageHandles::ephemeral(),
            "<!doctype html><body>first runtime object owner</body>".to_owned(),
            crate::RendererDocumentOptions {
                ..Default::default()
            },
        )
        .await
        .expect("first shared-isolate runtime-object page should load");
    assert!(first_download.is_none());

    let (mut second_page, _, _, _creation_artifacts, second_download) = runtime
        .create_html_page_from_response(
            second_url.clone(),
            second_url,
            None,
            false,
            0,
            200,
            vec![("content-type".to_owned(), b"text/html".to_vec())],
            &loader,
            crate::RendererWebStorageHandles::ephemeral(),
            "<!doctype html><body>second runtime object owner</body>".to_owned(),
            crate::RendererDocumentOptions {
                ..Default::default()
            },
        )
        .await
        .expect("second shared-isolate runtime-object page should load");
    assert!(second_download.is_none());

    let first_testing = RendererPageTestingHandle::new_for_testing(&first_page);
    let second_testing = RendererPageTestingHandle::new_for_testing(&second_page);
    assert!(first_testing.shares_local_host(&second_testing));
    assert_eq!(
        second_testing
            .host_unique_document_isolate_count_async()
            .await
            .expect("two shared attached unique document isolate count"),
        2
    );

    let first_object_id = runtime_protocol_object_id(
        &first_page,
        serde_json::json!({
            "id": 41,
            "method": "Runtime.evaluate",
            "params": {
                "expression": "({ marker: 'first-page-object' })"
            }
        }),
        41,
    )
    .await
    .expect("first page Runtime.evaluate should return an objectId");

    let first_call = dispatch_runtime_protocol_for_test(
        &first_page,
        serde_json::json!({
            "id": 42,
            "method": "Runtime.callFunctionOn",
            "params": {
                "objectId": first_object_id,
                "functionDeclaration": "function() { return this.marker; }",
                "returnByValue": true
            }
        }),
    )
    .await
    .expect("first page Runtime.callFunctionOn should dispatch");
    let first_call_response =
        runtime_protocol_response_by_id(&first_call, 42).expect("first page call response");
    assert_eq!(
        first_call_response["result"]["result"]["value"],
        serde_json::json!("first-page-object")
    );

    let first_promise_object_id = runtime_protocol_object_id(
        &first_page,
        serde_json::json!({
            "id": 45,
            "method": "Runtime.evaluate",
            "params": {
                "expression": "Promise.resolve('first-page-promise')"
            }
        }),
        45,
    )
    .await
    .expect("first page Runtime.evaluate should return a promise objectId");

    let first_await = dispatch_runtime_protocol_for_test(
        &first_page,
        serde_json::json!({
            "id": 46,
            "method": "Runtime.awaitPromise",
            "params": {
                "promiseObjectId": first_promise_object_id,
                "returnByValue": true
            }
        }),
    )
    .await
    .expect("first page Runtime.awaitPromise should dispatch");
    let first_await_response = runtime_protocol_response_by_id(&first_await, 46)
        .expect("first page awaitPromise response");
    assert_eq!(
        first_await_response["result"]["result"]["value"],
        serde_json::json!("first-page-promise")
    );

    let second_call = dispatch_runtime_protocol_for_test(
        &second_page,
        serde_json::json!({
            "id": 43,
            "method": "Runtime.callFunctionOn",
            "params": {
                "objectId": first_object_id,
                "functionDeclaration": "function() { return this.marker; }",
                "returnByValue": true
            }
        }),
    )
    .await
    .expect("second page Runtime.callFunctionOn should dispatch");
    let second_call_response =
        runtime_protocol_response_by_id(&second_call, 43).expect("second page call response");
    assert!(
        second_call_response.get("error").is_some()
            || second_call_response["result"]["exceptionDetails"].is_object(),
        "page B must reject or fail closed for page A's Runtime objectId: {second_call_response:?}"
    );

    let second_await = dispatch_runtime_protocol_for_test(
        &second_page,
        serde_json::json!({
            "id": 47,
            "method": "Runtime.awaitPromise",
            "params": {
                "promiseObjectId": first_promise_object_id,
                "returnByValue": true
            }
        }),
    )
    .await
    .expect("second page Runtime.awaitPromise should dispatch");
    let second_await_response = runtime_protocol_response_by_id(&second_await, 47)
        .expect("second page awaitPromise response");
    assert!(
        second_await_response.get("error").is_some()
            || second_await_response["result"]["exceptionDetails"].is_object(),
        "page B must reject or fail closed for page A's Runtime promiseObjectId: {second_await_response:?}"
    );

    let (second_marker, _) = second_page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"globalThis.marker ?? "missing""#.to_owned(),
            await_promise: false,
        })
        .await
        .expect("second page marker check should evaluate");
    assert_eq!(
        renderer_json_value(second_marker),
        Some(serde_json::json!("missing")),
        "cross-page Runtime.callFunctionOn must not execute with page A's receiver in page B"
    );

    first_page
        .close_async()
        .await
        .expect("first runtime-object page should close");

    let closed_target_call = dispatch_runtime_protocol_for_test(
        &second_page,
        serde_json::json!({
            "id": 44,
            "method": "Runtime.callFunctionOn",
            "params": {
                "objectId": first_object_id,
                "functionDeclaration": "function() { globalThis.__closedTargetObjectMutatedPeer = true; return this.marker; }",
                "returnByValue": true
            }
        }),
    )
    .await
    .expect("closed target Runtime.callFunctionOn should dispatch through peer page");
    let closed_target_call_response = runtime_protocol_response_by_id(&closed_target_call, 44)
        .expect("closed target call response");
    assert!(
        closed_target_call_response.get("error").is_some()
            || closed_target_call_response["result"]["exceptionDetails"].is_object(),
        "page B must reject or fail closed for page A's Runtime objectId after page A closes: {closed_target_call_response:?}"
    );

    let closed_target_await = dispatch_runtime_protocol_for_test(
        &second_page,
        serde_json::json!({
            "id": 48,
            "method": "Runtime.awaitPromise",
            "params": {
                "promiseObjectId": first_promise_object_id,
                "returnByValue": true
            }
        }),
    )
    .await
    .expect("closed target Runtime.awaitPromise should dispatch through peer page");
    let closed_target_await_response = runtime_protocol_response_by_id(&closed_target_await, 48)
        .expect("closed target awaitPromise response");
    assert!(
        closed_target_await_response.get("error").is_some()
            || closed_target_await_response["result"]["exceptionDetails"].is_object(),
        "page B must reject or fail closed for page A's Runtime promiseObjectId after page A closes: {closed_target_await_response:?}"
    );

    let (second_closed_target_marker, _) = second_page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"globalThis.__closedTargetObjectMutatedPeer ?? "not-mutated""#.to_owned(),
            await_promise: false,
        })
        .await
        .expect("second page closed-target mutation marker should evaluate");
    assert_eq!(
        renderer_json_value(second_closed_target_marker),
        Some(serde_json::json!("not-mutated")),
        "closed target Runtime.callFunctionOn must not execute page A's stale object in page B"
    );

    second_page
        .close_async()
        .await
        .expect("second runtime-object page should close");
}
#[tokio::test(flavor = "multi_thread")]
async fn per_page_isolate_policy_scopes_same_numeric_runtime_evaluate_context_id_to_page() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let first_url = url::Url::parse("https://example.test/shared-runtime-context-a").unwrap();
    let second_url = url::Url::parse("https://example.test/shared-runtime-context-b").unwrap();

    let (mut first_page, _, _, _creation_artifacts, first_download) = runtime
        .create_html_page_from_response(
            first_url.clone(),
            first_url,
            None,
            false,
            0,
            200,
            vec![("content-type".to_owned(), b"text/html".to_vec())],
            &loader,
            crate::RendererWebStorageHandles::ephemeral(),
            "<!doctype html><body>first runtime context owner</body>".to_owned(),
            crate::RendererDocumentOptions {
                ..Default::default()
            },
        )
        .await
        .expect("first shared-isolate runtime-context page should load");
    assert!(first_download.is_none());

    let (mut second_page, _, _, _creation_artifacts, second_download) = runtime
        .create_html_page_from_response(
            second_url.clone(),
            second_url,
            None,
            false,
            0,
            200,
            vec![("content-type".to_owned(), b"text/html".to_vec())],
            &loader,
            crate::RendererWebStorageHandles::ephemeral(),
            "<!doctype html><body>second runtime context owner</body>".to_owned(),
            crate::RendererDocumentOptions {
                ..Default::default()
            },
        )
        .await
        .expect("second shared-isolate runtime-context page should load");
    assert!(second_download.is_none());

    let first_testing = RendererPageTestingHandle::new_for_testing(&first_page);
    let second_testing = RendererPageTestingHandle::new_for_testing(&second_page);
    assert!(first_testing.shares_local_host(&second_testing));
    assert_eq!(
        second_testing
            .host_unique_document_isolate_count_async()
            .await
            .expect("two shared attached unique document isolate count"),
        2
    );

    let first_world_context_id =
        create_isolated_world_for_test(&first_page, "shared-runtime-context-world")
            .await
            .expect("first runtime-context isolated world should be created");
    let second_world_context_id =
        create_isolated_world_for_test(&second_page, "shared-runtime-context-world")
            .await
            .expect("second runtime-context isolated world should be created");
    assert_eq!(
        first_world_context_id, second_world_context_id,
        "fresh isolates should be allowed to reuse target-scoped executionContextId values"
    );

    let first_evaluate = dispatch_runtime_protocol_with_context_resolution_for_test(
        &first_page,
        "evaluate",
        serde_json::json!({
            "id": 61,
            "method": "Runtime.evaluate",
            "params": {
                "contextId": first_world_context_id,
                "expression": "globalThis.__runtimeContextOwner = 'first-page'; globalThis.__runtimeContextOwner",
                "returnByValue": true
            }
        }),
    )
    .await
    .expect("page A Runtime.evaluate should dispatch with its own context id");
    let first_evaluate_response =
        runtime_protocol_response_by_id(&first_evaluate, 61).expect("page A evaluate response");
    assert_eq!(
        first_evaluate_response["result"]["result"]["value"],
        serde_json::json!("first-page")
    );

    let second_same_numeric_evaluate = dispatch_runtime_protocol_with_context_resolution_for_test(
        &second_page,
        "evaluate",
        serde_json::json!({
            "id": 62,
            "method": "Runtime.evaluate",
            "params": {
                "contextId": first_world_context_id,
                "expression": "globalThis.__runtimeContextOwner = 'cross-page'; globalThis.__runtimeContextOwner",
                "returnByValue": true
            }
        }),
    )
    .await
    .expect("page B Runtime.evaluate should dispatch with peer context id");
    let second_same_numeric_evaluate_response =
        runtime_protocol_response_by_id(&second_same_numeric_evaluate, 62)
            .expect("page B target-local evaluate response");
    assert_eq!(
        second_same_numeric_evaluate_response["result"]["result"]["value"],
        serde_json::json!("cross-page"),
        "the reused numeric id must resolve to page B's own isolated world"
    );

    let (first_context_marker_after_peer_attempt, _) = first_page
        .run_async_command(RendererPageCommand::EvaluateExpressionInExecutionContext {
            execution_context_id: first_world_context_id,
            expression: r#"globalThis.__runtimeContextOwner"#.to_owned(),
            await_promise: false,
        })
        .await
        .expect("first isolated world marker should evaluate after peer attempt");
    assert_eq!(
        renderer_json_value(first_context_marker_after_peer_attempt),
        Some(serde_json::json!("first-page")),
        "page B's target-local Runtime.evaluate must not mutate page A"
    );

    let (second_default_marker_after_peer_attempt, _) = second_page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"globalThis.__runtimeContextOwner ?? "missing""#.to_owned(),
            await_promise: false,
        })
        .await
        .expect("second default marker should evaluate after target-local attempt");
    assert_eq!(
        renderer_json_value(second_default_marker_after_peer_attempt),
        Some(serde_json::json!("missing")),
        "target-local isolated-world context id must not fall back to page B's default world"
    );

    let (second_world_marker_after_peer_attempt, _) = second_page
        .run_async_command(RendererPageCommand::EvaluateExpressionInExecutionContext {
            execution_context_id: second_world_context_id,
            expression: r#"globalThis.__runtimeContextOwner ?? "missing""#.to_owned(),
            await_promise: false,
        })
        .await
        .expect("second isolated world marker should evaluate after target-local attempt");
    assert_eq!(
        renderer_json_value(second_world_marker_after_peer_attempt),
        Some(serde_json::json!("cross-page")),
        "the reused numeric id must mutate only page B's isolated world"
    );

    first_page
        .close_async()
        .await
        .expect("first runtime-context page should close");
    second_page
        .close_async()
        .await
        .expect("second runtime-context page should close");
}
#[tokio::test(flavor = "multi_thread")]
async fn per_page_isolate_policy_scopes_same_numeric_runtime_evaluate_default_context_id_to_page() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let first_url =
        url::Url::parse("https://example.test/shared-runtime-default-context-a").unwrap();
    let second_url =
        url::Url::parse("https://example.test/shared-runtime-default-context-b").unwrap();

    let (mut first_page, _, _, _creation_artifacts, first_download) = runtime
        .create_html_page_from_response(
            first_url.clone(),
            first_url,
            None,
            false,
            0,
            200,
            vec![("content-type".to_owned(), b"text/html".to_vec())],
            &loader,
            crate::RendererWebStorageHandles::ephemeral(),
            "<!doctype html><body>first default runtime context owner</body>".to_owned(),
            crate::RendererDocumentOptions {
                ..Default::default()
            },
        )
        .await
        .expect("first shared-isolate default runtime-context page should load");
    assert!(first_download.is_none());

    let (mut second_page, _, _, _creation_artifacts, second_download) = runtime
        .create_html_page_from_response(
            second_url.clone(),
            second_url,
            None,
            false,
            0,
            200,
            vec![("content-type".to_owned(), b"text/html".to_vec())],
            &loader,
            crate::RendererWebStorageHandles::ephemeral(),
            "<!doctype html><body>second default runtime context owner</body>".to_owned(),
            crate::RendererDocumentOptions {
                ..Default::default()
            },
        )
        .await
        .expect("second shared-isolate default runtime-context page should load");
    assert!(second_download.is_none());

    let first_testing = RendererPageTestingHandle::new_for_testing(&first_page);
    let second_testing = RendererPageTestingHandle::new_for_testing(&second_page);
    assert!(first_testing.shares_local_host(&second_testing));
    assert_eq!(
        second_testing
            .host_unique_document_isolate_count_async()
            .await
            .expect("two shared attached unique document isolate count"),
        2
    );

    let first_default_context_id = default_or_initial_execution_context_id_for_test(&first_page)
        .await
        .expect("first page should expose a default execution context id");
    let second_default_context_id = default_or_initial_execution_context_id_for_test(&second_page)
        .await
        .expect("second page should expose a default execution context id");
    assert_eq!(
        first_default_context_id, second_default_context_id,
        "fresh isolates should be allowed to reuse target-scoped default context ids"
    );

    let first_evaluate = dispatch_runtime_protocol_with_context_resolution_for_test(
        &first_page,
        "evaluate",
        serde_json::json!({
            "id": 63,
            "method": "Runtime.evaluate",
            "params": {
                "contextId": first_default_context_id,
                "expression": "globalThis.__runtimeDefaultContextOwner = 'first-page'; globalThis.__runtimeDefaultContextOwner",
                "returnByValue": true
            }
        }),
    )
    .await
    .expect("page A Runtime.evaluate should dispatch with its own default context id");
    let first_evaluate_response =
        runtime_protocol_response_by_id(&first_evaluate, 63).expect("page A evaluate response");
    assert_eq!(
        first_evaluate_response["result"]["result"]["value"],
        serde_json::json!("first-page")
    );

    let second_same_numeric_evaluate = dispatch_runtime_protocol_with_context_resolution_for_test(
        &second_page,
        "evaluate",
        serde_json::json!({
            "id": 64,
            "method": "Runtime.evaluate",
            "params": {
                "contextId": first_default_context_id,
                "expression": "globalThis.__runtimeDefaultContextOwner = 'cross-page'; globalThis.__runtimeDefaultContextOwner",
                "returnByValue": true
            }
        }),
    )
    .await
    .expect("page B Runtime.evaluate should dispatch with peer default context id");
    let second_same_numeric_evaluate_response =
        runtime_protocol_response_by_id(&second_same_numeric_evaluate, 64)
            .expect("page B target-local default evaluate response");
    assert_eq!(
        second_same_numeric_evaluate_response["result"]["result"]["value"],
        serde_json::json!("cross-page"),
        "the reused numeric id must resolve to page B's own default world"
    );

    let (first_marker_after_peer_attempt, _) = first_page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"globalThis.__runtimeDefaultContextOwner"#.to_owned(),
            await_promise: false,
        })
        .await
        .expect("first default marker should evaluate after peer attempt");
    assert_eq!(
        renderer_json_value(first_marker_after_peer_attempt),
        Some(serde_json::json!("first-page")),
        "page B's target-local Runtime.evaluate must not mutate page A"
    );

    let (second_marker_after_peer_attempt, _) = second_page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"globalThis.__runtimeDefaultContextOwner ?? "missing""#.to_owned(),
            await_promise: false,
        })
        .await
        .expect("second default marker should evaluate after target-local access");
    assert_eq!(
        renderer_json_value(second_marker_after_peer_attempt),
        Some(serde_json::json!("cross-page")),
        "the reused numeric id must mutate only page B's default world"
    );

    first_page
        .close_async()
        .await
        .expect("first default runtime-context page should close");
    second_page
        .close_async()
        .await
        .expect("second default runtime-context page should close");
}
#[tokio::test(flavor = "multi_thread")]
async fn per_page_isolate_policy_scopes_same_numeric_runtime_evaluate_child_context_id_to_page() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let first_url = url::Url::parse("https://example.test/shared-runtime-child-context-a").unwrap();
    let second_url =
        url::Url::parse("https://example.test/shared-runtime-child-context-b").unwrap();

    let (mut first_page, _, _, _creation_artifacts, first_download) = runtime
        .create_html_page_from_response(
first_url.clone(),
first_url,
None,
false,
0,
200,
vec![("content-type".to_owned(), b"text/html".to_vec())],
&loader,
crate::RendererWebStorageHandles::ephemeral(),
r#"<!doctype html><body><iframe srcdoc="<body>first runtime child</body>"></iframe></body>"#
                .to_owned(),
crate::RendererDocumentOptions { ..Default::default() },
)
        .await
        .expect("first shared-isolate child runtime-context page should load");
    assert!(first_download.is_none());

    let (mut second_page, _, _, _creation_artifacts, second_download) = runtime
        .create_html_page_from_response(
second_url.clone(),
second_url,
None,
false,
0,
200,
vec![("content-type".to_owned(), b"text/html".to_vec())],
&loader,
crate::RendererWebStorageHandles::ephemeral(),
r#"<!doctype html><body><iframe srcdoc="<body>second runtime child</body>"></iframe></body>"#
                .to_owned(),
crate::RendererDocumentOptions { ..Default::default() },
)
        .await
        .expect("second shared-isolate child runtime-context page should load");
    assert!(second_download.is_none());

    let first_testing = RendererPageTestingHandle::new_for_testing(&first_page);
    let second_testing = RendererPageTestingHandle::new_for_testing(&second_page);
    assert!(first_testing.shares_local_host(&second_testing));
    assert_eq!(
        second_testing
            .host_unique_document_isolate_count_async()
            .await
            .expect("two shared attached unique document isolate count"),
        2
    );

    let first_child_context_ids = child_default_context_ids_for_test(&first_page)
        .await
        .expect("first child context events should replay");
    let second_child_context_ids = child_default_context_ids_for_test(&second_page)
        .await
        .expect("second child context events should replay");
    assert_eq!(
        first_child_context_ids.len(),
        1,
        "first page should expose exactly one child default context"
    );
    assert_eq!(
        second_child_context_ids.len(),
        1,
        "second page should expose exactly one child default context"
    );
    let first_child_context_id = first_child_context_ids[0];
    let second_child_context_id = second_child_context_ids[0];
    assert_eq!(
        first_child_context_id, second_child_context_id,
        "fresh isolates should be allowed to reuse target-scoped child context ids"
    );

    let first_evaluate = dispatch_runtime_protocol_with_context_resolution_for_test(
        &first_page,
        "evaluate",
        serde_json::json!({
            "id": 65,
            "method": "Runtime.evaluate",
            "params": {
                "contextId": first_child_context_id,
                "expression": "globalThis.__runtimeChildContextOwner = 'first-child'; globalThis.__runtimeChildContextOwner",
                "returnByValue": true
            }
        }),
    )
    .await
    .expect("page A Runtime.evaluate should dispatch with its own child context id");
    let first_evaluate_response = runtime_protocol_response_by_id(&first_evaluate, 65)
        .expect("page A child evaluate response");
    assert_eq!(
        first_evaluate_response["result"]["result"]["value"],
        serde_json::json!("first-child")
    );

    let second_same_numeric_evaluate = dispatch_runtime_protocol_with_context_resolution_for_test(
        &second_page,
        "evaluate",
        serde_json::json!({
            "id": 66,
            "method": "Runtime.evaluate",
            "params": {
                "contextId": first_child_context_id,
                "expression": "globalThis.__runtimeChildContextOwner = 'cross-page'; globalThis.__runtimeChildContextOwner",
                "returnByValue": true
            }
        }),
    )
    .await
    .expect("page B Runtime.evaluate should dispatch with peer child context id");
    let second_same_numeric_evaluate_response =
        runtime_protocol_response_by_id(&second_same_numeric_evaluate, 66)
            .expect("page B target-local child evaluate response");
    assert_eq!(
        second_same_numeric_evaluate_response["result"]["result"]["value"],
        serde_json::json!("cross-page"),
        "the reused numeric id must resolve to page B's own child realm"
    );

    let (first_marker_after_peer_attempt, _) = first_page
        .run_async_command(RendererPageCommand::EvaluateExpressionInExecutionContext {
            execution_context_id: first_child_context_id,
            expression: r#"globalThis.__runtimeChildContextOwner"#.to_owned(),
            await_promise: false,
        })
        .await
        .expect("first child marker should evaluate after peer attempt");
    assert_eq!(
        renderer_json_value(first_marker_after_peer_attempt),
        Some(serde_json::json!("first-child")),
        "page B's target-local Runtime.evaluate must not mutate page A's child frame"
    );

    let (second_default_marker_after_peer_attempt, _) = second_page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"globalThis.__runtimeChildContextOwner ?? "missing""#.to_owned(),
            await_promise: false,
        })
        .await
        .expect("second default marker should evaluate after target-local child access");
    assert_eq!(
        renderer_json_value(second_default_marker_after_peer_attempt),
        Some(serde_json::json!("missing")),
        "target-local child context access must not fall back to page B's default world"
    );

    let (second_child_marker_after_peer_attempt, _) = second_page
        .run_async_command(RendererPageCommand::EvaluateExpressionInExecutionContext {
            execution_context_id: second_child_context_id,
            expression: r#"globalThis.__runtimeChildContextOwner ?? "missing""#.to_owned(),
            await_promise: false,
        })
        .await
        .expect("second child marker should evaluate after target-local access");
    assert_eq!(
        renderer_json_value(second_child_marker_after_peer_attempt),
        Some(serde_json::json!("cross-page")),
        "the reused numeric id must mutate only page B's child realm"
    );

    first_page
        .close_async()
        .await
        .expect("first child runtime-context page should close");
    second_page
        .close_async()
        .await
        .expect("second child runtime-context page should close");
}
#[tokio::test(flavor = "multi_thread")]
async fn per_page_isolate_policy_keeps_release_object_group_page_local() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let first_url = url::Url::parse("https://example.test/shared-runtime-release-a").unwrap();
    let second_url = url::Url::parse("https://example.test/shared-runtime-release-b").unwrap();

    let (mut first_page, _, _, _creation_artifacts, first_download) = runtime
        .create_html_page_from_response(
            first_url.clone(),
            first_url,
            None,
            false,
            0,
            200,
            vec![("content-type".to_owned(), b"text/html".to_vec())],
            &loader,
            crate::RendererWebStorageHandles::ephemeral(),
            "<!doctype html><body>first release group owner</body>".to_owned(),
            crate::RendererDocumentOptions {
                ..Default::default()
            },
        )
        .await
        .expect("first shared-isolate release-group page should load");
    assert!(first_download.is_none());

    let (mut second_page, _, _, _creation_artifacts, second_download) = runtime
        .create_html_page_from_response(
            second_url.clone(),
            second_url,
            None,
            false,
            0,
            200,
            vec![("content-type".to_owned(), b"text/html".to_vec())],
            &loader,
            crate::RendererWebStorageHandles::ephemeral(),
            "<!doctype html><body>second release group owner</body>".to_owned(),
            crate::RendererDocumentOptions {
                ..Default::default()
            },
        )
        .await
        .expect("second shared-isolate release-group page should load");
    assert!(second_download.is_none());

    let first_testing = RendererPageTestingHandle::new_for_testing(&first_page);
    let second_testing = RendererPageTestingHandle::new_for_testing(&second_page);
    assert!(first_testing.shares_local_host(&second_testing));
    assert_eq!(
        second_testing
            .host_unique_document_isolate_count_async()
            .await
            .expect("two shared attached unique document isolate count"),
        2
    );

    let first_object_id = runtime_protocol_object_id(
        &first_page,
        serde_json::json!({
            "id": 51,
            "method": "Runtime.evaluate",
            "params": {
                "expression": "({ marker: 'first-release-group-object' })",
                "objectGroup": "same-release-group"
            }
        }),
        51,
    )
    .await
    .expect("first page Runtime.evaluate should return grouped objectId");
    let second_object_id = runtime_protocol_object_id(
        &second_page,
        serde_json::json!({
            "id": 52,
            "method": "Runtime.evaluate",
            "params": {
                "expression": "({ marker: 'second-release-group-object' })",
                "objectGroup": "same-release-group"
            }
        }),
        52,
    )
    .await
    .expect("second page Runtime.evaluate should return grouped objectId");

    let release = dispatch_runtime_protocol_for_test(
        &first_page,
        serde_json::json!({
            "id": 53,
            "method": "Runtime.releaseObjectGroup",
            "params": { "objectGroup": "same-release-group" }
        }),
    )
    .await
    .expect("first page Runtime.releaseObjectGroup should dispatch");
    let release_response =
        runtime_protocol_response_by_id(&release, 53).expect("first page release response");
    assert_eq!(release_response["result"], serde_json::json!({}));

    let first_call_after_release = dispatch_runtime_protocol_for_test(
        &first_page,
        serde_json::json!({
            "id": 54,
            "method": "Runtime.callFunctionOn",
            "params": {
                "objectId": first_object_id,
                "functionDeclaration": "function() { return this.marker; }",
                "returnByValue": true
            }
        }),
    )
    .await
    .expect("first page call after release should dispatch");
    let first_call_after_release_response =
        runtime_protocol_response_by_id(&first_call_after_release, 54)
            .expect("first page call-after-release response");
    assert!(
        first_call_after_release_response.get("error").is_some()
            || first_call_after_release_response["result"]["exceptionDetails"].is_object(),
        "page A releaseObjectGroup should remove page A's grouped handle: {first_call_after_release_response:?}"
    );

    let second_call_after_peer_release = dispatch_runtime_protocol_for_test(
        &second_page,
        serde_json::json!({
            "id": 55,
            "method": "Runtime.callFunctionOn",
            "params": {
                "objectId": second_object_id,
                "functionDeclaration": "function() { return this.marker; }",
                "returnByValue": true
            }
        }),
    )
    .await
    .expect("second page call after peer release should dispatch");
    let second_call_after_peer_release_response =
        runtime_protocol_response_by_id(&second_call_after_peer_release, 55)
            .expect("second page call-after-peer-release response");
    assert_eq!(
        second_call_after_peer_release_response["result"]["result"]["value"],
        serde_json::json!("second-release-group-object"),
        "page A releaseObjectGroup must not clear page B's same-name group in the shared isolate"
    );

    first_page
        .close_async()
        .await
        .expect("first release-group page should close");
    second_page
        .close_async()
        .await
        .expect("second release-group page should close");
}
#[tokio::test(flavor = "multi_thread")]
async fn per_page_isolate_policy_scopes_runtime_bindings_to_page_worlds() {
    let runtime = JsRuntime::initialize();
    let (output_tx, mut output_rx) = renderer_external_activity_test_channel();
    runtime.set_renderer_output_transport_sender(output_tx);
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let first_url = url::Url::parse("https://example.test/shared-runtime-binding-a").unwrap();
    let second_url = url::Url::parse("https://example.test/shared-runtime-binding-b").unwrap();

    let (mut first_page, _, _, _creation_artifacts, first_download) = runtime
        .create_html_page_from_response(
            first_url.clone(),
            first_url,
            None,
            false,
            0,
            200,
            vec![("content-type".to_owned(), b"text/html".to_vec())],
            &loader,
            crate::RendererWebStorageHandles::ephemeral(),
            "<!doctype html><body>first runtime binding owner</body>".to_owned(),
            crate::RendererDocumentOptions {
                ..Default::default()
            },
        )
        .await
        .expect("first shared-isolate runtime-binding page should load");
    assert!(first_download.is_none());

    let (mut second_page, _, _, _creation_artifacts, second_download) = runtime
        .create_html_page_from_response(
            second_url.clone(),
            second_url,
            None,
            false,
            0,
            200,
            vec![("content-type".to_owned(), b"text/html".to_vec())],
            &loader,
            crate::RendererWebStorageHandles::ephemeral(),
            "<!doctype html><body>second runtime binding owner</body>".to_owned(),
            crate::RendererDocumentOptions {
                ..Default::default()
            },
        )
        .await
        .expect("second shared-isolate runtime-binding page should load");
    assert!(second_download.is_none());

    let first_testing = RendererPageTestingHandle::new_for_testing(&first_page);
    let second_testing = RendererPageTestingHandle::new_for_testing(&second_page);
    assert!(first_testing.shares_local_host(&second_testing));
    assert_eq!(
        second_testing
            .host_unique_document_isolate_count_async()
            .await
            .expect("two shared attached unique document isolate count"),
        2
    );
    output_rx.drain();

    let first_world_context_id =
        create_isolated_world_for_test(&first_page, "shared-binding-world")
            .await
            .expect("first runtime binding isolated world should be created");
    let second_world_context_id =
        create_isolated_world_for_test(&second_page, "shared-binding-world")
            .await
            .expect("second runtime binding isolated world should be created");
    add_runtime_binding_for_test(
        &first_page,
        "sharedBinding",
        Some("shared-binding-world"),
        None,
    )
    .await
    .expect("first page scoped runtime binding should install");

    let (first_binding_type, _) = first_page
        .run_async_command(RendererPageCommand::EvaluateExpressionInExecutionContext {
            execution_context_id: first_world_context_id,
            expression: r#"typeof sharedBinding"#.to_owned(),
            await_promise: false,
        })
        .await
        .expect("first isolated world binding type should evaluate");
    assert_eq!(
        renderer_json_value(first_binding_type),
        Some(serde_json::json!("function"))
    );

    let (second_binding_type, _) = second_page
        .run_async_command(RendererPageCommand::EvaluateExpressionInExecutionContext {
            execution_context_id: second_world_context_id,
            expression: r#"typeof sharedBinding"#.to_owned(),
            await_promise: false,
        })
        .await
        .expect("second isolated world binding type should evaluate");
    assert_eq!(
        renderer_json_value(second_binding_type),
        Some(serde_json::json!("undefined")),
        "Runtime.addBinding scoped to page A's isolated world name must not install on page B's same-name isolated world"
    );

    let (first_binding_call_result, _) = first_page
        .run_async_command(RendererPageCommand::EvaluateExpressionInExecutionContext {
            execution_context_id: first_world_context_id,
            expression: r#"sharedBinding("from-first-world"); "called""#.to_owned(),
            await_promise: false,
        })
        .await
        .expect("first isolated world binding call should evaluate");
    assert_eq!(
        renderer_json_value(first_binding_call_result),
        Some(serde_json::json!("called"))
    );
    let first_binding_calls = output_rx.drain_runtime_binding_calls_for_page(&first_page);
    assert_eq!(first_binding_calls.len(), 1);
    assert_eq!(first_binding_calls[0].name, "sharedBinding");
    assert_eq!(first_binding_calls[0].payload, "from-first-world");
    assert_eq!(
        first_binding_calls[0].execution_context_id, first_world_context_id,
        "binding call should map back to page A's compatibility context id"
    );

    let second_binding_calls = output_rx.drain_runtime_binding_calls_for_page(&second_page);
    assert!(
        second_binding_calls.is_empty(),
        "page B must not receive binding calls from page A's isolated world"
    );

    first_page
        .close_async()
        .await
        .expect("first runtime-binding page should close");
    second_page
        .close_async()
        .await
        .expect("second runtime-binding page should close");
}
#[tokio::test(flavor = "multi_thread")]
async fn per_page_isolate_policy_scopes_same_numeric_runtime_binding_context_id_to_page() {
    let runtime = JsRuntime::initialize();
    let (output_tx, mut output_rx) = renderer_external_activity_test_channel();
    runtime.set_renderer_output_transport_sender(output_tx);
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let first_url = url::Url::parse("https://example.test/shared-binding-context-a").unwrap();
    let second_url = url::Url::parse("https://example.test/shared-binding-context-b").unwrap();

    let (mut first_page, _, _, _creation_artifacts, first_download) = runtime
        .create_html_page_from_response(
            first_url.clone(),
            first_url,
            None,
            false,
            0,
            200,
            vec![("content-type".to_owned(), b"text/html".to_vec())],
            &loader,
            crate::RendererWebStorageHandles::ephemeral(),
            "<!doctype html><body>first binding context owner</body>".to_owned(),
            crate::RendererDocumentOptions {
                ..Default::default()
            },
        )
        .await
        .expect("first shared-isolate binding-context page should load");
    assert!(first_download.is_none());

    let (mut second_page, _, _, _creation_artifacts, second_download) = runtime
        .create_html_page_from_response(
            second_url.clone(),
            second_url,
            None,
            false,
            0,
            200,
            vec![("content-type".to_owned(), b"text/html".to_vec())],
            &loader,
            crate::RendererWebStorageHandles::ephemeral(),
            "<!doctype html><body>second binding context owner</body>".to_owned(),
            crate::RendererDocumentOptions {
                ..Default::default()
            },
        )
        .await
        .expect("second shared-isolate binding-context page should load");
    assert!(second_download.is_none());

    let first_testing = RendererPageTestingHandle::new_for_testing(&first_page);
    let second_testing = RendererPageTestingHandle::new_for_testing(&second_page);
    assert!(first_testing.shares_local_host(&second_testing));
    assert_eq!(
        second_testing
            .host_unique_document_isolate_count_async()
            .await
            .expect("two shared attached unique document isolate count"),
        2
    );
    output_rx.drain();

    let first_world_context_id =
        create_isolated_world_for_test(&first_page, "shared-binding-context-world")
            .await
            .expect("first binding-context isolated world should be created");
    let second_world_context_id =
        create_isolated_world_for_test(&second_page, "shared-binding-context-world")
            .await
            .expect("second binding-context isolated world should be created");
    assert_eq!(
        first_world_context_id, second_world_context_id,
        "fresh isolates should be allowed to reuse target-scoped binding context ids"
    );

    add_runtime_binding_for_test(
        &second_page,
        "sharedContextBinding",
        None,
        Some(first_world_context_id),
    )
    .await
    .expect("the reused numeric id should install a binding in page B's local realm");

    let (first_binding_type_after_peer_attempt, _) = first_page
        .run_async_command(RendererPageCommand::EvaluateExpressionInExecutionContext {
            execution_context_id: first_world_context_id,
            expression: r#"typeof sharedContextBinding"#.to_owned(),
            await_promise: false,
        })
        .await
        .expect("first isolated world binding type should evaluate after page B install");
    assert_eq!(
        renderer_json_value(first_binding_type_after_peer_attempt),
        Some(serde_json::json!("undefined")),
        "page B's target-scoped context id must not install into page A's realm"
    );

    let (second_binding_type_after_peer_attempt, _) = second_page
        .run_async_command(RendererPageCommand::EvaluateExpressionInExecutionContext {
            execution_context_id: second_world_context_id,
            expression: r#"typeof sharedContextBinding"#.to_owned(),
            await_promise: false,
        })
        .await
        .expect("second isolated world binding type should evaluate after local install");
    assert_eq!(
        renderer_json_value(second_binding_type_after_peer_attempt),
        Some(serde_json::json!("function")),
        "the reused numeric id must resolve to page B's own realm"
    );

    let (second_binding_call_result, _) = second_page
        .run_async_command(RendererPageCommand::EvaluateExpressionInExecutionContext {
            execution_context_id: second_world_context_id,
            expression: r#"sharedContextBinding("from-second-context-id"); "called""#.to_owned(),
            await_promise: false,
        })
        .await
        .expect("second isolated-world binding call should evaluate");
    assert_eq!(
        renderer_json_value(second_binding_call_result),
        Some(serde_json::json!("called"))
    );
    let second_binding_calls = output_rx.drain_runtime_binding_calls_for_page(&second_page);
    assert_eq!(second_binding_calls.len(), 1);
    assert_eq!(second_binding_calls[0].name, "sharedContextBinding");
    assert_eq!(second_binding_calls[0].payload, "from-second-context-id");
    assert_eq!(
        second_binding_calls[0].execution_context_id, second_world_context_id,
        "binding call should map back to page B's target-scoped context id"
    );

    add_runtime_binding_for_test(
        &first_page,
        "sharedContextBinding",
        None,
        Some(first_world_context_id),
    )
    .await
    .expect("page A should install binding into its own isolated world by context id");

    let (first_binding_type_after_owner_install, _) = first_page
        .run_async_command(RendererPageCommand::EvaluateExpressionInExecutionContext {
            execution_context_id: first_world_context_id,
            expression: r#"typeof sharedContextBinding"#.to_owned(),
            await_promise: false,
        })
        .await
        .expect("first isolated world binding type should evaluate after owner install");
    assert_eq!(
        renderer_json_value(first_binding_type_after_owner_install),
        Some(serde_json::json!("function"))
    );

    let (first_binding_call_result, _) = first_page
        .run_async_command(RendererPageCommand::EvaluateExpressionInExecutionContext {
            execution_context_id: first_world_context_id,
            expression: r#"sharedContextBinding("from-first-context-id"); "called""#.to_owned(),
            await_promise: false,
        })
        .await
        .expect("first isolated world binding call should evaluate");
    assert_eq!(
        renderer_json_value(first_binding_call_result),
        Some(serde_json::json!("called"))
    );

    let first_binding_calls = output_rx.drain_runtime_binding_calls_for_page(&first_page);
    assert_eq!(first_binding_calls.len(), 1);
    assert_eq!(first_binding_calls[0].name, "sharedContextBinding");
    assert_eq!(first_binding_calls[0].payload, "from-first-context-id");
    assert_eq!(
        first_binding_calls[0].execution_context_id, first_world_context_id,
        "binding call should map back to page A's compatibility context id"
    );

    let second_binding_calls_after_first_call =
        output_rx.drain_runtime_binding_calls_for_page(&second_page);
    assert!(
        second_binding_calls_after_first_call.is_empty(),
        "page B must not receive binding calls from page A's context-id binding"
    );

    first_page
        .close_async()
        .await
        .expect("first binding-context page should close");
    second_page
        .close_async()
        .await
        .expect("second binding-context page should close");
}
#[tokio::test(flavor = "multi_thread")]
async fn per_page_isolate_policy_keeps_remove_binding_page_local() {
    let runtime = JsRuntime::initialize();
    let (output_tx, mut output_rx) = renderer_external_activity_test_channel();
    runtime.set_renderer_output_transport_sender(output_tx);
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let first_url = url::Url::parse("https://example.test/shared-remove-binding-a").unwrap();
    let second_url = url::Url::parse("https://example.test/shared-remove-binding-b").unwrap();

    let (mut first_page, _, _, _creation_artifacts, first_download) = runtime
        .create_html_page_from_response(
            first_url.clone(),
            first_url,
            None,
            false,
            0,
            200,
            vec![("content-type".to_owned(), b"text/html".to_vec())],
            &loader,
            crate::RendererWebStorageHandles::ephemeral(),
            "<!doctype html><body>first remove-binding owner</body>".to_owned(),
            crate::RendererDocumentOptions {
                ..Default::default()
            },
        )
        .await
        .expect("first shared-isolate remove-binding page should load");
    assert!(first_download.is_none());

    let (mut second_page, _, _, _creation_artifacts, second_download) = runtime
        .create_html_page_from_response(
            second_url.clone(),
            second_url,
            None,
            false,
            0,
            200,
            vec![("content-type".to_owned(), b"text/html".to_vec())],
            &loader,
            crate::RendererWebStorageHandles::ephemeral(),
            "<!doctype html><body>second remove-binding owner</body>".to_owned(),
            crate::RendererDocumentOptions {
                ..Default::default()
            },
        )
        .await
        .expect("second shared-isolate remove-binding page should load");
    assert!(second_download.is_none());

    let first_testing = RendererPageTestingHandle::new_for_testing(&first_page);
    let second_testing = RendererPageTestingHandle::new_for_testing(&second_page);
    assert!(first_testing.shares_local_host(&second_testing));
    assert_eq!(
        second_testing
            .host_unique_document_isolate_count_async()
            .await
            .expect("two shared attached unique document isolate count"),
        2
    );
    output_rx.drain();

    let first_world_context_id =
        create_isolated_world_for_test(&first_page, "shared-remove-binding-world")
            .await
            .expect("first remove-binding isolated world should be created");
    let second_world_context_id =
        create_isolated_world_for_test(&second_page, "shared-remove-binding-world")
            .await
            .expect("second remove-binding isolated world should be created");
    add_runtime_binding_for_test(
        &first_page,
        "sharedRemoveBinding",
        Some("shared-remove-binding-world"),
        None,
    )
    .await
    .expect("first page scoped runtime binding should install");
    add_runtime_binding_for_test(
        &second_page,
        "sharedRemoveBinding",
        Some("shared-remove-binding-world"),
        None,
    )
    .await
    .expect("second page scoped runtime binding should install");

    remove_runtime_binding_for_test(&first_page, "sharedRemoveBinding")
        .await
        .expect("first page scoped runtime binding should be removed");

    let (first_binding_type_after_remove, _) = first_page
        .run_async_command(RendererPageCommand::EvaluateExpressionInExecutionContext {
            execution_context_id: first_world_context_id,
            expression: r#"typeof sharedRemoveBinding"#.to_owned(),
            await_promise: false,
        })
        .await
        .expect("first isolated world binding type should evaluate after remove");
    assert_eq!(
        renderer_json_value(first_binding_type_after_remove),
        Some(serde_json::json!("undefined"))
    );

    let (second_binding_type_after_peer_remove, _) = second_page
        .run_async_command(RendererPageCommand::EvaluateExpressionInExecutionContext {
            execution_context_id: second_world_context_id,
            expression: r#"typeof sharedRemoveBinding"#.to_owned(),
            await_promise: false,
        })
        .await
        .expect("second isolated world binding type should evaluate after peer remove");
    assert_eq!(
        renderer_json_value(second_binding_type_after_peer_remove),
        Some(serde_json::json!("function")),
        "Runtime.removeBinding on page A must not remove page B's same-name binding in a shared document isolate"
    );

    let (second_binding_call_result, _) = second_page
        .run_async_command(RendererPageCommand::EvaluateExpressionInExecutionContext {
            execution_context_id: second_world_context_id,
            expression: r#"sharedRemoveBinding("from-second-world"); "called""#.to_owned(),
            await_promise: false,
        })
        .await
        .expect("second isolated world binding call should evaluate after peer remove");
    assert_eq!(
        renderer_json_value(second_binding_call_result),
        Some(serde_json::json!("called"))
    );
    let second_binding_calls = output_rx.drain_runtime_binding_calls_for_page(&second_page);
    assert_eq!(second_binding_calls.len(), 1);
    assert_eq!(second_binding_calls[0].name, "sharedRemoveBinding");
    assert_eq!(second_binding_calls[0].payload, "from-second-world");
    assert_eq!(
        second_binding_calls[0].execution_context_id, second_world_context_id,
        "binding call should map back to page B's compatibility context id after page A removal"
    );

    let first_binding_calls = output_rx.drain_runtime_binding_calls_for_page(&first_page);
    assert!(
        first_binding_calls.is_empty(),
        "page A must not receive binding calls after removing its page-local binding"
    );

    first_page
        .close_async()
        .await
        .expect("first remove-binding page should close");
    second_page
        .close_async()
        .await
        .expect("second remove-binding page should close");
}
