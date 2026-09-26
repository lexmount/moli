use super::*;

#[tokio::test(flavor = "multi_thread")]
async fn owner_loop_runs_file_reading_callback_without_retry_command() {
    let runtime = JsRuntime::initialize();
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default())
        .expect("default resource request client");
    let (base_url, callback_request_seen, release_callback_response, server) =
        spawn_owner_wake_gated_server_with_content_type(
            "/file-reading-owner-loop",
            "ok",
            "text/plain; charset=utf-8",
        )
        .await;
    let url = url::Url::parse(&format!("{base_url}/page")).expect("page URL");
    let mut page = create_test_html_page(
        &runtime,
        &loader,
        url,
        r#"<!doctype html><body>
<div id="drop" style="width: 100px; height: 100px">drop</div>
<script>
document.getElementById("drop").addEventListener("drop", event => {
  const reader =
    event.dataTransfer.items[0].webkitGetAsEntry().createReader();
  reader.readEntries(entries => {
    fetch("/file-reading-owner-loop", {
      method: "POST",
      body: String(entries.length)
    });
  });
});
</script>
</body>"#,
    )
    .await;

    let (reply, _) = page
        .run_async_command(RendererPageCommand::DispatchDragEventAtPoint {
            x: 10.0,
            y: 10.0,
            event_name: "drop".to_owned(),
            data: RendererDragData {
                items: Vec::new(),
                files: Vec::new(),
                directories: vec![RendererDraggedDirectory {
                    name: "fixture".to_owned(),
                    files: vec![RendererDraggedFile {
                        bytes: b"body".to_vec(),
                        mime_type: "text/plain".to_owned(),
                        name: "entry.txt".to_owned(),
                        last_modified: 1.0,
                    }],
                    directories: Vec::new(),
                }],
                drag_operations_mask: 1,
            },
            modifiers: 0,
        })
        .await
        .expect("directory drop command should run");
    assert!(
        matches!(
            reply,
            RendererPageReply::InputDispatchOutcome(ref outcome) if outcome.handled
        ),
        "directory drop command should dispatch to the target"
    );

    tokio::time::timeout(Duration::from_secs(2), callback_request_seen)
        .await
        .expect("FileReading owner wake must run the callback without a retry command")
        .expect("callback request signal should remain open");
    release_callback_response
        .send(())
        .expect("callback response should release once");
    server
        .await
        .expect("FileReading owner-loop witness server should finish");
    page.close_async()
        .await
        .expect("FileReading owner-loop page should close");
}
#[tokio::test(flavor = "multi_thread")]
async fn owner_loop_runs_misc_platform_api_callback_without_retry_command() {
    let runtime = JsRuntime::initialize();
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default())
        .expect("default resource request client");
    let (base_url, callback_request_seen, release_callback_response, server) =
        spawn_owner_wake_gated_server_with_content_type(
            "/misc-platform-api-owner-loop",
            "ok",
            "text/plain; charset=utf-8",
        )
        .await;
    let url = url::Url::parse(&format!("{base_url}/page")).expect("page URL");
    let mut page =
        create_test_html_page(&runtime, &loader, url, "<!doctype html><body></body>").await;

    let (reply, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"
navigator.webkitTemporaryStorage.queryUsageAndQuota((usage, quota) => {
  fetch("/misc-platform-api-owner-loop", {
    method: "POST",
    body: `${usage}:${quota}`
  });
});
"queued"
"#
            .to_owned(),
            await_promise: false,
        })
        .await
        .expect("deprecated storage quota callback should queue");
    assert_eq!(
        renderer_json_value(reply),
        Some(serde_json::json!("queued"))
    );

    tokio::time::timeout(Duration::from_secs(2), callback_request_seen)
        .await
        .expect("MiscPlatformApi owner wake must run the callback without a retry command")
        .expect("callback request signal should remain open");
    release_callback_response
        .send(())
        .expect("callback response should release once");
    server
        .await
        .expect("MiscPlatformApi owner-loop witness server should finish");
    page.close_async()
        .await
        .expect("MiscPlatformApi owner-loop page should close");
}
#[tokio::test(flavor = "multi_thread")]
async fn child_frame_lifecycle_best_effort_observes_autonomous_page_turns() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let url = url::Url::parse("https://child-lifecycle-observer.test/page").unwrap();
    let mut page = create_test_html_page(&runtime, &loader, url, "<!doctype html>").await;

    page.run_async_command(RendererPageCommand::EvaluateExpression {
        expression: r#"
(() => {
  globalThis.__childLifecycleObserverEvents = [];
  const frame = document.createElement("iframe");
  frame.onload = () => globalThis.__childLifecycleObserverEvents.push("frameload");
  frame.srcdoc = `<script>
    parent.__childLifecycleObserverEvents.push("child-script:" + (globalThis === self));
  <\/script>`;
  document.body.appendChild(frame);
  return true;
})()
"#
        .to_owned(),
        await_promise: false,
    })
    .await
    .expect("child lifecycle setup should evaluate");

    let (reply, _) = page
        .run_async_command(
            RendererPageCommand::CompleteChildFrameLifecycleWorkBestEffort {
                timeout_ms: 2_000,
                loader: loader.clone(),
            },
        )
        .await
        .expect("child lifecycle observer should finish");
    assert!(
        matches!(reply, RendererPageReply::Bool(true)),
        "owner-scheduled child work should complete before the observer deadline"
    );

    let (reply, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: "__childLifecycleObserverEvents.join('|')".to_owned(),
            await_promise: false,
        })
        .await
        .expect("child lifecycle results should remain observable");
    assert_eq!(
        renderer_json_value(reply),
        Some(serde_json::json!("child-script:true|frameload"))
    );

    page.close_async()
        .await
        .expect("child lifecycle observer test page should close");
}
#[tokio::test(flavor = "multi_thread")]
async fn owner_scheduler_ignored_child_navigation_releases_parent_load() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let (download_base_url, download_server) = spawn_owner_wake_server_with_content_type(
        "/download.asis",
        "download me",
        "application/octet-stream",
        Duration::ZERO,
    )
    .await;
    let (completion_base_url, completion_server) = spawn_owner_wake_server_with_content_type(
        "/ignored-child-navigation-complete",
        "ok",
        "text/plain; charset=utf-8",
        Duration::ZERO,
    )
    .await;
    let download_url = format!("{download_base_url}/download.asis");
    let completion_url = format!("{completion_base_url}/ignored-child-navigation-complete");
    let page_url = url::Url::parse(&format!("{completion_base_url}/page")).expect("page URL");
    let html = format!(
        r#"<!doctype html><body><script>
globalThis.__ignoredChildNavigationEvents = [];
globalThis.__ignoredChildNavigationCompletionRequested = false;
const maybeCompleteIgnoredChildNavigation = () => {{
  const events = globalThis.__ignoredChildNavigationEvents;
  if (!globalThis.__ignoredChildNavigationCompletionRequested &&
      events.includes("timer") && events.includes("parent-load")) {{
    globalThis.__ignoredChildNavigationCompletionRequested = true;
    fetch({completion_url_literal});
  }}
}};
addEventListener("load", () => {{
  globalThis.__ignoredChildNavigationEvents.push("parent-load");
  maybeCompleteIgnoredChildNavigation();
}});
setTimeout(() => {{
  globalThis.__ignoredChildNavigationEvents.push("timer");
  maybeCompleteIgnoredChildNavigation();
}}, 0);
const frame = document.createElement("iframe");
frame.id = "download-frame";
frame.src = {download_url_literal};
document.body.appendChild(frame);
</script></body>"#,
        completion_url_literal =
            serde_json::to_string(&completion_url).expect("serialize completion URL"),
        download_url_literal =
            serde_json::to_string(&download_url).expect("serialize download URL"),
    );
    let mut page =
        create_test_html_page_at_document_commit(&runtime, &loader, page_url, &html).await;

    tokio::time::timeout(Duration::from_secs(2), download_server)
        .await
        .expect("unsupported child response should be requested")
        .expect("unsupported child response server should finish");
    if let Err(error) = tokio::time::timeout(Duration::from_secs(2), completion_server).await {
        let (state, _) = page
            .run_async_command(RendererPageCommand::EvaluateExpression {
                expression: format!(
                    r#"JSON.stringify({{
  readyState: document.readyState,
  childUrl: document.getElementById("download-frame").contentDocument.URL,
  resourceEntries: performance.getEntriesByType("resource")
    .filter(entry => entry.name === {download_url_literal}).length,
  events: globalThis.__ignoredChildNavigationEvents
}})"#,
                    download_url_literal = serde_json::to_string(&download_url)
                        .expect("serialize diagnostic download URL"),
                ),
                await_promise: false,
            })
            .await
            .expect("timed-out ignored-navigation state should remain observable");
        panic!(
            "ignored child navigation should autonomously release parent load: {error:?}; state={:?}",
            renderer_json_value(state)
        );
    }

    let (observed, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: format!(
                r#"JSON.stringify({{
  readyState: document.readyState,
  childUrl: document.getElementById("download-frame").contentDocument.URL,
  resourceEntries: performance.getEntriesByType("resource")
    .filter(entry => entry.name === {download_url_literal}).length,
  events: globalThis.__ignoredChildNavigationEvents
}})"#,
                download_url_literal =
                    serde_json::to_string(&download_url).expect("serialize download URL"),
            ),
            await_promise: false,
        })
        .await
        .expect("ignored child navigation outcome should remain observable");
    assert_eq!(
        renderer_json_value(observed),
        Some(serde_json::json!(
            r#"{"readyState":"complete","childUrl":"about:blank","resourceEntries":0,"events":["timer","parent-load"]}"#
        ))
    );

    page.close_async()
        .await
        .expect("ignored child navigation page should close");
}
#[tokio::test(flavor = "multi_thread")]
async fn owner_scheduler_continues_from_stale_to_latest_child_navigation_generation() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let (base_url, completion_server) = spawn_owner_wake_server_with_content_type(
        "/latest-child-navigation-complete",
        "ok",
        "text/plain; charset=utf-8",
        Duration::ZERO,
    )
    .await;
    let page_url = url::Url::parse(&format!("{base_url}/page")).expect("page URL");
    let html = r#"<!doctype html><body><script>
globalThis.__rapidChildNavigationLoads = 0;
const frame = document.createElement("iframe");
frame.id = "rapid-child-navigation";
frame.onload = () => {
  globalThis.__rapidChildNavigationLoads++;
  if (frame.contentDocument.body.textContent.trim() === "latest") {
    fetch("/latest-child-navigation-complete");
  }
};
frame.srcdoc = "<!doctype html><body>superseded</body>";
document.body.appendChild(frame);
frame.srcdoc = "<!doctype html><body>latest</body>";
</script></body>"#;
    let mut page =
        create_test_html_page_at_document_commit(&runtime, &loader, page_url, html).await;

    tokio::time::timeout(Duration::from_secs(2), completion_server)
        .await
        .expect(
            "the latest child generation should run after a stale FIFO head without another command",
        )
        .expect("latest child navigation completion server should finish");
    let (observed, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"JSON.stringify({
  text: document.getElementById("rapid-child-navigation").contentDocument.body.textContent.trim(),
  loads: globalThis.__rapidChildNavigationLoads
})"#
            .to_owned(),
            await_promise: false,
        })
        .await
        .expect("latest child navigation state should remain observable");
    assert_eq!(
        renderer_json_value(observed),
        Some(serde_json::json!(r#"{"text":"latest","loads":1}"#))
    );

    page.close_async()
        .await
        .expect("rapid child navigation test page should close");
}
#[tokio::test(flavor = "multi_thread")]
async fn owner_scheduler_completes_window_load_after_child_self_navigation() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let (base_url, completion_server) = spawn_owner_wake_server_with_content_type(
        "/owner-child-self-navigation-load-complete",
        "ok",
        "text/plain; charset=utf-8",
        Duration::ZERO,
    )
    .await;
    let page_url = url::Url::parse(&format!("{base_url}/page")).expect("page URL");
    let html = r#"<!doctype html><body><script>
globalThis.__childSelfNavigateEvents = [];
globalThis.__childSelfNavigateCompletionRequested = false;
const maybeCompleteChildSelfNavigation = () => {
  const events = globalThis.__childSelfNavigateEvents;
  if (!globalThis.__childSelfNavigateCompletionRequested &&
      events.includes("message:can navigate") &&
      events.includes("parent-load")) {
    globalThis.__childSelfNavigateCompletionRequested = true;
    fetch("/owner-child-self-navigation-load-complete");
  }
};
onmessage = event => {
  globalThis.__childSelfNavigateEvents.push(`message:${event.data}`);
  maybeCompleteChildSelfNavigation();
};
addEventListener("load", () => {
  globalThis.__childSelfNavigateEvents.push("parent-load");
  maybeCompleteChildSelfNavigation();
});
const frame = document.createElement("iframe");
frame.sandbox = "allow-scripts";
frame.srcdoc = `
  <!doctype html>
  <script>
    onload = () => {
      location.href = "data:text/html,<!doctype html><script>parent.postMessage('can navigate', '*')<\\/script>";
    };
  <\/script>`;
document.body.appendChild(frame);
</script></body>"#;
    let mut page =
        create_test_html_page_at_document_commit(&runtime, &loader, page_url, html).await;

    match tokio::time::timeout(Duration::from_secs(2), completion_server).await {
        Ok(server) => server.expect("child self-navigation completion server should finish"),
        Err(error) => {
            let (state, _) = page
                .run_async_command(RendererPageCommand::EvaluateExpression {
                    expression: r#"JSON.stringify({
  events: globalThis.__childSelfNavigateEvents,
  readyState: document.readyState,
  frameCount: document.querySelectorAll('iframe').length
})"#
                    .to_owned(),
                    await_promise: false,
                })
                .await
                .expect("timed-out child self-navigation state should remain observable");
            panic!(
                "child self-navigation and parent load should complete without another command: {error:?}; state={:?}",
                renderer_json_value(state)
            );
        }
    }
    let (observed, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: "globalThis.__childSelfNavigateEvents.join('|')".to_owned(),
            await_promise: false,
        })
        .await
        .expect("child self-navigation events should remain observable");
    assert_eq!(
        renderer_json_value(observed),
        Some(serde_json::json!("message:can navigate|parent-load"))
    );

    page.close_async()
        .await
        .expect("child self-navigation page should close");
}
#[tokio::test(flavor = "multi_thread")]
async fn owner_scheduler_completes_window_load_after_child_descendant_navigation() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let (base_url, completion_server) = spawn_owner_wake_server_with_content_type(
        "/owner-child-descendant-navigation-load-complete",
        "ok",
        "text/plain; charset=utf-8",
        Duration::ZERO,
    )
    .await;
    let page_url = url::Url::parse(&format!("{base_url}/page")).expect("page URL");
    let html = r#"<!doctype html><body><script>
globalThis.__childDescendantNavigateEvents = [];
globalThis.__childDescendantNavigateCompletionRequested = false;
globalThis.maybeCompleteChildDescendantNavigation = () => {
  const events = globalThis.__childDescendantNavigateEvents;
  if (!globalThis.__childDescendantNavigateCompletionRequested &&
      events.includes("descendant-load") &&
      events.includes("parent-load")) {
    globalThis.__childDescendantNavigateCompletionRequested = true;
    fetch("/owner-child-descendant-navigation-load-complete");
  }
};
addEventListener("load", () => {
  globalThis.__childDescendantNavigateEvents.push("parent-load");
  globalThis.maybeCompleteChildDescendantNavigation();
});
const frame = document.createElement("iframe");
frame.srcdoc = `
  <!doctype html>
  <iframe src="data:text/html,initial"></iframe>
  <script>
    onload = () => {
      const descendant = document.querySelector("iframe");
      descendant.onload = () => {
        parent.__childDescendantNavigateEvents.push("descendant-load");
        parent.maybeCompleteChildDescendantNavigation();
      };
      descendant.contentWindow.location.href = "data:text/html,done";
    };
  <\/script>`;
document.body.appendChild(frame);
</script></body>"#;
    let mut page =
        create_test_html_page_at_document_commit(&runtime, &loader, page_url, html).await;

    tokio::time::timeout(Duration::from_secs(2), completion_server)
        .await
        .expect(
            "child descendant navigation and parent load should complete without another command",
        )
        .expect("child descendant-navigation completion server should finish");
    let (observed, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: "globalThis.__childDescendantNavigateEvents.join('|')".to_owned(),
            await_promise: false,
        })
        .await
        .expect("child descendant-navigation events should remain observable");
    assert_eq!(
        renderer_json_value(observed),
        Some(serde_json::json!("descendant-load|parent-load"))
    );

    page.close_async()
        .await
        .expect("child descendant-navigation page should close");
}
#[tokio::test(flavor = "multi_thread")]
async fn owner_loop_applies_subresource_fetch_completion_without_wait_command() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let (base_url, server) = spawn_owner_service_worker_response_sequence(vec![
        ("/api", "text/plain; charset=utf-8", "owner-wake-body"),
        ("/effect", "text/plain; charset=utf-8", "ok"),
    ])
    .await;
    let page_url = url::Url::parse(&format!("{base_url}/page")).expect("page url");
    let (page, _, _creation_diagnostics, _creation_artifacts, pending_download) = runtime
        .create_html_page_from_response(
            page_url.clone(),
            page_url,
            None,
            false,
            0,
            200,
            vec![("content-type".to_owned(), b"text/html".to_vec())],
            &loader,
            crate::RendererWebStorageHandles::ephemeral(),
            "<!doctype html><body>owner wake</body>".to_owned(),
            crate::RendererDocumentOptions {
                ..Default::default()
            },
        )
        .await
        .expect("page should load");
    assert!(pending_download.is_none());

    let (scheduled, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"(() => {
  globalThis.__lm_owner_wake_fetch_marker = "pending";
  fetch("/api")
    .then(response => response.text())
    .then(text => {
      globalThis.__lm_owner_wake_fetch_marker = text;
      return fetch("/effect");
    });
  return "scheduled";
})()"#
                .to_owned(),
            await_promise: false,
        })
        .await
        .expect("fetch scheduling evaluate should run");
    assert_eq!(
        renderer_json_value(scheduled),
        Some(serde_json::json!("scheduled"))
    );

    tokio::time::timeout(Duration::from_secs(2), server)
        .await
        .expect("the fetch reaction must issue its effect request without another Page command")
        .expect("the subresource response server should finish");

    let (observed, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"globalThis.__lm_owner_wake_fetch_marker"#.to_owned(),
            await_promise: false,
        })
        .await
        .expect("completed fetch marker should remain observable");
    assert_eq!(
        renderer_json_value(observed),
        Some(serde_json::json!("owner-wake-body"))
    );
}
#[tokio::test(flavor = "multi_thread")]
async fn owner_scheduler_applies_main_parser_module_terminal_without_observation_command() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let OwnerMainModuleLivenessServer {
        base_url,
        module_request_seen,
        release_module_response,
        effect_request_seen,
        task: server,
    } = spawn_owner_main_module_liveness_server(
        "/owner-main-parser-module.js",
        r#"globalThis.__lm_owner_main_parser_module = "executed";
fetch("/owner-main-parser-module-effect");"#,
        "/owner-main-parser-module-effect",
    )
    .await;
    let page_url = url::Url::parse(&format!("{base_url}/page")).expect("page URL");
    let mut page = create_test_html_page(
        &runtime,
        &loader,
        page_url,
        "<!doctype html><body>initial main parser module page</body>",
    )
    .await;

    let replacement_html = format!(
        r#"<!doctype html><body>
<script type="module" src="{base_url}/owner-main-parser-module.js"></script>
</body>"#
    );
    let (scheduled, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: format!(
                "document.open(); document.write({replacement_html:?}); document.close(); 'scheduled'"
            ),
            await_promise: false,
        })
        .await
        .expect("document.write parser module should schedule");
    assert_eq!(
        renderer_json_value(scheduled),
        Some(serde_json::json!("scheduled"))
    );
    tokio::time::timeout(Duration::from_secs(2), module_request_seen)
        .await
        .expect("main parser module request should start without another command")
        .expect("main parser module request signal should remain open");
    release_module_response
        .send(())
        .expect("main parser module response should be released once");
    tokio::time::timeout(Duration::from_secs(2), effect_request_seen)
        .await
        .expect(
            "the producer wake and owner continuations should evaluate the parser module without another command",
        )
        .expect("main parser module effect signal should remain open");

    let (observed, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: "globalThis.__lm_owner_main_parser_module".to_owned(),
            await_promise: false,
        })
        .await
        .expect("completed main parser module marker should remain observable");
    assert_eq!(
        renderer_json_value(observed),
        Some(serde_json::json!("executed"))
    );

    page.close_async()
        .await
        .expect("main parser module owner-liveness page should close");
    server
        .await
        .expect("main parser module owner-liveness server should finish");
}
#[tokio::test(flavor = "multi_thread")]
async fn owner_scheduler_applies_main_parser_module_reaction_and_followup_without_observation_command()
 {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let OwnerMainModuleReactionLivenessServer {
        base_url,
        module_request_seen,
        release_module_response,
        evaluation_started,
        effect_request_seen,
        script_load_event_seen,
        task: server,
    } = spawn_owner_main_module_reaction_liveness_server().await;
    let page_url = url::Url::parse(&format!("{base_url}/page")).expect("page URL");
    let mut page = create_test_html_page(
        &runtime,
        &loader,
        page_url,
        "<!doctype html><body>main parser module reaction owner liveness</body>",
    )
    .await;

    let replacement_html = format!(
        r#"<!doctype html><body>
<script type="module" src="{base_url}/owner-main-tla-module.js" onload="fetch('/owner-main-tla-script-load')"></script>
</body>"#
    );
    let (scheduled, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: format!(
                "document.open(); document.write({replacement_html:?}); document.close(); 'scheduled'"
            ),
            await_promise: false,
        })
        .await
        .expect("main parser TLA module should schedule");
    assert_eq!(
        renderer_json_value(scheduled),
        Some(serde_json::json!("scheduled"))
    );
    tokio::time::timeout(Duration::from_secs(2), module_request_seen)
        .await
        .expect("main parser TLA module request should start")
        .expect("main parser TLA module request signal should remain open");
    release_module_response
        .send(())
        .expect("main parser TLA module response should release once");
    tokio::time::timeout(Duration::from_secs(2), evaluation_started)
        .await
        .expect("owner scheduler should start the TLA evaluation")
        .expect("TLA evaluation-start signal should remain open");

    let (resolved, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: "__resolveLmOwnerMainTla(); 'resolved'".to_owned(),
            await_promise: false,
        })
        .await
        .expect("TLA gate should resolve");
    assert_eq!(
        renderer_json_value(resolved),
        Some(serde_json::json!("resolved"))
    );

    tokio::time::timeout(Duration::from_secs(2), effect_request_seen)
        .await
        .expect("resolving the TLA gate should resume module evaluation without another command")
        .expect("main parser TLA effect signal should remain open");
    tokio::time::timeout(Duration::from_secs(2), script_load_event_seen)
        .await
        .expect(
            "typed module reaction and its parser-owned follow-up should dispatch the script load event without another command",
        )
        .expect("main parser TLA script-load signal should remain open");

    let (observed, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: "globalThis.__lmOwnerMainTlaState".to_owned(),
            await_promise: false,
        })
        .await
        .expect("completed main parser TLA marker should remain observable");
    assert_eq!(
        renderer_json_value(observed),
        Some(serde_json::json!("completed"))
    );

    page.close_async()
        .await
        .expect("main parser module-reaction liveness page should close");
    server
        .await
        .expect("main parser module-reaction liveness server should finish");
}
#[tokio::test(flavor = "multi_thread")]
async fn owner_scheduler_completes_runtime_module_graph_without_observation_command() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let OwnerMainModuleLivenessServer {
        base_url,
        module_request_seen,
        release_module_response,
        effect_request_seen,
        task: server,
    } = spawn_owner_main_module_liveness_server(
        "/owner-main-runtime-dependency.js",
        "export const dependency = true;",
        "/owner-main-runtime-module-effect",
    )
    .await;
    let page_url = url::Url::parse(&format!("{base_url}/page")).expect("page URL");
    let mut page = create_test_html_page(
        &runtime,
        &loader,
        page_url,
        "<!doctype html><body>runtime module owner liveness</body>",
    )
    .await;

    let (scheduled, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: format!(
                r#"(() => {{
  globalThis.__lm_owner_main_runtime_module = "pending";
  const script = document.createElement("script");
  script.type = "module";
  script.textContent = `
    import "{base_url}/owner-main-runtime-dependency.js";
    globalThis.__lm_owner_main_runtime_module = "executed";
    fetch("{base_url}/owner-main-runtime-module-effect");
  `;
  document.body.appendChild(script);
  return "scheduled";
}})()"#,
            ),
            await_promise: false,
        })
        .await
        .expect("runtime module should install");
    assert_eq!(
        renderer_json_value(scheduled),
        Some(serde_json::json!("scheduled"))
    );
    tokio::time::timeout(Duration::from_secs(2), module_request_seen)
        .await
        .expect("runtime module dependency request should start without another command")
        .expect("runtime module dependency request signal should remain open");
    release_module_response
        .send(())
        .expect("runtime module dependency response should be released once");
    tokio::time::timeout(Duration::from_secs(2), effect_request_seen)
        .await
        .expect(
            "producer wake and owner continuations should evaluate the runtime module without another command",
        )
        .expect("runtime module effect signal should remain open");

    let (observed, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: "globalThis.__lm_owner_main_runtime_module".to_owned(),
            await_promise: false,
        })
        .await
        .expect("completed runtime module marker should remain observable");
    assert_eq!(
        renderer_json_value(observed),
        Some(serde_json::json!("executed"))
    );

    page.close_async()
        .await
        .expect("runtime module owner-liveness page should close");
    server
        .await
        .expect("runtime module owner-liveness server should finish");
}
#[tokio::test(flavor = "multi_thread")]
async fn owner_scheduler_applies_child_module_reaction_and_followup_without_observation_command() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let OwnerInlineModuleReactionLivenessServer {
        base_url,
        evaluation_started,
        effect_request_seen,
        task: server,
    } = spawn_owner_inline_module_reaction_liveness_server(
        "/owner-child-tla-evaluation-started",
        "/owner-child-tla-effect",
    )
    .await;
    let page_url = url::Url::parse(&format!("{base_url}/page")).expect("page URL");
    let mut page = create_test_html_page(
        &runtime,
        &loader,
        page_url,
        "<!doctype html><body>child module reaction owner liveness</body>",
    )
    .await;
    let child_html = r#"<!doctype html><body><script type="module">
const ownerChildTlaGate = new Promise(resolve => {
  parent.__resolveLmOwnerChildTla = resolve;
});
fetch("/owner-child-tla-evaluation-started");
await ownerChildTlaGate;
parent.__lmOwnerChildTlaState = "completed";
fetch("/owner-child-tla-effect");
</script></body>"#;

    let (scheduled, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: format!(
                r#"(() => {{
  globalThis.__lmOwnerChildTlaState = "pending";
  const frame = document.createElement("iframe");
  frame.srcdoc = {child_html:?};
  document.body.appendChild(frame);
  return "scheduled";
}})()"#
            ),
            await_promise: false,
        })
        .await
        .expect("child TLA module should schedule");
    assert_eq!(
        renderer_json_value(scheduled),
        Some(serde_json::json!("scheduled"))
    );
    tokio::time::timeout(Duration::from_secs(2), evaluation_started)
        .await
        .expect("owner scheduler should start the child TLA evaluation")
        .expect("child TLA evaluation-start signal should remain open");
    page.run_async_command(RendererPageCommand::EvaluateExpression {
        expression: "__resolveLmOwnerChildTla(); 'resolved'".to_owned(),
        await_promise: false,
    })
    .await
    .expect("child TLA gate should resolve");

    tokio::time::timeout(Duration::from_secs(2), effect_request_seen)
        .await
        .expect(
            "typed child module reaction and DocumentScriptReady follow-up should run without another command",
        )
        .expect("child TLA effect signal should remain open");

    let (observed, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: "globalThis.__lmOwnerChildTlaState".to_owned(),
            await_promise: false,
        })
        .await
        .expect("completed child TLA marker should remain observable");
    assert_eq!(
        renderer_json_value(observed),
        Some(serde_json::json!("completed"))
    );
    page.close_async()
        .await
        .expect("child module-reaction liveness page should close");
    server
        .await
        .expect("child module-reaction liveness server should finish");
}
#[tokio::test(flavor = "multi_thread")]
async fn owner_scheduler_applies_main_modulepreload_terminal_without_observation_command() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let OwnerModulepreloadLivenessServer {
        base_url,
        module_request_seen,
        release_module_response,
        effect_request_seen,
        task: server,
    } = spawn_owner_modulepreload_liveness_server(
        "/owner-main-modulepreload.js",
        "export const ownerMainModulepreload = true;",
        "/owner-main-modulepreload-load-event",
    )
    .await;
    let page_url = url::Url::parse(&format!("{base_url}/page")).expect("page URL");
    let mut page = create_test_html_page(
        &runtime,
        &loader,
        page_url,
        "<!doctype html><body>main modulepreload owner liveness</body>",
    )
    .await;

    let replacement_html = format!(
        r#"<!doctype html><head>
<link rel="modulepreload"
      href="{base_url}/owner-main-modulepreload.js"
      onload="globalThis.__lm_owner_main_modulepreload = 'loaded'; fetch('{base_url}/owner-main-modulepreload-load-event')"
      onerror="globalThis.__lm_owner_main_modulepreload = 'failed'">
</head><body>replacement</body>"#
    );
    let (scheduled, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: format!(
                "document.open(); document.write({replacement_html:?}); document.close(); 'scheduled'"
            ),
            await_promise: false,
        })
        .await
        .expect("document.write modulepreload should schedule");
    assert_eq!(
        renderer_json_value(scheduled),
        Some(serde_json::json!("scheduled"))
    );
    tokio::time::timeout(Duration::from_secs(2), module_request_seen)
        .await
        .expect("main modulepreload request should start without another command")
        .expect("main modulepreload request signal should remain open");
    release_module_response
        .send(())
        .expect("main modulepreload response should be released once");
    tokio::time::timeout(Duration::from_secs(2), effect_request_seen)
        .await
        .expect(
            "typed terminal and link-event continuations should finish without an observation command",
        )
        .expect("main modulepreload effect signal should remain open");

    let (observed, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: "globalThis.__lm_owner_main_modulepreload".to_owned(),
            await_promise: false,
        })
        .await
        .expect("main modulepreload load marker should remain observable");
    assert_eq!(
        renderer_json_value(observed),
        Some(serde_json::json!("loaded"))
    );

    page.close_async()
        .await
        .expect("main modulepreload owner-liveness page should close");
    server
        .await
        .expect("main modulepreload owner-liveness server should finish");
}
#[tokio::test(flavor = "multi_thread")]
async fn owner_scheduler_runs_joined_main_modulepreload_graph_without_observation_command() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let OwnerModulepreloadLivenessServer {
        base_url,
        module_request_seen,
        release_module_response,
        effect_request_seen,
        task: server,
    } = spawn_owner_modulepreload_liveness_server(
        "/owner-joined-main-module.js",
        r#"globalThis.__lmJoinedMainModulepreloadEvents.push("module");
globalThis.__lmMaybeFinishJoinedMainModulepreload();"#,
        "/owner-joined-main-module-executed",
    )
    .await;
    let page_url = url::Url::parse(&format!("{base_url}/page")).expect("page URL");
    let mut page = create_test_html_page(
        &runtime,
        &loader,
        page_url,
        "<!doctype html><body>joined main modulepreload owner liveness</body>",
    )
    .await;

    let replacement_html = format!(
        r#"<!doctype html><head>
<script>
globalThis.__lmJoinedMainModulepreloadEvents = [];
globalThis.__lmJoinedMainModulepreloadDone = false;
globalThis.__lmMaybeFinishJoinedMainModulepreload = () => {{
  const events = globalThis.__lmJoinedMainModulepreloadEvents;
  if (!globalThis.__lmJoinedMainModulepreloadDone &&
      events.includes("preload-load") && events.includes("module")) {{
    globalThis.__lmJoinedMainModulepreloadDone = true;
    fetch("/owner-joined-main-module-executed");
  }}
}};
</script>
<link rel="modulepreload"
      href="{base_url}/owner-joined-main-module.js"
      onload="globalThis.__lmJoinedMainModulepreloadEvents.push('preload-load'); globalThis.__lmMaybeFinishJoinedMainModulepreload()">
<script type="module" src="{base_url}/owner-joined-main-module.js"></script>
</head><body>replacement</body>"#
    );
    let (scheduled, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: format!(
                "document.open(); document.write({replacement_html:?}); document.close(); 'scheduled'"
            ),
            await_promise: false,
        })
        .await
        .expect("joined main modulepreload fixture should schedule");
    assert_eq!(
        renderer_json_value(scheduled),
        Some(serde_json::json!("scheduled"))
    );
    tokio::time::timeout(Duration::from_secs(2), module_request_seen)
        .await
        .expect("main modulepreload should own the joined root fetch")
        .expect("joined main module request signal should remain open");
    release_module_response
        .send(())
        .expect("joined main module response should be released once");
    tokio::time::timeout(Duration::from_secs(2), effect_request_seen)
        .await
        .expect("joined main module graph should finish through owner continuations")
        .expect("joined main module effect signal should remain open");

    let (events, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: "globalThis.__lmJoinedMainModulepreloadEvents.join('|')".to_owned(),
            await_promise: false,
        })
        .await
        .expect("joined main modulepreload ordering should remain observable");
    assert!(
        matches!(
            renderer_json_value(events),
            Some(value)
                if value == serde_json::json!("preload-load|module")
                    || value == serde_json::json!("module|preload-load")
        ),
        "module-map terminal must autonomously resume both clients; Chromium does not guarantee their relative order"
    );

    page.close_async()
        .await
        .expect("joined main modulepreload owner-liveness page should close");
    server
        .await
        .expect("joined main modulepreload owner-liveness server should finish");
}
#[tokio::test(flavor = "multi_thread")]
async fn owner_scheduler_applies_production_child_module_terminal() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let OwnerChildModuleGraphServer {
        base_url,
        root_request_seen,
        release_root_response,
        dependency_request_seen,
        release_dependency_response,
        effect_request_seen,
        task: server,
    } = spawn_owner_child_module_graph_server().await;
    let page_url = url::Url::parse(&format!("{base_url}/page")).expect("page url");
    let mut page = create_test_html_page(
        &runtime,
        &loader,
        page_url,
        "<!doctype html><body>child module owner turn</body>",
    )
    .await;
    let (scheduled, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"(() => {
  globalThis.__lm_owner_child_module_events = [];
  const frame = document.createElement("iframe");
  frame.srcdoc = `<script type="module" src="/child-owner-module.js"><\/script>`;
  document.body.appendChild(frame);
  return "scheduled";
})()"#
                .to_owned(),
            await_promise: false,
        })
        .await
        .expect("child parser module should schedule");
    assert_eq!(
        renderer_json_value(scheduled),
        Some(serde_json::json!("scheduled"))
    );
    tokio::time::timeout(Duration::from_secs(2), root_request_seen)
        .await
        .expect("child module root request should reach the test server")
        .expect("child module root request signal should remain open");
    release_root_response
        .send(())
        .expect("child module root response should be released once");

    tokio::time::timeout(Duration::from_secs(2), dependency_request_seen)
        .await
        .expect("authorized root application should start its static dependency request")
        .expect("child module dependency request signal should remain open");
    release_dependency_response
        .send(())
        .expect("child module dependency response should be released once");

    tokio::time::timeout(Duration::from_secs(2), effect_request_seen)
        .await
        .expect("child module evaluation must run without an observation command")
        .expect("child module effect request signal should remain open");

    let (observed, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: "globalThis.__lm_owner_child_module_events.join('|')".to_owned(),
            await_promise: false,
        })
        .await
        .expect("completed child module effects should remain observable");
    assert_eq!(
        renderer_json_value(observed),
        Some(serde_json::json!("dependency|root")),
        "the observation command must only read work already completed by owner turns"
    );

    page.close_async()
        .await
        .expect("child module owner-turn page should close");
    server.await.expect("child module server should finish");
}
#[tokio::test(flavor = "multi_thread")]
async fn owner_scheduler_dispatches_production_dedicated_worker_message_without_observation_command()
 {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let (base_url, effect_request_seen, release_effect_response, effect_server) =
        spawn_owner_wake_gated_server_with_content_type(
            "/owner-dedicated-worker-delivered",
            "ok",
            "text/plain; charset=utf-8",
        )
        .await;
    let page_url = url::Url::parse(&format!("{base_url}/page")).expect("page URL");
    let mut page = create_test_html_page(
        &runtime,
        &loader,
        page_url,
        "<!doctype html><body>DedicatedWorker owner turn</body>",
    )
    .await;
    let (scheduled, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"(() => {
  globalThis.__lmOwnerDedicatedWorkerEvents = [];
  const source = `postMessage("go")`;
  globalThis.__lmOwnerDedicatedWorker = new Worker(
    "data:text/javascript," + encodeURIComponent(source)
  );
  globalThis.__lmOwnerDedicatedWorker.onmessage = event => {
    globalThis.__lmOwnerDedicatedWorkerEvents.push("message:" + event.data);
    fetch("/owner-dedicated-worker-delivered");
  };
  return "scheduled";
})()"#
                .to_owned(),
            await_promise: false,
        })
        .await
        .expect("DedicatedWorker delivery should schedule");
    assert_eq!(
        renderer_json_value(scheduled),
        Some(serde_json::json!("scheduled"))
    );

    tokio::time::timeout(Duration::from_secs(2), effect_request_seen)
        .await
        .expect("owner scheduler should dispatch the typed DedicatedWorker task")
        .expect("DedicatedWorker handler effect signal should remain open");

    release_effect_response
        .send(())
        .expect("DedicatedWorker effect response should release once");
    effect_server
        .await
        .expect("DedicatedWorker effect server should finish");

    let (events, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: "globalThis.__lmOwnerDedicatedWorkerEvents.join('|')".to_owned(),
            await_promise: false,
        })
        .await
        .expect("DedicatedWorker handler result should remain observable");
    assert_eq!(
        renderer_json_value(events),
        Some(serde_json::json!("message:go"))
    );

    page.close_async()
        .await
        .expect("DedicatedWorker owner-liveness page should close");
}
#[tokio::test(flavor = "multi_thread")]
async fn owner_scheduler_dispatches_production_shared_worker_error_without_observation_command() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let (base_url, effect_request_seen, release_effect_response, effect_server) =
        spawn_owner_wake_gated_server_with_content_type(
            "/owner-shared-worker-error-delivered",
            "ok",
            "text/plain; charset=utf-8",
        )
        .await;
    let page_url = url::Url::parse(&format!("{base_url}/page")).expect("page URL");
    let mut page = create_test_html_page(
        &runtime,
        &loader,
        page_url,
        "<!doctype html><body>SharedWorker owner turn</body>",
    )
    .await;
    let (scheduled, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"(() => {
  globalThis.__lmOwnerSharedWorkerEvents = [];
  const brokenSource = "function( { broken syntax";
  globalThis.__lmOwnerSharedWorker = new SharedWorker(
    "data:text/javascript," + encodeURIComponent(brokenSource),
    "owner-shared-worker-error"
  );
  globalThis.__lmOwnerSharedWorker.onerror = event => {
    globalThis.__lmOwnerSharedWorkerEvents.push("error:" + event.type);
    fetch("/owner-shared-worker-error-delivered");
  };
  globalThis.__lmOwnerSharedWorker.port.start();
  return "scheduled";
})()"#
                .to_owned(),
            await_promise: false,
        })
        .await
        .expect("SharedWorker error should schedule");
    assert_eq!(
        renderer_json_value(scheduled),
        Some(serde_json::json!("scheduled"))
    );

    tokio::time::timeout(Duration::from_secs(2), effect_request_seen)
        .await
        .expect("owner scheduler should dispatch the typed SharedWorker client event")
        .expect("SharedWorker error-handler effect signal should remain open");

    release_effect_response
        .send(())
        .expect("SharedWorker effect response should release once");
    effect_server
        .await
        .expect("SharedWorker effect server should finish");

    let (events, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: "globalThis.__lmOwnerSharedWorkerEvents.join('|')".to_owned(),
            await_promise: false,
        })
        .await
        .expect("SharedWorker handler result should remain observable");
    assert_eq!(
        renderer_json_value(events),
        Some(serde_json::json!("error:error"))
    );

    page.close_async()
        .await
        .expect("SharedWorker owner-liveness page should close");
}
#[tokio::test(flavor = "multi_thread")]
async fn owner_scheduler_settles_production_webcrypto_without_observation_command() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let (base_url, effect_request_seen, release_effect_response, effect_server) =
        spawn_owner_wake_gated_server_with_content_type(
            "/owner-webcrypto-settled",
            "ok",
            "text/plain; charset=utf-8",
        )
        .await;
    let page_url = url::Url::parse(&format!("{base_url}/page")).expect("page URL");
    let mut page = create_test_html_page(
        &runtime,
        &loader,
        page_url,
        "<!doctype html><body>WebCrypto owner turn</body>",
    )
    .await;
    let (scheduled, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"(() => {
  globalThis.__lmOwnerWebCryptoResult = "pending";
  crypto.subtle.digest("SHA-256", new TextEncoder().encode("owner-webcrypto"))
    .then(bytes => {
      globalThis.__lmOwnerWebCryptoResult = String(bytes.byteLength);
      fetch("/owner-webcrypto-settled");
    }, error => {
      globalThis.__lmOwnerWebCryptoResult = `${error.name}:${error.message}`;
    });
  return "scheduled";
})()"#
                .to_owned(),
            await_promise: false,
        })
        .await
        .expect("WebCrypto digest should schedule");
    assert_eq!(
        renderer_json_value(scheduled),
        Some(serde_json::json!("scheduled"))
    );

    tokio::time::timeout(Duration::from_secs(2), effect_request_seen)
        .await
        .expect("owner scheduler should settle the typed WebCrypto task")
        .expect("WebCrypto Promise reaction effect signal should remain open");

    release_effect_response
        .send(())
        .expect("WebCrypto effect response should release once");
    effect_server
        .await
        .expect("WebCrypto effect server should finish");

    let (result, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: "globalThis.__lmOwnerWebCryptoResult".to_owned(),
            await_promise: false,
        })
        .await
        .expect("WebCrypto Promise result should remain observable");
    assert_eq!(renderer_json_value(result), Some(serde_json::json!("32")));

    page.close_async()
        .await
        .expect("WebCrypto owner-liveness page should close");
}
#[tokio::test(flavor = "multi_thread")]
async fn owner_scheduler_dispatches_production_broadcast_channel_delivery_without_observation_command()
 {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let (base_url, effect_request_seen, release_effect_response, effect_server) =
        spawn_owner_wake_gated_server_with_content_type(
            "/owner-broadcast-channel-delivered",
            "ok",
            "text/plain; charset=utf-8",
        )
        .await;
    let page_url = url::Url::parse(&format!("{base_url}/page")).expect("page URL");
    let mut page = create_test_html_page(
        &runtime,
        &loader,
        page_url,
        "<!doctype html><body>BroadcastChannel owner turn</body>",
    )
    .await;
    let (scheduled, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"(() => {
  globalThis.__lmOwnerBroadcastChannelEvents = [];
  globalThis.__lmOwnerBroadcastChannelReceiver = new BroadcastChannel("owner-delivery");
  globalThis.__lmOwnerBroadcastChannelReceiver.onmessage = event => {
    globalThis.__lmOwnerBroadcastChannelEvents.push("message:" + event.data);
    fetch("/owner-broadcast-channel-delivered");
  };
  globalThis.__lmOwnerBroadcastChannelSender = new BroadcastChannel("owner-delivery");
  globalThis.__lmOwnerBroadcastChannelSender.postMessage("go");
  return "scheduled";
})()"#
                .to_owned(),
            await_promise: false,
        })
        .await
        .expect("BroadcastChannel delivery should schedule");
    assert_eq!(
        renderer_json_value(scheduled),
        Some(serde_json::json!("scheduled"))
    );

    tokio::time::timeout(Duration::from_secs(2), effect_request_seen)
        .await
        .expect("owner scheduler should dispatch the typed BroadcastChannel task")
        .expect("BroadcastChannel handler effect signal should remain open");

    release_effect_response
        .send(())
        .expect("BroadcastChannel effect response should release once");
    effect_server
        .await
        .expect("BroadcastChannel effect server should finish");

    let (events, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: "globalThis.__lmOwnerBroadcastChannelEvents.join('|')".to_owned(),
            await_promise: false,
        })
        .await
        .expect("BroadcastChannel handler result should remain observable");
    assert_eq!(
        renderer_json_value(events),
        Some(serde_json::json!("message:go"))
    );

    page.close_async()
        .await
        .expect("BroadcastChannel owner-liveness page should close");
}
#[tokio::test(flavor = "multi_thread")]
async fn owner_scheduler_dispatches_storage_event_without_timer_or_observation_command() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let (base_url, effect_request_seen, release_effect_response, effect_server) =
        spawn_owner_wake_gated_server_with_content_type(
            "/owner-storage-event-delivered",
            "ok",
            "text/plain; charset=utf-8",
        )
        .await;
    let page_url = url::Url::parse(&format!("{base_url}/page")).expect("page URL");
    let mut page = create_test_html_page(
        &runtime,
        &loader,
        page_url,
        r#"<!doctype html><body>
<iframe id="recipient"></iframe>
<script>
globalThis.__lmOwnerStorageEvents = [];
recipient.contentWindow.addEventListener("storage", event => {
  parent.__lmOwnerStorageEvents.push(event.key + ":" + event.newValue);
  fetch("/owner-storage-event-delivered");
});
</script>
</body>"#,
    )
    .await;
    let (scheduled, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"localStorage.setItem("owner-storage-key", "go"); "scheduled""#
                .to_owned(),
            await_promise: false,
        })
        .await
        .expect("StorageEvent delivery should schedule");
    assert_eq!(
        renderer_json_value(scheduled),
        Some(serde_json::json!("scheduled"))
    );

    tokio::time::timeout(Duration::from_secs(2), effect_request_seen)
        .await
        .expect("owner scheduler should dispatch StorageEvent without another command")
        .expect("StorageEvent handler effect signal should remain open");

    release_effect_response
        .send(())
        .expect("StorageEvent effect response should release once");
    effect_server
        .await
        .expect("StorageEvent effect server should finish");

    let (events, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: "globalThis.__lmOwnerStorageEvents.join('|')".to_owned(),
            await_promise: false,
        })
        .await
        .expect("StorageEvent handler result should remain observable");
    assert_eq!(
        renderer_json_value(events),
        Some(serde_json::json!("owner-storage-key:go"))
    );

    page.close_async()
        .await
        .expect("StorageEvent owner-liveness page should close");
}
#[tokio::test(flavor = "multi_thread")]
async fn owner_scheduler_dispatches_hashchange_without_timer_driving_or_observation_command() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let (base_url, effect_request_seen, release_effect_response, effect_server) =
        spawn_owner_wake_gated_server_with_content_type(
            "/owner-hashchange-delivered",
            "ok",
            "text/plain; charset=utf-8",
        )
        .await;
    let page_url = url::Url::parse(&format!("{base_url}/page")).expect("page URL");
    let mut page = create_test_html_page(
        &runtime,
        &loader,
        page_url,
        r#"<!doctype html><body>
<script>
globalThis.__lmOwnerHashChanges = [];
addEventListener("hashchange", event => {
  __lmOwnerHashChanges.push(event.oldURL + "->" + event.newURL);
  fetch("/owner-hashchange-delivered");
});
</script>
</body>"#,
    )
    .await;
    let (scheduled, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r##"location.hash = "#typed"; "scheduled""##.to_owned(),
            await_promise: false,
        })
        .await
        .expect("fragment navigation should schedule hashchange");
    assert_eq!(
        renderer_json_value(scheduled),
        Some(serde_json::json!("scheduled"))
    );

    tokio::time::timeout(Duration::from_secs(2), effect_request_seen)
        .await
        .expect("owner scheduler should dispatch hashchange without another command")
        .expect("hashchange handler effect signal should remain open");

    release_effect_response
        .send(())
        .expect("hashchange effect response should release once");
    effect_server
        .await
        .expect("hashchange effect server should finish");

    let (events, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: "globalThis.__lmOwnerHashChanges.length.toString()".to_owned(),
            await_promise: false,
        })
        .await
        .expect("hashchange handler result should remain observable");
    assert_eq!(renderer_json_value(events), Some(serde_json::json!("1")));

    page.close_async()
        .await
        .expect("hashchange owner-liveness page should close");
}
#[tokio::test(flavor = "multi_thread")]
async fn owner_scheduler_runs_history_traversal_without_timer_or_observation_command() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let (base_url, effect_request_seen, release_effect_response, effect_server) =
        spawn_owner_wake_gated_server_with_content_type(
            "/owner-history-traversal-applied",
            "ok",
            "text/plain; charset=utf-8",
        )
        .await;
    let page_url = url::Url::parse(&format!("{base_url}/page")).expect("page URL");
    let mut page = create_test_html_page(
        &runtime,
        &loader,
        page_url,
        r##"<!doctype html><body>
<script>
history.pushState(null, "", "#entry");
globalThis.__lmOwnerHistoryTraversalLog = [];
addEventListener("popstate", () => {
  __lmOwnerHistoryTraversalLog.push("popstate:" + location.hash);
  fetch("/owner-history-traversal-applied");
}, { once: true });
</script>
</body>"##,
    )
    .await;
    let (scheduled, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"history.back(); "scheduled""#.to_owned(),
            await_promise: false,
        })
        .await
        .expect("history traversal should schedule");
    assert_eq!(
        renderer_json_value(scheduled),
        Some(serde_json::json!("scheduled"))
    );

    tokio::time::timeout(Duration::from_secs(2), effect_request_seen)
        .await
        .expect("owner scheduler should run history traversal without another command")
        .expect("history traversal handler effect signal should remain open");

    release_effect_response
        .send(())
        .expect("history traversal effect response should release once");
    effect_server
        .await
        .expect("history traversal effect server should finish");

    let (events, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: "globalThis.__lmOwnerHistoryTraversalLog.join('|')".to_owned(),
            await_promise: false,
        })
        .await
        .expect("history traversal handler result should remain observable");
    assert_eq!(
        renderer_json_value(events),
        Some(serde_json::json!("popstate:"))
    );

    page.close_async()
        .await
        .expect("history traversal owner-liveness page should close");
}
#[tokio::test(flavor = "multi_thread")]
async fn owner_scheduler_runs_document_scroll_rendering_update_without_timer_or_command() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let (base_url, effect_request_seen, release_effect_response, effect_server) =
        spawn_owner_wake_gated_server_with_content_type(
            "/owner-scroll-rendering-update-applied",
            "ok",
            "text/plain; charset=utf-8",
        )
        .await;
    let page_url = url::Url::parse(&format!("{base_url}/page")).expect("page URL");
    let mut page = create_test_html_page(
        &runtime,
        &loader,
        page_url,
        r#"<!doctype html><body>
<script>
globalThis.__lmOwnerScrollLog = [];
document.addEventListener("scroll", () => {
  __lmOwnerScrollLog.push("scroll:" + scrollY);
  fetch("/owner-scroll-rendering-update-applied");
}, { once: true });
document.addEventListener("scrollend", () => {
  __lmOwnerScrollLog.push("scrollend:" + scrollY);
}, { once: true });
</script>
</body>"#,
    )
    .await;
    let (scheduled, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"scrollTo(0, 25); "scheduled""#.to_owned(),
            await_promise: false,
        })
        .await
        .expect("Window scroll should enter the rendering source");
    assert_eq!(
        renderer_json_value(scheduled),
        Some(serde_json::json!("scheduled"))
    );

    tokio::time::timeout(Duration::from_secs(2), effect_request_seen)
        .await
        .expect("owner scheduler should dispatch scroll without another command")
        .expect("scroll handler effect signal should remain open");

    release_effect_response
        .send(())
        .expect("scroll effect response should release once");
    effect_server
        .await
        .expect("scroll effect server should finish");

    let (events, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: "globalThis.__lmOwnerScrollLog.join('|')".to_owned(),
            await_promise: false,
        })
        .await
        .expect("scroll handler result should remain observable");
    assert_eq!(
        renderer_json_value(events),
        Some(serde_json::json!("scroll:25|scrollend:25"))
    );

    page.close_async()
        .await
        .expect("rendering-update owner-liveness page should close");
}
#[tokio::test(flavor = "multi_thread")]
async fn owner_scheduler_runs_post_parse_autofocus_after_domcontentloaded_without_a_command() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let (base_url, effect_request_seen, release_effect_response, effect_server) =
        spawn_owner_wake_gated_server_with_content_type(
            "/owner-post-parse-autofocus-applied",
            "ok",
            "text/plain; charset=utf-8",
        )
        .await;
    let page_url = url::Url::parse(&format!("{base_url}/page")).expect("page URL");
    let mut page = create_test_html_page_at_document_commit(
        &runtime,
        &loader,
        page_url,
        r#"<!doctype html><body>
<input id="owner-autofocus" autofocus>
<script>
globalThis.__lmOwnerAutofocusLog = [];
document.addEventListener("DOMContentLoaded", () => {
  __lmOwnerAutofocusLog.push("dcl");
  Promise.resolve().then(() => __lmOwnerAutofocusLog.push("dcl-microtask"));
});
document.getElementById("owner-autofocus").addEventListener("focus", () => {
  __lmOwnerAutofocusLog.push("focus");
  Promise.resolve().then(() => __lmOwnerAutofocusLog.push("focus-microtask"));
  fetch("/owner-post-parse-autofocus-applied");
}, { once: true });
</script>
</body>"#,
    )
    .await;

    tokio::time::timeout(Duration::from_secs(2), effect_request_seen)
        .await
        .expect("owner scheduler should run autofocus without another command")
        .expect("autofocus handler effect signal should remain open");

    release_effect_response
        .send(())
        .expect("autofocus effect response should release once");
    effect_server
        .await
        .expect("autofocus effect server should finish");

    let (events, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: "globalThis.__lmOwnerAutofocusLog.join('|')".to_owned(),
            await_promise: false,
        })
        .await
        .expect("autofocus handler result should remain observable");
    assert_eq!(
        renderer_json_value(events),
        Some(serde_json::json!("dcl|dcl-microtask|focus|focus-microtask"))
    );

    page.close_async()
        .await
        .expect("autofocus rendering owner-liveness page should close");
}
#[tokio::test(flavor = "multi_thread")]
async fn owner_scheduler_runs_animation_start_rendering_update_without_timer_or_command() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let (base_url, effect_request_seen, release_effect_response, effect_server) =
        spawn_owner_wake_gated_server_with_content_type(
            "/owner-animation-rendering-update-applied",
            "ok",
            "text/plain; charset=utf-8",
        )
        .await;
    let page_url = url::Url::parse(&format!("{base_url}/page")).expect("page URL");
    let mut page = create_test_html_page(
        &runtime,
        &loader,
        page_url,
        r#"<!doctype html><head>
<style>
@keyframes owner-animation { from { left: 0px; } to { left: 10px; } }
#animated { position: relative; animation: owner-animation 1s linear; }
</style>
</head><body><div id="animated"></div>
<script>globalThis.__lmOwnerAnimationEvents = 0;</script>
</body>"#,
    )
    .await;
    let (scheduled, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"
document.getElementById("animated").addEventListener("animationstart", () => {
  __lmOwnerAnimationEvents++;
  fetch("/owner-animation-rendering-update-applied");
}, { once: true });
"scheduled"
"#
            .to_owned(),
            await_promise: false,
        })
        .await
        .expect("animation listener should enter the rendering source");
    assert_eq!(
        renderer_json_value(scheduled),
        Some(serde_json::json!("scheduled"))
    );

    tokio::time::timeout(Duration::from_secs(2), effect_request_seen)
        .await
        .expect("owner scheduler should dispatch animationstart without another command")
        .expect("animation handler effect signal should remain open");

    release_effect_response
        .send(())
        .expect("animation effect response should release once");
    effect_server
        .await
        .expect("animation effect server should finish");

    let (events, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: "String(globalThis.__lmOwnerAnimationEvents)".to_owned(),
            await_promise: false,
        })
        .await
        .expect("animation handler result should remain observable");
    assert_eq!(renderer_json_value(events), Some(serde_json::json!("1")));

    page.close_async()
        .await
        .expect("animation rendering owner-liveness page should close");
}
#[tokio::test(flavor = "multi_thread")]
async fn owner_scheduler_dispatches_dom_manipulation_fifo_without_timer_or_observation_command() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let (base_url, effect_request_seen, release_effect_response, effect_server) =
        spawn_owner_wake_gated_server_with_content_type(
            "/owner-dom-manipulation-applied",
            "ok",
            "text/plain; charset=utf-8",
        )
        .await;
    let page_url = url::Url::parse(&format!("{base_url}/page")).expect("page URL");
    let mut page = create_test_html_page(
        &runtime,
        &loader,
        page_url,
        r#"<!doctype html><body>
<details id="owner-toggle"><summary>summary</summary></details>
<script>
globalThis.__lmOwnerDomManipulationLog = [];
document.getElementById("owner-toggle").addEventListener("toggle", event => {
  __lmOwnerDomManipulationLog.push("toggle:" + event.oldState + "->" + event.newState);
  Promise.resolve().then(() => {
    __lmOwnerDomManipulationLog.push("toggle:microtask");
  });
}, { once: true });
</script>
</body>"#,
    )
    .await;
    let (scheduled, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"
const image = new Image();
image.addEventListener("load", () => {
  __lmOwnerDomManipulationLog.push("image:load");
  Promise.resolve().then(() => {
    __lmOwnerDomManipulationLog.push("image:microtask");
    fetch("/owner-dom-manipulation-applied");
  });
}, { once: true });
document.body.appendChild(image);
document.getElementById("owner-toggle").open = true;
image.src = "/not-fetched-by-policy.png";
"scheduled"
"#
            .to_owned(),
            await_promise: false,
        })
        .await
        .expect("toggle and image mutation should enter the DOM-manipulation source");
    assert_eq!(
        renderer_json_value(scheduled),
        Some(serde_json::json!("scheduled"))
    );

    tokio::time::timeout(Duration::from_secs(2), effect_request_seen)
        .await
        .expect("owner scheduler should dispatch the shared DOM FIFO without another command")
        .expect("image handler effect signal should remain open");

    release_effect_response
        .send(())
        .expect("DOM-manipulation effect response should release once");
    effect_server
        .await
        .expect("DOM-manipulation effect server should finish");

    let (events, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: "globalThis.__lmOwnerDomManipulationLog.join('|')".to_owned(),
            await_promise: false,
        })
        .await
        .expect("DOM-manipulation handler order should remain observable");
    assert_eq!(
        renderer_json_value(events),
        Some(serde_json::json!(
            "toggle:closed->open|toggle:microtask|image:load|image:microtask"
        ))
    );

    page.close_async()
        .await
        .expect("DOM-manipulation owner-liveness page should close");
}
#[tokio::test(flavor = "multi_thread")]
async fn owner_scheduler_completes_text_track_load_without_a_timer_or_observation_command() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let (base_url, effect_request_seen, release_effect_response, effect_server) =
        spawn_owner_wake_gated_server_with_content_type(
            "/owner-text-track-load-applied",
            "ok",
            "text/plain; charset=utf-8",
        )
        .await;
    let mut page = create_test_html_page(
        &runtime,
        &loader,
        url::Url::parse(&format!("{base_url}/page")).expect("page URL"),
        "<!doctype html><body></body>",
    )
    .await;
    let (scheduled, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"
const video = document.createElement("video");
const track = document.createElement("track");
track.default = true;
track.src = "data:text/vtt,WEBVTT";
globalThis.__lmOwnerTypedTrackEvents = [];
track.addEventListener("load", () => {
  __lmOwnerTypedTrackEvents.push(`load:${track.readyState}`);
  fetch("/owner-text-track-load-applied");
}, { once: true });
video.append(track);
document.body.append(video);
globalThis.__lmOwnerTypedDefaultTrack = track;
track.track.mode
"#
            .to_owned(),
            await_promise: false,
        })
        .await
        .expect("default track insertion should enter the shared DOM source");
    assert_eq!(
        renderer_json_value(scheduled),
        Some(serde_json::json!("disabled")),
        "the insertion command itself may not apply the later task"
    );

    tokio::time::timeout(Duration::from_secs(2), effect_request_seen)
        .await
        .expect("text-track load must dispatch without another command")
        .expect("text-track load effect signal should remain open");

    release_effect_response
        .send(())
        .expect("text-track effect response should release once");
    effect_server
        .await
        .expect("text-track owner-liveness server should finish");

    let (state, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"JSON.stringify({
  mode: globalThis.__lmOwnerTypedDefaultTrack.track.mode,
  readyState: globalThis.__lmOwnerTypedDefaultTrack.readyState,
  events: globalThis.__lmOwnerTypedTrackEvents
})"#
            .to_owned(),
            await_promise: false,
        })
        .await
        .expect("completed text-track load should remain observable");
    assert_eq!(
        renderer_json_value(state),
        Some(serde_json::json!(
            r#"{"mode":"showing","readyState":2,"events":["load:2"]}"#
        ))
    );

    page.close_async()
        .await
        .expect("text-track owner-liveness page should close");
}
#[tokio::test(flavor = "multi_thread")]
async fn owner_scheduler_dispatches_user_interaction_without_timer_or_observation_command() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let (base_url, effect_request_seen, release_effect_response, effect_server) =
        spawn_owner_wake_gated_server_with_content_type(
            "/owner-user-interaction-applied",
            "ok",
            "text/plain; charset=utf-8",
        )
        .await;
    let page_url = url::Url::parse(&format!("{base_url}/page")).expect("page URL");
    let mut page = create_test_html_page(
        &runtime,
        &loader,
        page_url,
        r#"<!doctype html><body>
<input id="owner-selection" value="abcd">
<script>
globalThis.__lmOwnerUserInteractionLog = [];
document.getElementById("owner-selection").addEventListener("select", event => {
  __lmOwnerUserInteractionLog.push(`${event.type}:${event.bubbles}`);
  fetch("/owner-user-interaction-applied");
}, { once: true });
</script>
</body>"#,
    )
    .await;
    let (scheduled, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression:
                r#"document.getElementById("owner-selection").setSelectionRange(0, 2); "scheduled""#
                    .to_owned(),
            await_promise: false,
        })
        .await
        .expect("selection mutation should schedule one user-interaction task");
    assert_eq!(
        renderer_json_value(scheduled),
        Some(serde_json::json!("scheduled"))
    );

    tokio::time::timeout(Duration::from_secs(2), effect_request_seen)
        .await
        .expect("owner scheduler should dispatch select without another command")
        .expect("select handler effect signal should remain open");

    release_effect_response
        .send(())
        .expect("user-interaction effect response should release once");
    effect_server
        .await
        .expect("user-interaction effect server should finish");

    let (events, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: "globalThis.__lmOwnerUserInteractionLog.join('|')".to_owned(),
            await_promise: false,
        })
        .await
        .expect("user-interaction handler result should remain observable");
    assert_eq!(
        renderer_json_value(events),
        Some(serde_json::json!("select:true"))
    );

    page.close_async()
        .await
        .expect("user-interaction owner-liveness page should close");
}
#[tokio::test(flavor = "multi_thread")]
async fn owner_scheduler_dispatches_media_events_without_timer_or_observation_command() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let (base_url, effect_request_seen, release_effect_response, effect_server) =
        spawn_owner_wake_gated_server_with_content_type(
            "/owner-media-event-applied",
            "ok",
            "text/plain; charset=utf-8",
        )
        .await;
    let page_url = url::Url::parse(&format!("{base_url}/page")).expect("page URL");
    let mut page = create_test_html_page(
        &runtime,
        &loader,
        page_url,
        r#"<!doctype html><body><video id="owner-media"></video>
<script>
globalThis.__lmOwnerMediaEventLog = [];
const media = document.getElementById("owner-media");
for (const type of ["seeking", "seeked"]) {
  media.addEventListener(type, () => {
    __lmOwnerMediaEventLog.push(type);
    if (type === "seeked") fetch("/owner-media-event-applied");
  });
}
</script></body>"#,
    )
    .await;
    let (scheduled, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"document.getElementById("owner-media").currentTime = 1; "scheduled""#
                .to_owned(),
            await_promise: false,
        })
        .await
        .expect("media seek should enter the media-element event source");
    assert_eq!(
        renderer_json_value(scheduled),
        Some(serde_json::json!("scheduled"))
    );

    tokio::time::timeout(Duration::from_secs(2), effect_request_seen)
        .await
        .expect("owner scheduler should dispatch both media events without another command")
        .expect("media event effect signal should remain open");

    release_effect_response
        .send(())
        .expect("media event effect response should release once");
    effect_server
        .await
        .expect("media event effect server should finish");

    let (events, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"JSON.stringify({
  events: globalThis.__lmOwnerMediaEventLog,
  seeking: document.getElementById("owner-media").seeking
})"#
            .to_owned(),
            await_promise: false,
        })
        .await
        .expect("media event result should remain observable");
    assert_eq!(
        renderer_json_value(events),
        Some(serde_json::json!(
            r#"{"events":["seeking","seeked"],"seeking":false}"#
        ))
    );

    page.close_async()
        .await
        .expect("media event owner-liveness page should close");
}
#[tokio::test(flavor = "multi_thread")]
async fn owner_scheduler_finishes_navigation_api_task_without_observation_command() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let (base_url, effect_request_seen, release_effect_response, effect_server) =
        spawn_owner_wake_gated_server_with_content_type(
            "/owner-navigation-api-task-applied",
            "ok",
            "text/plain; charset=utf-8",
        )
        .await;
    let page_url = url::Url::parse(&format!("{base_url}/page")).expect("page URL");
    let mut page = create_test_html_page(
        &runtime,
        &loader,
        page_url,
        "<!doctype html><body>Navigation API owner turn</body>",
    )
    .await;
    let (scheduled, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r##"(() => {
  globalThis.__lmOwnerNavigationApiTaskLog = [];
  navigation.onnavigatesuccess = () => {
    __lmOwnerNavigationApiTaskLog.push("success:" + location.hash);
    fetch("/owner-navigation-api-task-applied");
  };
  navigation.navigate("/next-document");
  const result = navigation.navigate("#replacement");
  result.finished.then(() => __lmOwnerNavigationApiTaskLog.push("finished"));
  return "scheduled";
})()"##
                .to_owned(),
            await_promise: false,
        })
        .await
        .expect("Navigation API finished task should schedule");
    assert_eq!(
        renderer_json_value(scheduled),
        Some(serde_json::json!("scheduled"))
    );

    tokio::time::timeout(Duration::from_secs(2), effect_request_seen)
        .await
        .expect("owner scheduler should run the Navigation API task without another command")
        .expect("Navigation API handler effect signal should remain open");

    release_effect_response
        .send(())
        .expect("Navigation API effect response should release once");
    effect_server
        .await
        .expect("Navigation API effect server should finish");

    let (events, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: "globalThis.__lmOwnerNavigationApiTaskLog.join('|')".to_owned(),
            await_promise: false,
        })
        .await
        .expect("Navigation API task result should remain observable");
    assert_eq!(
        renderer_json_value(events),
        Some(serde_json::json!("success:#replacement|finished"))
    );

    page.close_async()
        .await
        .expect("Navigation API owner-liveness page should close");
}
#[tokio::test(flavor = "multi_thread")]
async fn owner_scheduler_dispatches_production_window_message_without_observation_command() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let (base_url, effect_request_seen, release_effect_response, effect_server) =
        spawn_owner_wake_gated_server_with_content_type(
            "/owner-window-message-delivered",
            "ok",
            "text/plain; charset=utf-8",
        )
        .await;
    let page_url = url::Url::parse(&format!("{base_url}/page")).expect("page URL");
    let mut page = create_test_html_page(
        &runtime,
        &loader,
        page_url,
        "<!doctype html><body>Window.postMessage owner turn</body>",
    )
    .await;
    let (scheduled, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"(() => {
  globalThis.__lmOwnerWindowMessageEvents = [];
  onmessage = event => {
    __lmOwnerWindowMessageEvents.push("message:" + event.data);
    fetch("/owner-window-message-delivered");
  };
  postMessage("go", "*");
  return "scheduled";
})()"#
                .to_owned(),
            await_promise: false,
        })
        .await
        .expect("Window.postMessage should schedule");
    assert_eq!(
        renderer_json_value(scheduled),
        Some(serde_json::json!("scheduled"))
    );

    tokio::time::timeout(Duration::from_secs(2), effect_request_seen)
        .await
        .expect("owner scheduler should dispatch the typed Window.postMessage task")
        .expect("Window.postMessage handler effect signal should remain open");

    release_effect_response
        .send(())
        .expect("Window.postMessage effect response should release once");
    effect_server
        .await
        .expect("Window.postMessage effect server should finish");

    let (events, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: "globalThis.__lmOwnerWindowMessageEvents.join('|')".to_owned(),
            await_promise: false,
        })
        .await
        .expect("Window.postMessage handler result should remain observable");
    assert_eq!(
        renderer_json_value(events),
        Some(serde_json::json!("message:go"))
    );

    page.close_async()
        .await
        .expect("Window.postMessage owner-liveness page should close");
}
#[tokio::test(flavor = "multi_thread")]
async fn owner_scheduler_dispatches_production_message_port_without_observation_command() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let (base_url, effect_request_seen, release_effect_response, effect_server) =
        spawn_owner_wake_gated_server_with_content_type(
            "/owner-message-port-delivered",
            "ok",
            "text/plain; charset=utf-8",
        )
        .await;
    let page_url = url::Url::parse(&format!("{base_url}/page")).expect("page URL");
    let mut page = create_test_html_page(
        &runtime,
        &loader,
        page_url,
        "<!doctype html><body>MessagePort owner turn</body>",
    )
    .await;
    let (scheduled, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"(() => {
  globalThis.__lmOwnerMessagePortEvents = [];
  globalThis.__lmOwnerMessagePortChannel = new MessageChannel();
  const { port1, port2 } = __lmOwnerMessagePortChannel;
  port1.onmessage = event => {
    __lmOwnerMessagePortEvents.push("message:" + event.data);
    fetch("/owner-message-port-delivered");
  };
  port2.postMessage("go");
  return "scheduled";
})()"#
                .to_owned(),
            await_promise: false,
        })
        .await
        .expect("MessagePort delivery should schedule");
    assert_eq!(
        renderer_json_value(scheduled),
        Some(serde_json::json!("scheduled"))
    );

    tokio::time::timeout(Duration::from_secs(2), effect_request_seen)
        .await
        .expect("owner scheduler should dispatch the typed MessagePort task")
        .expect("MessagePort handler effect signal should remain open");

    release_effect_response
        .send(())
        .expect("MessagePort effect response should release once");
    effect_server
        .await
        .expect("MessagePort effect server should finish");

    let (events, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: "globalThis.__lmOwnerMessagePortEvents.join('|')".to_owned(),
            await_promise: false,
        })
        .await
        .expect("MessagePort handler result should remain observable");
    assert_eq!(
        renderer_json_value(events),
        Some(serde_json::json!("message:go"))
    );

    page.close_async()
        .await
        .expect("MessagePort owner-liveness page should close");
}
#[tokio::test(flavor = "multi_thread")]
async fn owner_scheduler_service_worker_responds_with_body_accessed_opaque_cache_response() {
    let (base_url, worker_server) = spawn_owner_service_worker_response_sequence(vec![(
        "/app/worker.js",
        "text/javascript; charset=utf-8",
        r#"
self.addEventListener("install", event => {
  event.waitUntil(Promise.resolve());
});
self.addEventListener("activate", event => {
  event.waitUntil(clients.claim());
});

function assertOpaqueResponse(response, label) {
  response.body;
  if (response.type !== "opaque" ||
      response.status !== 0 ||
      response.body !== null ||
      response.bodyUsed) {
    throw new Error(label + ":" + [
      response.type,
      response.status,
      response.body === null,
      response.bodyUsed
    ].join("/"));
  }
}

function maybeClone(response, cloneMode) {
  if (cloneMode === "clone-response") {
    const clone = response.clone();
    assertOpaqueResponse(clone, "clone-response");
    return clone;
  }
  if (cloneMode === "clone-unused") {
    const unused = response.clone();
    assertOpaqueResponse(unused, "clone-unused");
  }
  return response;
}

async function passThroughCacheIfNeeded(event, response, cacheMode) {
  if (cacheMode !== "cache") {
    return response;
  }
  const cacheName = event.request.url;
  await self.caches.delete(cacheName);
  const cache = await self.caches.open(cacheName);
  await cache.put(event.request, response);
  const matched = await cache.match(event.request.url);
  assertOpaqueResponse(matched, "matched");
  await self.caches.delete(cacheName);
  return matched;
}

self.addEventListener("fetch", event => {
  const url = new URL(event.request.url);
  if (!url.pathname.endsWith("/TestRequest")) {
    return;
  }
  event.respondWith(fetch(url.searchParams.get("jsonp"), { mode: "no-cors" })
    .then(async response => {
      assertOpaqueResponse(response, "original");
      const selected = maybeClone(response, url.searchParams.get("clone"));
      assertOpaqueResponse(selected, "selected");
      const finalResponse = await passThroughCacheIfNeeded(
        event,
        selected,
        url.searchParams.get("passThroughCache")
      );
      assertOpaqueResponse(finalResponse, "final");
      return finalResponse;
    }));
});
"#,
    )])
    .await;
    let (cross_base_url, cross_server) = spawn_owner_service_worker_response_sequence(vec![
        (
            "/app/respond-with-body-accessed-response.jsonp",
            "application/javascript",
            "globalThis.__serviceWorkerOpaqueBodyAccessedCallback('OK');",
        );
        6
    ])
    .await;
    let opaque_jsonp_url =
        format!("{cross_base_url}/app/respond-with-body-accessed-response.jsonp");
    let opaque_jsonp_url_literal =
        serde_json::to_string(&opaque_jsonp_url).expect("serialize opaque JSONP URL");

    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let page_url = url::Url::parse(&format!("{base_url}/app/page.html")).expect("page URL");
    let mut page = create_test_html_page(
        &runtime,
        &loader,
        page_url,
        "<!doctype html><body>service worker opaque response</body>",
    )
    .await;

    let (reply, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: format!(
                r#"
(async () => {{
  await navigator.serviceWorker.register("worker.js", {{ scope: "./" }});
  await navigator.serviceWorker.ready;
  const body = document.body;
  const run = (clone, cacheMode) => new Promise((resolve, reject) => {{
    const script = document.createElement("script");
    const callbackName = "__serviceWorkerOpaqueBodyAccessedCallback";
    globalThis[callbackName] = value => {{
      delete globalThis[callbackName];
      script.remove();
      resolve(["opaque", clone, cacheMode, value].join(":"));
    }};
    script.onerror = () => {{
      delete globalThis[callbackName];
      reject(new Error("script error:" + clone + "/" + cacheMode));
    }};
    script.src =
      "TestRequest?clone=" + clone +
      "&passThroughCache=" + cacheMode +
      "&jsonp=" + encodeURIComponent({opaque_jsonp_url_literal});
    body.appendChild(script);
  }});
  const runMode = async cacheMode => [
    await run("none", cacheMode),
    await run("clone-response", cacheMode),
    await run("clone-unused", cacheMode)
  ].join(",");
  return [await runMode("direct"), await runMode("cache")].join("|");
}})()
"#
            ),
            await_promise: true,
        })
        .await
        .expect("owner scheduler should settle the ServiceWorker response sequence");
    assert_eq!(
        renderer_json_value(reply),
        Some(serde_json::json!(concat!(
            "opaque:none:direct:OK,",
            "opaque:clone-response:direct:OK,",
            "opaque:clone-unused:direct:OK|",
            "opaque:none:cache:OK,",
            "opaque:clone-response:cache:OK,",
            "opaque:clone-unused:cache:OK"
        )))
    );

    worker_server
        .await
        .expect("service worker script server should finish");
    cross_server
        .await
        .expect("opaque response server should finish");
    page.close_async()
        .await
        .expect("ServiceWorker opaque-response page should close");
}
#[tokio::test(flavor = "multi_thread")]
async fn owner_scheduler_bounds_ordinary_starvation_of_document_lifecycle() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let (base_url, completion_request_seen, release_completion_response, server) =
        spawn_owner_wake_gated_server_with_content_type(
            "/page-turn-class-fairness-complete",
            "ok",
            "text/plain; charset=utf-8",
        )
        .await;
    let page_url = url::Url::parse(&format!("{base_url}/page")).expect("page URL");
    let html = r#"<!doctype html><body><script>
globalThis.__lmPageTurnClassFairness = {
  deliveries: 0,
  domContentLoadedAt: null,
  loadAt: null,
  completionRequested: false,
};
const maybeCompletePageTurnClassFairness = () => {
  const state = globalThis.__lmPageTurnClassFairness;
  if (state.deliveries === 64 && state.loadAt !== null && !state.completionRequested) {
    state.completionRequested = true;
    fetch('/page-turn-class-fairness-complete');
  }
};
const receiver = new BroadcastChannel('page-turn-class-fairness');
const sender = new BroadcastChannel('page-turn-class-fairness');
globalThis.__lmPageTurnClassFairnessChannels = { receiver, sender };
receiver.onmessage = () => {
  const state = globalThis.__lmPageTurnClassFairness;
  state.deliveries += 1;
  if (state.deliveries < 64) sender.postMessage(state.deliveries);
  maybeCompletePageTurnClassFairness();
};
document.addEventListener('DOMContentLoaded', () => {
  globalThis.__lmPageTurnClassFairness.domContentLoadedAt =
    globalThis.__lmPageTurnClassFairness.deliveries;
});
window.addEventListener('load', () => {
  globalThis.__lmPageTurnClassFairness.loadAt =
    globalThis.__lmPageTurnClassFairness.deliveries;
  maybeCompletePageTurnClassFairness();
});
sender.postMessage(0);
</script></body>"#;
    let mut page =
        create_test_html_page_at_document_commit(&runtime, &loader, page_url, html).await;

    tokio::time::timeout(Duration::from_secs(2), completion_request_seen)
        .await
        .expect("ordinary and lifecycle turns should both make autonomous progress")
        .expect("fairness completion request signal should remain open");
    let (observed, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: "JSON.stringify(globalThis.__lmPageTurnClassFairness)".to_owned(),
            await_promise: false,
        })
        .await
        .expect("page-turn fairness state should remain observable");
    let serialized = renderer_json_value(observed).expect("fairness state should serialize");
    let state: serde_json::Value = serde_json::from_str(
        serialized
            .as_str()
            .expect("fairness state should serialize as JSON"),
    )
    .expect("fairness state JSON should parse");
    assert_eq!(state["deliveries"], serde_json::json!(64));
    assert_eq!(state["completionRequested"], serde_json::json!(true));
    assert!(
        state["domContentLoadedAt"]
            .as_u64()
            .is_some_and(|at| at < 64),
        "sustained ordinary delivery must yield to DOMContentLoaded before draining: {state}"
    );
    assert!(
        state["loadAt"].as_u64().is_some_and(|at| at < 64),
        "sustained ordinary delivery must yield to load before draining: {state}"
    );

    release_completion_response
        .send(())
        .expect("fairness completion response should release once");
    server
        .await
        .expect("fairness completion server should finish");
    page.close_async()
        .await
        .expect("page-turn fairness page should close");
}
#[tokio::test(flavor = "multi_thread")]
async fn blocked_lifecycle_fairness_yield_preserves_ordinary_liveness() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    loader.set_image_fetch_enabled(true);
    let (resource_base_url, resource_request_seen, release_resource_response, resource_server) =
        spawn_owner_wake_gated_server_with_content_type(
            "/blocked-lifecycle-resource",
            "<svg xmlns='http://www.w3.org/2000/svg' width='1' height='1'/>",
            "image/svg+xml",
        )
        .await;
    let (ordinary_base_url, ordinary_request_seen, release_ordinary_response, ordinary_server) =
        spawn_owner_wake_gated_server_with_content_type(
            "/blocked-lifecycle-ordinary-complete",
            "ok",
            "text/plain; charset=utf-8",
        )
        .await;
    let (load_base_url, load_request_seen, release_load_response, load_server) =
        spawn_owner_wake_gated_server_with_content_type(
            "/blocked-lifecycle-load-complete",
            "ok",
            "text/plain; charset=utf-8",
        )
        .await;
    let page_url = url::Url::parse(&format!("{ordinary_base_url}/page")).expect("page URL");
    let resource_url = format!("{resource_base_url}/blocked-lifecycle-resource");
    let ordinary_completion_url =
        format!("{ordinary_base_url}/blocked-lifecycle-ordinary-complete");
    let load_completion_url = format!("{load_base_url}/blocked-lifecycle-load-complete");
    let html = format!(
        r#"<!doctype html><body>
<img src={resource_url_literal}>
<script>
globalThis.__lmBlockedLifecycleFairness = {{ deliveries: 0, load: false }};
const receiver = new BroadcastChannel('blocked-lifecycle-fairness');
const sender = new BroadcastChannel('blocked-lifecycle-fairness');
globalThis.__lmBlockedLifecycleFairnessChannels = {{ receiver, sender }};
receiver.onmessage = () => {{
  const state = globalThis.__lmBlockedLifecycleFairness;
  state.deliveries += 1;
  if (state.deliveries < 64) {{
    sender.postMessage(state.deliveries);
  }} else {{
    fetch({ordinary_completion_url_literal});
  }}
}};
window.addEventListener('load', () => {{
  globalThis.__lmBlockedLifecycleFairness.load = true;
  fetch({load_completion_url_literal});
}});
sender.postMessage(0);
</script></body>"#,
        resource_url_literal =
            serde_json::to_string(&resource_url).expect("serialize resource URL"),
        ordinary_completion_url_literal = serde_json::to_string(&ordinary_completion_url)
            .expect("serialize ordinary completion URL"),
        load_completion_url_literal =
            serde_json::to_string(&load_completion_url).expect("serialize load completion URL"),
    );
    let mut page =
        create_test_html_page_at_document_commit(&runtime, &loader, page_url, &html).await;

    tokio::time::timeout(Duration::from_secs(2), resource_request_seen)
        .await
        .expect("load-blocking resource should start")
        .expect("load-blocking resource signal should remain open");
    tokio::time::timeout(Duration::from_secs(2), ordinary_request_seen)
        .await
        .expect("ordinary source must continue after lifecycle reports Blocked")
        .expect("ordinary completion signal should remain open");
    let (before_release, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: "JSON.stringify(globalThis.__lmBlockedLifecycleFairness)".to_owned(),
            await_promise: false,
        })
        .await
        .expect("blocked-lifecycle fairness state should remain observable");
    assert_eq!(
        renderer_json_value(before_release),
        Some(serde_json::json!(r#"{"deliveries":64,"load":false}"#)),
        "ordinary work must drain while load remains blocked"
    );

    release_ordinary_response
        .send(())
        .expect("ordinary completion response should release once");
    ordinary_server
        .await
        .expect("ordinary completion server should finish");
    release_resource_response
        .send(())
        .expect("load-blocking response should release once");
    resource_server
        .await
        .expect("load-blocking resource server should finish");
    tokio::time::timeout(Duration::from_secs(2), load_request_seen)
        .await
        .expect("released resource should wake the exact lifecycle resident")
        .expect("load completion signal should remain open");
    release_load_response
        .send(())
        .expect("load completion response should release once");
    load_server
        .await
        .expect("load completion server should finish");

    page.close_async()
        .await
        .expect("blocked-lifecycle fairness page should close");
}
#[tokio::test(flavor = "multi_thread")]
async fn owner_scheduler_dispatches_universal_isolated_world_broadcast_channel_to_exact_realm() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let (base_url, effect_server) = spawn_owner_wake_server_with_content_type(
        "/owner-universal-world-broadcast-delivered",
        "ok",
        "text/plain; charset=utf-8",
        Duration::ZERO,
    )
    .await;
    let page_url = url::Url::parse(&format!("{base_url}/page")).expect("page URL");
    let mut page = create_test_html_page(
        &runtime,
        &loader,
        page_url,
        "<!doctype html><body>universal isolated BroadcastChannel world</body>",
    )
    .await;

    let (world_reply, _) = page
        .run_async_command(RendererPageCommand::CreateIsolatedWorld {
            name: "universal-broadcast-channel".to_owned(),
            grant_universal_access: true,
            frame_id: None,
        })
        .await
        .expect("universal isolated world should be created");
    let RendererPageReply::ExecutionContextId(world_context_id) = world_reply else {
        panic!("CreateIsolatedWorld should return its execution context id");
    };

    let (scheduled, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpressionInExecutionContext {
            execution_context_id: world_context_id,
            expression: r#"(() => {
  globalThis.__lmUniversalBroadcastEvents = [];
  globalThis.__lmUniversalBroadcastReceiver = new BroadcastChannel("universal-world-owner");
  globalThis.__lmUniversalBroadcastReceiver.onmessage = event => {
    __lmUniversalBroadcastEvents.push("message:" + event.data);
    fetch("/owner-universal-world-broadcast-delivered");
  };
  globalThis.__lmUniversalBroadcastSender = new BroadcastChannel("universal-world-owner");
  __lmUniversalBroadcastSender.postMessage("go");
  return "scheduled";
})()"#
                .to_owned(),
            await_promise: false,
        })
        .await
        .expect("isolated-world BroadcastChannel delivery should schedule");
    assert_eq!(
        renderer_json_value(scheduled),
        Some(serde_json::json!("scheduled"))
    );

    tokio::time::timeout(Duration::from_secs(2), effect_server)
        .await
        .expect("owner scheduler should dispatch the universal-world delivery")
        .expect("universal-world BroadcastChannel effect server should finish");

    let (world_events, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpressionInExecutionContext {
            execution_context_id: world_context_id,
            expression: "globalThis.__lmUniversalBroadcastEvents.join('|')".to_owned(),
            await_promise: false,
        })
        .await
        .expect("isolated-world BroadcastChannel result should remain observable");
    assert_eq!(
        renderer_json_value(world_events),
        Some(serde_json::json!("message:go"))
    );

    let (default_world_marker, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: "typeof globalThis.__lmUniversalBroadcastEvents".to_owned(),
            await_promise: false,
        })
        .await
        .expect("default world should remain observable");
    assert_eq!(
        renderer_json_value(default_world_marker),
        Some(serde_json::json!("undefined")),
        "delivery must remain bound to the accepting isolated realm"
    );

    page.close_async()
        .await
        .expect("universal-world BroadcastChannel page should close");
}
#[tokio::test(flavor = "multi_thread")]
async fn owner_scheduler_applies_child_document_terminal_without_observation_command() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let OwnerChildDocumentLivenessServer {
        base_url,
        document_request_seen,
        release_document_response,
        effect_request_seen,
        task: server,
    } = spawn_owner_child_document_liveness_server().await;
    let page_url = url::Url::parse(&format!("{base_url}/page")).expect("page URL");
    let mut page = create_test_html_page(
        &runtime,
        &loader,
        page_url,
        "<!doctype html><body>child document owner liveness</body>",
    )
    .await;

    let (scheduled, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"(() => {
  globalThis.__lm_owner_child_document = "pending";
  const frame = document.createElement("iframe");
  frame.src = "/owner-child-document.html";
  document.body.appendChild(frame);
  return "scheduled";
})()"#
                .to_owned(),
            await_promise: false,
        })
        .await
        .expect("external child document should schedule");
    assert_eq!(
        renderer_json_value(scheduled),
        Some(serde_json::json!("scheduled"))
    );
    tokio::time::timeout(Duration::from_secs(2), document_request_seen)
        .await
        .expect("owner scheduler should start the child navigation without another command")
        .expect("child document request signal should remain open");
    release_document_response
        .send(())
        .expect("child document response should be released once");
    tokio::time::timeout(Duration::from_secs(2), effect_request_seen)
        .await
        .expect("typed terminal and child script follow-up should run without observation")
        .expect("child document effect request signal should remain open");

    let (observed, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: "globalThis.__lm_owner_child_document".to_owned(),
            await_promise: false,
        })
        .await
        .expect("child document marker should remain observable");
    assert_eq!(
        renderer_json_value(observed),
        Some(serde_json::json!("committed"))
    );

    page.close_async()
        .await
        .expect("child document owner-liveness page should close");
    server
        .await
        .expect("child document owner-liveness server should finish");
}
#[tokio::test(flavor = "multi_thread")]
async fn owner_scheduler_runs_child_parser_classic_without_observation_command() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let OwnerChildClassicLivenessServer {
        base_url,
        source_request_seen,
        release_source_response,
        effect_request_seen,
        task: server,
    } = spawn_owner_child_classic_liveness_server().await;
    let page_url = url::Url::parse(&format!("{base_url}/page")).expect("page URL");
    let mut page = create_test_html_page(
        &runtime,
        &loader,
        page_url,
        "<!doctype html><body>child classic owner liveness</body>",
    )
    .await;

    let (scheduled, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: format!(
                r#"(() => {{
  globalThis.__lm_owner_child_classic = "pending";
  const frame = document.createElement("iframe");
  frame.srcdoc = `<base href="{base_url}/">
    <script src="{base_url}/owner-child-classic.js"><\/script>`;
  document.body.appendChild(frame);
  return "scheduled";
}})()"#
            ),
            await_promise: false,
        })
        .await
        .expect("child parser classic should schedule");
    assert_eq!(
        renderer_json_value(scheduled),
        Some(serde_json::json!("scheduled"))
    );

    tokio::time::timeout(Duration::from_secs(2), source_request_seen)
        .await
        .expect("typed classic fetch-start should reach the network without another command")
        .expect("classic source request signal should remain open");
    release_source_response
        .send(())
        .expect("classic source response should be released once");
    tokio::time::timeout(Duration::from_secs(2), effect_request_seen)
        .await
        .expect("typed completion and script execution should finish without observation commands")
        .expect("classic script effect request signal should remain open");

    let (observed, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: "globalThis.__lm_owner_child_classic".to_owned(),
            await_promise: false,
        })
        .await
        .expect("child classic marker should remain observable");
    assert_eq!(
        renderer_json_value(observed),
        Some(serde_json::json!("executed"))
    );

    page.close_async()
        .await
        .expect("child classic owner-liveness page should close");
    server
        .await
        .expect("child classic owner-liveness server should finish");
}
#[tokio::test(flavor = "multi_thread")]
async fn owner_scheduler_runs_child_dynamic_import_fanout_without_observation_command() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let OwnerDynamicImportLivenessServer {
        base_url,
        dynamic_root_request_seen,
        release_dynamic_root_response,
        effect_request_seen,
        task: server,
    } = spawn_owner_dynamic_import_liveness_server().await;
    let page_url = url::Url::parse(&format!("{base_url}/page")).expect("page url");
    let mut page = create_test_html_page(
        &runtime,
        &loader,
        page_url,
        "<!doctype html><body>dynamic import owner liveness</body>",
    )
    .await;

    let (scheduled, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: format!(
                r#"(() => {{
  globalThis.__lm_dynamic_owner_liveness = "pending";
  const frame = document.createElement("iframe");
  frame.srcdoc = `
    <base href="{base_url}/">
    <script type="module" src="{base_url}/dynamic-owner-entry.js"><\/script>
  `;
  document.body.appendChild(frame);
  return "scheduled";
}})()"#
            ),
            await_promise: false,
        })
        .await
        .expect("child dynamic import should schedule");
    assert_eq!(
        renderer_json_value(scheduled),
        Some(serde_json::json!("scheduled"))
    );

    tokio::time::timeout(Duration::from_secs(2), dynamic_root_request_seen)
        .await
        .expect("dynamic-import root request should start without another command")
        .expect("dynamic-import root request signal should remain open");
    release_dynamic_root_response
        .send(())
        .expect("dynamic-import root response should be released once");
    tokio::time::timeout(Duration::from_secs(2), effect_request_seen)
        .await
        .expect("dynamic-import fanout and evaluation should finish without observation commands")
        .expect("dynamic-import effect request signal should remain open");

    let (observed, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: "globalThis.__lm_dynamic_owner_liveness".to_owned(),
            await_promise: false,
        })
        .await
        .expect("completed dynamic-import marker should remain observable");
    assert_eq!(
        renderer_json_value(observed),
        Some(serde_json::json!("fulfilled:42"))
    );

    page.close_async()
        .await
        .expect("dynamic-import owner-liveness page should close");
    server
        .await
        .expect("dynamic-import owner-liveness server should finish");
}
#[tokio::test(flavor = "multi_thread")]
async fn owner_scheduler_starts_and_completes_child_modulepreload_without_observation_command() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let OwnerModulepreloadLivenessServer {
        base_url,
        module_request_seen,
        release_module_response,
        effect_request_seen,
        task: server,
    } = spawn_owner_modulepreload_liveness_server(
        "/owner-modulepreload.js",
        "export const ownerModulepreload = true;",
        "/owner-modulepreload-load-event",
    )
    .await;
    let page_url = url::Url::parse(&format!("{base_url}/page")).expect("page url");
    let mut page = create_test_html_page(
        &runtime,
        &loader,
        page_url,
        "<!doctype html><body>modulepreload owner liveness</body>",
    )
    .await;

    let (scheduled, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: format!(
                r#"(() => {{
  setTimeout(() => {{
    const frame = document.createElement("iframe");
    frame.srcdoc = `
      <link rel="modulepreload"
            href="{base_url}/owner-modulepreload.js"
            onload="fetch('{base_url}/owner-modulepreload-load-event')">
    `;
    document.body.appendChild(frame);
  }}, 0);
  return "scheduled";
}})()"#
            ),
            await_promise: false,
        })
        .await
        .expect("timer-backed modulepreload fixture should schedule");
    assert_eq!(
        renderer_json_value(scheduled),
        Some(serde_json::json!("scheduled"))
    );

    match tokio::time::timeout(Duration::from_secs(2), module_request_seen).await {
        Ok(result) => result.expect("modulepreload request signal should remain open"),
        Err(_) => {
            let (diagnostic, _) = page
                .run_async_command(RendererPageCommand::EvaluateExpression {
                    expression: r#"JSON.stringify((() => {
  const frame = document.querySelector("iframe");
  const child = frame?.contentDocument;
  return {
    frame: Boolean(frame),
    child: Boolean(child),
    readyState: child?.readyState ?? "missing",
    link: Boolean(child?.querySelector('link[rel="modulepreload"]'))
  };
})())"#
                        .to_owned(),
                    await_promise: false,
                })
                .await
                .expect("modulepreload timeout diagnostic should evaluate");
            panic!(
                "typed modulepreload start did not reach the network without another command; child state: {:?}",
                renderer_json_value(diagnostic)
            );
        }
    }
    release_module_response
        .send(())
        .expect("modulepreload response should be released once");
    tokio::time::timeout(Duration::from_secs(2), effect_request_seen)
        .await
        .expect("modulepreload completion and link event should run without another command")
        .expect("modulepreload link-event effect signal should remain open");

    page.close_async()
        .await
        .expect("modulepreload owner-liveness page should close");
    server
        .await
        .expect("modulepreload owner-liveness server should finish");
}
#[tokio::test(flavor = "multi_thread")]
async fn owner_scheduler_runs_joined_modulepreload_graph_without_observation_command() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let OwnerModulepreloadLivenessServer {
        base_url,
        module_request_seen,
        release_module_response,
        effect_request_seen,
        task: server,
    } = spawn_owner_modulepreload_liveness_server(
        "/owner-joined-module.js",
        r#"parent.__lmJoinedModulepreloadEvents.push("module");
fetch(parent.__lmJoinedModulepreloadEvents[0] === "preload-load"
  ? "/owner-joined-module-executed"
  : "/owner-joined-module-order-error");"#,
        "/owner-joined-module-executed",
    )
    .await;
    let page_url = url::Url::parse(&format!("{base_url}/page")).expect("page url");
    let mut page = create_test_html_page(
        &runtime,
        &loader,
        page_url,
        "<!doctype html><body>joined modulepreload owner liveness</body>",
    )
    .await;

    let (scheduled, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: format!(
                r#"(() => {{
  globalThis.__lmJoinedModulepreloadEvents = [];
  setTimeout(() => {{
    const frame = document.createElement("iframe");
    frame.srcdoc = `
      <link rel="modulepreload"
            href="{base_url}/owner-joined-module.js"
            onload="parent.__lmJoinedModulepreloadEvents.push('preload-load')">
      <script type="module" src="{base_url}/owner-joined-module.js"><\/script>
    `;
    document.body.appendChild(frame);
  }}, 0);
  return "scheduled";
}})()"#
            ),
            await_promise: false,
        })
        .await
        .expect("joined modulepreload fixture should schedule");
    assert_eq!(
        renderer_json_value(scheduled),
        Some(serde_json::json!("scheduled"))
    );

    tokio::time::timeout(Duration::from_secs(2), module_request_seen)
        .await
        .expect("modulepreload should own the joined root fetch without another command")
        .expect("joined module request signal should remain open");
    release_module_response
        .send(())
        .expect("joined module response should be released once");
    tokio::time::timeout(Duration::from_secs(2), effect_request_seen)
        .await
        .expect("joined parser root should execute from owner continuations")
        .expect("joined module execution effect signal should remain open");

    let (events, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: "globalThis.__lmJoinedModulepreloadEvents.join('|')".to_owned(),
            await_promise: false,
        })
        .await
        .expect("joined modulepreload ordering should remain observable after liveness proof");
    assert_eq!(
        renderer_json_value(events),
        Some(serde_json::json!("preload-load|module")),
        "link terminal fanout should precede execution of the same-URL joined module"
    );

    page.close_async()
        .await
        .expect("joined modulepreload owner-liveness page should close");
    server
        .await
        .expect("joined modulepreload owner-liveness server should finish");
}
#[tokio::test(flavor = "multi_thread")]
async fn owner_loop_ticks_page_timer_from_active_timer_index() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let (base_url, timer_request_seen, release_timer_response, server) =
        spawn_owner_wake_gated_server_with_content_type(
            "/timer-index-fired",
            "ok",
            "text/plain; charset=utf-8",
        )
        .await;
    let page_url = url::Url::parse(&format!("{base_url}/page")).expect("page url");
    let (page, _, _creation_diagnostics, _creation_artifacts, pending_download) = runtime
        .create_html_page_from_response(
            page_url.clone(),
            page_url,
            None,
            false,
            0,
            200,
            vec![("content-type".to_owned(), b"text/html".to_vec())],
            &loader,
            crate::RendererWebStorageHandles::ephemeral(),
            "<!doctype html><body>timer index</body>".to_owned(),
            crate::RendererDocumentOptions {
                ..Default::default()
            },
        )
        .await
        .expect("page should load");
    assert!(pending_download.is_none());

    let (scheduled, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"(() => {
  globalThis.__lm_owner_timer_index_marker = "pending";
  setTimeout(() => {
    globalThis.__lm_owner_timer_index_marker = "fired";
    fetch("/timer-index-fired");
  }, 25);
  return "scheduled";
})()"#
                .to_owned(),
            await_promise: false,
        })
        .await
        .expect("timer scheduling evaluate should run");
    assert_eq!(
        renderer_json_value(scheduled),
        Some(serde_json::json!("scheduled"))
    );

    tokio::time::timeout(Duration::from_secs(2), timer_request_seen)
        .await
        .expect("the active timer index must run the callback without another Page command")
        .expect("timer callback effect signal should remain open");
    release_timer_response
        .send(())
        .expect("timer callback response should release once");
    server.await.expect("timer callback server should finish");

    let (observed, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"globalThis.__lm_owner_timer_index_marker"#.to_owned(),
            await_promise: false,
        })
        .await
        .expect("marker evaluate should run after owner timer wake");
    assert_eq!(
        renderer_json_value(observed),
        Some(serde_json::json!("fired")),
        "owner loop did not tick the page timer from the active timer index"
    );
}
#[tokio::test(flavor = "multi_thread")]
async fn owner_loop_applies_indexed_db_task_without_an_observation_command() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let (base_url, effect_request_seen, release_effect_response, server) =
        spawn_owner_wake_gated_server_with_content_type(
            "/indexed-db-task-applied",
            "ok",
            "text/plain; charset=utf-8",
        )
        .await;
    let page_url = url::Url::parse(&format!("{base_url}/page")).expect("page url");
    let indexed_db_manager =
        crate::new_indexed_db_manager(None).expect("IndexedDB manager should initialize");
    let mut page = create_test_html_page_with_indexed_db_manager(
        &runtime,
        &loader,
        page_url,
        "<!doctype html><body>indexed db source</body>",
        &indexed_db_manager,
    )
    .await;

    let (scheduled, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"(() => {
  globalThis.__lmIndexedDbSourceProbe = { idb: "pending" };
  const request = indexedDB.open(`source-${Math.random()}`, 1);
  request.onupgradeneeded = () => { globalThis.__lmIndexedDbSourceProbe.idb = "upgrade"; };
  request.onerror = () => {
    globalThis.__lmIndexedDbSourceProbe.idb =
      `error:${request.error && request.error.name}`;
  };
  request.onsuccess = () => {
    globalThis.__lmIndexedDbSourceProbe.idb = "success";
    request.result.close();
    fetch("/indexed-db-task-applied");
  };
  return "scheduled";
})()"#
                .to_owned(),
            await_promise: false,
        })
        .await
        .expect("IndexedDB source probe should schedule");
    assert_eq!(
        renderer_json_value(scheduled),
        Some(serde_json::json!("scheduled"))
    );

    tokio::time::timeout(Duration::from_secs(2), effect_request_seen)
        .await
        .expect("the IndexedDB success task must run without another Page command")
        .expect("IndexedDB effect signal should remain open");
    release_effect_response
        .send(())
        .expect("IndexedDB effect response should release once");
    server.await.expect("IndexedDB effect server should finish");

    let (observed, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: "globalThis.__lmIndexedDbSourceProbe.idb".to_owned(),
            await_promise: false,
        })
        .await
        .expect("IndexedDB owner-turn result should remain observable");
    assert_eq!(
        renderer_json_value(observed),
        Some(serde_json::json!("success")),
        "the production wake must follow application of the concrete IDB task, not merely its enqueue"
    );

    page.close_async()
        .await
        .expect("IndexedDB owner-turn page should close");
}
#[tokio::test(flavor = "multi_thread")]
async fn owner_scheduler_applies_popup_terminal_from_stable_page_route() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let (base_url, server) = spawn_owner_wake_server_with_content_type(
        "/owner-popup.html",
        concat!(
            "<!doctype html><script>",
            "opener.__lm_owner_popup_events.push('response-script');",
            "opener.__lm_resolve_owner_popup('applied');",
            "</script><p id='owner-popup-body'>popup body</p>",
        ),
        "text/html",
        Duration::ZERO,
    )
    .await;
    let page_url = url::Url::parse(&format!("{base_url}/page")).expect("page url");
    let html = r#"<!doctype html><body><script>
globalThis.__lm_owner_popup_events = ['before-open'];
globalThis.__lm_owner_popup_applied = new Promise(resolve => {
  globalThis.__lm_resolve_owner_popup = resolve;
});
globalThis.__lm_owner_popup = open('/owner-popup.html', 'owner-popup');
globalThis.__lm_owner_popup_events.push('after-open');
</script></body>"#;

    let mut page = create_test_html_page(&runtime, &loader, page_url, html).await;
    server
        .await
        .expect("owner-routed popup response server should finish");

    let (reply, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"__lm_owner_popup_applied.then(() => JSON.stringify({
  events: __lm_owner_popup_events,
  body: __lm_owner_popup.document.getElementById('owner-popup-body').textContent
}))"#
                .to_owned(),
            await_promise: true,
        })
        .await
        .expect("owner scheduler should apply the typed popup terminal and resolve its observer");
    assert_eq!(
        renderer_json_value(reply),
        Some(serde_json::json!(
            r#"{"events":["before-open","after-open","response-script"],"body":"popup body"}"#
        )),
        "the public owner path must admit one popup wake, authorize its exact target, and apply the terminal"
    );

    page.close_async()
        .await
        .expect("owner-routed popup page should close");
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn lifecycle_record_precedes_handler_navigation_action_record() {
    let runtime = JsRuntime::initialize();
    let (activity_wake_tx, mut activity_wake_rx) = renderer_external_activity_test_channel();
    runtime.set_renderer_output_transport_sender(activity_wake_tx);
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let page_url =
        url::Url::parse("https://example.test/dcl-handler-navigation").expect("page url");
    let html = r#"<!doctype html>
<script>
document.addEventListener("DOMContentLoaded", () => {
  location.href = "https://example.test/final";
}, { once: true });
</script>
<main>DCL handler navigation</main>"#;
    let mut page = create_test_html_page_at_document_commit_with_navigation_dispatch(
        &runtime,
        &loader,
        page_url,
        html,
        RendererTopLevelNavigationDispatch::DelegateToBrowser,
        RendererNavigationReplyPolicy::ReturnWithPendingNavigation,
    )
    .await;

    // DocumentCommit returns before the owner resumes through DCL, so the
    // lifecycle/action tail may already be queued here. Keep that concrete
    // FIFO intact; the page and record filters below ignore creation output
    // that is unrelated to this witness.

    let observed = tokio::time::timeout(Duration::from_secs(2), async {
        let mut observed = Vec::new();
        while observed.len() < 2 {
            let publication = activity_wake_rx
                .recv()
                .await
                .expect("renderer output channel should stay open");
            if !publication_is_for_page(&publication, &page) {
                continue;
            }
            for record in publication.records() {
                match record.item() {
                    super::RendererOutputItem::Observation(
                        super::RendererProtocolObservation::DocumentLifecycle(event),
                    ) if event.kind
                        == RendererDocumentLifecycleEventKind::Milestone(
                            RendererDocumentLifecycleMilestone::DomContentLoaded,
                        ) =>
                    {
                        observed.push("lifecycle")
                    }
                    super::RendererOutputItem::OwnerAction(
                        super::RendererOwnerAction::TopLevelLocationNavigation(_),
                    ) => observed.push("action"),
                    _ => {}
                }
            }
        }
        observed
    })
    .await
    .expect("DCL handler navigation should publish lifecycle and action records");

    assert_eq!(
        observed,
        vec!["lifecycle", "action"],
        "a milestone reached before a handler side effect must precede that action in the concrete FIFO"
    );

    page.close_async()
        .await
        .expect("DCL handler navigation page should close");
}
#[tokio::test(flavor = "multi_thread")]
async fn owner_loop_completes_post_dcl_async_script_without_wait_command() {
    let runtime = JsRuntime::initialize();
    let (activity_wake_tx, mut activity_wake_rx) = renderer_external_activity_test_channel();
    runtime.set_renderer_output_transport_sender(activity_wake_tx);
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let (base_url, async_script_request_seen, release_async_script_response, server) =
        spawn_owner_wake_gated_server_with_content_type(
            "/async.js",
            "globalThis.__lm_post_dcl_async_marker = 'executed';",
            "application/javascript",
        )
        .await;
    let page_url = url::Url::parse(&format!("{base_url}/page")).expect("page url");
    let (completion_tx, completion_rx) = oneshot::channel();
    let (body_tx, raw_body) = ExternalRawDocumentBodyStream::channel(completion_rx);
    let html = r#"<!doctype html><body>
<script async src="/async.js"></script>
<script>globalThis.__lm_dcl_script_marker = "inline";</script>
</body>"#;
    let producer = tokio::spawn(async move {
        body_tx
            .send(html.as_bytes().to_vec())
            .await
            .expect("html chunk should send");
        drop(body_tx);
        completion_tx.send(Ok(())).expect("completion should send");
    });

    let (page, _, _creation_diagnostics, creation_artifacts, pending_download) = runtime
        .create_streaming_raw_page_from_external_body(
            page_url.clone(),
            page_url,
            None,
            false,
            0,
            Vec::new(),
            200,
            vec![("content-type".to_owned(), b"text/html".to_vec())],
            &loader,
            crate::RendererWebStorageHandles::ephemeral(),
            raw_body,
            false,
            PageVmInitStage::DomContentLoaded,
            crate::RendererReplyBoundary::Stage,
            RendererTopLevelNavigationDispatch::FollowInStandaloneAdapter,
            RendererNavigationReplyPolicy::FollowBeforeReply,
            None,
            None,
            crate::RendererDocumentOptions {
                ..Default::default()
            },
        )
        .await
        .expect("page should reach DOMContentLoaded");
    producer.await.expect("producer should finish");
    assert!(pending_download.is_none());
    tokio::time::timeout(Duration::from_millis(500), async_script_request_seen)
        .await
        .expect("async script request should reach the gated server")
        .expect("gated async script request channel should stay open");

    assert!(
        creation_artifacts
            .lifecycle_snapshot
            .dom_content_loaded
            .is_some()
    );
    assert!(creation_artifacts.lifecycle_snapshot.load.is_none());
    while activity_wake_rx.try_recv().is_ok() {}

    release_async_script_response
        .send(())
        .expect("release gated async script response");

    let load_events = tokio::time::timeout(
        Duration::from_secs(2),
        recv_page_lifecycle_until(
            &mut activity_wake_rx,
            &page,
            RendererDocumentLifecycleMilestone::Load,
        ),
    )
    .await
    .expect("page owner should reach load after the async-script completion");
    assert!(load_events.iter().any(|event| matches!(
        event.kind,
        RendererDocumentLifecycleEventKind::Milestone(RendererDocumentLifecycleMilestone::Load)
    )));

    let (observed, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"globalThis.__lm_post_dcl_async_marker ?? "pending""#.to_owned(),
            await_promise: false,
        })
        .await
        .expect("marker evaluate should run");
    assert_eq!(
        renderer_json_value(observed),
        Some(serde_json::json!("executed")),
        "page owner should execute post-DCL async script work without a wait-driver command"
    );
    server
        .await
        .expect("post-DCL async script server should finish");
}
#[tokio::test(flavor = "multi_thread")]
async fn synchronous_document_close_schedules_replacement_lifecycle_turn() {
    let runtime = JsRuntime::initialize();
    let (activity_wake_tx, mut activity_wake_rx) = renderer_external_activity_test_channel();
    runtime.set_renderer_output_transport_sender(activity_wake_tx);
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let page_url =
        url::Url::parse("https://example.test/synchronous-document-close").expect("page url");
    let mut page = create_test_html_page(
        &runtime,
        &loader,
        page_url,
        "<!doctype html><body>initial</body>",
    )
    .await;

    let _ = page
        .run_async_command(RendererPageCommand::TakeDocumentLifecycleEvents)
        .await
        .expect("initial lifecycle events should be drainable");
    while activity_wake_rx.try_recv().is_ok() {}

    let (reply, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"
document.open();
document.write('<main id="replacement">replacement</main>');
document.close();
'closed'
"#
            .to_owned(),
            await_promise: false,
        })
        .await
        .expect("synchronous document replacement should return");
    assert_eq!(
        renderer_json_value(reply),
        Some(serde_json::json!("closed"))
    );

    let replacement_events = tokio::time::timeout(
        Duration::from_millis(500),
        recv_page_lifecycle_until(
            &mut activity_wake_rx,
            &page,
            RendererDocumentLifecycleMilestone::Load,
        ),
    )
    .await
    .expect("the installed replacement resident should be scheduled without another driver");
    assert!(replacement_events.iter().any(|event| matches!(
        event.kind,
        RendererDocumentLifecycleEventKind::Milestone(
            RendererDocumentLifecycleMilestone::DomContentLoaded
        )
    )));

    let (replacement, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: "document.querySelector('#replacement')?.textContent ?? 'missing'"
                .to_owned(),
            await_promise: false,
        })
        .await
        .expect("replacement DOM should evaluate");
    assert_eq!(
        renderer_json_value(replacement),
        Some(serde_json::json!("replacement"))
    );

    page.close_async()
        .await
        .expect("synchronous document.close test page should close");
}
#[tokio::test(flavor = "multi_thread")]
async fn runtime_document_close_completion_parks_lifecycle_until_capability_release() {
    let runtime = JsRuntime::initialize();
    let (activity_wake_tx, mut activity_wake_rx) = renderer_external_activity_test_channel();
    runtime.set_renderer_output_transport_sender(activity_wake_tx);
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let page_url =
        url::Url::parse("https://example.test/runtime-command-boundary").expect("page url");
    let mut page = create_test_html_page(
        &runtime,
        &loader,
        page_url,
        "<!doctype html><body>initial</body>",
    )
    .await;

    let _ = page
        .run_async_command(RendererPageCommand::TakeDocumentLifecycleEvents)
        .await
        .expect("initial lifecycle events should be drainable");
    while activity_wake_rx.try_recv().is_ok() {}

    let call_id = 710_221;
    let (response_tx, _response_rx) = oneshot::channel();
    let completion = page
        .enqueue_async_command(
            RendererPageCommand::dispatch_runtime_protocol_message_with_deferred_response(
                None,
                serde_json::json!({
                    "id": call_id,
                    "method": "Runtime.evaluate",
                    "params": {
                        "expression": "document.open(); document.write('<main>replacement</main>'); document.close(); 'done'",
                        "returnByValue": true,
                    },
                })
                .to_string(),
                RendererRuntimeInspectorResponseSender::new(
                    call_id,
                    response_tx,
                ),
            ),
        )
        .expect("Runtime.evaluate should enqueue")
        .wait()
        .await
        .expect("Runtime.evaluate should complete at its renderer command boundary");
    assert!(
        completion.completion().has_post_response_continuation(),
        "the exact post-response continuation belongs to the final completion"
    );
    let (completion, renderer_output_predecessor) = completion.into_completion_and_predecessor();
    assert_eq!(
        completion
            .runtime_inspector_output()
            .and_then(|output| output.protocol_response(call_id))
            .expect("Runtime command completion should retain the response")["result"]["result"]["value"],
        serde_json::json!("done")
    );
    let (reply, _, continuation) = completion.into_parts();
    assert!(matches!(
        reply,
        RendererPageReply::RuntimeInspectorProtocolMessages(ref messages) if !messages.is_empty()
    ));
    let renderer_output_predecessor = renderer_output_predecessor
        .expect("document replacement output must fence the Runtime response");
    let command_publications = activity_wake_rx.drain();
    assert!(command_publications.iter().any(|publication| {
        publication.cursor() == renderer_output_predecessor.cursor()
            && publication.records().iter().any(|record| {
                matches!(
                    record.item(),
                    super::RendererOutputItem::Observation(
                        super::RendererProtocolObservation::DocumentLifecycle(event)
                    ) if matches!(
                        event.kind,
                        RendererDocumentLifecycleEventKind::Started { .. }
                    )
                )
            })
    }));
    let continuation = continuation
        .expect("document.close should return an exact post-response lifecycle capability");

    let (reply, _) = page
        .run_async_command(RendererPageCommand::TakeDocumentLifecycleEvents)
        .await
        .expect("parked lifecycle facts should be inspectable");
    let RendererPageReply::DocumentLifecycleEvents(events_before_release) = reply else {
        panic!("unexpected parked lifecycle reply");
    };
    assert!(events_before_release.iter().all(|event| !matches!(
        event.kind,
        RendererDocumentLifecycleEventKind::Milestone(
            RendererDocumentLifecycleMilestone::DomContentLoaded
                | RendererDocumentLifecycleMilestone::Load
        )
    )));

    continuation.release();
    let reached_load = tokio::time::timeout(
        Duration::from_millis(500),
        recv_page_lifecycle_until(
            &mut activity_wake_rx,
            &page,
            RendererDocumentLifecycleMilestone::Load,
        ),
    )
    .await
    .expect("released capability should schedule the exact lifecycle resident");
    assert!(reached_load.iter().any(|event| {
        event.kind
            == RendererDocumentLifecycleEventKind::Milestone(
                RendererDocumentLifecycleMilestone::Load,
            )
    }));

    page.close_async()
        .await
        .expect("runtime command boundary test page should close");
}
#[tokio::test(flavor = "multi_thread")]
async fn blocked_lifecycle_page_does_not_prevent_peer_page_load() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let (base_url, async_script_request_seen, release_async_script_response, server) =
        spawn_owner_wake_gated_server_with_content_type(
            "/blocked-page-async.js",
            "globalThis.__lm_blocked_page_async_marker = 'executed';",
            "application/javascript",
        )
        .await;
    let blocked_page_url =
        url::Url::parse(&format!("{base_url}/blocked-page")).expect("blocked page url");
    let (completion_tx, completion_rx) = oneshot::channel();
    let (body_tx, raw_body) = ExternalRawDocumentBodyStream::channel(completion_rx);
    let producer = tokio::spawn(async move {
        body_tx
            .send(
                br#"<!doctype html><body>
<script async src="/blocked-page-async.js"></script>
blocked page
</body>"#
                    .to_vec(),
            )
            .await
            .expect("blocked page html chunk should send");
        drop(body_tx);
        completion_tx.send(Ok(())).expect("completion should send");
    });

    let blocked_creation = runtime.create_streaming_raw_page_from_external_body(
        blocked_page_url.clone(),
        blocked_page_url,
        None,
        false,
        0,
        Vec::new(),
        200,
        vec![("content-type".to_owned(), b"text/html".to_vec())],
        &loader,
        crate::RendererWebStorageHandles::ephemeral(),
        raw_body,
        false,
        PageVmInitStage::Load,
        crate::RendererReplyBoundary::Stage,
        RendererTopLevelNavigationDispatch::FollowInStandaloneAdapter,
        RendererNavigationReplyPolicy::FollowBeforeReply,
        None,
        None,
        crate::RendererDocumentOptions {
            ..Default::default()
        },
    );
    tokio::pin!(blocked_creation);
    tokio::select! {
        _result = &mut blocked_creation => {
            panic!("blocked page reached Load before its async response")
        }
        seen = async_script_request_seen => {
            seen.expect("blocked page async request channel should stay open");
        }
    }
    producer.await.expect("blocked page producer should finish");

    let peer_url = url::Url::parse("https://peer-page.test/ready").expect("peer page url");
    let mut peer_page = tokio::time::timeout(
        Duration::from_secs(2),
        create_test_html_page(
            &runtime,
            &loader,
            peer_url,
            r#"<!doctype html><script>globalThis.__lm_peer_page_marker = "loaded";</script>"#,
        ),
    )
    .await
    .expect("a blocked lifecycle page must not starve a peer page");
    let (peer_marker, _) = peer_page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: "globalThis.__lm_peer_page_marker".to_owned(),
            await_promise: false,
        })
        .await
        .expect("peer page marker should evaluate");
    assert_eq!(
        renderer_json_value(peer_marker),
        Some(serde_json::json!("loaded"))
    );

    let (probe_tx, probe_rx) = oneshot::channel();
    probe_tx.send(()).expect("readiness probe should send");
    tokio::select! {
        biased;
        _result = &mut blocked_creation => {
            panic!("blocked page completed while its producer was still gated")
        }
        _ = probe_rx => {}
    }

    release_async_script_response
        .send(())
        .expect("release blocked page async response");
    let (mut blocked_page, _, _, creation_artifacts, pending_download) =
        tokio::time::timeout(Duration::from_secs(2), blocked_creation)
            .await
            .expect("blocked page should resume from its producer wake")
            .expect("blocked page should reach Load");
    assert!(pending_download.is_none());
    assert!(creation_artifacts.lifecycle_snapshot.load.is_some());
    assert!(
        RendererPageTestingHandle::new_for_testing(&blocked_page)
            .shares_local_host(&RendererPageTestingHandle::new_for_testing(&peer_page)),
        "the isolation check must exercise two pages scheduled by the same owner-local host"
    );

    blocked_page
        .close_async()
        .await
        .expect("blocked lifecycle page should close");
    peer_page
        .close_async()
        .await
        .expect("peer lifecycle page should close");
    server.await.expect("blocked page server should finish");
}
#[cfg(debug_assertions)]
#[test]
fn renderer_owner_local_runtime_thread_affinity_is_sticky() {
    let runtime = JsRuntime::initialize();
    let renderer_owner = runtime.renderer_owner_handle();

    renderer_owner
        .bind_or_check_local_runtime_thread()
        .expect("first local-runtime entry should bind current thread");

    let renderer_owner_for_thread = renderer_owner.clone();
    let error = std::thread::spawn(move || {
        renderer_owner_for_thread
            .bind_or_check_local_runtime_thread()
            .expect_err("cross-thread local-runtime entry should fail")
            .to_string()
    })
    .join()
    .expect("worker should finish");

    assert!(
        error.contains("different thread"),
        "cross-thread local-runtime entry should report thread-affinity mismatch"
    );
}
#[test]
fn owner_local_runtime_access_is_allowed_on_plain_current_thread_runtime() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime should build");
    let executor = JsLocalExecutor::new();

    runtime.block_on(async move {
        assert_eq!(
            super::owner_local_runtime_access_path(&executor),
            super::OwnerLocalRuntimeAccessPath::CurrentThreadFallback,
            "plain current-thread runtime should use the current-thread owner-local runtime fallback"
        );
    });
}
#[test]
fn owner_local_runtime_access_is_allowed_on_matching_executor_lane() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime should build");
    let local = tokio::task::LocalSet::new();
    let executor = JsLocalExecutor::new();

    local.block_on(&runtime, async move {
        let executor_for_assert = executor.clone();
        executor
            .run(async move {
                assert_eq!(
                    super::owner_local_runtime_access_path(&executor_for_assert),
                    super::OwnerLocalRuntimeAccessPath::DirectNamedLane,
                    "matching executor lane should use the direct named-lane owner-local runtime path"
                );
            })
            .await;
    });
}
#[test]
fn owner_local_runtime_access_is_rejected_on_different_executor_lane() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime should build");
    let local = tokio::task::LocalSet::new();
    let first_executor = JsLocalExecutor::new();
    let second_executor = JsLocalExecutor::new();

    local.block_on(&runtime, async move {
        first_executor
            .run(async move {
                assert_eq!(
                    super::owner_local_runtime_access_path(&second_executor),
                    super::OwnerLocalRuntimeAccessPath::ExecutorHop,
                    "different executor lane should require an owner-local runtime hop"
                );
            })
            .await;
    });
}
#[test]
fn owner_local_runtime_access_is_rejected_on_scaffold_lane() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime should build");
    let local = tokio::task::LocalSet::new();
    let executor = JsLocalExecutor::new();

    local.block_on(&runtime, async move {
        scope_on_scaffold_js_local_executor(async move {
            assert_eq!(
                super::owner_local_runtime_access_path(&executor),
                super::OwnerLocalRuntimeAccessPath::ExecutorHop,
                "parse-time scaffold lane should not access page owner-local runtime directly"
            );
        })
        .await;
    });
}
