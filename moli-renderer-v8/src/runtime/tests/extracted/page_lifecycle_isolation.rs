use super::*;

#[tokio::test(flavor = "multi_thread")]
async fn per_page_isolate_policy_uses_distinct_isolates_and_isolates_contexts() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let first_url = url::Url::parse("https://example.test/shared-isolate-a").unwrap();
    let second_url = url::Url::parse("https://example.test/shared-isolate-b").unwrap();

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
            "<!doctype html><body>first shared</body>".to_owned(),
            crate::RendererDocumentOptions {
                ..Default::default()
            },
        )
        .await
        .expect("first shared-isolate page should load");
    assert!(first_download.is_none());
    let first_testing = RendererPageTestingHandle::new_for_testing(&first_page);
    assert_eq!(
        first_testing
            .host_unique_document_isolate_count_async()
            .await
            .expect("first shared page unique document isolate count"),
        1
    );

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
            "<!doctype html><body>second shared</body>".to_owned(),
            crate::RendererDocumentOptions {
                ..Default::default()
            },
        )
        .await
        .expect("second shared-isolate page should load");
    assert!(second_download.is_none());
    let second_testing = RendererPageTestingHandle::new_for_testing(&second_page);
    assert!(first_testing.shares_local_host(&second_testing));
    assert_eq!(
        second_testing
            .host_unique_document_isolate_count_async()
            .await
            .expect("two per-page attached unique document isolate count"),
        2
    );

    let first_heap = runtime_heap_usage_for_test(&first_page).await;
    let second_heap = runtime_heap_usage_for_test(&second_page).await;
    let first_runtime = &first_heap["moli"]["runtime"];
    let second_runtime = &second_heap["moli"]["runtime"];
    assert_eq!(
        first_runtime["inspectorContextGroupScope"],
        serde_json::json!("local-root-agent"),
        "inspector context-group id should be local-root agent scoped: {first_heap:?}"
    );
    assert_eq!(
        first_runtime["inspectorSessionRegistryOwner"],
        serde_json::json!("renderer-devtools-agent"),
        "page inspector session registry should be owned by the local-root agent: {first_heap:?}"
    );
    assert_eq!(
        first_runtime["inspectorSessionRegistryLifetimeScope"],
        serde_json::json!("local-root-agent"),
        "page inspector session registry should be local-root agent scoped: {first_heap:?}"
    );
    assert_eq!(
        first_runtime["inspectorSessionCount"],
        serde_json::json!(1),
        "default page inspector session should be created at bootstrap: {first_heap:?}"
    );
    assert_eq!(
        first_runtime["inspectorDefaultContextRegistryScope"],
        serde_json::json!("page-vm-document-isolate"),
        "default context registry should be page-isolate scoped: {first_heap:?}"
    );
    assert_eq!(
        first_runtime["v8ForegroundTaskWakeScope"],
        serde_json::json!("page-vm-document-isolate"),
        "V8 foreground task wakes should be labelled as page-isolate scoped: {first_heap:?}"
    );
    assert_eq!(
        first_runtime["v8ForegroundTaskWakeContextGroupIdAvailable"],
        serde_json::json!(false),
        "V8 foreground task wakes should not claim a context-group id: {first_heap:?}"
    );
    assert_eq!(
        first_runtime["v8ForegroundTaskWakeInternalPolicy"],
        serde_json::json!("typed-page-source-and-owner-scheduler"),
        "page document isolate must expose the typed Page-source policy: {first_heap:?}"
    );
    assert_eq!(
        first_runtime["v8ForegroundTaskWakeExternalPolicy"],
        serde_json::json!("post-turn-runtime-output"),
        "external observation must follow the completed owner turn: {first_heap:?}"
    );
    let first_context_group_id = first_runtime["inspectorContextGroupId"]
        .as_i64()
        .expect("first heap usage should report inspector context group id");
    let second_context_group_id = second_runtime["inspectorContextGroupId"]
        .as_i64()
        .expect("second heap usage should report inspector context group id");
    assert!(first_context_group_id > 0);
    assert!(second_context_group_id > 0);
    assert_ne!(
        first_context_group_id, second_context_group_id,
        "distinct page document isolates must expose distinct page inspector context groups"
    );
    assert_eq!(
        first_runtime["inspectorDefaultContextRegistryCount"],
        serde_json::json!(1),
        "first isolate backend should retain only its page default context: {first_heap:?}"
    );
    assert_eq!(
        second_runtime["inspectorDefaultContextRegistryCount"],
        serde_json::json!(1),
        "second isolate backend should retain only its page default context: {second_heap:?}"
    );
    assert_eq!(
        first_runtime["inspectorContextRegistrationCount"],
        serde_json::json!(1),
        "first document should own one default Inspector context registration: {first_heap:?}"
    );
    assert_eq!(
        second_runtime["inspectorContextRegistrationCount"],
        serde_json::json!(1),
        "second document should own an independent default Inspector context registration: {second_heap:?}"
    );

    let (first_marker, _) = first_page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression:
                r#"globalThis.__lm_shared_isolate_marker = "first"; globalThis.__lm_shared_isolate_marker"#
                    .to_owned(),
            await_promise: false,
        })
        .await
        .expect("first page marker evaluate should run");
    assert_eq!(
        renderer_json_value(first_marker),
        Some(serde_json::json!("first"))
    );

    let (missing_marker, _) = second_page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"globalThis.__lm_shared_isolate_marker ?? "missing""#.to_owned(),
            await_promise: false,
        })
        .await
        .expect("second page marker read should run");
    assert_eq!(
        renderer_json_value(missing_marker),
        Some(serde_json::json!("missing"))
    );

    let (second_marker, _) = second_page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression:
                r#"globalThis.__lm_shared_isolate_marker = "second"; globalThis.__lm_shared_isolate_marker"#
                    .to_owned(),
            await_promise: false,
        })
        .await
        .expect("second page marker evaluate should run");
    assert_eq!(
        renderer_json_value(second_marker),
        Some(serde_json::json!("second"))
    );

    let (first_marker_after_second_write, _) = first_page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"globalThis.__lm_shared_isolate_marker"#.to_owned(),
            await_promise: false,
        })
        .await
        .expect("first page marker reread should run");
    assert_eq!(
        renderer_json_value(first_marker_after_second_write),
        Some(serde_json::json!("first"))
    );

    first_page
        .close_async()
        .await
        .expect("first shared page should close");
    assert_eq!(
        second_testing
            .host_unique_document_isolate_count_async()
            .await
            .expect("remaining shared unique document isolate count"),
        1
    );
    let second_heap_after_first_close = runtime_heap_usage_for_test(&second_page).await;
    let second_runtime_after_first_close = &second_heap_after_first_close["moli"]["runtime"];
    assert_eq!(
        second_runtime_after_first_close["inspectorContextGroupId"],
        serde_json::json!(second_context_group_id),
        "remaining page should keep its own inspector context group after peer close"
    );
    assert_eq!(
        second_runtime_after_first_close["inspectorDefaultContextRegistryCount"],
        serde_json::json!(1),
        "closing peer page must not affect the remaining isolate registry: {second_heap_after_first_close:?}"
    );
    assert_eq!(
        second_runtime_after_first_close["inspectorContextRegistrationCount"],
        serde_json::json!(1),
        "closing a peer document must not release the remaining document's registration"
    );

    let (observed, _) = second_page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"(async () => {
  try {
    await WebAssembly.compile(new Uint8Array([0, 97, 115, 109, 1, 0, 0, 0]));
    return "compiled";
  } catch (error) {
    return "error:" + error.name;
  }
})()"#
                .to_owned(),
            await_promise: true,
        })
        .await
        .expect("remaining Page isolate should complete async wasm compilation");
    assert_eq!(
        renderer_json_value(observed),
        Some(serde_json::json!("compiled")),
        "retiring a peer Page must not break the remaining Page's typed V8 route"
    );

    second_page
        .close_async()
        .await
        .expect("second shared page should close");
}
#[tokio::test(flavor = "multi_thread")]
async fn per_page_isolate_policy_routes_unhandled_rejections_to_originating_page() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let first_url = url::Url::parse("https://example.test/shared-rejection-a").unwrap();
    let second_url = url::Url::parse("https://example.test/shared-rejection-b").unwrap();

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
            "<!doctype html><body>first rejection listener</body>".to_owned(),
            crate::RendererDocumentOptions {
                ..Default::default()
            },
        )
        .await
        .expect("first shared-isolate page should load");
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
            "<!doctype html><body>second rejection listener</body>".to_owned(),
            crate::RendererDocumentOptions {
                ..Default::default()
            },
        )
        .await
        .expect("second shared-isolate page should load");
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

    for page in [&first_page, &second_page] {
        let (installed, _) = page
            .run_async_command(RendererPageCommand::EvaluateExpression {
                expression: r#"(() => {
  globalThis.__lm_shared_isolate_unhandled_rejections = [];
  globalThis.__lm_shared_isolate_rejectionhandled = [];
  globalThis.__lm_shared_isolate_unhandled_done = new Promise(resolve => {
    globalThis.__lm_shared_isolate_resolve_unhandled = resolve;
  });
  globalThis.__lm_shared_isolate_handled_done = new Promise(resolve => {
    globalThis.__lm_shared_isolate_resolve_handled = resolve;
  });
  addEventListener("unhandledrejection", event => {
    event.preventDefault();
    globalThis.__lm_shared_isolate_unhandled_rejections.push(String(event.reason));
    globalThis.__lm_shared_isolate_resolve_unhandled();
  });
  addEventListener("rejectionhandled", event => {
    globalThis.__lm_shared_isolate_rejectionhandled.push(
      event.promise === globalThis.__lm_shared_isolate_late_rejection
        ? "same-promise"
        : "wrong-promise"
    );
    globalThis.__lm_shared_isolate_resolve_handled();
  });
  return "installed";
})()"#
                    .to_owned(),
                await_promise: false,
            })
            .await
            .expect("unhandledrejection listener install should run");
        assert_eq!(
            renderer_json_value(installed),
            Some(serde_json::json!("installed"))
        );
    }

    let (scheduled, _) = second_page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"(() => {
  globalThis.__lm_shared_isolate_late_rejection = Promise.reject("second-page-only");
  return "scheduled";
})()"#
                .to_owned(),
            await_promise: false,
        })
        .await
        .expect("second page rejection should schedule");
    assert_eq!(
        renderer_json_value(scheduled),
        Some(serde_json::json!("scheduled"))
    );

    // Rejection notifications are queued tasks. A later Evaluate command is
    // not a barrier for their delivery; await the originating page's event.
    let (second_rejections, _) = tokio::time::timeout(
        Duration::from_secs(2),
        second_page.run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"__lm_shared_isolate_unhandled_done.then(() => JSON.stringify(__lm_shared_isolate_unhandled_rejections))"#
                .to_owned(),
            await_promise: true,
        }),
    )
        .await
        .expect("second page must receive its unhandled rejection task")
        .expect("second page rejection list should evaluate");
    assert_eq!(
        renderer_json_value(second_rejections),
        Some(serde_json::json!("[\"second-page-only\"]")),
        "second page should receive its own unhandled rejection"
    );

    let (first_rejections, _) = first_page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"JSON.stringify(globalThis.__lm_shared_isolate_unhandled_rejections)"#
                .to_owned(),
            await_promise: false,
        })
        .await
        .expect("first page rejection list should evaluate");
    assert_eq!(
        renderer_json_value(first_rejections),
        Some(serde_json::json!("[]")),
        "first page must not receive the second page's unhandled rejection"
    );

    let (handler_added, _) = second_page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"(() => {
  globalThis.__lm_shared_isolate_late_rejection.catch(() => "handled");
  return "handler-added";
})()"#
                .to_owned(),
            await_promise: false,
        })
        .await
        .expect("second page late rejection handler should run");
    assert_eq!(
        renderer_json_value(handler_added),
        Some(serde_json::json!("handler-added"))
    );

    let (second_rejectionhandled, _) = tokio::time::timeout(
        Duration::from_secs(2),
        second_page.run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"__lm_shared_isolate_handled_done.then(() => JSON.stringify(__lm_shared_isolate_rejectionhandled))"#
                .to_owned(),
            await_promise: true,
        }),
    )
        .await
        .expect("second page must receive its rejectionhandled task")
        .expect("second page rejectionhandled list should evaluate");
    assert_eq!(
        renderer_json_value(second_rejectionhandled),
        Some(serde_json::json!("[\"same-promise\"]")),
        "second page should receive rejectionhandled for its own promise"
    );

    let (first_rejectionhandled, _) = first_page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"JSON.stringify(globalThis.__lm_shared_isolate_rejectionhandled)"#
                .to_owned(),
            await_promise: false,
        })
        .await
        .expect("first page rejectionhandled list should evaluate");
    assert_eq!(
        renderer_json_value(first_rejectionhandled),
        Some(serde_json::json!("[]")),
        "first page must not receive rejectionhandled for the second page's promise"
    );

    first_page
        .close_async()
        .await
        .expect("first shared page should close");
    second_page
        .close_async()
        .await
        .expect("second shared page should close");
}
#[tokio::test(flavor = "multi_thread")]
async fn per_page_isolate_policy_keeps_window_open_routes_page_owned() {
    let runtime = JsRuntime::initialize();
    let (output_tx, mut output_rx) = renderer_external_activity_test_channel();
    runtime.set_renderer_output_transport_sender(output_tx);
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let first_url = url::Url::parse("https://example.test/shared-window-open-a").unwrap();
    let second_url = url::Url::parse("https://example.test/shared-window-open-b").unwrap();

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
            "<!doctype html><body>first window opener</body>".to_owned(),
            crate::RendererDocumentOptions {
                ..Default::default()
            },
        )
        .await
        .expect("first shared-isolate window-open page should load");
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
            "<!doctype html><body>second window opener</body>".to_owned(),
            crate::RendererDocumentOptions {
                ..Default::default()
            },
        )
        .await
        .expect("second shared-isolate window-open page should load");
    assert!(second_download.is_none());

    let first_testing = RendererPageTestingHandle::new_for_testing(&first_page);
    let second_testing = RendererPageTestingHandle::new_for_testing(&second_page);
    assert!(first_testing.shares_local_host(&second_testing));
    assert_eq!(
        second_testing
            .host_unique_document_isolate_count_async()
            .await
            .expect("shared window-open unique document isolate count"),
        2
    );
    output_rx.drain();

    let (popup_result, _) = second_page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression:
                r#"window.open("https://example.test/shared-popup-from-second", "_blank") !== null"#
                    .to_owned(),
            await_promise: false,
        })
        .await
        .expect("second page window.open(_blank) should run");
    assert_eq!(
        renderer_json_value(popup_result),
        Some(serde_json::json!(true))
    );

    let popup_publications = output_rx.drain();
    assert!(
        popup_activations_for_page(&popup_publications, &first_page).is_empty(),
        "page A must not receive page B's popup activation"
    );
    let second_popups = popup_activations_for_page(&popup_publications, &second_page);
    assert_eq!(second_popups.len(), 1);
    assert_eq!(second_popups[0].target_name(), "_blank");
    assert_eq!(
        second_popups[0].url(),
        "https://example.test/shared-popup-from-second"
    );
    assert!(matches!(
        second_popups[0].source(),
        crate::RendererPopupActivationSource::Window {
            window: crate::RendererWindowDocumentSource::RootFrame,
            exposes_opener: true,
            ..
        }
    ));

    let (self_result, _) = first_page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression:
                r#"window.open("data:text/html,<main>first self target</main>", "_self") !== null"#
                    .to_owned(),
            await_promise: false,
        })
        .await
        .expect("first page window.open(_self) should run");
    assert_eq!(
        renderer_json_value(self_result),
        Some(serde_json::json!(true))
    );

    let self_target_publications = output_rx.drain();
    assert!(
        popup_activations_for_page(&self_target_publications, &first_page).is_empty(),
        "_self must not create a popup activation on its owner page"
    );
    assert!(
        popup_activations_for_page(&self_target_publications, &second_page).is_empty(),
        "_self on page A must not create a popup activation on page B"
    );
    assert_eq!(
        has_pending_location_navigation_for_test(&first_page).await,
        Some(true),
        "_self should queue cross-document navigation only on page A"
    );
    assert_eq!(
        has_pending_location_navigation_for_test(&second_page).await,
        Some(false),
        "page B must not inherit page A's _self navigation"
    );

    first_page
        .close_async()
        .await
        .expect("first window-open page should close");
    second_page
        .close_async()
        .await
        .expect("second window-open page should close");
}
#[tokio::test(flavor = "multi_thread")]
async fn per_page_isolate_policy_keeps_domparser_detached_docs_cleanup_neutral() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let first_url = url::Url::parse("https://example.test/shared-domparser-a").unwrap();
    let second_url = url::Url::parse("https://example.test/shared-domparser-b").unwrap();

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
            "<!doctype html><body>first detached owner</body>".to_owned(),
            crate::RendererDocumentOptions {
                ..Default::default()
            },
        )
        .await
        .expect("first shared-isolate DOMParser page should load");
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
            "<!doctype html><body>second detached peer</body>".to_owned(),
            crate::RendererDocumentOptions {
                ..Default::default()
            },
        )
        .await
        .expect("second shared-isolate DOMParser page should load");
    assert!(second_download.is_none());

    let first_testing = RendererPageTestingHandle::new_for_testing(&first_page);
    let second_testing = RendererPageTestingHandle::new_for_testing(&second_page);
    assert!(first_testing.shares_local_host(&second_testing));
    assert_eq!(
        second_testing
            .host_unique_document_isolate_count_async()
            .await
            .expect("two shared attached DOMParser unique document isolate count"),
        2
    );

    let (detached_status, _) = first_page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"(() => {
  globalThis.__lm_shared_isolate_detached_doc =
    new DOMParser().parseFromString(
      "<!doctype html><body><p id='marker'>detached-owner</p></body>",
      "text/html"
    );
  return globalThis.__lm_shared_isolate_detached_doc
    .getElementById("marker")
    .textContent;
})()"#
                .to_owned(),
            await_promise: false,
        })
        .await
        .expect("first page detached DOMParser document should evaluate");
    assert_eq!(
        renderer_json_value(detached_status),
        Some(serde_json::json!("detached-owner"))
    );

    let (second_blob_url, _) = second_page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"URL.createObjectURL(new Blob(["second-peer"], { type: "text/plain" }))"#
                .to_owned(),
            await_promise: false,
        })
        .await
        .expect("second page blob URL should be created");
    let second_blob_url = renderer_json_value(second_blob_url)
        .and_then(|value| value.as_str().map(str::to_owned))
        .expect("second page should return a blob URL");
    assert!(second_blob_url.starts_with("blob:https://example.test/"));

    first_page
        .close_async()
        .await
        .expect("first shared DOMParser page should close");

    let (second_blob_text, _) = second_page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: format!("fetch({second_blob_url:?}).then(response => response.text())"),
            await_promise: true,
        })
        .await
        .expect("second page blob URL should remain fetchable after first page closes");
    assert_eq!(
        renderer_json_value(second_blob_text),
        Some(serde_json::json!("second-peer")),
        "closing a page that holds DOMParser detached documents must not clean peer page resources"
    );

    second_page
        .close_async()
        .await
        .expect("second shared DOMParser page should close");
}
#[tokio::test(flavor = "multi_thread")]
async fn per_page_isolate_policy_keeps_isolated_worlds_page_owned() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let first_url = url::Url::parse("https://example.test/shared-world-a").unwrap();
    let second_url = url::Url::parse("https://example.test/shared-world-b").unwrap();

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
            "<!doctype html><body>first isolated world</body>".to_owned(),
            crate::RendererDocumentOptions {
                ..Default::default()
            },
        )
        .await
        .expect("first shared-isolate page should load");
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
            "<!doctype html><body>second isolated world</body>".to_owned(),
            crate::RendererDocumentOptions {
                ..Default::default()
            },
        )
        .await
        .expect("second shared-isolate page should load");
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

    let first_world_context_id = create_isolated_world_for_test(&first_page, "shared-utility")
        .await
        .expect("first page isolated world should be created");
    let second_world_context_id = create_isolated_world_for_test(&second_page, "shared-utility")
        .await
        .expect("second page isolated world should be created");
    assert_eq!(
        first_world_context_id, second_world_context_id,
        "fresh per-page isolates should expose independent, target-scoped context-id namespaces"
    );
    let first_world_heap = runtime_heap_usage_for_test(&first_page).await;
    let second_world_heap = runtime_heap_usage_for_test(&second_page).await;
    assert_eq!(
        first_world_heap["moli"]["runtime"]["inspectorContextRegistrationCount"],
        serde_json::json!(2),
        "first document should own default and isolated Inspector registrations"
    );
    assert_eq!(
        second_world_heap["moli"]["runtime"]["inspectorContextRegistrationCount"],
        serde_json::json!(2),
        "second document should own an independent default/isolated registration pair"
    );

    let (first_world_marker, _) = first_page
        .run_async_command(RendererPageCommand::EvaluateExpressionInExecutionContext {
            execution_context_id: first_world_context_id,
            expression: r#"document.__lm_shared_isolate_world_document_marker = "isolated-document"; globalThis.__lm_shared_isolate_world_marker = document.body.textContent.trim(); globalThis.__lm_shared_isolate_world_marker"#.to_owned(),
            await_promise: false,
        })
        .await
        .expect("first isolated world marker should evaluate");
    assert_eq!(
        renderer_json_value(first_world_marker),
        Some(serde_json::json!("first isolated world"))
    );

    let (second_world_missing, _) = second_page
        .run_async_command(RendererPageCommand::EvaluateExpressionInExecutionContext {
            execution_context_id: second_world_context_id,
            expression: r#"globalThis.__lm_shared_isolate_world_marker ?? "missing""#.to_owned(),
            await_promise: false,
        })
        .await
        .expect("second isolated world marker read should evaluate");
    assert_eq!(
        renderer_json_value(second_world_missing),
        Some(serde_json::json!("missing")),
        "same-name isolated worlds on different pages must not share global state"
    );

    let (second_world_marker_through_same_numeric_id, _) = second_page
        .run_async_command(RendererPageCommand::EvaluateExpressionInExecutionContext {
            execution_context_id: first_world_context_id,
            expression:
                r#"globalThis.__lm_shared_isolate_world_marker = "cross-page"; "cross-page""#
                    .to_owned(),
            await_promise: false,
        })
        .await
        .expect("the reused numeric id should resolve within page B's target");
    assert_eq!(
        renderer_json_value(second_world_marker_through_same_numeric_id),
        Some(serde_json::json!("cross-page")),
        "a target-scoped numeric id must resolve to page B's own isolated world"
    );

    let (first_world_marker_after_second_page_evaluate, _) = first_page
        .run_async_command(RendererPageCommand::EvaluateExpressionInExecutionContext {
            execution_context_id: first_world_context_id,
            expression: r#"globalThis.__lm_shared_isolate_world_marker"#.to_owned(),
            await_promise: false,
        })
        .await
        .expect("first isolated world marker should remain readable");
    assert_eq!(
        renderer_json_value(first_world_marker_after_second_page_evaluate),
        Some(serde_json::json!("first isolated world")),
        "the same numeric id on page B must never route into page A's isolate"
    );

    let (second_default_missing_after_cross_page, _) = second_page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"globalThis.__lm_shared_isolate_world_marker ?? "missing""#.to_owned(),
            await_promise: false,
        })
        .await
        .expect("second default world marker read should evaluate after target-local world access");
    assert_eq!(
        renderer_json_value(second_default_missing_after_cross_page),
        Some(serde_json::json!("missing")),
        "target-local isolated-world access must not fall back to page B's default world"
    );

    let (first_default_missing, _) = first_page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"globalThis.__lm_shared_isolate_world_marker ?? "missing""#.to_owned(),
            await_promise: false,
        })
        .await
        .expect("first default world marker read should evaluate");
    assert_eq!(
        renderer_json_value(first_default_missing),
        Some(serde_json::json!("missing")),
        "isolated world globals must not leak into the page default world"
    );

    let (first_default_document_missing, _) = first_page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"document.__lm_shared_isolate_world_document_marker ?? "missing""#
                .to_owned(),
            await_promise: false,
        })
        .await
        .expect("first default-world document wrapper should evaluate");
    assert_eq!(
        renderer_json_value(first_default_document_missing),
        Some(serde_json::json!("missing")),
        "main isolated and default worlds must keep distinct wrappers for the same Document"
    );

    first_page
        .close_async()
        .await
        .expect("first shared page should close");

    let (second_world_marker, _) = second_page
        .run_async_command(RendererPageCommand::EvaluateExpressionInExecutionContext {
            execution_context_id: second_world_context_id,
            expression: r#"globalThis.__lm_shared_isolate_world_marker = "second-world"; globalThis.__lm_shared_isolate_world_marker"#.to_owned(),
            await_promise: false,
        })
        .await
        .expect("second isolated world should remain usable after first page closes");
    assert_eq!(
        renderer_json_value(second_world_marker),
        Some(serde_json::json!("second-world")),
        "closing page A must not tear down page B's isolated world in a shared document isolate"
    );
    let second_world_heap_after_first_close = runtime_heap_usage_for_test(&second_page).await;
    assert_eq!(
        second_world_heap_after_first_close["moli"]["runtime"]["inspectorContextRegistrationCount"],
        serde_json::json!(2),
        "closing page A must not release page B's document-owned registrations"
    );

    second_page
        .close_async()
        .await
        .expect("second shared page should close");
}
#[tokio::test(flavor = "multi_thread")]
async fn per_page_isolate_policy_keeps_child_default_contexts_page_owned() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let first_url = url::Url::parse("https://example.test/shared-child-frame-a").unwrap();
    let second_url = url::Url::parse("https://example.test/shared-child-frame-b").unwrap();

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
            r#"<!doctype html><body><iframe srcdoc="<body>first child</body>"></iframe></body>"#
                .to_owned(),
            crate::RendererDocumentOptions {
                ..Default::default()
            },
        )
        .await
        .expect("first shared child-frame page should load");
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
            r#"<!doctype html><body><iframe srcdoc="<body>second child</body>"></iframe></body>"#
                .to_owned(),
            crate::RendererDocumentOptions {
                ..Default::default()
            },
        )
        .await
        .expect("second shared child-frame page should load");
    assert!(second_download.is_none());

    let first_testing = RendererPageTestingHandle::new_for_testing(&first_page);
    let second_testing = RendererPageTestingHandle::new_for_testing(&second_page);
    assert!(first_testing.shares_local_host(&second_testing));
    assert_eq!(
        second_testing
            .host_unique_document_isolate_count_async()
            .await
            .expect("two shared child-frame unique document isolate count"),
        2
    );

    let first_child_context_ids = child_default_context_ids_for_test(&first_page)
        .await
        .expect("first child default context events should replay");
    let second_child_context_ids = child_default_context_ids_for_test(&second_page)
        .await
        .expect("second child default context events should replay");
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
        "fresh per-page isolates should expose independent, target-scoped child context ids"
    );
    let first_child_heap = runtime_heap_usage_for_test(&first_page).await;
    let second_child_heap = runtime_heap_usage_for_test(&second_page).await;
    assert_eq!(
        first_child_heap["moli"]["runtime"]["inspectorContextRegistrationCount"],
        serde_json::json!(2),
        "first document should own default and child-default Inspector registrations"
    );
    assert_eq!(
        second_child_heap["moli"]["runtime"]["inspectorContextRegistrationCount"],
        serde_json::json!(2),
        "second document should own an independent child-default registration"
    );

    let (first_child_marker, _) = first_page
        .run_async_command(RendererPageCommand::EvaluateExpressionInExecutionContext {
            execution_context_id: first_child_context_id,
            expression: r#"globalThis.__lm_shared_child_marker = "first-child"; globalThis.__lm_shared_child_marker"#.to_owned(),
            await_promise: false,
        })
        .await
        .expect("first child default marker should evaluate");
    assert_eq!(
        renderer_json_value(first_child_marker),
        Some(serde_json::json!("first-child"))
    );

    let (second_child_missing, _) = second_page
        .run_async_command(RendererPageCommand::EvaluateExpressionInExecutionContext {
            execution_context_id: second_child_context_id,
            expression: r#"globalThis.__lm_shared_child_marker ?? "missing""#.to_owned(),
            await_promise: false,
        })
        .await
        .expect("second child default marker read should evaluate");
    assert_eq!(
        renderer_json_value(second_child_missing),
        Some(serde_json::json!("missing")),
        "child default contexts on different pages must not share global state"
    );

    let (second_child_marker_through_same_numeric_id, _) = second_page
        .run_async_command(RendererPageCommand::EvaluateExpressionInExecutionContext {
            execution_context_id: first_child_context_id,
            expression: r#"globalThis.__lm_shared_child_marker = "cross-page"; "cross-page""#
                .to_owned(),
            await_promise: false,
        })
        .await
        .expect("the reused child context id should resolve within page B's target");
    assert_eq!(
        renderer_json_value(second_child_marker_through_same_numeric_id),
        Some(serde_json::json!("cross-page")),
        "a target-scoped child context id must resolve to page B's own child realm"
    );

    let (first_child_marker_after_second_page_evaluate, _) = first_page
        .run_async_command(RendererPageCommand::EvaluateExpressionInExecutionContext {
            execution_context_id: first_child_context_id,
            expression: r#"globalThis.__lm_shared_child_marker"#.to_owned(),
            await_promise: false,
        })
        .await
        .expect("first child marker should remain readable");
    assert_eq!(
        renderer_json_value(first_child_marker_after_second_page_evaluate),
        Some(serde_json::json!("first-child")),
        "the same numeric child id on page B must never route into page A's isolate"
    );

    let (second_default_missing, _) = second_page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"globalThis.__lm_shared_child_marker ?? "missing""#.to_owned(),
            await_promise: false,
        })
        .await
        .expect("second default marker read should evaluate");
    assert_eq!(
        renderer_json_value(second_default_missing),
        Some(serde_json::json!("missing")),
        "target-local child context evaluation must not fall back to page B's default world"
    );

    first_page
        .close_async()
        .await
        .expect("first shared child-frame page should close");

    let (second_child_marker, _) = second_page
        .run_async_command(RendererPageCommand::EvaluateExpressionInExecutionContext {
            execution_context_id: second_child_context_id,
            expression: r#"globalThis.__lm_shared_child_marker = "second-child"; globalThis.__lm_shared_child_marker"#.to_owned(),
            await_promise: false,
        })
        .await
        .expect("second child default context should remain usable after first page closes");
    assert_eq!(
        renderer_json_value(second_child_marker),
        Some(serde_json::json!("second-child")),
        "closing page A must not tear down page B's child default context"
    );
    let second_child_heap_after_first_close = runtime_heap_usage_for_test(&second_page).await;
    assert_eq!(
        second_child_heap_after_first_close["moli"]["runtime"]["inspectorContextRegistrationCount"],
        serde_json::json!(2),
        "closing page A must not release page B's child-default registration"
    );

    second_page
        .close_async()
        .await
        .expect("second shared child-frame page should close");
}
#[tokio::test(flavor = "multi_thread")]
async fn per_page_isolate_policy_keeps_child_isolated_worlds_page_owned() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let first_url = url::Url::parse("https://example.test/shared-child-world-a").unwrap();
    let second_url = url::Url::parse("https://example.test/shared-child-world-b").unwrap();

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
r#"<!doctype html><body><iframe srcdoc="<body>first child isolated</body>"></iframe></body>"#
                .to_owned(),
crate::RendererDocumentOptions { ..Default::default() },
)
        .await
        .expect("first shared child-isolated-world page should load");
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
r#"<!doctype html><body><iframe srcdoc="<body>second child isolated</body>"></iframe></body>"#
                .to_owned(),
crate::RendererDocumentOptions { ..Default::default() },
)
        .await
        .expect("second shared child-isolated-world page should load");
    assert!(second_download.is_none());

    let first_testing = RendererPageTestingHandle::new_for_testing(&first_page);
    let second_testing = RendererPageTestingHandle::new_for_testing(&second_page);
    assert!(first_testing.shares_local_host(&second_testing));
    assert_eq!(
        second_testing
            .host_unique_document_isolate_count_async()
            .await
            .expect("two shared child-isolated-world unique document isolate count"),
        2
    );

    let first_child_context_ids = child_default_context_ids_for_test(&first_page)
        .await
        .expect("first child default context events should replay");
    let second_child_context_ids = child_default_context_ids_for_test(&second_page)
        .await
        .expect("second child default context events should replay");
    assert_eq!(first_child_context_ids.len(), 1);
    assert_eq!(second_child_context_ids.len(), 1);
    let first_child_frame_id =
        child_frame_id_for_default_context_id_for_test(&first_page, first_child_context_ids[0])
            .await
            .expect("first child default context should map to a frame id");
    let second_child_frame_id =
        child_frame_id_for_default_context_id_for_test(&second_page, second_child_context_ids[0])
            .await
            .expect("second child default context should map to a frame id");

    let first_child_world_context_id = create_isolated_world_for_frame_for_test(
        &first_page,
        &first_child_frame_id,
        "shared-child-utility",
    )
    .await
    .expect("first child isolated world should be created");
    let second_child_world_context_id = create_isolated_world_for_frame_for_test(
        &second_page,
        &second_child_frame_id,
        "shared-child-utility",
    )
    .await
    .expect("second child isolated world should be created");
    assert_eq!(
        first_child_world_context_id, second_child_world_context_id,
        "fresh per-page isolates should expose independent child-world context-id namespaces"
    );

    let (first_child_world_marker, _) = first_page
        .run_async_command(RendererPageCommand::EvaluateExpressionInExecutionContext {
            execution_context_id: first_child_world_context_id,
            expression: r#"document.__lm_shared_child_world_document_marker = "isolated-document"; globalThis.__lm_shared_child_world_marker = document.body.textContent.trim(); globalThis.__lm_shared_child_world_marker"#.to_owned(),
            await_promise: false,
        })
        .await
        .expect("first child isolated world marker should evaluate");
    assert_eq!(
        renderer_json_value(first_child_world_marker),
        Some(serde_json::json!("first child isolated"))
    );

    let (second_child_world_missing, _) = second_page
        .run_async_command(RendererPageCommand::EvaluateExpressionInExecutionContext {
            execution_context_id: second_child_world_context_id,
            expression: r#"globalThis.__lm_shared_child_world_marker ?? "missing""#.to_owned(),
            await_promise: false,
        })
        .await
        .expect("second child isolated world marker read should evaluate");
    assert_eq!(
        renderer_json_value(second_child_world_missing),
        Some(serde_json::json!("missing")),
        "same-name child isolated worlds on different pages must not share global state"
    );

    let (second_child_world_marker_through_same_numeric_id, _) = second_page
        .run_async_command(RendererPageCommand::EvaluateExpressionInExecutionContext {
            execution_context_id: first_child_world_context_id,
            expression: r#"globalThis.__lm_shared_child_world_marker = "cross-page"; "cross-page""#
                .to_owned(),
            await_promise: false,
        })
        .await
        .expect("the reused child-world id should resolve within page B's target");
    assert_eq!(
        renderer_json_value(second_child_world_marker_through_same_numeric_id),
        Some(serde_json::json!("cross-page")),
        "a target-scoped child-world id must resolve to page B's own isolated world"
    );

    let (first_child_world_marker_after_second_page_evaluate, _) = first_page
        .run_async_command(RendererPageCommand::EvaluateExpressionInExecutionContext {
            execution_context_id: first_child_world_context_id,
            expression: r#"globalThis.__lm_shared_child_world_marker"#.to_owned(),
            await_promise: false,
        })
        .await
        .expect("first child isolated-world marker should remain readable");
    assert_eq!(
        renderer_json_value(first_child_world_marker_after_second_page_evaluate),
        Some(serde_json::json!("first child isolated")),
        "the same numeric child-world id on page B must never route into page A's isolate"
    );

    let (second_default_missing, _) = second_page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"globalThis.__lm_shared_child_world_marker ?? "missing""#.to_owned(),
            await_promise: false,
        })
        .await
        .expect("second default marker read should evaluate");
    assert_eq!(
        renderer_json_value(second_default_missing),
        Some(serde_json::json!("missing")),
        "target-local child isolated-world access must not fall back to page B's default world"
    );

    let (second_child_default_missing, _) = second_page
        .run_async_command(RendererPageCommand::EvaluateExpressionInExecutionContext {
            execution_context_id: second_child_context_ids[0],
            expression: r#"globalThis.__lm_shared_child_world_marker ?? "missing""#.to_owned(),
            await_promise: false,
        })
        .await
        .expect("second child default marker read should evaluate");
    assert_eq!(
        renderer_json_value(second_child_default_missing),
        Some(serde_json::json!("missing")),
        "child isolated-world globals must not leak into page B's child default world"
    );

    let (first_child_default_document_missing, _) = first_page
        .run_async_command(RendererPageCommand::EvaluateExpressionInExecutionContext {
            execution_context_id: first_child_context_ids[0],
            expression: r#"document.__lm_shared_child_world_document_marker ?? "missing""#
                .to_owned(),
            await_promise: false,
        })
        .await
        .expect("first child default-world document wrapper should evaluate");
    assert_eq!(
        renderer_json_value(first_child_default_document_missing),
        Some(serde_json::json!("missing")),
        "child isolated and default worlds must keep distinct wrappers for the same Document"
    );

    first_page
        .close_async()
        .await
        .expect("first shared child-isolated-world page should close");

    let (second_child_world_marker, _) = second_page
        .run_async_command(RendererPageCommand::EvaluateExpressionInExecutionContext {
            execution_context_id: second_child_world_context_id,
            expression: r#"globalThis.__lm_shared_child_world_marker = document.body.textContent.trim(); globalThis.__lm_shared_child_world_marker"#.to_owned(),
            await_promise: false,
        })
        .await
        .expect("second child isolated world should remain usable after first page closes");
    assert_eq!(
        renderer_json_value(second_child_world_marker),
        Some(serde_json::json!("second child isolated")),
        "closing page A must not tear down page B's child isolated world"
    );

    second_page
        .close_async()
        .await
        .expect("second shared child-isolated-world page should close");
}
#[tokio::test(flavor = "multi_thread")]
async fn per_page_isolate_policy_reuses_navigation_isolate_and_replaces_contexts() {
    let runtime = JsRuntime::initialize();
    let (output_tx, mut output_rx) = renderer_external_activity_test_channel();
    runtime.set_renderer_output_transport_sender(output_tx);
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let navigated_url = url::Url::parse("https://example.test/shared-navigation-a").unwrap();
    let peer_url = url::Url::parse("https://example.test/shared-navigation-b").unwrap();

    let (mut navigated_page, _, _, _creation_artifacts, navigated_download) = runtime
        .create_html_page_from_response(
            navigated_url.clone(),
            navigated_url,
            None,
            false,
            0,
            200,
            vec![("content-type".to_owned(), b"text/html".to_vec())],
            &loader,
            crate::RendererWebStorageHandles::ephemeral(),
            "<!doctype html><body>old shared navigation document</body>".to_owned(),
            crate::RendererDocumentOptions {
                ..Default::default()
            },
        )
        .await
        .expect("first shared-isolate navigation page should load");
    assert!(navigated_download.is_none());

    let (mut peer_page, _, _, _creation_artifacts, peer_download) = runtime
        .create_html_page_from_response(
            peer_url.clone(),
            peer_url,
            None,
            false,
            0,
            200,
            vec![("content-type".to_owned(), b"text/html".to_vec())],
            &loader,
            crate::RendererWebStorageHandles::ephemeral(),
            "<!doctype html><body>peer shared navigation document</body>".to_owned(),
            crate::RendererDocumentOptions {
                ..Default::default()
            },
        )
        .await
        .expect("peer shared-isolate page should load");
    assert!(peer_download.is_none());

    let navigated_testing = RendererPageTestingHandle::new_for_testing(&navigated_page);
    let peer_testing = RendererPageTestingHandle::new_for_testing(&peer_page);
    assert!(navigated_testing.shares_local_host(&peer_testing));
    assert_eq!(
        peer_testing
            .host_unique_document_isolate_count_async()
            .await
            .expect("two shared attached unique document isolate count"),
        2
    );
    assert_window_performance_surface_for_test(&navigated_page, "old document").await;
    let old_heap = runtime_heap_usage_for_test(&navigated_page).await;
    let old_runtime = &old_heap["moli"]["runtime"];
    let old_context_group_id = old_runtime["inspectorContextGroupId"]
        .as_i64()
        .expect("old document should expose inspector context group id");
    let old_window_proxy_identity_hash = old_runtime["mainWindowProxyIdentityHash"]
        .as_i64()
        .expect("old document should expose main WindowProxy identity");
    assert_eq!(
        old_runtime["inspectorSessionRegistryOwner"],
        serde_json::json!("renderer-devtools-agent"),
        "old document inspector session registry should be local-root agent owned: {old_heap:?}"
    );

    let old_world_context_id = create_isolated_world_runtime_activity_for_test(
        &navigated_page,
        None,
        "navigation-utility",
    )
    .await
    .expect("old document isolated world should be created through runtime activity");
    let old_world_heap = runtime_heap_usage_for_test(&navigated_page).await;
    assert_eq!(
        old_world_heap["moli"]["runtime"]["inspectorContextRegistrationCount"],
        serde_json::json!(2),
        "old document should own its default and isolated context registrations"
    );
    let (old_world_marker, _) = navigated_page
        .run_async_command(RendererPageCommand::EvaluateExpressionInExecutionContext {
            execution_context_id: old_world_context_id,
            expression: r#"globalThis.__lm_shared_isolate_navigation_marker = "old-world"; globalThis.__lm_shared_isolate_navigation_marker"#.to_owned(),
            await_promise: false,
        })
        .await
        .expect("old isolated world marker should evaluate");
    assert_eq!(
        renderer_json_value(old_world_marker),
        Some(serde_json::json!("old-world"))
    );
    let old_document_object_id = runtime_protocol_object_id(
        &navigated_page,
        serde_json::json!({
            "id": 51,
            "method": "Runtime.evaluate",
            "params": {
                "expression": "({ marker: 'old-document-object' })"
            }
        }),
        51,
    )
    .await
    .expect("old document Runtime.evaluate should return an objectId");
    let old_runtime_enable_events = runtime_enable_events_for_test(&navigated_page)
        .await
        .expect("old document Runtime.enable should run through V8 inspector");
    assert!(
        old_runtime_enable_events.iter().any(|message| {
            message["method"] == serde_json::json!("Runtime.executionContextCreated")
        }),
        "old document Runtime.enable should connect the renderer Runtime agent: {old_runtime_enable_events:?}"
    );
    output_rx.drain();

    let replacement_url = "data:text/html;charset=utf-8,%3C!doctype%20html%3E%3Cbody%3Enew%20shared%20navigation%20document%3C/body%3E";
    let (navigation_reply, _) = navigated_page
        .run_async_command(
            RendererPageCommand::EvaluateExpressionAndFollowPendingNavigation {
                expression: format!(
                    r#"(() => {{
  globalThis.__lm_shared_isolate_navigation_marker = "old-default";
  location.href = {replacement_url:?};
  return "navigating";
}})()"#
                ),
                await_promise: false,
            },
        )
        .await
        .expect("shared isolate navigation should replace the live page");
    assert_eq!(
        renderer_json_value(navigation_reply),
        Some(serde_json::json!("navigating"))
    );
    assert_eq!(
        peer_testing
            .host_unique_document_isolate_count_async()
            .await
            .expect("shared unique document isolate count after navigation replacement"),
        2,
        "navigation replacement must retain one distinct isolate for each live page"
    );

    let (new_document_text, _) = navigated_page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"document.body.textContent"#.to_owned(),
            await_promise: false,
        })
        .await
        .expect("new document body text should evaluate");
    assert_eq!(
        renderer_json_value(new_document_text),
        Some(serde_json::json!("new shared navigation document"))
    );
    assert_window_performance_surface_for_test(&navigated_page, "replacement document").await;
    let new_heap = runtime_heap_usage_for_test(&navigated_page).await;
    let new_runtime = &new_heap["moli"]["runtime"];
    assert_eq!(
        new_runtime["inspectorSessionRegistryOwner"],
        serde_json::json!("renderer-devtools-agent"),
        "replacement document inspector session registry should be local-root agent owned: {new_heap:?}"
    );
    assert_ne!(
        new_runtime["inspectorContextGroupId"],
        serde_json::json!(old_context_group_id),
        "cross-Page replacement must create a distinct local-root context group"
    );
    assert_eq!(
        new_runtime["mainWindowProxyIdentityHash"],
        serde_json::json!(old_window_proxy_identity_hash),
        "committed top-level navigation must detach and reuse the same V8 global proxy identity"
    );
    assert!(
        new_runtime["inspectorSessionCount"]
            .as_u64()
            .unwrap_or_default()
            >= 1,
        "replacement document should keep at least the default inspector session: {new_heap:?}"
    );
    assert_eq!(
        new_runtime["inspectorContextRegistrationCount"],
        serde_json::json!(1),
        "replacement must release every old-document registration and retain only its new default context"
    );
    let replacement_runtime_messages =
        output_rx.drain_runtime_inspector_messages_for_page(&navigated_page);
    let replacement_context_events = replacement_runtime_messages
        .iter()
        .filter_map(|message| message["method"].as_str())
        .filter(|method| {
            matches!(
                *method,
                "Runtime.executionContextsCleared" | "Runtime.executionContextCreated"
            )
        })
        .collect::<Vec<_>>();
    assert!(
        replacement_context_events.len() >= 2
            && replacement_context_events.last() == Some(&"Runtime.executionContextCreated")
            && replacement_context_events[..replacement_context_events.len() - 1]
                .iter()
                .all(|method| *method == "Runtime.executionContextsCleared"),
        "renderer-side document replacement should publish every old/reattached V8 context reset before exactly one new default context: {replacement_runtime_messages:?}"
    );
    assert!(
        default_execution_context_id_for_test(&navigated_page)
            .await
            .expect("replacement default execution context lookup")
            .is_some(),
        "the old backend's context-clear event must not clear the replacement backend's local default-context identity"
    );

    let old_world_still_registered = navigated_page
        .run_async_command(RendererPageCommand::HasIsolatedExecutionContextId(
            old_world_context_id,
        ))
        .await
        .expect("old isolated context membership should evaluate");
    assert_eq!(
        renderer_bool(old_world_still_registered.0),
        Some(false),
        "navigation replacement must remove the old document's isolated world from the page facade"
    );

    let old_marker_after_navigation = navigated_page
        .run_async_command(RendererPageCommand::EvaluateExpressionInExecutionContext {
            execution_context_id: old_world_context_id,
            expression: r#"globalThis.__lm_shared_isolate_navigation_marker = "stale"; "stale""#
                .to_owned(),
            await_promise: false,
        })
        .await;
    assert!(
        old_marker_after_navigation.is_err(),
        "stale isolated-world execution context id must fail closed after navigation replacement"
    );

    let (replacement_marker_after_stale_context, _) = navigated_page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"globalThis.__lm_shared_isolate_navigation_marker ?? "missing""#
                .to_owned(),
            await_promise: false,
        })
        .await
        .expect("replacement default world marker read should evaluate");
    assert_eq!(
        renderer_json_value(replacement_marker_after_stale_context),
        Some(serde_json::json!("missing")),
        "stale isolated-world context id failure must not fall back to the replacement document"
    );

    let stale_object_call = dispatch_runtime_protocol_for_test(
        &navigated_page,
        serde_json::json!({
            "id": 52,
            "method": "Runtime.callFunctionOn",
            "params": {
                "objectId": old_document_object_id,
                "functionDeclaration": "function() { globalThis.__staleObjectMutatedReplacement = true; return this.marker; }",
                "returnByValue": true
            }
        }),
    )
    .await
    .expect("stale old document Runtime.callFunctionOn should dispatch");
    let stale_object_response = runtime_protocol_response_by_id(&stale_object_call, 52)
        .expect("stale object call response");
    assert!(
        stale_object_response.get("error").is_some()
            || stale_object_response["result"]["exceptionDetails"].is_object(),
        "navigation replacement must reject or fail closed for the old document Runtime objectId: {stale_object_response:?}"
    );
    let (replacement_mutation_marker, _) = navigated_page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"globalThis.__staleObjectMutatedReplacement ?? "not-mutated""#.to_owned(),
            await_promise: false,
        })
        .await
        .expect("replacement stale object mutation marker should evaluate");
    assert_eq!(
        renderer_json_value(replacement_mutation_marker),
        Some(serde_json::json!("not-mutated")),
        "stale old document Runtime objectId must not execute against the replacement document global"
    );

    navigated_page
        .close_async()
        .await
        .expect("navigated shared page should close");
    peer_page
        .close_async()
        .await
        .expect("peer shared page should close");
}
#[tokio::test(flavor = "multi_thread")]
async fn per_page_isolate_parser_blocking_navigation_restores_replacement_inspector_session() {
    let runtime = JsRuntime::initialize();
    let (output_tx, mut output_rx) = renderer_external_activity_test_channel();
    runtime.set_renderer_output_transport_sender(output_tx);
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let (base_url, script_request_seen, release_script_response, server) =
        spawn_owner_wake_gated_server_with_content_type(
            "/replacement-parser-blocking.js",
            r#"globalThis.__lm_parser_blocking_replacement = "ran";"#,
            "application/javascript",
        )
        .await;
    let initial_url = url::Url::parse(&format!("{base_url}/parser-blocking-navigation-source"))
        .expect("initial parser-blocking navigation url");

    let (mut page, _, _, _creation_artifacts, pending_download) = runtime
        .create_html_page_from_response(
            initial_url.clone(),
            initial_url,
            None,
            false,
            0,
            200,
            vec![("content-type".to_owned(), b"text/html".to_vec())],
            &loader,
            crate::RendererWebStorageHandles::ephemeral(),
            "<!doctype html><body>parser-blocking navigation source</body>".to_owned(),
            crate::RendererDocumentOptions {
                ..Default::default()
            },
        )
        .await
        .expect("parser-blocking navigation source page should load");
    assert!(pending_download.is_none());

    runtime_enable_events_for_test(&page)
        .await
        .expect("old document Runtime.enable should establish persistent session state");
    output_rx.drain();

    let replacement_html = format!(
        r#"<!doctype html><script src="{base_url}/replacement-parser-blocking.js"></script><body>parser-blocking replacement</body>"#
    );
    let encoded_replacement_html = percent_encoding::utf8_percent_encode(
        &replacement_html,
        percent_encoding::NON_ALPHANUMERIC,
    );
    let replacement_url = format!("data:text/html;charset=utf-8,{encoded_replacement_html}");

    let release = tokio::spawn(async move {
        tokio::time::timeout(Duration::from_secs(2), script_request_seen)
            .await
            .expect("replacement parser-blocking script request should start")
            .expect("replacement parser-blocking script request signal should remain open");
        release_script_response
            .send(())
            .expect("replacement parser-blocking script response should release");
    });

    let (navigation_reply, _) = page
        .run_async_command(
            RendererPageCommand::EvaluateExpressionAndFollowPendingNavigation {
                expression: format!(r#"location.href = {replacement_url:?}; "navigating""#),
                await_promise: false,
            },
        )
        .await
        .expect("parser-blocking navigation should install the replacement PageVm");
    assert_eq!(
        renderer_json_value(navigation_reply),
        Some(serde_json::json!("navigating"))
    );
    release
        .await
        .expect("parser-blocking response release task should not panic");
    tokio::time::timeout(Duration::from_secs(2), server)
        .await
        .expect("parser-blocking server should finish")
        .expect("parser-blocking server task should not panic");

    let (replacement_state, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"[document.body.textContent, globalThis.__lm_parser_blocking_replacement].join("|")"#.to_owned(),
            await_promise: false,
        })
        .await
        .expect("replacement parser-blocking document should remain executable");
    assert_eq!(
        renderer_json_value(replacement_state),
        Some(serde_json::json!("parser-blocking replacement|ran"))
    );

    let replacement_runtime_messages = output_rx.drain_runtime_inspector_messages_for_page(&page);
    assert!(
        replacement_runtime_messages.iter().any(|message| {
            message["method"] == serde_json::json!("Runtime.executionContextsCleared")
        }),
        "replacement session restore should deliver the old-context clear event: {replacement_runtime_messages:?}"
    );
    assert!(
        replacement_runtime_messages.iter().any(|message| {
            message["method"] == serde_json::json!("Runtime.executionContextCreated")
        }),
        "replacement session restore should deliver the new context event: {replacement_runtime_messages:?}"
    );

    page.close_async()
        .await
        .expect("parser-blocking replacement page should close");
}
#[tokio::test(flavor = "multi_thread")]
async fn per_page_isolate_navigation_churn_disposes_replaced_page_vms() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let initial_url = url::Url::parse("https://example.test/shared-navigation-churn").unwrap();

    let (mut page, _, _, _creation_artifacts, pending_download) = runtime
        .create_html_page_from_response(
            initial_url.clone(),
            initial_url,
            None,
            false,
            0,
            200,
            vec![("content-type".to_owned(), b"text/html".to_vec())],
            &loader,
            crate::RendererWebStorageHandles::ephemeral(),
            "<!doctype html><body>churn-initial</body>".to_owned(),
            crate::RendererDocumentOptions {
                ..Default::default()
            },
        )
        .await
        .expect("shared-isolate churn page should load");
    assert!(pending_download.is_none());

    let testing = RendererPageTestingHandle::new_for_testing(&page);
    let baseline_pending = testing
        .deferred_page_vm_drop_pending_count_async()
        .await
        .expect("initial deferred PageVm drop pending count");

    for index in 0..8 {
        let (finalizer_setup, _) = page
            .run_async_command(RendererPageCommand::EvaluateExpression {
                expression: format!(
                    r#"(async () => {{
  globalThis.__lm_context_owned_finalizer_objects = [];
  for (let objectIndex = 0; objectIndex < 32; objectIndex += 1) {{
    const element = document.createElement("div");
    element.style.color = "red";

    const sheet = new CSSStyleSheet();
    sheet.replaceSync(`.item-${{objectIndex}} {{ color: red; }}`);
    sheet.cssRules[0].style.setProperty("color", "blue");

    const blob = new Blob([`payload-${{objectIndex}}`], {{ type: "text/plain" }});
    globalThis.__lm_context_owned_finalizer_objects.push(element, sheet, blob);
  }}
  const responses = await Promise.all(
    Array.from({{ length: 8 }}, (_, responseIndex) =>
      fetch(`data:text/plain;charset=utf-8,response-{index}-${{responseIndex}}`)
    )
  );
  globalThis.__lm_context_owned_finalizer_objects.push(...responses);
  return globalThis.__lm_context_owned_finalizer_objects.length;
}})()"#
                ),
                await_promise: true,
            })
            .await
            .expect("context-owned finalizer setup should complete before navigation");
        assert_eq!(
            renderer_json_value(finalizer_setup),
            Some(serde_json::json!(104)),
            "navigation churn must retain CSS, Blob, and network body objects until old-context teardown"
        );

        let replacement_url = format!(
            "data:text/html;charset=utf-8,%3C!doctype%20html%3E%3Cbody%3Echurn-{index}%3C/body%3E"
        );
        let (navigation_reply, _) = page
            .run_async_command(
                RendererPageCommand::EvaluateExpressionAndFollowPendingNavigation {
                    expression: format!(
                        r#"location.href = {replacement_url:?}; "navigating-{index}""#
                    ),
                    await_promise: false,
                },
            )
            .await
            .expect("shared-isolate churn navigation should replace the live page");
        assert_eq!(
            renderer_json_value(navigation_reply),
            Some(serde_json::json!(format!("navigating-{index}")))
        );
        let (body_text, _) = page
            .run_async_command(RendererPageCommand::EvaluateExpression {
                expression: r#"document.body.textContent"#.to_owned(),
                await_promise: false,
            })
            .await
            .expect("replacement churn body text should evaluate");
        assert_eq!(
            renderer_json_value(body_text),
            Some(serde_json::json!(format!("churn-{index}")))
        );
        assert_eq!(
            testing
                .deferred_page_vm_drop_pending_count_async()
                .await
                .expect("deferred PageVm drop pending count after navigation churn"),
            baseline_pending,
            "replaced per-page PageVms must dispose without a deferred LIFO backlog"
        );
    }

    page.close_async()
        .await
        .expect("shared-isolate churn page should close");
}
#[tokio::test(flavor = "multi_thread")]
async fn per_page_isolate_policy_drops_stale_timer_after_navigation_replacement() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let navigated_url = url::Url::parse("https://example.test/shared-stale-timer-a").unwrap();
    let peer_url = url::Url::parse("https://example.test/shared-stale-timer-b").unwrap();

    let (mut navigated_page, _, _, _creation_artifacts, navigated_download) = runtime
        .create_html_page_from_response(
            navigated_url.clone(),
            navigated_url,
            None,
            false,
            0,
            200,
            vec![("content-type".to_owned(), b"text/html".to_vec())],
            &loader,
            crate::RendererWebStorageHandles::ephemeral(),
            "<!doctype html><body>stale timer source</body>".to_owned(),
            crate::RendererDocumentOptions {
                ..Default::default()
            },
        )
        .await
        .expect("stale-timer source page should load");
    assert!(navigated_download.is_none());

    let (mut peer_page, _, _, _creation_artifacts, peer_download) = runtime
        .create_html_page_from_response(
            peer_url.clone(),
            peer_url,
            None,
            false,
            0,
            200,
            vec![("content-type".to_owned(), b"text/html".to_vec())],
            &loader,
            crate::RendererWebStorageHandles::ephemeral(),
            "<!doctype html><body>stale timer peer</body>".to_owned(),
            crate::RendererDocumentOptions {
                ..Default::default()
            },
        )
        .await
        .expect("stale-timer peer page should load");
    assert!(peer_download.is_none());

    let navigated_testing = RendererPageTestingHandle::new_for_testing(&navigated_page);
    let peer_testing = RendererPageTestingHandle::new_for_testing(&peer_page);
    assert!(navigated_testing.shares_local_host(&peer_testing));
    assert_eq!(
        peer_testing
            .host_unique_document_isolate_count_async()
            .await
            .expect("shared stale-timer unique document isolate count"),
        2
    );

    let replacement_url = "data:text/html;charset=utf-8,%3C!doctype%20html%3E%3Cbody%3Etimer%20replacement%20document%3C/body%3E";
    let (navigation_reply, _) = navigated_page
        .run_async_command(
            RendererPageCommand::EvaluateExpressionAndFollowPendingNavigation {
                expression: format!(
                    r#"(() => {{
  globalThis.__lm_stale_timer_marker = "old-document";
  setTimeout(() => {{
    globalThis.__lm_stale_timer_marker = "stale-timer-fired";
    globalThis.__lm_stale_timer_mutated_replacement = "stale-timer-fired";
  }}, 0);
  location.href = {replacement_url:?};
  return "navigating";
}})()"#
                ),
                await_promise: false,
            },
        )
        .await
        .expect("stale-timer page should navigate to replacement document");
    assert_eq!(
        renderer_json_value(navigation_reply),
        Some(serde_json::json!("navigating"))
    );
    assert_eq!(
        peer_testing
            .host_unique_document_isolate_count_async()
            .await
            .expect(
                "shared unique document isolate count after stale-timer navigation replacement"
            ),
        2,
        "timer navigation replacement must retain one distinct isolate for each live page"
    );

    let (advance, _) = navigated_page
        .run_async_command(RendererPageCommand::MsToNextTimeout)
        .await
        .expect(
            "replacement timer deadline should remain observable after stale timer replacement",
        );
    match advance {
        RendererPageReply::OptionalU64(ms_to_next_timeout) => {
            assert_eq!(
                ms_to_next_timeout, None,
                "replacement document should not inherit the old document timer deadline"
            );
        }
        _ => panic!("unexpected timer-deadline reply after stale timer replacement"),
    }

    let (replacement_marker, _) = navigated_page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"[
  document.body.textContent,
  globalThis.__lm_stale_timer_marker ?? "missing",
  globalThis.__lm_stale_timer_mutated_replacement ?? "not-mutated"
].join("|")"#
                .to_owned(),
            await_promise: false,
        })
        .await
        .expect("replacement stale-timer marker should evaluate");
    assert_eq!(
        renderer_json_value(replacement_marker),
        Some(serde_json::json!(
            "timer replacement document|missing|not-mutated"
        )),
        "stale old-document timer callback must not mutate the replacement document"
    );

    navigated_page
        .close_async()
        .await
        .expect("stale-timer navigated page should close");
    peer_page
        .close_async()
        .await
        .expect("stale-timer peer page should close");
}
#[tokio::test(flavor = "multi_thread")]
async fn per_page_isolate_policy_drops_stale_fetch_after_navigation_replacement() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let (base_url, fetch_request_seen, release_fetch_response, server) =
        spawn_owner_wake_gated_server_with_content_type(
            "/stale-fetch.txt",
            "late-fetch",
            "text/plain; charset=utf-8",
        )
        .await;
    let navigated_url =
        url::Url::parse(&format!("{base_url}/shared-stale-fetch-a")).expect("fetch source url");
    let peer_url =
        url::Url::parse(&format!("{base_url}/shared-stale-fetch-b")).expect("fetch peer url");

    let (mut navigated_page, _, _, _creation_artifacts, navigated_download) = runtime
        .create_html_page_from_response(
            navigated_url.clone(),
            navigated_url,
            None,
            false,
            0,
            200,
            vec![("content-type".to_owned(), b"text/html".to_vec())],
            &loader,
            crate::RendererWebStorageHandles::ephemeral(),
            "<!doctype html><body>stale fetch source</body>".to_owned(),
            crate::RendererDocumentOptions {
                ..Default::default()
            },
        )
        .await
        .expect("stale-fetch source page should load");
    assert!(navigated_download.is_none());

    let (mut peer_page, _, _, _creation_artifacts, peer_download) = runtime
        .create_html_page_from_response(
            peer_url.clone(),
            peer_url,
            None,
            false,
            0,
            200,
            vec![("content-type".to_owned(), b"text/html".to_vec())],
            &loader,
            crate::RendererWebStorageHandles::ephemeral(),
            "<!doctype html><body>stale fetch peer</body>".to_owned(),
            crate::RendererDocumentOptions {
                ..Default::default()
            },
        )
        .await
        .expect("stale-fetch peer page should load");
    assert!(peer_download.is_none());

    let navigated_testing = RendererPageTestingHandle::new_for_testing(&navigated_page);
    let peer_testing = RendererPageTestingHandle::new_for_testing(&peer_page);
    assert!(navigated_testing.shares_local_host(&peer_testing));
    assert_eq!(
        peer_testing
            .host_unique_document_isolate_count_async()
            .await
            .expect("shared stale-fetch unique document isolate count"),
        2
    );

    let (scheduled, _) = navigated_page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"(() => {
  globalThis.__lm_stale_fetch_marker = "old-document";
  fetch("./stale-fetch.txt").then(
    response => response.text()
  ).then(
    text => {
      globalThis.__lm_stale_fetch_continuation = text;
      globalThis.__lm_stale_fetch_mutated_replacement = "stale-fetch-continuation";
    },
    error => {
      globalThis.__lm_stale_fetch_continuation = "error:" + error.name;
    }
  );
  return "scheduled";
})()"#
                .to_owned(),
            await_promise: false,
        })
        .await
        .expect("stale-fetch page should schedule fetch");
    assert_eq!(
        renderer_json_value(scheduled),
        Some(serde_json::json!("scheduled"))
    );

    tokio::time::timeout(Duration::from_secs(2), fetch_request_seen)
        .await
        .expect("stale fetch request should reach the server before navigation")
        .expect("stale fetch request signal should send");

    let replacement_url = "data:text/html;charset=utf-8,%3C!doctype%20html%3E%3Cbody%3Efetch%20replacement%20document%3C/body%3E";
    let (navigation_reply, _) = navigated_page
        .run_async_command(
            RendererPageCommand::EvaluateExpressionAndFollowPendingNavigation {
                expression: format!(
                    r#"(() => {{
  location.href = {replacement_url:?};
  return "navigating";
}})()"#
                ),
                await_promise: false,
            },
        )
        .await
        .expect("stale-fetch page should navigate to replacement document");
    assert_eq!(
        renderer_json_value(navigation_reply),
        Some(serde_json::json!("navigating"))
    );
    assert_eq!(
        peer_testing
            .host_unique_document_isolate_count_async()
            .await
            .expect(
                "shared unique document isolate count after stale-fetch navigation replacement"
            ),
        2,
        "fetch navigation replacement must retain one distinct isolate for each live page"
    );

    release_fetch_response
        .send(())
        .expect("stale fetch response release should send");
    tokio::time::timeout(Duration::from_secs(2), server)
        .await
        .expect("stale fetch server should finish after response release")
        .expect("stale fetch server task should not panic");

    let (_advance, _) = navigated_page
        .run_async_command(RendererPageCommand::MsToNextTimeout)
        .await
        .expect("replacement timer deadline should remain observable after stale fetch response");

    let (replacement_marker, _) = navigated_page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"[
  document.body.textContent,
  globalThis.__lm_stale_fetch_marker ?? "missing",
  globalThis.__lm_stale_fetch_continuation ?? "missing",
  globalThis.__lm_stale_fetch_mutated_replacement ?? "not-mutated"
].join("|")"#
                .to_owned(),
            await_promise: false,
        })
        .await
        .expect("replacement stale-fetch marker should evaluate");
    assert_eq!(
        renderer_json_value(replacement_marker),
        Some(serde_json::json!(
            "fetch replacement document|missing|missing|not-mutated"
        )),
        "stale old-document fetch completion must not mutate the replacement document"
    );

    navigated_page
        .close_async()
        .await
        .expect("stale-fetch navigated page should close");
    peer_page
        .close_async()
        .await
        .expect("stale-fetch peer page should close");
}
#[tokio::test(flavor = "multi_thread")]
async fn per_page_isolate_policy_drops_stale_module_fetch_after_navigation_replacement() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let (base_url, module_request_seen, release_module_response, server) =
        spawn_owner_wake_gated_server_with_content_type(
            "/stale-module.js",
            r#"globalThis.__lm_stale_module_mutated_replacement = "stale-module-evaluated";
export const marker = "late-module";"#,
            "application/javascript",
        )
        .await;
    let navigated_url =
        url::Url::parse(&format!("{base_url}/shared-stale-module-a")).expect("module source url");
    let peer_url =
        url::Url::parse(&format!("{base_url}/shared-stale-module-b")).expect("module peer url");

    let (mut navigated_page, _, _, _creation_artifacts, navigated_download) = runtime
        .create_html_page_from_response(
            navigated_url.clone(),
            navigated_url,
            None,
            false,
            0,
            200,
            vec![("content-type".to_owned(), b"text/html".to_vec())],
            &loader,
            crate::RendererWebStorageHandles::ephemeral(),
            "<!doctype html><body>stale module source</body>".to_owned(),
            crate::RendererDocumentOptions {
                ..Default::default()
            },
        )
        .await
        .expect("stale-module source page should load");
    assert!(navigated_download.is_none());

    let (mut peer_page, _, _, _creation_artifacts, peer_download) = runtime
        .create_html_page_from_response(
            peer_url.clone(),
            peer_url,
            None,
            false,
            0,
            200,
            vec![("content-type".to_owned(), b"text/html".to_vec())],
            &loader,
            crate::RendererWebStorageHandles::ephemeral(),
            "<!doctype html><body>stale module peer</body>".to_owned(),
            crate::RendererDocumentOptions {
                ..Default::default()
            },
        )
        .await
        .expect("stale-module peer page should load");
    assert!(peer_download.is_none());

    let navigated_testing = RendererPageTestingHandle::new_for_testing(&navigated_page);
    let peer_testing = RendererPageTestingHandle::new_for_testing(&peer_page);
    assert!(navigated_testing.shares_local_host(&peer_testing));
    assert_eq!(
        peer_testing
            .host_unique_document_isolate_count_async()
            .await
            .expect("shared stale-module unique document isolate count"),
        2
    );

    let (scheduled, _) = navigated_page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"(() => {
  globalThis.__lm_stale_module_marker = "old-document";
  import("./stale-module.js").then(
    module => {
      globalThis.__lm_stale_module_continuation = module.marker;
      globalThis.__lm_stale_module_mutated_replacement = "stale-module-continuation";
    },
    error => {
      globalThis.__lm_stale_module_continuation = "error:" + error.name;
    }
  );
  return "scheduled";
})()"#
                .to_owned(),
            await_promise: false,
        })
        .await
        .expect("stale-module page should schedule dynamic import");
    assert_eq!(
        renderer_json_value(scheduled),
        Some(serde_json::json!("scheduled"))
    );

    tokio::time::timeout(Duration::from_secs(2), module_request_seen)
        .await
        .expect("stale module request should reach the server before navigation")
        .expect("stale module request signal should send");

    let replacement_url = "data:text/html;charset=utf-8,%3C!doctype%20html%3E%3Cbody%3Emodule%20replacement%20document%3C/body%3E";
    let (navigation_reply, _) = navigated_page
        .run_async_command(
            RendererPageCommand::EvaluateExpressionAndFollowPendingNavigation {
                expression: format!(
                    r#"(() => {{
  location.href = {replacement_url:?};
  return "navigating";
}})()"#
                ),
                await_promise: false,
            },
        )
        .await
        .expect("stale-module page should navigate to replacement document");
    assert_eq!(
        renderer_json_value(navigation_reply),
        Some(serde_json::json!("navigating"))
    );
    assert_eq!(
        peer_testing
            .host_unique_document_isolate_count_async()
            .await
            .expect(
                "shared unique document isolate count after stale-module navigation replacement"
            ),
        2,
        "module navigation replacement must retain one distinct isolate for each live page"
    );

    release_module_response
        .send(())
        .expect("stale module response release should send");
    tokio::time::timeout(Duration::from_secs(2), server)
        .await
        .expect("stale module server should finish after response release")
        .expect("stale module server task should not panic");

    let (_advance, _) = navigated_page
        .run_async_command(RendererPageCommand::MsToNextTimeout)
        .await
        .expect("replacement timer deadline should remain observable after stale module response");

    let (replacement_marker, _) = navigated_page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"[
  document.body.textContent,
  globalThis.__lm_stale_module_marker ?? "missing",
  globalThis.__lm_stale_module_continuation ?? "missing",
  globalThis.__lm_stale_module_mutated_replacement ?? "not-mutated"
].join("|")"#
                .to_owned(),
            await_promise: false,
        })
        .await
        .expect("replacement stale-module marker should evaluate");
    assert_eq!(
        renderer_json_value(replacement_marker),
        Some(serde_json::json!(
            "module replacement document|missing|missing|not-mutated"
        )),
        "stale old-document module fetch completion must not mutate the replacement document"
    );

    navigated_page
        .close_async()
        .await
        .expect("stale-module navigated page should close");
    peer_page
        .close_async()
        .await
        .expect("stale-module peer page should close");
}
#[tokio::test(flavor = "multi_thread")]
async fn per_page_isolate_policy_routes_dynamic_import_to_originating_page() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let (base_url, server) = spawn_owner_wake_server_with_content_type(
        "/module.js",
        r#"export const marker = "second-dynamic"; export const metaUrl = import.meta.url;"#,
        "application/javascript",
        Duration::ZERO,
    )
    .await;
    let first_url =
        url::Url::parse(&format!("{base_url}/shared-dynamic-import-a")).expect("first page url");
    let second_url =
        url::Url::parse(&format!("{base_url}/shared-dynamic-import-b")).expect("second page url");

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
            "<!doctype html><body>first dynamic import owner</body>".to_owned(),
            crate::RendererDocumentOptions {
                ..Default::default()
            },
        )
        .await
        .expect("first shared-isolate page should load");
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
            "<!doctype html><body>second dynamic import owner</body>".to_owned(),
            crate::RendererDocumentOptions {
                ..Default::default()
            },
        )
        .await
        .expect("second shared-isolate page should load");
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

    first_page
        .close_async()
        .await
        .expect("first shared page should close");

    let (scheduled, _) = second_page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"(() => {
  globalThis.__lm_shared_isolate_dynamic_import_marker = "pending";
  import("./module.js").then(
    module => {
      globalThis.__lm_shared_isolate_dynamic_import_marker =
        module.marker + "|" + String(module.metaUrl === new URL("./module.js", location.href).href);
    },
    error => {
      globalThis.__lm_shared_isolate_dynamic_import_marker = "error:" + error.name + ":" + error.message;
    }
  );
  return "scheduled";
})()"#
            .to_owned(),
            await_promise: false,
        })
        .await
        .expect("second page dynamic import should schedule");
    assert_eq!(
        renderer_json_value(scheduled),
        Some(serde_json::json!("scheduled"))
    );

    tokio::time::timeout(
        Duration::from_secs(2),
        second_page.run_async_command(RendererPageCommand::WaitForScriptTruthy {
            expression: r#"globalThis.__lm_shared_isolate_dynamic_import_marker !== "pending""#
                .to_owned(),
            timeout_ms: 2_000,
            loader: loader.clone(),
        }),
    )
    .await
    .expect("shared isolate dynamic import wait should complete")
    .expect("shared isolate dynamic import should resolve through the originating page");

    let (marker, _) = second_page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"globalThis.__lm_shared_isolate_dynamic_import_marker"#.to_owned(),
            await_promise: false,
        })
        .await
        .expect("second page dynamic import marker should evaluate");
    server.abort();
    assert_eq!(
        renderer_json_value(marker),
        Some(serde_json::json!("second-dynamic|true")),
        "dynamic import callbacks must route through page B's context bridge after page A closes"
    );

    second_page
        .close_async()
        .await
        .expect("second shared page should close");
}
#[tokio::test(flavor = "multi_thread")]
async fn per_page_isolate_policy_replays_runtime_contexts_per_page() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let first_url = url::Url::parse("https://example.test/shared-runtime-replay-a").unwrap();
    let second_url = url::Url::parse("https://example.test/shared-runtime-replay-b").unwrap();

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
            "<!doctype html><body>first runtime replay owner</body>".to_owned(),
            crate::RendererDocumentOptions {
                ..Default::default()
            },
        )
        .await
        .expect("first shared-isolate runtime-replay page should load");
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
            "<!doctype html><body>second runtime replay owner</body>".to_owned(),
            crate::RendererDocumentOptions {
                ..Default::default()
            },
        )
        .await
        .expect("second shared-isolate runtime-replay page should load");
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
        create_isolated_world_for_test(&first_page, "shared-runtime-world")
            .await
            .expect("first runtime replay isolated world should be created");
    let second_world_context_id =
        create_isolated_world_for_test(&second_page, "shared-runtime-world")
            .await
            .expect("second runtime replay isolated world should be created");
    let first_events = runtime_enable_events_for_test(&first_page)
        .await
        .expect("first page Runtime.enable replay should run");
    let first_context_ids = runtime_execution_context_ids(&first_events);
    assert!(
        first_context_ids.contains(&first_world_context_id),
        "first Runtime.enable replay should include first page isolated world: {first_events:?}"
    );
    let first_isolated_context =
        runtime_execution_context_by_id(&first_events, first_world_context_id)
            .expect("first Runtime.enable replay should expose the first isolated world context");
    assert!(
        first_isolated_context["uniqueId"].as_str().is_some(),
        "first isolated context should come from V8 RuntimeAgent native replay, not Moli synthetic fallback: {first_isolated_context:?}"
    );
    let first_isolated_unique_id = first_isolated_context["uniqueId"]
        .as_str()
        .expect("first isolated context uniqueId")
        .to_owned();

    let second_events = runtime_enable_events_for_test(&second_page)
        .await
        .expect("second page Runtime.enable replay should run");
    let second_context_ids = runtime_execution_context_ids(&second_events);
    assert!(
        second_context_ids.contains(&second_world_context_id),
        "second Runtime.enable replay should include second page isolated world: {second_events:?}"
    );
    let second_isolated_context =
        runtime_execution_context_by_id(&second_events, second_world_context_id)
            .expect("second Runtime.enable replay should expose the second isolated world context");
    assert!(
        second_isolated_context["uniqueId"].as_str().is_some(),
        "second isolated context should come from V8 RuntimeAgent native replay, not Moli synthetic fallback: {second_isolated_context:?}"
    );
    let second_isolated_unique_id = second_isolated_context["uniqueId"]
        .as_str()
        .expect("second isolated context uniqueId");
    assert_ne!(
        first_isolated_unique_id, second_isolated_unique_id,
        "target-scoped numeric context ids may collide, but V8 uniqueIds must identify different realms"
    );
    assert!(
        !runtime_execution_context_unique_ids(&first_events).contains(&second_isolated_unique_id),
        "first Runtime.enable replay must not include page B's realm uniqueId: {first_events:?}"
    );
    assert!(
        !runtime_execution_context_unique_ids(&second_events)
            .contains(&first_isolated_unique_id.as_str()),
        "second Runtime.enable replay must not include page A's realm uniqueId: {second_events:?}"
    );

    let first_default_context_count = runtime_default_context_count(&first_events);
    let second_default_context_count = runtime_default_context_count(&second_events);
    assert_eq!(
        first_default_context_count, 1,
        "first Runtime.enable replay should expose one page default context"
    );
    assert_eq!(
        second_default_context_count, 1,
        "second Runtime.enable replay should expose one page default context"
    );

    first_page
        .close_async()
        .await
        .expect("first runtime-replay page should close");
    second_page
        .close_async()
        .await
        .expect("second runtime-replay page should close");
}
#[tokio::test(flavor = "multi_thread")]
async fn per_page_isolate_policy_scopes_document_start_scripts_to_page_worlds() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let first_url = url::Url::parse("https://example.test/shared-preload-a").unwrap();
    let second_url = url::Url::parse("https://example.test/shared-preload-b").unwrap();

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
            "<!doctype html><body>first document-start owner</body>".to_owned(),
            crate::RendererDocumentOptions {
                ..Default::default()
            },
        )
        .await
        .expect("first shared-isolate document-start page should load");
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
            "<!doctype html><body>second document-start owner</body>".to_owned(),
            crate::RendererDocumentOptions {
                ..Default::default()
            },
        )
        .await
        .expect("second shared-isolate document-start page should load");
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

    let script = crate::DocumentStartScript {
        registry_key: None,
        devtools_session: None,
        source: r#"globalThis.__sharedPreloadOwner = "first-page";"#.to_owned(),
        world_name: Some("shared-preload-world".to_owned()),
        has_bidi_channel_argument: false,
        bidi_channel_handoffs: Vec::new(),
    };
    let first_preload_result = first_page
        .run_async_command(RendererPageCommand::AddDocumentStartScriptRuntimeActivity {
            inspector_session_id: None,
            script: script.clone(),
            run_immediately: true,
        })
        .await
        .expect("first page document-start script should run")
        .0;
    let RendererPageReply::DocumentStartScriptResult(Some((first_world_context_id, first_created))) =
        first_preload_result
    else {
        panic!("expected first document-start script to create an isolated world");
    };
    assert!(first_created);

    let second_world_context_id =
        create_isolated_world_for_test(&second_page, "shared-preload-world")
            .await
            .expect("second document-start isolated world should be created");
    let (first_preload_value, _) = first_page
        .run_async_command(RendererPageCommand::EvaluateExpressionInExecutionContext {
            execution_context_id: first_world_context_id,
            expression: r#"globalThis.__sharedPreloadOwner"#.to_owned(),
            await_promise: false,
        })
        .await
        .expect("first isolated world preload value should evaluate");
    assert_eq!(
        renderer_json_value(first_preload_value),
        Some(serde_json::json!("first-page"))
    );

    let (second_preload_value, _) = second_page
        .run_async_command(RendererPageCommand::EvaluateExpressionInExecutionContext {
            execution_context_id: second_world_context_id,
            expression: r#"globalThis.__sharedPreloadOwner ?? "absent""#.to_owned(),
            await_promise: false,
        })
        .await
        .expect("second isolated world preload value should evaluate");
    assert_eq!(
        renderer_json_value(second_preload_value),
        Some(serde_json::json!("absent")),
        "Page.addScriptToEvaluateOnNewDocument(worldName=...) state must not leak into another page's same-name isolated world on a shared document isolate"
    );

    first_page
        .close_async()
        .await
        .expect("first document-start page should close");
    second_page
        .close_async()
        .await
        .expect("second document-start page should close");
}
#[tokio::test(flavor = "multi_thread")]
async fn per_page_isolate_policy_keeps_stored_document_start_scripts_page_local() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let first_url = url::Url::parse("https://example.test/shared-stored-preload-a").unwrap();
    let second_url = url::Url::parse("https://example.test/shared-stored-preload-b").unwrap();

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
            "<!doctype html><body>first stored preload owner</body>".to_owned(),
            crate::RendererDocumentOptions {
                ..Default::default()
            },
        )
        .await
        .expect("first shared-isolate stored-preload page should load");
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
            "<!doctype html><body>second stored preload owner</body>".to_owned(),
            crate::RendererDocumentOptions {
                ..Default::default()
            },
        )
        .await
        .expect("second shared-isolate stored-preload page should load");
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

    let stored_script = crate::DocumentStartScript {
        registry_key: None,
        devtools_session: None,
        source: r#"globalThis.__sharedStoredPreloadOwner = "first-page";"#.to_owned(),
        world_name: Some("shared-stored-preload-world".to_owned()),
        has_bidi_channel_argument: false,
        bidi_channel_handoffs: Vec::new(),
    };
    set_stored_document_start_scripts_for_test(&first_page, vec![stored_script])
        .await
        .expect("first page stored document-start script should install");

    let first_replacement_url = "data:text/html;charset=utf-8,%3C!doctype%20html%3E%3Cbody%3Efirst%20stored%20replacement%3C/body%3E";
    let (first_navigation_reply, _) = first_page
        .run_async_command(
            RendererPageCommand::EvaluateExpressionAndFollowPendingNavigation {
                expression: format!(
                    r#"(() => {{
  location.href = {first_replacement_url:?};
  return "navigating-first";
}})()"#
                ),
                await_promise: false,
            },
        )
        .await
        .expect("first stored-preload page should navigate");
    assert_eq!(
        renderer_json_value(first_navigation_reply),
        Some(serde_json::json!("navigating-first"))
    );

    let first_world_context_id =
        create_isolated_world_for_test(&first_page, "shared-stored-preload-world")
            .await
            .expect("first stored-preload isolated world should be available");
    let (first_stored_preload_value, _) = first_page
        .run_async_command(RendererPageCommand::EvaluateExpressionInExecutionContext {
            execution_context_id: first_world_context_id,
            expression: r#"globalThis.__sharedStoredPreloadOwner ?? "absent""#.to_owned(),
            await_promise: false,
        })
        .await
        .expect("first stored-preload isolated world should evaluate");
    assert_eq!(
        renderer_json_value(first_stored_preload_value),
        Some(serde_json::json!("first-page"))
    );

    let second_replacement_url = "data:text/html;charset=utf-8,%3C!doctype%20html%3E%3Cbody%3Esecond%20stored%20replacement%3C/body%3E";
    let (second_navigation_reply, _) = second_page
        .run_async_command(
            RendererPageCommand::EvaluateExpressionAndFollowPendingNavigation {
                expression: format!(
                    r#"(() => {{
  location.href = {second_replacement_url:?};
  return "navigating-second";
}})()"#
                ),
                await_promise: false,
            },
        )
        .await
        .expect("second stored-preload page should navigate");
    assert_eq!(
        renderer_json_value(second_navigation_reply),
        Some(serde_json::json!("navigating-second"))
    );

    let second_world_context_id =
        create_isolated_world_for_test(&second_page, "shared-stored-preload-world")
            .await
            .expect("second stored-preload isolated world should be created");
    let (second_stored_preload_value, _) = second_page
        .run_async_command(RendererPageCommand::EvaluateExpressionInExecutionContext {
            execution_context_id: second_world_context_id,
            expression: r#"globalThis.__sharedStoredPreloadOwner ?? "absent""#.to_owned(),
            await_promise: false,
        })
        .await
        .expect("second stored-preload isolated world should evaluate");
    assert_eq!(
        renderer_json_value(second_stored_preload_value),
        Some(serde_json::json!("absent")),
        "stored Page.addScriptToEvaluateOnNewDocument(worldName=...) state must not leak into another page's later navigation on a shared document isolate"
    );

    first_page
        .close_async()
        .await
        .expect("first stored-preload page should close");
    second_page
        .close_async()
        .await
        .expect("second stored-preload page should close");
}
