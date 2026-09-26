use super::*;

#[tokio::test(flavor = "multi_thread")]
async fn per_page_isolate_policy_keeps_dedicated_worker_events_page_owned() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let first_url = url::Url::parse("https://example.test/shared-worker-events-a").unwrap();
    let second_url = url::Url::parse("https://example.test/shared-worker-events-b").unwrap();

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
            "<!doctype html><body>first worker owner</body>".to_owned(),
            crate::RendererDocumentOptions {
                ..Default::default()
            },
        )
        .await
        .expect("first shared-isolate worker-event page should load");
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
            "<!doctype html><body>second worker owner</body>".to_owned(),
            crate::RendererDocumentOptions {
                ..Default::default()
            },
        )
        .await
        .expect("second shared-isolate worker-event page should load");
    assert!(second_download.is_none());

    let first_testing = RendererPageTestingHandle::new_for_testing(&first_page);
    let second_testing = RendererPageTestingHandle::new_for_testing(&second_page);
    assert!(first_testing.shares_local_host(&second_testing));
    assert_eq!(
        second_testing
            .host_unique_document_isolate_count_async()
            .await
            .expect("shared dedicated-worker unique document isolate count"),
        2
    );

    let (installed, _) = second_page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"
(() => {
  globalThis.__lm_dedicated_worker_events = [];
  const messageSource = `
    postMessage([
      self instanceof DedicatedWorkerGlobalScope,
      typeof Window === "function" && self instanceof Window,
      typeof document
    ].join("|"));
  `;
  const messageWorker = new Worker(
    "data:text/javascript," + encodeURIComponent(messageSource)
  );
  messageWorker.onmessage = event => {
    globalThis.__lm_dedicated_worker_events.push("message:" + event.data);
  };

  const errorWorker = new Worker(
    "data:text/javascript," + encodeURIComponent("throw new Error('worker-boom')")
  );
  errorWorker.onerror = event => {
    globalThis.__lm_dedicated_worker_events.push(
      "error:" + event.type + ":" + event.message.includes("worker-boom")
    );
    event.preventDefault();
  };
  return "installed";
})()
"#
            .to_owned(),
            await_promise: false,
        })
        .await
        .expect("second page dedicated-worker probes should install");
    assert_eq!(
        renderer_json_value(installed),
        Some(serde_json::json!("installed"))
    );

    second_page
        .run_async_command(RendererPageCommand::WaitForScriptTruthy {
            expression: r#"globalThis.__lm_dedicated_worker_events?.length >= 2"#.to_owned(),
            timeout_ms: 2_000,
            loader: loader.clone(),
        })
        .await
        .expect("dedicated worker message and error events should arrive on page B");

    let (second_events, _) = second_page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"JSON.stringify(globalThis.__lm_dedicated_worker_events.slice().sort())"#
                .to_owned(),
            await_promise: false,
        })
        .await
        .expect("second page worker events should evaluate");
    assert_eq!(
        renderer_json_value(second_events),
        Some(serde_json::json!(
            "[\"error:error:true\",\"message:true|false|undefined\"]"
        )),
        "page B should receive worker message/error events from a worker global, not a page realm"
    );

    let (first_events, _) = first_page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"globalThis.__lm_dedicated_worker_events ?? "missing""#.to_owned(),
            await_promise: false,
        })
        .await
        .expect("first page worker event marker should evaluate");
    assert_eq!(
        renderer_json_value(first_events),
        Some(serde_json::json!("missing")),
        "page A must not receive page B's dedicated-worker events"
    );

    first_page
        .close_async()
        .await
        .expect("first worker-event page should close");
    second_page
        .close_async()
        .await
        .expect("second worker-event page should close");
}
#[tokio::test(flavor = "multi_thread")]
async fn per_page_isolate_policy_keeps_indexed_db_managers_page_owned() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let first_url = url::Url::parse("https://shared-idb.example/app-a").unwrap();
    let second_url = url::Url::parse("https://shared-idb.example/app-b").unwrap();
    let storage_key = moli_storage_key::MoliStorageKey::first_party_from_url(&first_url, None)
        .serialized_storage_key();
    let first_root = runtime_indexed_db_test_root("shared-isolate-a");
    let second_root = runtime_indexed_db_test_root("shared-isolate-b");
    let first_manager = crate::new_indexed_db_manager(Some(first_root.clone()))
        .expect("first indexedDB manager should initialize");
    let second_manager = crate::new_indexed_db_manager(Some(second_root.clone()))
        .expect("second indexedDB manager should initialize");

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
            "<!doctype html><body>first idb</body>".to_owned(),
            crate::RendererDocumentOptions {
                indexed_db_manager: Some(crate::downgrade_indexed_db_manager(&first_manager)),
                ..Default::default()
            },
        )
        .await
        .expect("first shared-isolate indexedDB page should load");
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
            "<!doctype html><body>second idb</body>".to_owned(),
            crate::RendererDocumentOptions {
                indexed_db_manager: Some(crate::downgrade_indexed_db_manager(&second_manager)),
                ..Default::default()
            },
        )
        .await
        .expect("second shared-isolate indexedDB page should load");
    assert!(second_download.is_none());

    let first_testing = RendererPageTestingHandle::new_for_testing(&first_page);
    let second_testing = RendererPageTestingHandle::new_for_testing(&second_page);
    assert!(first_testing.shares_local_host(&second_testing));
    assert_eq!(
        second_testing
            .host_unique_document_isolate_count_async()
            .await
            .expect("shared indexedDB unique document isolate count"),
        2
    );

    store_indexed_db_value_for_test(&first_page, &loader, "first").await;
    store_indexed_db_value_for_test(&second_page, &loader, "second").await;

    assert!(
        runtime_indexed_db_origin_file(&first_root, &storage_key).exists(),
        "first page must write through its own browser-context indexedDB manager"
    );
    assert!(
        runtime_indexed_db_origin_file(&second_root, &storage_key).exists(),
        "second page must write through its own browser-context indexedDB manager"
    );

    first_page
        .close_async()
        .await
        .expect("first indexedDB page should close");
    second_page
        .close_async()
        .await
        .expect("second indexedDB page should close");
    let _ = std::fs::remove_dir_all(first_root);
    let _ = std::fs::remove_dir_all(second_root);
}
#[tokio::test(flavor = "multi_thread")]
async fn per_page_isolate_policy_keeps_blob_urls_page_owned_after_other_page_close() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let first_url = url::Url::parse("https://example.test/shared-blob-a").unwrap();
    let second_url = url::Url::parse("https://example.test/shared-blob-b").unwrap();

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
            "<!doctype html><body>first blob owner</body>".to_owned(),
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
            "<!doctype html><body>second blob owner</body>".to_owned(),
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

    let (first_blob_url, _) = first_page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"URL.createObjectURL(new Blob(["first-owned"], { type: "text/plain" }))"#
                .to_owned(),
            await_promise: false,
        })
        .await
        .expect("first page blob URL should be created");
    assert!(
        renderer_json_value(first_blob_url)
            .and_then(|value| value.as_str().map(str::to_owned))
            .is_some_and(|url| url.starts_with("blob:https://example.test/")),
        "first page should create a blob URL"
    );

    let (second_blob_url, _) = second_page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression:
                r#"URL.createObjectURL(new Blob(["second-owned"], { type: "text/plain" }))"#
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
        .expect("first shared page should close");

    let (second_blob_text, _) = second_page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: format!("fetch({second_blob_url:?}).then(response => response.text())"),
            await_promise: true,
        })
        .await
        .expect("second page blob URL should remain fetchable after first page closes");
    assert_eq!(
        renderer_json_value(second_blob_text),
        Some(serde_json::json!("second-owned")),
        "closing page A must not clean page B's blob URL in a shared document isolate"
    );

    second_page
        .close_async()
        .await
        .expect("second shared page should close");
}
#[tokio::test(flavor = "multi_thread")]
async fn per_page_isolate_policy_drops_stale_worker_message_after_navigation_replacement() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let (base_url, worker_fetch_seen, release_worker_fetch_response, server) =
        spawn_gated_worker_message_server().await;
    let navigated_url = url::Url::parse(&format!("{base_url}/shared-stale-worker-message-a"))
        .expect("worker message source url");
    let peer_url = url::Url::parse(&format!("{base_url}/shared-stale-worker-message-b"))
        .expect("worker message peer url");

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
            "<!doctype html><body>stale worker message source</body>".to_owned(),
            crate::RendererDocumentOptions {
                ..Default::default()
            },
        )
        .await
        .expect("stale-worker-message source page should load");
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
            "<!doctype html><body>stale worker message peer</body>".to_owned(),
            crate::RendererDocumentOptions {
                ..Default::default()
            },
        )
        .await
        .expect("stale-worker-message peer page should load");
    assert!(peer_download.is_none());

    let navigated_testing = RendererPageTestingHandle::new_for_testing(&navigated_page);
    let peer_testing = RendererPageTestingHandle::new_for_testing(&peer_page);
    assert!(navigated_testing.shares_local_host(&peer_testing));
    assert_eq!(
        peer_testing
            .host_unique_document_isolate_count_async()
            .await
            .expect("shared stale-worker-message unique document isolate count"),
        2
    );

    let (installed, _) = navigated_page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"(() => {
  globalThis.__lm_stale_worker_message_events = [];
  globalThis.__lm_stale_worker_message_worker = new Worker("/stale-worker.js");
  globalThis.__lm_stale_worker_message_worker.onmessage = event => {
    const value = String(event.data);
    globalThis.__lm_stale_worker_message_events.push(value);
    if (value.startsWith("late:") || value.startsWith("error:")) {
      globalThis.__lm_stale_worker_message_mutated_replacement = value;
    }
  };
  return "installed";
})()"#
                .to_owned(),
            await_promise: false,
        })
        .await
        .expect("stale-worker-message probe should install");
    assert_eq!(
        renderer_json_value(installed),
        Some(serde_json::json!("installed"))
    );

    navigated_page
        .run_async_command(RendererPageCommand::WaitForScriptTruthy {
            expression: r#"globalThis.__lm_stale_worker_message_events?.includes("ready")"#
                .to_owned(),
            timeout_ms: 2_000,
            loader: loader.clone(),
        })
        .await
        .expect("stale-worker-message worker should report ready");

    let (scheduled_request, _) = navigated_page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"(() => {
  globalThis.__lm_stale_worker_message_worker.postMessage("schedule-late");
  return "schedule-requested";
})()"#
                .to_owned(),
            await_promise: false,
        })
        .await
        .expect("stale-worker-message worker fetch should schedule");
    assert_eq!(
        renderer_json_value(scheduled_request),
        Some(serde_json::json!("schedule-requested"))
    );
    navigated_page
        .run_async_command(RendererPageCommand::WaitForScriptTruthy {
            expression: r#"globalThis.__lm_stale_worker_message_events?.includes("scheduled")"#
                .to_owned(),
            timeout_ms: 2_000,
            loader: loader.clone(),
        })
        .await
        .expect("stale-worker-message worker should confirm schedule");
    tokio::time::timeout(Duration::from_secs(2), worker_fetch_seen)
        .await
        .expect("stale worker fetch request should reach the server before navigation")
        .expect("stale worker fetch request signal should send");

    let replacement_url = "data:text/html;charset=utf-8,%3C!doctype%20html%3E%3Cbody%3Eworker%20message%20replacement%20document%3C/body%3E";
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
        .expect("stale-worker-message page should navigate to replacement document");
    assert_eq!(
        renderer_json_value(navigation_reply),
        Some(serde_json::json!("navigating"))
    );
    assert_eq!(
        peer_testing
            .host_unique_document_isolate_count_async()
            .await
            .expect(
                "shared unique document isolate count after stale-worker-message navigation replacement"
            ),
        2,
        "worker-message navigation replacement must retain one distinct isolate for each live page"
    );

    release_worker_fetch_response
        .send(())
        .expect("stale worker fetch response release should send");
    tokio::time::timeout(Duration::from_secs(2), server)
        .await
        .expect("stale worker message server should finish after response release")
        .expect("stale worker message server task should not panic");

    let (_advance, _) = navigated_page
        .run_async_command(RendererPageCommand::MsToNextTimeout)
        .await
        .expect("replacement timer deadline should remain observable after stale worker response");

    let (replacement_marker, _) = navigated_page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"[
  document.body.textContent,
  globalThis.__lm_stale_worker_message_events
    ? globalThis.__lm_stale_worker_message_events.join(",")
    : "missing",
  globalThis.__lm_stale_worker_message_mutated_replacement ?? "not-mutated"
].join("|")"#
                .to_owned(),
            await_promise: false,
        })
        .await
        .expect("replacement stale-worker-message marker should evaluate");
    assert_eq!(
        renderer_json_value(replacement_marker),
        Some(serde_json::json!(
            "worker message replacement document|missing|not-mutated"
        )),
        "stale old-document worker completion/message must not mutate the replacement document"
    );

    let (peer_marker, _) = peer_page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"globalThis.__lm_stale_worker_message_events ?? "missing""#.to_owned(),
            await_promise: false,
        })
        .await
        .expect("peer stale-worker-message marker should evaluate");
    assert_eq!(
        renderer_json_value(peer_marker),
        Some(serde_json::json!("missing")),
        "stale worker message from page A must not route to peer page B"
    );

    navigated_page
        .close_async()
        .await
        .expect("stale-worker-message navigated page should close");
    peer_page
        .close_async()
        .await
        .expect("stale-worker-message peer page should close");
}
#[tokio::test(flavor = "multi_thread")]
async fn per_page_isolate_policy_keeps_shared_worker_alive_after_peer_page_close() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let first_url = url::Url::parse("https://example.test/shared-worker-a").unwrap();
    let second_url = url::Url::parse("https://example.test/shared-worker-b").unwrap();

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
            "<!doctype html><body>first shared worker client</body>".to_owned(),
            crate::RendererDocumentOptions {
                ..Default::default()
            },
        )
        .await
        .expect("first shared-isolate SharedWorker page should load");
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
            "<!doctype html><body>second shared worker client</body>".to_owned(),
            crate::RendererDocumentOptions {
                ..Default::default()
            },
        )
        .await
        .expect("second shared-isolate SharedWorker page should load");
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

    install_shared_worker_count_probe(&first_page, "shared-isolate-worker")
        .await
        .expect("first shared worker count probe should install");
    wait_for_shared_worker_probe_messages(&first_page, &loader, 1, "first shared worker connect")
        .await
        .expect("first shared worker should connect");
    assert_eq!(
        shared_worker_probe_messages(&first_page)
            .await
            .expect("first shared worker messages"),
        "1"
    );

    install_shared_worker_count_probe(&second_page, "shared-isolate-worker")
        .await
        .expect("second shared worker count probe should install");
    wait_for_shared_worker_probe_messages(&second_page, &loader, 1, "second shared worker connect")
        .await
        .expect("second shared worker should connect");
    assert_eq!(
        shared_worker_probe_messages(&second_page)
            .await
            .expect("second shared worker messages"),
        "2",
        "same-key SharedWorker clients in one browser-context runtime should share a running host"
    );
    assert_eq!(
        runtime
            .browser_context_runtime()
            .shared_worker_running_worker_isolate_count_for_diagnostics(),
        1,
        "two same-key page clients should still produce one SharedWorker worker isolate"
    );

    first_page
        .close_async()
        .await
        .expect("first shared worker page should close");
    assert_eq!(
        runtime
            .browser_context_runtime()
            .shared_worker_running_worker_isolate_count_for_diagnostics(),
        1,
        "closing one page client must not terminate a SharedWorker still used by another page"
    );

    request_shared_worker_probe_count(&second_page)
        .await
        .expect("second page should post count request after first closes");
    wait_for_shared_worker_probe_messages(
        &second_page,
        &loader,
        2,
        "second shared worker count after peer close",
    )
    .await
    .expect("second page should receive count after peer closes");
    assert_eq!(
        shared_worker_probe_messages(&second_page)
            .await
            .expect("second shared worker messages after peer close"),
        "2|2",
        "remaining page client should stay connected to the same running SharedWorker host"
    );

    second_page
        .close_async()
        .await
        .expect("second shared worker page should close");
}
#[tokio::test(flavor = "multi_thread")]
async fn per_page_isolate_policy_isolates_shared_workers_across_browser_context_runtimes() {
    let first_runtime = JsRuntime::initialize();
    let second_runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let page_url = url::Url::parse("https://example.test/shared-worker-cross-runtime").unwrap();

    let mut first_page = create_test_html_page(
        &first_runtime,
        &loader,
        page_url.clone(),
        "<!doctype html><body>first cross-runtime shared worker client</body>",
    )
    .await;
    let mut second_page = create_test_html_page(
        &second_runtime,
        &loader,
        page_url,
        "<!doctype html><body>second cross-runtime shared worker client</body>",
    )
    .await;

    install_shared_worker_count_probe(&first_page, "cross-runtime-worker")
        .await
        .expect("first cross-runtime shared worker count probe should install");
    wait_for_shared_worker_probe_messages(
        &first_page,
        &loader,
        1,
        "first cross-runtime shared worker connect",
    )
    .await
    .expect("first cross-runtime shared worker should connect");
    assert_eq!(
        shared_worker_probe_messages(&first_page)
            .await
            .expect("first cross-runtime shared worker messages"),
        "1"
    );

    install_shared_worker_count_probe(&second_page, "cross-runtime-worker")
        .await
        .expect("second cross-runtime shared worker count probe should install");
    wait_for_shared_worker_probe_messages(
        &second_page,
        &loader,
        1,
        "second cross-runtime shared worker connect",
    )
    .await
    .expect("second cross-runtime shared worker should connect");
    assert_eq!(
        shared_worker_probe_messages(&second_page)
            .await
            .expect("second cross-runtime shared worker messages"),
        "1",
        "same-key SharedWorker clients in different browser-context runtimes must not share a host"
    );

    request_shared_worker_probe_count(&first_page)
        .await
        .expect("first cross-runtime page should post count request");
    wait_for_shared_worker_probe_messages(
        &first_page,
        &loader,
        2,
        "first cross-runtime shared worker count after second runtime connects",
    )
    .await
    .expect("first cross-runtime page should receive count after second runtime connects");
    assert_eq!(
        shared_worker_probe_messages(&first_page)
            .await
            .expect("first cross-runtime shared worker messages after count request"),
        "1|1",
        "first runtime SharedWorker host must remain isolated from the second runtime host"
    );
    assert_eq!(
        first_runtime
            .browser_context_runtime()
            .shared_worker_running_worker_isolate_count_for_diagnostics(),
        1
    );
    assert_eq!(
        second_runtime
            .browser_context_runtime()
            .shared_worker_running_worker_isolate_count_for_diagnostics(),
        1
    );

    first_page
        .close_async()
        .await
        .expect("first cross-runtime shared worker page should close");
    second_page
        .close_async()
        .await
        .expect("second cross-runtime shared worker page should close");
}
#[tokio::test(flavor = "multi_thread")]
async fn per_page_isolate_policy_recreates_shared_worker_after_last_page_client_close() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let first_url = url::Url::parse("https://example.test/shared-worker-last-client-a").unwrap();
    let second_url = url::Url::parse("https://example.test/shared-worker-last-client-b").unwrap();

    let mut first_page = create_test_html_page(
        &runtime,
        &loader,
        first_url,
        "<!doctype html><body>first last-client shared worker client</body>",
    )
    .await;
    install_shared_worker_count_probe(&first_page, "last-client-fresh-worker")
        .await
        .expect("first last-client shared worker count probe should install");
    wait_for_shared_worker_probe_messages(&first_page, &loader, 1, "first last-client connect")
        .await
        .expect("first last-client shared worker should connect");
    assert_eq!(
        shared_worker_probe_messages(&first_page)
            .await
            .expect("first last-client shared worker messages"),
        "1"
    );
    first_page
        .close_async()
        .await
        .expect("first last-client shared worker page should close");

    let mut second_page = create_test_html_page(
        &runtime,
        &loader,
        second_url,
        "<!doctype html><body>second last-client shared worker client</body>",
    )
    .await;
    install_shared_worker_count_probe(&second_page, "last-client-fresh-worker")
        .await
        .expect("second last-client shared worker count probe should install");
    wait_for_shared_worker_probe_messages(
        &second_page,
        &loader,
        1,
        "fresh same-key shared worker after last client close",
    )
    .await
    .expect("fresh same-key shared worker should connect after last client close");
    assert_eq!(
        shared_worker_probe_messages(&second_page)
            .await
            .expect("second last-client shared worker messages"),
        "1",
        "closing the last page client should terminate the old host before the same key is constructed again"
    );

    second_page
        .close_async()
        .await
        .expect("second last-client shared worker page should close");
}
#[tokio::test(flavor = "multi_thread")]
async fn per_page_isolate_policy_keeps_opaque_document_shared_worker_storage_keys_distinct() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let document_url = url::Url::parse("data:text/html,<p>opaque</p>").unwrap();

    let mut first_page = create_test_html_page(
        &runtime,
        &loader,
        document_url.clone(),
        "<!doctype html><body>first opaque shared worker client</body>",
    )
    .await;
    let mut second_page = create_test_html_page(
        &runtime,
        &loader,
        document_url,
        "<!doctype html><body>second opaque shared worker client</body>",
    )
    .await;

    install_shared_worker_count_probe(&first_page, "opaque-storage-key-worker")
        .await
        .expect("first opaque shared worker count probe should install");
    wait_for_shared_worker_probe_messages(&first_page, &loader, 1, "first opaque connect")
        .await
        .expect("first opaque shared worker should connect");
    assert_eq!(
        shared_worker_probe_messages(&first_page)
            .await
            .expect("first opaque shared worker messages"),
        "1"
    );

    install_shared_worker_count_probe(&second_page, "opaque-storage-key-worker")
        .await
        .expect("second opaque shared worker count probe should install");
    wait_for_shared_worker_probe_messages(&second_page, &loader, 1, "second opaque connect")
        .await
        .expect("second opaque shared worker should connect");
    assert_eq!(
        shared_worker_probe_messages(&second_page)
            .await
            .expect("second opaque shared worker messages"),
        "1",
        "same-key SharedWorker clients from distinct opaque documents must not share a host"
    );

    request_shared_worker_probe_count(&first_page)
        .await
        .expect("first opaque page should post count request");
    wait_for_shared_worker_probe_messages(
        &first_page,
        &loader,
        2,
        "first opaque shared worker count after second opaque page connects",
    )
    .await
    .expect("first opaque page should receive count after second opaque page connects");
    assert_eq!(
        shared_worker_probe_messages(&first_page)
            .await
            .expect("first opaque shared worker messages after count request"),
        "1|1",
        "first opaque document SharedWorker host must remain isolated from the second opaque document host"
    );

    first_page
        .close_async()
        .await
        .expect("first opaque shared worker page should close");
    second_page
        .close_async()
        .await
        .expect("second opaque shared worker page should close");
}
#[tokio::test(flavor = "multi_thread")]
async fn per_page_isolate_policy_removes_only_navigated_shared_worker_client() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let first_url = url::Url::parse("https://example.test/shared-worker-navigation-a").unwrap();
    let second_url = url::Url::parse("https://example.test/shared-worker-navigation-b").unwrap();

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
            "<!doctype html><body>first shared worker navigation client</body>".to_owned(),
            crate::RendererDocumentOptions {
                ..Default::default()
            },
        )
        .await
        .expect("first shared-isolate SharedWorker navigation page should load");
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
            "<!doctype html><body>second shared worker navigation client</body>".to_owned(),
            crate::RendererDocumentOptions {
                ..Default::default()
            },
        )
        .await
        .expect("second shared-isolate SharedWorker navigation page should load");
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

    install_shared_worker_count_probe(&first_page, "shared-isolate-navigation-worker")
        .await
        .expect("first shared worker navigation count probe should install");
    wait_for_shared_worker_probe_messages(
        &first_page,
        &loader,
        1,
        "first shared worker navigation connect",
    )
    .await
    .expect("first shared worker navigation client should connect");
    assert_eq!(
        shared_worker_probe_messages(&first_page)
            .await
            .expect("first shared worker navigation messages"),
        "1"
    );

    install_shared_worker_count_probe(&second_page, "shared-isolate-navigation-worker")
        .await
        .expect("second shared worker navigation count probe should install");
    wait_for_shared_worker_probe_messages(
        &second_page,
        &loader,
        1,
        "second shared worker navigation connect",
    )
    .await
    .expect("second shared worker navigation client should connect");
    assert_eq!(
        shared_worker_probe_messages(&second_page)
            .await
            .expect("second shared worker navigation messages"),
        "2"
    );
    assert_eq!(
        runtime
            .browser_context_runtime()
            .shared_worker_running_worker_isolate_count_for_diagnostics(),
        1,
        "same-key SharedWorker clients should share one worker isolate before navigation"
    );

    let replacement_url = "data:text/html;charset=utf-8,%3C!doctype%20html%3E%3Cbody%3Ereplacement%20shared%20worker%20navigation%20document%3C/body%3E";
    let (navigation_reply, _) = first_page
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
        .expect("shared isolate peer navigation should replace only page A");
    assert_eq!(
        renderer_json_value(navigation_reply),
        Some(serde_json::json!("navigating"))
    );
    assert_eq!(
        second_testing
            .host_unique_document_isolate_count_async()
            .await
            .expect("shared unique document isolate count after peer navigation replacement"),
        2,
        "page navigation replacement must retain one distinct isolate for each live page"
    );
    assert_eq!(
        runtime
            .browser_context_runtime()
            .shared_worker_running_worker_isolate_count_for_diagnostics(),
        1,
        "navigating one page client must not terminate a SharedWorker still used by another page"
    );

    request_shared_worker_probe_count(&second_page)
        .await
        .expect("second page should post count request after peer navigation");
    wait_for_shared_worker_probe_messages(
        &second_page,
        &loader,
        2,
        "second shared worker count after peer navigation",
    )
    .await
    .expect("second page should receive count after peer navigation");
    assert_eq!(
        shared_worker_probe_messages(&second_page)
            .await
            .expect("second shared worker messages after peer navigation"),
        "2|2",
        "navigation replacement must remove only page A's client endpoint"
    );

    first_page
        .close_async()
        .await
        .expect("navigated shared worker page should close");
    second_page
        .close_async()
        .await
        .expect("second shared worker navigation page should close");
}
