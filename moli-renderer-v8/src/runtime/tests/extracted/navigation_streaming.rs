use super::*;

#[tokio::test(flavor = "multi_thread")]
async fn streaming_xml_document_executes_parser_blocking_xhtml_script() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let url = url::Url::parse("https://example.test/initial.xhtml").unwrap();
    let source = concat!(
        "<html xmlns='http://www.w3.org/1999/xhtml'><head>",
        "<script>try { document.write('&lt;future/&gt;'); } ",
        "catch (error) { globalThis.__xmlWriteError = error.name; } ",
        "globalThis.__initialXhtmlHandoff = 'executed';</script>",
        "</head><body /></html>",
    );
    let prepared = prepare_test_external_raw_document_with_content_type(
        &runtime,
        &loader,
        url,
        "application/xhtml+xml",
        ExternalRawDocumentBodyStream::from_bytes(source.as_bytes().to_vec()),
    )
    .await;
    let permit = prepared.issue_commit_permit();
    let (mut page, page_state, _, _, pending_download) = prepared
        .commit(permit)
        .await
        .expect("incremental XHTML document should commit");
    assert!(pending_download.is_none());

    let (reply, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: concat!(
                "JSON.stringify([document.contentType, ",
                "document.getElementsByTagName('script').length, ",
                "String(globalThis.__initialXhtmlHandoff), ",
                "String(globalThis.__xmlWriteError), ",
                "document.getElementsByTagName('future').length])",
            )
            .to_owned(),
            await_promise: false,
        })
        .await
        .expect("initial XHTML script side effect should be observable");
    assert_eq!(
        renderer_json_value(reply),
        Some(serde_json::json!(
            r#"["application/xhtml+xml",1,"executed","InvalidStateError",0]"#
        )),
        "XML script report: {:#?}",
        page_state.script_execution.runs,
    );
    page.close_async().await.expect("XHTML page should close");
}
#[tokio::test(flavor = "multi_thread")]
async fn streaming_unstyled_xml_converts_live_document_before_domcontentloaded() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let url = url::Url::parse("https://example.test/document.xml").unwrap();
    let source = "<semantic-root id='source'><child>xml-ready</child></semantic-root>";
    let prepared = prepare_test_external_raw_document_with_content_type(
        &runtime,
        &loader,
        url,
        "application/xml",
        ExternalRawDocumentBodyStream::from_bytes(source.as_bytes().to_vec()),
    )
    .await;
    prepared
        .update_commit_configuration(RendererPreparedDocumentCommitConfiguration {
            document_start_scripts: vec![crate::DocumentStartScript {
                registry_key: None,
                devtools_session: None,
                source: concat!(
                    "globalThis.__xmlViewerDcl = null;",
                    "document.addEventListener('DOMContentLoaded', () => {",
                    "  const source = document.getElementById('source');",
                    "  const style = document.getElementById('xml-viewer-style');",
                    "  globalThis.__xmlViewerDcl = [",
                    "    document.documentElement.localName,",
                    "    source && source.parentNode && source.parentNode.id,",
                    "    source && source.textContent,",
                    "    !!(style && style.sheet),",
                    "    getComputedStyle(source.parentNode).display",
                    "  ];",
                    "});",
                )
                .to_owned(),
                world_name: None,
                has_bidi_channel_argument: false,
                bidi_channel_handoffs: Vec::new(),
            }],
            runtime_bindings: Vec::new(),
            runtime_inspector_session_restore_snapshots: Vec::new(),
            runtime_isolated_worlds: Vec::new(),
            permission_overrides: Vec::new(),
            extra_http_headers: Default::default(),
            script_execution_disabled: false,
            bypass_content_security_policy: false,
            emulated_media: Default::default(),
            idle_override: None,
            navigator_overrides: Default::default(),
            viewport_surface: None,
            document_activity: Default::default(),
            browser_resource_runtime: loader.browser_resource_runtime(),
            navigator_identity: loader.browser_identity().clone(),
            network_offline: false,
            bypass_service_worker: false,
            cache_disabled: false,
            blocked_url_patterns: Vec::new(),
            fetch_subresource_interception_enabled: false,
            fetch_subresource_interception_resource_type: None,
        })
        .await
        .expect("prepared XML should accept the lifecycle probe");

    let permit = prepared.issue_commit_permit();
    let (mut page, _, _, _, pending_download) = prepared
        .commit(permit)
        .await
        .expect("unstyled XML document should commit");
    assert!(pending_download.is_none());
    let (reply, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: concat!(
                "JSON.stringify([",
                "document.documentElement.localName,",
                "document.documentElement.namespaceURI,",
                "globalThis.__xmlViewerDcl",
                "])",
            )
            .to_owned(),
            await_promise: false,
        })
        .await
        .expect("live XML viewer state should evaluate");
    assert_eq!(
        renderer_json_value(reply),
        Some(serde_json::json!(
            r#"["html","http://www.w3.org/1999/xhtml",["html","webkit-xml-viewer-source-xml","xml-ready",true,"none"]]"#
        ))
    );
    page.close_async()
        .await
        .expect("unstyled XML page should close");
}
#[tokio::test(flavor = "multi_thread")]
async fn prepared_streaming_xml_document_waits_for_permit_and_uses_latest_configuration() {
    let runtime = JsRuntime::initialize();
    let baseline_isolates = runtime.document_isolate_accounting_for_diagnostics();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let (base_url, mut side_effect_request_seen, release_side_effect_response, server) =
        spawn_owner_wake_gated_server_with_content_type(
            "/native-author-side-effect",
            "ok",
            "text/plain",
        )
        .await;
    let url = url::Url::parse(&format!("{base_url}/prepared.xhtml")).expect("prepared XHTML url");
    let source = concat!(
        "<html xmlns='http://www.w3.org/1999/xhtml'><head><script>",
        "globalThis.__nativeCommitObserved = JSON.stringify([",
        "globalThis.__nativePreload, typeof nativeBinding]);",
        "fetch('/native-author-side-effect');",
        "</script></head><body /></html>",
    );
    let prepared = prepare_test_external_raw_document_with_content_type(
        &runtime,
        &loader,
        url.clone(),
        "application/xhtml+xml",
        ExternalRawDocumentBodyStream::from_bytes(source.as_bytes().to_vec()),
    )
    .await;
    let prepared_agent = prepared.renderer_devtools_agent_token();
    let prepared_isolates = runtime.document_isolate_accounting_for_diagnostics();
    assert_eq!(prepared_isolates.created, baseline_isolates.created + 1);
    assert_eq!(prepared_isolates.live, baseline_isolates.live + 1);
    assert_eq!(prepared_isolates.reserved, baseline_isolates.reserved + 1);
    assert_eq!(
        runtime.renderer_owner_handle().len(),
        0,
        "preparing a streaming XML document must not install a Page"
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(100), &mut side_effect_request_seen)
            .await
            .is_err(),
        "the XML parser handoff must not execute before the commit permit"
    );

    prepared
        .update_commit_configuration(RendererPreparedDocumentCommitConfiguration {
            document_start_scripts: vec![
                crate::DocumentStartScript {
                    registry_key: None,
                    devtools_session: None,
                    source: r#"globalThis.__nativePreload = "ready";"#.to_owned(),
                    world_name: None,
                    has_bidi_channel_argument: false,
                    bidi_channel_handoffs: Vec::new(),
                },
                crate::DocumentStartScript {
                    registry_key: None,
                    devtools_session: None,
                    source: r#"globalThis.__nativeWorldPreload = "ready";"#.to_owned(),
                    world_name: Some("native-world".to_owned()),
                    has_bidi_channel_argument: false,
                    bidi_channel_handoffs: Vec::new(),
                },
            ],
            runtime_bindings: vec![crate::protocol_types::RuntimeBindingRegistration {
                devtools_session: None,
                name: "nativeBinding".to_owned(),
                execution_context_name: None,
            }],
            runtime_inspector_session_restore_snapshots: vec![
                RendererInspectorSessionRestoreSnapshot {
                    protocol_configuration: RendererInspectorProtocolConfiguration {
                        runtime_frontend_enabled: true,
                        ..Default::default()
                    },
                    ..Default::default()
                },
            ],
            runtime_isolated_worlds: vec![crate::protocol_types::RuntimeIsolatedWorldDefinition {
                name: "native-world".to_owned(),
                grant_universal_access: false,
            }],
            permission_overrides: Vec::new(),
            extra_http_headers: Default::default(),
            script_execution_disabled: false,
            bypass_content_security_policy: false,
            emulated_media: Default::default(),
            idle_override: None,
            navigator_overrides: Default::default(),
            viewport_surface: None,
            document_activity: Default::default(),
            browser_resource_runtime: loader.browser_resource_runtime(),
            navigator_identity: loader.browser_identity().clone(),
            network_offline: false,
            bypass_service_worker: false,
            cache_disabled: false,
            blocked_url_patterns: Vec::new(),
            fetch_subresource_interception_enabled: false,
            fetch_subresource_interception_resource_type: None,
        })
        .await
        .expect("prepared streaming XML should accept live commit configuration");

    let permit = prepared.issue_commit_permit();
    let (mut page, _, diagnostics, _, pending_download) =
        prepared.commit(permit).await.expect("permit should commit");
    assert!(pending_download.is_none());
    assert_eq!(
        page.devtools_agent_token(),
        prepared_agent,
        "streaming XML commit must attach the agent reserved during prepare"
    );
    tokio::time::timeout(Duration::from_secs(2), &mut side_effect_request_seen)
        .await
        .expect("the XML parser handoff should run after commit")
        .expect("author fetch observation should stay open");
    release_side_effect_response
        .send(())
        .expect("release author fetch response");

    let (observed, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: "globalThis.__nativeCommitObserved".to_owned(),
            await_promise: false,
        })
        .await
        .expect("XML author-observed configuration should evaluate");
    assert_eq!(
        renderer_json_value(observed),
        Some(serde_json::json!(r#"["ready","function"]"#))
    );
    let world_context_id = diagnostics
        .initial_runtime_realms
        .iter()
        .find(|realm| realm.name == "native-world")
        .map(|realm| realm.context_id)
        .expect("the streaming XML named world should exist at initial commit");
    let (world_marker, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpressionInExecutionContext {
            execution_context_id: world_context_id,
            expression: "globalThis.__nativeWorldPreload".to_owned(),
            await_promise: false,
        })
        .await
        .expect("streaming XML named-world preload marker should evaluate");
    assert_eq!(
        renderer_json_value(world_marker),
        Some(serde_json::json!("ready"))
    );

    page.close_async()
        .await
        .expect("committed streaming XML page should close");
    server
        .await
        .expect("NativeDom author side-effect server should finish");
}
#[tokio::test(flavor = "current_thread")]
async fn external_raw_streaming_page_command_builds_phase_one_page() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let requested_url = url::Url::parse("https://example.test/raw-stream").unwrap();
    let final_url = url::Url::parse("https://example.test/raw-stream-final").unwrap();
    let (completion_tx, completion_rx) = oneshot::channel();
    let (body_tx, raw_body) = ExternalRawDocumentBodyStream::channel(completion_rx);
    let html = "<!doctype html><html><body><main id='external-raw'>外部 raw stream</main><script>document.body.setAttribute('data-streamed','yes')</script></body></html>";
    let split = html.find("raw stream").expect("split marker");
    let first_chunk = html.as_bytes()[..split].to_vec();
    let second_chunk = html.as_bytes()[split..].to_vec();

    let producer = tokio::spawn(async move {
        body_tx
            .send(first_chunk)
            .await
            .expect("first chunk should send");
        body_tx
            .send(second_chunk)
            .await
            .expect("second chunk should send");
        drop(body_tx);
        completion_tx.send(Ok(())).expect("completion should send");
    });

    let (page, snapshot, _creation_diagnostics, _creation_artifacts, pending_download) = runtime
        .create_streaming_raw_page_from_external_body(
            requested_url.clone(),
            final_url.clone(),
            None,
            true,
            1,
            Vec::new(),
            203,
            vec![(
                "content-type".to_owned(),
                b"text/html; charset=UTF-8".to_vec(),
            )],
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
        )
        .await
        .expect("external raw streaming page should build");
    producer.await.expect("producer should finish");

    assert!(pending_download.is_none());
    assert_eq!(snapshot.requested_url, requested_url);
    assert_eq!(snapshot.final_url(), &final_url);
    assert_eq!(snapshot.status, 203);
    assert_eq!(snapshot.navigation_redirect_count, 1);
    assert!(snapshot.navigation_redirected);
    let html = serialize_html_for_renderer_page(&page).await;
    assert!(html.contains("id=\"external-raw\""));
    assert!(html.contains("外部 raw stream"));
    assert!(html.contains("data-streamed=\"yes\""));
}
#[tokio::test(flavor = "multi_thread")]
async fn external_raw_streaming_body_failure_preserves_committed_document_and_owner() {
    let runtime = JsRuntime::initialize();
    let (output_tx, mut output_rx) = renderer_external_activity_test_channel();
    runtime.set_renderer_output_transport_sender(output_tx);
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let url = url::Url::parse("https://example.test/failed-main-document")
        .expect("failed main document url");
    let (completion_tx, completion_rx) = oneshot::channel();
    let (body_tx, raw_body) = ExternalRawDocumentBodyStream::channel(completion_rx);
    body_tx
        .send(
            b"<!doctype html><main id='partial'>partial body before transport failure</main>"
                .to_vec(),
        )
        .await
        .expect("partial main document body should send");
    let prepared = prepare_test_external_raw_document_with_content_type_and_reply_boundary(
        &runtime,
        &loader,
        url,
        "text/html",
        raw_body,
        RendererReplyBoundary::DocumentCommit,
    )
    .await;
    drop(body_tx);
    completion_tx
        .send(Err(anyhow::anyhow!(
            "synthetic partial main document body failure"
        )))
        .expect("main document body failure should send");
    let permit = prepared.issue_commit_permit();
    let (mut page, _, _, creation_artifacts, pending_download) =
        tokio::time::timeout(Duration::from_secs(5), prepared.commit(permit))
            .await
            .expect("partial main document should reach its response commit boundary")
            .expect("partial main document should attach before its body terminal");
    assert!(pending_download.is_none());
    assert!(
        creation_artifacts.lifecycle_snapshot.load.is_none(),
        "open main document must attach before load"
    );
    page.take_committed_document_post_response_continuation()
        .expect("DocumentCommit should retain parser work until its response boundary")
        .release();

    let failure_events = tokio::time::timeout(Duration::from_secs(5), async {
        let mut events = Vec::new();
        loop {
            let publication = output_rx
                .recv()
                .await
                .expect("renderer output transport should stay open");
            if !publication_is_for_page(&publication, &page) {
                continue;
            }
            let publication_events = publication_document_lifecycle_events(&publication)
                .copied()
                .collect::<Vec<_>>();
            let main_resource_failed = publication_events.iter().any(|event| {
                event.kind
                    == RendererDocumentLifecycleEventKind::Terminated {
                        last_reached: None,
                        reason: super::RendererDocumentTerminationReason::MainResourceLoadFailed,
                    }
            });
            events.extend(publication_events);
            if main_resource_failed {
                return events;
            }
        }
    })
    .await
    .expect("main-resource failure should terminate the parser lifecycle");
    assert!(failure_events.iter().all(|event| {
        event.kind
            != RendererDocumentLifecycleEventKind::Milestone(
                RendererDocumentLifecycleMilestone::DomContentLoaded,
            )
            && event.kind
                != RendererDocumentLifecycleEventKind::Milestone(
                    RendererDocumentLifecycleMilestone::Load,
                )
    }));
    assert_eq!(
        runtime.renderer_owner_handle().len(),
        1,
        "a committed partial Document should remain resident like Blink's failed DocumentLoader"
    );

    let (reply, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: "document.querySelector('#partial').textContent".to_owned(),
            await_promise: false,
        })
        .await
        .expect("committed partial Document should remain script-observable");
    assert_eq!(
        renderer_json_value(reply),
        Some(serde_json::json!("partial body before transport failure"))
    );
    let (reply, _) = tokio::time::timeout(
        Duration::from_secs(2),
        page.run_async_command(RendererPageCommand::EvaluateExpression {
            expression: "new Promise(resolve => setTimeout(() => resolve(document.readyState), 0))"
                .to_owned(),
            await_promise: true,
        }),
    )
    .await
    .expect("a failed committed Document should keep running ordinary Page tasks")
    .expect("a failed committed Document should resolve a newly scheduled timer");
    assert_eq!(
        renderer_json_value(reply),
        Some(serde_json::json!("loading")),
        "stopping a failed parser must not synthesize DCL or a successful EOF"
    );
    page.close_async()
        .await
        .expect("failed committed page should close normally");

    let recovery_url =
        url::Url::parse("https://example.test/recovery-after-body-failure").expect("recovery url");
    let mut recovery_page = tokio::time::timeout(
        Duration::from_secs(5),
        create_test_html_page(
            &runtime,
            &loader,
            recovery_url,
            "<!doctype html><main id='recovered'>recovered</main>",
        ),
    )
    .await
    .expect("renderer owner should accept a page after the failed candidate");
    let (reply, _) = recovery_page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: "document.querySelector('#recovered').textContent".to_owned(),
            await_promise: false,
        })
        .await
        .expect("recovery page should remain usable");
    assert_eq!(
        renderer_json_value(reply),
        Some(serde_json::json!("recovered"))
    );
    recovery_page
        .close_async()
        .await
        .expect("recovery page should close");
}
#[tokio::test(flavor = "multi_thread")]
async fn prepared_external_raw_document_waits_for_matching_commit_permit() {
    let runtime = JsRuntime::initialize();
    let baseline_isolates = runtime.document_isolate_accounting_for_diagnostics();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let (base_url, mut side_effect_request_seen, release_side_effect_response, server) =
        spawn_owner_wake_gated_server_with_content_type("/author-side-effect", "ok", "text/plain")
            .await;
    let url = url::Url::parse(&format!("{base_url}/prepared")).expect("prepared url");
    let (completion_tx, completion_rx) = oneshot::channel();
    let (body_tx, raw_body) = ExternalRawDocumentBodyStream::channel(completion_rx);
    let producer = tokio::spawn(async move {
        body_tx
            .send(
                br#"<!doctype html><script>
globalThis.__preparedAuthorScript = "executed";
localStorage.setItem("prepared-commit", "executed");
fetch("/author-side-effect");
</script><main>prepared</main>"#
                    .to_vec(),
            )
            .await
            .expect("prepared document body should send");
        drop(body_tx);
        completion_tx.send(Ok(())).expect("completion should send");
    });

    let prepared = prepare_test_external_raw_document(&runtime, &loader, url, raw_body).await;
    let prepared_agent = prepared.renderer_devtools_agent_token();
    producer
        .await
        .expect("prepared body producer should finish");
    let prepared_isolates = runtime.document_isolate_accounting_for_diagnostics();
    assert_eq!(prepared_isolates.created, baseline_isolates.created + 1);
    assert_eq!(prepared_isolates.live, baseline_isolates.live + 1);
    assert_eq!(prepared_isolates.reserved, baseline_isolates.reserved + 1);
    assert_eq!(
        runtime.renderer_owner_handle().len(),
        0,
        "prepare must not install a Page before the commit permit"
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(100), &mut side_effect_request_seen)
            .await
            .is_err(),
        "author fetch must not run while the prepared document is held"
    );

    let permit = prepared.issue_commit_permit();
    let (mut page, _, _, _, pending_download) =
        prepared.commit(permit).await.expect("permit should commit");
    assert!(pending_download.is_none());
    assert_eq!(
        page.devtools_agent_token(),
        prepared_agent,
        "commit must attach the agent allocated before the permit"
    );
    tokio::time::timeout(Duration::from_secs(2), &mut side_effect_request_seen)
        .await
        .expect("author fetch should run after commit")
        .expect("author fetch observation should stay open");
    release_side_effect_response
        .send(())
        .expect("release author fetch response");

    let (reply, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"JSON.stringify([
globalThis.__preparedAuthorScript,
localStorage.getItem("prepared-commit")
])"#
            .to_owned(),
            await_promise: false,
        })
        .await
        .expect("committed author state should evaluate");
    assert_eq!(
        renderer_json_value(reply),
        Some(serde_json::json!(r#"["executed","executed"]"#))
    );
    page.close_async()
        .await
        .expect("committed prepared page should close");
    server
        .await
        .expect("author side-effect server should finish");
}
#[tokio::test(flavor = "multi_thread")]
async fn prepared_document_uses_latest_commit_configuration_before_author_script() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let url =
        url::Url::parse("https://example.test/prepared-latest-inspector").expect("prepared url");
    let (completion_tx, completion_rx) = oneshot::channel();
    let (body_tx, raw_body) = ExternalRawDocumentBodyStream::channel(completion_rx);
    let producer = tokio::spawn(async move {
        body_tx
            .send(
                br#"<!doctype html><script>
globalThis.__preparedCommitObserved = JSON.stringify([
  globalThis.__latestPreload,
  typeof latestBinding,
  Notification.permission
]);
</script>"#
                    .to_vec(),
            )
            .await
            .expect("prepared body should send");
        drop(body_tx);
        completion_tx.send(Ok(())).expect("completion should send");
    });

    let prepared = prepare_test_external_raw_document(&runtime, &loader, url, raw_body).await;
    producer
        .await
        .expect("prepared body producer should finish");
    prepared
        .update_commit_configuration(RendererPreparedDocumentCommitConfiguration {
            document_start_scripts: vec![
                crate::DocumentStartScript {
                    registry_key: None,
                    devtools_session: None,
                    source: r#"globalThis.__latestPreload = "ready";"#.to_owned(),
                    world_name: None,
                    has_bidi_channel_argument: false,
                    bidi_channel_handoffs: Vec::new(),
                },
                crate::DocumentStartScript {
                    registry_key: None,
                    devtools_session: None,
                    source: r#"globalThis.__latestWorld = "ready";"#.to_owned(),
                    world_name: Some("latest-world".to_owned()),
                    has_bidi_channel_argument: false,
                    bidi_channel_handoffs: Vec::new(),
                },
            ],
            runtime_bindings: vec![crate::protocol_types::RuntimeBindingRegistration {
                devtools_session: None,
                name: "latestBinding".to_owned(),
                execution_context_name: None,
            }],
            runtime_inspector_session_restore_snapshots: vec![
                RendererInspectorSessionRestoreSnapshot {
                    protocol_configuration: RendererInspectorProtocolConfiguration {
                        runtime_frontend_enabled: true,
                        ..Default::default()
                    },
                    ..Default::default()
                },
            ],
            runtime_isolated_worlds: vec![crate::protocol_types::RuntimeIsolatedWorldDefinition {
                name: "latest-world".to_owned(),
                grant_universal_access: false,
            }],
            permission_overrides: vec![crate::protocol_types::PermissionOverrideRegistration {
                permission: serde_json::Value::String("notifications".to_owned()),
                setting: "granted".to_owned(),
                origin: None,
                embedded_origin: None,
            }],
            extra_http_headers: Default::default(),
            script_execution_disabled: false,
            bypass_content_security_policy: false,
            emulated_media: Default::default(),
            idle_override: None,
            navigator_overrides: Default::default(),
            viewport_surface: None,
            document_activity: Default::default(),
            browser_resource_runtime: loader.browser_resource_runtime(),
            navigator_identity: loader.browser_identity().clone(),
            network_offline: false,
            bypass_service_worker: false,
            cache_disabled: false,
            blocked_url_patterns: Vec::new(),
            fetch_subresource_interception_enabled: false,
            fetch_subresource_interception_resource_type: None,
        })
        .await
        .expect("prepared document should accept the latest commit configuration");

    let permit = prepared.issue_commit_permit();
    let (mut page, _, diagnostics, _, pending_download) =
        prepared.commit(permit).await.expect("permit should commit");
    assert!(pending_download.is_none());
    assert!(
        diagnostics.renderer_output_predecessor.is_some(),
        "the commit-time Runtime enable/context output must be published through the concrete Page stream"
    );
    assert!(
        diagnostics
            .initial_runtime_realms
            .iter()
            .any(|realm| realm.is_default),
        "the committed Page must expose its authoritative default-realm inventory"
    );
    let (observed, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: "globalThis.__preparedCommitObserved".to_owned(),
            await_promise: false,
        })
        .await
        .expect("author-observed commit configuration should evaluate");
    assert_eq!(
        renderer_json_value(observed),
        Some(serde_json::json!(r#"["ready","function","granted"]"#))
    );
    let world_context_id = diagnostics
        .initial_runtime_realms
        .iter()
        .find(|realm| realm.name == "latest-world")
        .map(|realm| realm.context_id)
        .expect("commit-time named world should register before initial diagnostics complete");
    let (world_marker, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpressionInExecutionContext {
            execution_context_id: world_context_id,
            expression: "globalThis.__latestWorld".to_owned(),
            await_promise: false,
        })
        .await
        .expect("commit-time named-world preload marker should evaluate");
    assert_eq!(
        renderer_json_value(world_marker),
        Some(serde_json::json!("ready"))
    );

    page.close_async()
        .await
        .expect("committed prepared page should close");
}
#[tokio::test(flavor = "multi_thread")]
async fn prepared_document_rejects_a_peer_commit_permit_without_consuming_its_owner() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let first_url =
        url::Url::parse("https://example.test/prepared-first").expect("first prepared url");
    let second_url =
        url::Url::parse("https://example.test/prepared-second").expect("second prepared url");
    let (first_completion_tx, first_completion_rx) = oneshot::channel();
    let (first_body_tx, first_raw_body) =
        ExternalRawDocumentBodyStream::channel(first_completion_rx);
    let first_producer = tokio::spawn(async move {
        first_body_tx
            .send(b"<!doctype html><main id='first'>first</main>".to_vec())
            .await
            .expect("first prepared body should send");
        drop(first_body_tx);
        first_completion_tx
            .send(Ok(()))
            .expect("first completion should send");
    });
    let (second_completion_tx, second_completion_rx) = oneshot::channel();
    let (second_body_tx, second_raw_body) =
        ExternalRawDocumentBodyStream::channel(second_completion_rx);
    let second_producer = tokio::spawn(async move {
        second_body_tx
            .send(b"<!doctype html><main id='second'>second</main>".to_vec())
            .await
            .expect("second prepared body should send");
        drop(second_body_tx);
        second_completion_tx
            .send(Ok(()))
            .expect("second completion should send");
    });

    let first =
        prepare_test_external_raw_document(&runtime, &loader, first_url, first_raw_body).await;
    let second =
        prepare_test_external_raw_document(&runtime, &loader, second_url, second_raw_body).await;
    first_producer
        .await
        .expect("first prepared producer should finish");
    second_producer
        .await
        .expect("second prepared producer should finish");

    let first_permit = first.issue_commit_permit();
    let mismatch = second.commit(first_permit).await;
    assert!(
        mismatch
            .as_ref()
            .is_err_and(|error| error.to_string().contains("does not belong")),
        "a peer permit must be rejected before either residence is consumed"
    );

    let first_permit = first.issue_commit_permit();
    let (mut page, _snapshot, _, _, pending_download) = first
        .commit(first_permit)
        .await
        .expect("the matching owner should remain committable");
    assert!(pending_download.is_none());
    let html = serialize_html_for_renderer_page(&page).await;
    assert!(html.contains("id=\"first\""));
    assert!(!html.contains("id=\"second\""));
    page.close_async()
        .await
        .expect("matching prepared page should close");
}
#[tokio::test(flavor = "multi_thread")]
async fn canceled_prepared_external_raw_document_has_no_author_side_effects() {
    let runtime = JsRuntime::initialize();
    let baseline_isolates = runtime.document_isolate_accounting_for_diagnostics();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let (base_url, mut side_effect_request_seen, _release_side_effect_response, server) =
        spawn_owner_wake_gated_server_with_content_type(
            "/canceled-author-side-effect",
            "ok",
            "text/plain",
        )
        .await;
    let url = url::Url::parse(&format!("{base_url}/canceled")).expect("canceled url");
    let (completion_tx, completion_rx) = oneshot::channel();
    let (body_tx, raw_body) = ExternalRawDocumentBodyStream::channel(completion_rx);
    let producer = tokio::spawn(async move {
        body_tx
            .send(
                br#"<!doctype html><script>
globalThis.__canceledPreparedAuthorScript = "executed";
localStorage.setItem("prepared-cancel", "executed");
fetch("/canceled-author-side-effect");
</script><main>cancel me</main>"#
                    .to_vec(),
            )
            .await
            .expect("canceled prepared document body should send");
        drop(body_tx);
        completion_tx.send(Ok(())).expect("completion should send");
    });

    let prepared =
        prepare_test_external_raw_document(&runtime, &loader, url.clone(), raw_body).await;
    producer
        .await
        .expect("canceled body producer should finish");
    let prepared_isolates = runtime.document_isolate_accounting_for_diagnostics();
    assert_eq!(prepared_isolates.created, baseline_isolates.created + 1);
    assert_eq!(prepared_isolates.live, baseline_isolates.live + 1);
    assert_eq!(prepared_isolates.reserved, baseline_isolates.reserved + 1);
    prepared
        .cancel()
        .await
        .expect("prepared document should cancel");
    let canceled_isolates = runtime.document_isolate_accounting_for_diagnostics();
    assert_eq!(canceled_isolates.live, baseline_isolates.live);
    assert_eq!(canceled_isolates.reserved, baseline_isolates.reserved);
    assert_eq!(canceled_isolates.destroyed, baseline_isolates.destroyed + 1);
    assert_eq!(
        runtime.renderer_owner_handle().len(),
        0,
        "cancel must not install a Page"
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(100), &mut side_effect_request_seen)
            .await
            .is_err(),
        "canceled author fetch must never start"
    );

    let probe_url = url.join("/probe").expect("same-origin probe url");
    let mut probe = create_test_html_page(
        &runtime,
        &loader,
        probe_url,
        "<!doctype html><main>probe</main>",
    )
    .await;
    let (reply, _) = probe
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"JSON.stringify([
typeof globalThis.__canceledPreparedAuthorScript,
localStorage.getItem("prepared-cancel")
])"#
            .to_owned(),
            await_promise: false,
        })
        .await
        .expect("same-origin cancellation probe should evaluate");
    assert_eq!(
        renderer_json_value(reply),
        Some(serde_json::json!(r#"["undefined",null]"#)),
        "cancel must leave neither JS nor storage side effects"
    );
    probe
        .close_async()
        .await
        .expect("cancellation probe should close");
    server.abort();
}
#[tokio::test(flavor = "current_thread")]
async fn canceled_prepared_document_closes_its_ordered_output_stream() {
    let runtime = JsRuntime::initialize();
    let (output_tx, mut output_rx) = renderer_external_activity_test_channel();
    runtime.set_renderer_output_transport_sender(output_tx);
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let url = url::Url::parse("https://example.test/prepared-output-cancel")
        .expect("prepared output URL");
    let (completion_tx, completion_rx) = oneshot::channel();
    let (body_tx, raw_body) = ExternalRawDocumentBodyStream::channel(completion_rx);
    drop(body_tx);
    completion_tx
        .send(Ok(()))
        .expect("prepared output completion should send");

    let prepared = prepare_test_external_raw_document(&runtime, &loader, url, raw_body).await;
    let token = prepared.token();
    let expected_residence = RendererOutputResidenceIdentity::Page {
        owner_local_host_id: token.local_host_id(),
        page_id: token.page_id(),
    };
    let opened_stream = match output_rx.recv_message().await {
        RendererOutputTransportMessage::StreamControl(
            super::RendererOutputStreamControl::Opened { stream },
        ) => stream,
        other => panic!("prepared isolate reservation must open its stream first, got {other:?}"),
    };
    assert_eq!(opened_stream.residence(), expected_residence);
    assert!(matches!(
        output_rx.recv_message().await,
        RendererOutputTransportMessage::PageReservationReleased {
            owner_local_host_id,
            page_id,
        } if owner_local_host_id == token.local_host_id() && page_id == token.page_id()
    ));

    prepared
        .cancel()
        .await
        .expect("prepared document should cancel");
    assert!(matches!(
        output_rx.recv_message().await,
        RendererOutputTransportMessage::StreamControl(
            super::RendererOutputStreamControl::Closed {
                stream,
                reason: super::RendererOutputStreamCloseReason::ResidenceRetired,
                ..
            },
        ) if stream == opened_stream
    ));
}
#[tokio::test(flavor = "multi_thread")]
async fn dropping_prepared_external_raw_document_releases_only_its_residence() {
    let runtime = JsRuntime::initialize();
    let baseline_isolates = runtime.document_isolate_accounting_for_diagnostics();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let (base_url, mut side_effect_request_seen, _release_side_effect_response, server) =
        spawn_owner_wake_gated_server_with_content_type(
            "/dropped-author-side-effect",
            "ok",
            "text/plain",
        )
        .await;
    let dropped_url = url::Url::parse(&format!("{base_url}/dropped")).expect("dropped url");
    let (completion_tx, completion_rx) = oneshot::channel();
    let (body_tx, raw_body) = ExternalRawDocumentBodyStream::channel(completion_rx);
    let producer = tokio::spawn(async move {
        body_tx
            .send(
                br#"<!doctype html><script>fetch("/dropped-author-side-effect")</script>"#.to_vec(),
            )
            .await
            .expect("dropped prepared body should send");
        drop(body_tx);
        completion_tx.send(Ok(())).expect("completion should send");
    });
    let dropped =
        prepare_test_external_raw_document(&runtime, &loader, dropped_url, raw_body).await;
    producer.await.expect("dropped body producer should finish");
    drop(dropped);

    let barrier_url =
        url::Url::parse("https://example.test/prepared-drop-barrier").expect("barrier url");
    let (barrier_completion_tx, barrier_completion_rx) = oneshot::channel();
    let (barrier_body_tx, barrier_raw_body) =
        ExternalRawDocumentBodyStream::channel(barrier_completion_rx);
    drop(barrier_body_tx);
    barrier_completion_tx
        .send(Ok(()))
        .expect("barrier completion should send");
    let barrier =
        prepare_test_external_raw_document(&runtime, &loader, barrier_url, barrier_raw_body).await;
    let after_drop = runtime.document_isolate_accounting_for_diagnostics();
    assert_eq!(after_drop.created, baseline_isolates.created + 2);
    assert_eq!(after_drop.destroyed, baseline_isolates.destroyed + 1);
    assert_eq!(after_drop.live, baseline_isolates.live + 1);
    assert_eq!(after_drop.reserved, baseline_isolates.reserved + 1);
    assert!(
        tokio::time::timeout(Duration::from_millis(100), &mut side_effect_request_seen)
            .await
            .is_err(),
        "dropping a prepared document must not execute its author body"
    );

    barrier
        .cancel()
        .await
        .expect("barrier prepared document should cancel");
    let after_barrier = runtime.document_isolate_accounting_for_diagnostics();
    assert_eq!(after_barrier.live, baseline_isolates.live);
    assert_eq!(after_barrier.reserved, baseline_isolates.reserved);
    assert_eq!(after_barrier.destroyed, baseline_isolates.destroyed + 2);
    server.abort();
}
#[tokio::test(flavor = "current_thread")]
async fn external_raw_streaming_empty_document_reaches_dom_content_loaded() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let url = url::Url::parse("https://example.test/empty.html").unwrap();
    let (completion_tx, completion_rx) = oneshot::channel();
    let (body_tx, raw_body) = ExternalRawDocumentBodyStream::channel(completion_rx);

    let producer = tokio::spawn(async move {
        body_tx
            .send(b"<!DOCTYPE html>\n<html></html>".to_vec())
            .await
            .expect("empty html chunk should send");
        drop(body_tx);
        completion_tx.send(Ok(())).expect("completion should send");
    });

    let (page, snapshot, _creation_diagnostics, _creation_artifacts, pending_download) = runtime
        .create_streaming_raw_page_from_external_body(
            url.clone(),
            url.clone(),
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
        .expect("empty external raw streaming page should reach DOMContentLoaded");
    producer.await.expect("producer should finish");

    assert!(pending_download.is_none());
    assert_eq!(snapshot.final_url(), &url);
    assert_eq!(snapshot.status, 200);
    assert!(
        serialize_html_for_renderer_page(&page)
            .await
            .contains("<html")
    );
}
#[tokio::test(flavor = "current_thread")]
async fn same_document_navigation_publication_carries_exact_source_document() {
    let runtime = JsRuntime::initialize();
    let (output_tx, mut output_rx) = renderer_external_activity_test_channel();
    runtime.set_renderer_output_transport_sender(output_tx);
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let page_url = url::Url::parse("https://example.test/source.html").unwrap();
    let mut page = create_test_html_page(
        &runtime,
        &loader,
        page_url,
        "<!doctype html><body>source</body>",
    )
    .await;

    let (snapshot, _) = page
        .run_async_command(RendererPageCommand::PageDiagnosticsSnapshot)
        .await
        .expect("page diagnostics snapshot should be readable");
    let RendererPageReply::PageDiagnosticsSnapshot(snapshot) = snapshot else {
        panic!("expected page diagnostics snapshot");
    };
    let source_document = snapshot
        .document_lifecycle_identity()
        .expect("attached Page should expose its exact Document identity");
    while output_rx.try_recv().is_ok() {}

    page.enqueue_async_command(RendererPageCommand::EvaluateExpression {
        expression: r##"history.pushState(null, "", "#captured");"done""##.to_owned(),
        await_promise: false,
    })
    .expect("same-Document navigation command should enqueue")
    .wait()
    .await
    .expect("same-Document navigation should execute");
    let navigations = output_rx
        .drain()
        .into_iter()
        .flat_map(RendererOutputPublication::into_records)
        .filter_map(|record| match record.into_parts().1 {
            RendererOutputItem::OwnerAction(RendererOwnerAction::SameDocumentNavigation(
                navigation,
            )) => Some(navigation),
            _ => None,
        })
        .collect::<Vec<_>>();

    assert_eq!(navigations.len(), 1);
    assert_eq!(navigations[0].source_document(), source_document);
    assert_eq!(
        navigations[0].navigation().url,
        "https://example.test/source.html#captured"
    );
    page.close_async()
        .await
        .expect("same-Document navigation test page should close");
}
#[tokio::test(flavor = "current_thread")]
async fn document_open_preserves_document_sourced_navigation_handoffs() {
    let runtime = JsRuntime::initialize();
    let (output_tx, mut output_rx) = renderer_external_activity_test_channel();
    runtime.set_renderer_output_transport_sender(output_tx);
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let page_url = url::Url::parse("https://example.test/source.html").unwrap();
    let mut page = create_test_html_page_with_navigation_dispatch(
        &runtime,
        &loader,
        page_url,
        "<!doctype html><body>source</body>",
        RendererTopLevelNavigationDispatch::DelegateToBrowser,
    )
    .await;

    let (snapshot, _) = page
        .run_async_command(RendererPageCommand::PageDiagnosticsSnapshot)
        .await
        .expect("source snapshot should be readable");
    let RendererPageReply::PageDiagnosticsSnapshot(snapshot) = snapshot else {
        panic!("expected source activity snapshot");
    };
    let source_document = snapshot
        .document_lifecycle_identity()
        .expect("source Document identity should exist");
    while output_rx.try_recv().is_ok() {}

    page.enqueue_async_command(RendererPageCommand::EvaluateExpression {
        expression: r##"
history.pushState(null, "", "#retired");
location.href = "https://example.test/pending-target.html";
document.open();
document.write("<!doctype html><body>replacement</body>");
document.close();
"done";
"##
        .to_owned(),
        await_promise: false,
    })
    .expect("document.open replacement command should enqueue")
    .wait()
    .await
    .expect("document.open replacement should execute");
    let records = output_rx
        .drain()
        .into_iter()
        .flat_map(RendererOutputPublication::into_records)
        .collect::<Vec<_>>();

    let (snapshot, _) = page
        .run_async_command(RendererPageCommand::PageDiagnosticsSnapshot)
        .await
        .expect("replacement snapshot should be readable");
    let RendererPageReply::PageDiagnosticsSnapshot(snapshot) = snapshot else {
        panic!("expected replacement activity snapshot");
    };
    let replacement_document = snapshot
        .document_lifecycle_identity()
        .expect("replacement Document identity should exist");
    assert_ne!(replacement_document, source_document);

    let navigations = records
        .iter()
        .filter_map(|record| match record.item() {
            RendererOutputItem::OwnerAction(RendererOwnerAction::SameDocumentNavigation(
                navigation,
            )) => Some(navigation),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(navigations.len(), 1);
    assert_eq!(
        navigations[0].source_document(),
        source_document,
        "document.open must not relabel the already-applied history mutation"
    );
    assert_eq!(
        navigations[0].navigation().url,
        "https://example.test/source.html#retired"
    );

    let navigation = records
        .iter()
        .find_map(|record| match record.item() {
            RendererOutputItem::OwnerAction(RendererOwnerAction::TopLevelLocationNavigation(
                navigation,
            )) => Some(navigation),
            _ => None,
        })
        .expect("expected concrete top-level location navigation action");
    assert_eq!(
        navigation.source_document(),
        source_document,
        "location action must retain the producer Document rather than adopt the replacement"
    );
    assert_ne!(navigation.source_document(), replacement_document);
    assert_eq!(navigation.url(), "https://example.test/pending-target.html");
    page.close_async()
        .await
        .expect("document.open navigation identity test page should close");
}
#[tokio::test(flavor = "current_thread")]
async fn committed_navigation_bootstrap_failure_retires_page_before_follow_settlement() {
    assert_committed_navigation_bootstrap_injection_retires_page(
        "x-moli-test-fail-after-navigation-commit",
        "injected failure after main navigation commit for testing",
    )
    .await;
}
#[tokio::test(flavor = "current_thread")]
async fn committed_navigation_bootstrap_panic_returns_committed_entry_to_owner() {
    assert_committed_navigation_bootstrap_injection_retires_page(
        "x-moli-test-panic-after-navigation-commit",
        "local task panicked before restoring its page entry",
    )
    .await;
}
#[tokio::test(flavor = "current_thread")]
async fn external_raw_streaming_delegates_post_load_meta_refresh_to_browser() {
    let runtime = JsRuntime::initialize();
    let (activity_wake_tx, mut activity_wake_rx) = renderer_external_activity_test_channel();
    runtime.set_renderer_output_transport_sender(activity_wake_tx);
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let url = url::Url::parse("https://example.test/redirect_http_equiv.html").unwrap();
    let (completion_tx, completion_rx) = oneshot::channel();
    let (body_tx, raw_body) = ExternalRawDocumentBodyStream::channel(completion_rx);

    let producer = tokio::spawn(async move {
        body_tx
            .send(
                br#"<!doctype html><head><meta http-equiv="refresh" content="0;redirected.html"></head>"#
                    .to_vec(),
            )
            .await
            .expect("meta refresh html chunk should send");
        drop(body_tx);
        completion_tx.send(Ok(())).expect("completion should send");
    });

    let (page, snapshot, _creation_diagnostics, creation_artifacts, pending_download) = runtime
        .create_streaming_raw_page_from_external_body(
            url.clone(),
            url.clone(),
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
            RendererTopLevelNavigationDispatch::DelegateToBrowser,
            RendererNavigationReplyPolicy::ReturnWithPendingNavigation,
            None,
            None,
            crate::RendererDocumentOptions {
                ..Default::default()
            },
        )
        .await
        .expect("meta refresh streaming page should attach first document");
    producer.await.expect("producer should finish");
    assert!(pending_download.is_none());
    assert_eq!(snapshot.final_url(), &url);
    assert!(
        creation_artifacts.lifecycle_snapshot.load.is_some(),
        "the source Document must reach load before an immediate refresh comes due"
    );
    assert_eq!(
        creation_artifacts.active_document,
        creation_artifacts.lifecycle_snapshot.document
    );
    assert_eq!(
        creation_artifacts.active_epoch,
        creation_artifacts.lifecycle_snapshot.epoch
    );
    let expected_source_document = creation_artifacts.lifecycle_snapshot.into();
    let navigation = tokio::time::timeout(
        Duration::from_millis(500),
        activity_wake_rx.recv_top_level_location_navigation(),
    )
    .await
    .expect("load and its exact-source meta refresh should publish a concrete navigation action")
    .expect("external activity transport should stay open");
    assert_eq!(navigation.source_document(), expected_source_document);
    assert_eq!(navigation.url(), "https://example.test/redirected.html");
    assert_eq!(
        has_pending_location_navigation_for_test(&page).await,
        Some(false),
        "the browser-owned action must be moved into concrete output instead of remaining mutable Page state"
    );
}
#[tokio::test(flavor = "current_thread")]
async fn external_raw_streaming_defers_dcl_handler_navigation_to_page_reply() {
    let runtime = JsRuntime::initialize();
    let (activity_wake_tx, mut activity_wake_rx) = renderer_external_activity_test_channel();
    runtime.set_renderer_output_transport_sender(activity_wake_tx);
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let url = url::Url::parse("https://example.test/dcl-handler.html").unwrap();
    let (completion_tx, completion_rx) = oneshot::channel();
    let (body_tx, raw_body) = ExternalRawDocumentBodyStream::channel(completion_rx);
    let producer = tokio::spawn(async move {
        body_tx
            .send(
                br#"<!doctype html><script>document.addEventListener('DOMContentLoaded', () => location.href = '/next.html', {once:true})</script><main>source</main>"#
                    .to_vec(),
            )
            .await
            .expect("DCL handler document should send");
        drop(body_tx);
        completion_tx.send(Ok(())).expect("completion should send");
    });

    let (page, _, _, creation_artifacts, pending_download) = runtime
        .create_streaming_raw_page_from_external_body(
            url.clone(),
            url,
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
            RendererTopLevelNavigationDispatch::DelegateToBrowser,
            RendererNavigationReplyPolicy::ReturnWithPendingNavigation,
            None,
            None,
            crate::RendererDocumentOptions {
                ..Default::default()
            },
        )
        .await
        .expect("DCL handler page should reach its reply boundary");
    producer.await.expect("producer should finish");

    assert!(pending_download.is_none());
    assert!(
        creation_artifacts
            .lifecycle_snapshot
            .dom_content_loaded
            .is_some()
    );
    let expected_source_document = creation_artifacts.lifecycle_snapshot.into();
    let navigation = tokio::time::timeout(
        Duration::from_millis(500),
        activity_wake_rx.recv_top_level_location_navigation(),
    )
    .await
    .expect("the exact DCL action should publish its concrete browser navigation")
    .expect("external activity transport should stay open");
    assert_eq!(navigation.source_document(), expected_source_document);
    assert_eq!(navigation.url(), "https://example.test/next.html");
    assert_eq!(
        has_pending_location_navigation_for_test(&page).await,
        Some(false),
        "the browser-owned DCL navigation must not remain as mutable Page state"
    );
}
#[tokio::test(flavor = "current_thread")]
async fn external_raw_streaming_dcl_reply_resumes_ordinary_page_work() {
    let runtime = JsRuntime::initialize();
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default())
        .expect("default resource request client");
    let url = url::Url::parse("https://example.test/dcl-resume.html").unwrap();
    let (completion_tx, completion_rx) = oneshot::channel();
    let (body_tx, raw_body) = ExternalRawDocumentBodyStream::channel(completion_rx);
    let producer = tokio::spawn(async move {
        body_tx
            .send(
                br#"<!doctype html><script>
globalThis.__ordinaryAfterDcl = new Promise(resolve => {
  document.addEventListener('DOMContentLoaded', () => {
    setTimeout(() => resolve('resumed'), 0);
  }, {once: true});
});
</script>"#
                    .to_vec(),
            )
            .await
            .expect("DCL resume document should send");
        drop(body_tx);
        completion_tx.send(Ok(())).expect("completion should send");
    });

    let (mut page, _, _, creation_artifacts, pending_download) = runtime
        .create_streaming_raw_page_from_external_body(
            url.clone(),
            url,
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
        .expect("page should reach its DCL reply boundary");
    producer.await.expect("producer should finish");

    assert!(pending_download.is_none());
    assert!(
        creation_artifacts
            .lifecycle_snapshot
            .dom_content_loaded
            .is_some()
    );
    let (reply, _) = tokio::time::timeout(
        Duration::from_secs(2),
        page.run_async_command(RendererPageCommand::EvaluateExpression {
            expression: "__ordinaryAfterDcl".to_owned(),
            await_promise: true,
        }),
    )
    .await
    .expect("ordinary work held at the DCL reply boundary should be resumed")
    .expect("DCL continuation result should evaluate");
    assert_eq!(
        renderer_json_value(reply),
        Some(serde_json::json!("resumed"))
    );

    page.close_async()
        .await
        .expect("DCL continuation test page should close");
}
#[tokio::test(flavor = "current_thread")]
async fn renderer_owned_navigation_survives_page_creation_observer_detach() {
    let runtime = JsRuntime::initialize();
    let (activity_wake_tx, mut activity_wake_rx) = renderer_external_activity_test_channel();
    runtime.set_renderer_output_transport_sender(activity_wake_tx);
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let source_url = url::Url::parse("https://example.test/renderer-owned-source").unwrap();
    let replacement_url = url::Url::parse(
        "data:text/html,%3Cmain%20id%3D%22replacement%22%3Erenderer-owned%3C%2Fmain%3E",
    )
    .unwrap();
    let (completion_tx, completion_rx) = oneshot::channel();
    let (body_tx, raw_body) = ExternalRawDocumentBodyStream::channel(completion_rx);
    let replacement_href = replacement_url.as_str().to_owned();
    let producer = tokio::spawn(async move {
        body_tx
            .send(
                format!(
                    "<!doctype html><script>window.addEventListener('load', () => location.href = {replacement_href:?}, {{once:true}})</script><main>source</main>"
                )
                .into_bytes(),
            )
            .await
            .expect("renderer-owned source should send");
        drop(body_tx);
        completion_tx.send(Ok(())).expect("completion should send");
    });

    let (mut page, _, _, creation_artifacts, pending_download) = runtime
        .create_streaming_raw_page_from_external_body(
            source_url.clone(),
            source_url,
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
            RendererReplyBoundary::DocumentCommit,
            RendererTopLevelNavigationDispatch::FollowInStandaloneAdapter,
            RendererNavigationReplyPolicy::FollowBeforeReply,
            None,
            None,
            crate::RendererDocumentOptions {
                ..Default::default()
            },
        )
        .await
        .expect("source should attach at its document commit boundary");
    producer.await.expect("producer should finish");
    assert!(pending_download.is_none());
    assert!(creation_artifacts.lifecycle_snapshot.load.is_none());
    page.take_committed_document_post_response_continuation()
        .expect("DocumentCommit should defer parser continuation")
        .release();

    let initial_document = creation_artifacts.active_document;
    let replacement_document = tokio::time::timeout(Duration::from_secs(2), async {
        while let Some(publication) = activity_wake_rx.recv().await {
            if let Some(document) = publication_document_lifecycle_events(&publication)
                .find(|event| {
                    event.document != initial_document
                        && event.kind
                            == RendererDocumentLifecycleEventKind::Milestone(
                                RendererDocumentLifecycleMilestone::Load,
                            )
                })
                .map(|event| event.document)
            {
                return document;
            }
        }
        panic!("external activity wake channel closed before renderer-owned replacement")
    })
    .await
    .expect("renderer ownership should outlive the detached creation observer");
    assert_ne!(replacement_document, initial_document);

    // A concrete lifecycle record is a source-owned fact, not a wake granting
    // permission to snapshot mutable Page state. The final navigation
    // continuation may still be immediately behind this publication and it
    // explicitly permits one ready command to overtake. Serializing the live
    // replacement Document consumes that bounded overtake; the following
    // owner-state query is therefore ordered after the final commit.
    let html = serialize_html_for_renderer_page(&page).await;
    assert!(html.contains("renderer-owned"));
    let final_snapshot = RendererPageTestingHandle::new_for_testing(&page)
        .current_page_state_async()
        .await
        .expect("replacement commit should refresh the owner Page state");
    assert_eq!(final_snapshot.final_url(), &replacement_url);
    assert_eq!(
        has_pending_location_navigation_for_test(&page).await,
        Some(false)
    );
    page.close_async()
        .await
        .expect("renderer-owned navigation test page should close");
}
#[tokio::test(flavor = "multi_thread")]
async fn replacement_navigation_releases_old_inspector_deferred_response_callbacks() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let page_url = url::Url::parse("https://example.test/deferred-navigation-source").unwrap();
    let mut page = create_test_html_page(
        &runtime,
        &loader,
        page_url,
        "<!doctype html><body>deferred navigation source</body>",
    )
    .await;
    let mut pending_responses = Vec::new();
    for index in 0..12 {
        let call_id = 710_001 + index;
        let inspector_session_id = (index % 2 == 1).then(|| "session-a".to_owned());
        let (response_tx, mut response_rx) = oneshot::channel();
        let (dispatch, _) = page
            .run_async_command(
                RendererPageCommand::dispatch_runtime_protocol_message_with_deferred_response(
                    inspector_session_id,
                    serde_json::json!({
                        "id": call_id,
                        "method": "Runtime.evaluate",
                        "params": {
                            "expression": format!(
                                "(globalThis.__pendingInspectorPromises ??= [], globalThis.__pendingInspectorPromises[{index}] = new Promise(() => {{}}))"
                            ),
                            "awaitPromise": true,
                        },
                    })
                    .to_string(),
                    RendererRuntimeInspectorResponseSender::new(
                        call_id,
                        response_tx,
                    ),
                ),
            )
            .await
            .expect("never-settling Runtime.evaluate should register a deferred callback");
        assert!(matches!(
            dispatch,
            RendererPageReply::RuntimeInspectorProtocolMessages(ref messages) if messages.is_empty()
        ));
        assert!(matches!(
            response_rx.try_recv(),
            Err(tokio::sync::oneshot::error::TryRecvError::Empty)
        ));
        pending_responses.push((call_id, response_rx));
    }

    let replacement_url = "data:text/html,<!doctype html><body>replacement</body>";
    page.run_async_command(
        RendererPageCommand::EvaluateExpressionAndFollowPendingNavigation {
            expression: format!("location.href = {replacement_url:?}; 'navigating'"),
            await_promise: false,
        },
    )
    .await
    .expect("renderer-owned navigation should install the replacement PageVm");

    for (call_id, mut response_rx) in pending_responses {
        let completion = response_rx
            .try_recv()
            .expect("V8 teardown should finish each old deferred command");
        assert_eq!(completion.call_id, call_id);
        let response = completion
            .output
            .protocol_response(call_id)
            .expect("teardown completion should contain its protocol response");
        assert_eq!(response["id"], serde_json::json!(call_id));
        assert_eq!(response["error"]["code"], serde_json::json!(-32000));
        assert_eq!(
            response["error"]["message"],
            serde_json::json!("Execution context was destroyed.")
        );
    }

    let reused_call_id = 710_001;
    let (replacement_response_tx, mut replacement_response_rx) = oneshot::channel();
    let replacement_completion = page
        .enqueue_async_command(
            RendererPageCommand::dispatch_runtime_protocol_message_with_deferred_response(
                Some("session-a".to_owned()),
                serde_json::json!({
                    "id": reused_call_id,
                    "method": "Runtime.evaluate",
                    "params": {
                        "expression": "42",
                        "awaitPromise": true,
                        "returnByValue": true,
                    },
                })
                .to_string(),
                RendererRuntimeInspectorResponseSender::new(
                    reused_call_id,
                    replacement_response_tx,
                ),
            ),
        )
        .expect("replacement PageVM command should enqueue")
        .wait()
        .await
        .expect("replacement PageVM should accept a reused frontend call id");
    let (replacement_completion, _renderer_output_predecessor) =
        replacement_completion.into_completion_and_predecessor();
    let replacement_response = replacement_completion
        .runtime_inspector_output()
        .and_then(|output| output.protocol_response(reused_call_id))
        .expect("synchronous replacement response should stay in the command completion");
    assert_eq!(
        replacement_response["result"]["result"]["value"],
        serde_json::json!(42)
    );
    let (replacement_dispatch, _, _) = replacement_completion.into_parts();
    assert!(matches!(
        replacement_dispatch,
        RendererPageReply::RuntimeInspectorProtocolMessages(ref messages) if !messages.is_empty()
    ));
    assert!(matches!(
        replacement_response_rx.try_recv(),
        Err(tokio::sync::oneshot::error::TryRecvError::Closed)
    ));
    page.close_async()
        .await
        .expect("replacement page should close");
}
#[tokio::test(flavor = "multi_thread")]
async fn protocol_navigation_never_reuses_the_replaced_document_globals_snapshot() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let initial_url =
        url::Url::parse("https://example.test/globals-snapshot/old").expect("page url");
    let mut page = create_test_html_page(
        &runtime,
        &loader,
        initial_url,
        r#"<!doctype html><script>globalThis.__lm_old_document_global = "old";</script>"#,
    )
    .await;
    let replacement_html = r#"<!doctype html><title>new document</title><script>
globalThis.__lm_new_document_global = "new";
</script>"#;
    let encoded_replacement_html =
        percent_encoding::utf8_percent_encode(replacement_html, percent_encoding::NON_ALPHANUMERIC);
    let replacement_url = format!("data:text/html;charset=utf-8,{encoded_replacement_html}");

    let output = page
        .enqueue_protocol_command(
            RendererPageCommand::EvaluateExpressionAndFollowPendingNavigation {
                expression: format!(r#"location.href = {replacement_url:?}; "navigating""#),
                await_promise: false,
            },
        )
        .expect("protocol navigation command should enqueue")
        .wait()
        .await
        .expect("protocol navigation command should finish");
    let replacement_state = output.completion().page_state();

    assert_eq!(replacement_state.document_title(), "new document");
    assert_eq!(
        replacement_state.script_execution.globals_snapshot_state(),
        crate::types::ScriptGlobalsSnapshotState::Dirty
    );
    assert!(
        replacement_state
            .script_execution
            .global("__lm_old_document_global")
            .is_none(),
        "an old-Document snapshot must never cross the replacement boundary"
    );
    assert!(
        replacement_state
            .script_execution
            .global("__lm_new_document_global")
            .is_none(),
        "the replacement's pre-lifecycle full capture may predate its script, so Dirty must not pretend that value is current"
    );

    let (_, refreshed) = page
        .run_async_command(RendererPageCommand::RefreshFullPageState)
        .await
        .expect("replacement full report refresh should finish");
    assert!(refreshed.script_execution.globals_are_fresh());
    assert_eq!(
        refreshed
            .script_execution
            .global("__lm_new_document_global"),
        Some(&crate::types::JsValueSnapshot::String("new".to_owned()))
    );

    page.close_async()
        .await
        .expect("replacement globals freshness test page should close");
}
#[tokio::test(flavor = "multi_thread")]
async fn followed_navigation_replays_ready_document_write_after_older_timer_turn() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let (base_url, request_seen, release_response, server) =
        spawn_owner_wake_gated_server_with_content_type(
            "/replacement-document-write-after-timer.js",
            "globalThis.__lm_replacement_document_write_events.push('external');",
            "application/javascript",
        )
        .await;
    let initial_url = url::Url::parse(&format!("{base_url}/initial")).expect("initial page url");
    let mut page = create_test_html_page(
        &runtime,
        &loader,
        initial_url,
        "<!doctype html><body>initial</body>",
    )
    .await;
    let replacement_html = format!(
        r#"<!doctype html><script>
globalThis.__lm_replacement_document_write_events = ['inline-before'];
setTimeout(() => globalThis.__lm_replacement_document_write_events.push('timer'), 0);
document.write(`<script src="{base_url}/replacement-document-write-after-timer.js" onload="globalThis.__lm_replacement_document_write_events.push('load')"><\/script>`);
globalThis.__lm_replacement_document_write_events.push('inline-after');
</script>"#,
    );
    let encoded_replacement_html = percent_encoding::utf8_percent_encode(
        &replacement_html,
        percent_encoding::NON_ALPHANUMERIC,
    );
    let replacement_url = format!("data:text/html;charset=utf-8,{encoded_replacement_html}");
    let release = tokio::spawn(async move {
        request_seen
            .await
            .expect("replacement document.write request should start");
        release_response
            .send(())
            .expect("replacement document.write response should release");
    });

    let (reply, _) = page
        .run_async_command(
            RendererPageCommand::EvaluateExpressionAndFollowPendingNavigation {
                expression: format!(r#"location.href = {replacement_url:?}; "navigating""#),
                await_promise: false,
            },
        )
        .await
        .expect("document.write replacement navigation should complete naturally");
    assert_eq!(
        renderer_json_value(reply),
        Some(serde_json::json!("navigating"))
    );
    release
        .await
        .expect("replacement response release task should finish");
    server
        .await
        .expect("replacement document.write server should finish");

    let (reply, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: "JSON.stringify(globalThis.__lm_replacement_document_write_events)"
                .to_owned(),
            await_promise: false,
        })
        .await
        .expect("replacement timer/document.write result should evaluate");
    assert_eq!(
        renderer_json_value(reply),
        Some(serde_json::json!(
            r#"["inline-before","inline-after","timer","external","load"]"#
        )),
        "followed navigation must preserve source-specific admission across PageVm installation"
    );

    page.close_async()
        .await
        .expect("replacement document.write page should close");
}
