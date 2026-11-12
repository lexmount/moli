use super::*;

#[tokio::test(flavor = "multi_thread")]
async fn scriptless_document_keeps_main_world_lazy_until_evaluate() {
    let runtime = JsRuntime::initialize();
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let url = url::Url::parse("https://example.test/lazy-main-world").unwrap();
    let (mut page, _, _, _, download) = runtime.create_html_page_from_response(
        url.clone(), url, None, false, 0, 200,
        vec![("content-type".to_owned(), b"text/html".to_vec())],
        &loader, crate::RendererWebStorageHandles::ephemeral(),
        "<!doctype html><title>Lazy Document</title><style>p {color:blue}</style><p>Native content</p>".to_owned(),
        crate::RendererDocumentOptions::default(),
    ).await.expect("scriptless page must reach load without a main world");
    assert!(download.is_none());
    for format in [
        crate::RendererPageDumpFormat::Html,
        crate::RendererPageDumpFormat::Markdown,
    ] {
        let (dump, _) = page
            .run_async_command(RendererPageCommand::RenderPageDump {
                options: crate::RendererPageDumpOptions {
                    format,
                    strip: Default::default(),
                    with_base: false,
                    with_frames: false,
                },
            })
            .await
            .expect("native export should not create a script realm");
        let RendererPageReply::OptionalString(Some(dump)) = dump else {
            panic!("missing dump")
        };
        assert!(dump.contains("Native content"), "{dump}");
    }
    let before = runtime_heap_usage_for_test(&page).await;
    assert_eq!(
        before["moli"]["runtime"]["inspectorContextRegistrationCount"], 0,
        "{before}"
    );
    assert_eq!(
        before["moli"]["runtime"]["mainWindowProxyIdentityHash"],
        serde_json::Value::Null,
        "{before}"
    );
    let (reply, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression:
                "document.title + ':' + document.readyState + ':' + document.body.textContent"
                    .to_owned(),
            await_promise: false,
        })
        .await
        .expect("evaluate must initialize the same live Document");
    assert_eq!(
        renderer_json_value(reply),
        Some(serde_json::json!("Lazy Document:complete:Native content"))
    );
    assert_eq!(
        evaluate_lazy_test(&page, "getComputedStyle(document.querySelector('p')).color").await,
        "rgb(0, 0, 255)"
    );
    let after = runtime_heap_usage_for_test(&page).await;
    assert_eq!(
        after["moli"]["runtime"]["inspectorContextRegistrationCount"], 1,
        "{after}"
    );
    page.close_async().await.unwrap();
}

async fn lazy_test_page(
    html: &str,
    options: crate::RendererDocumentOptions,
) -> (JsRuntimeOwner, RendererPageHandle) {
    let runtime = JsRuntime::initialize();
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let url = url::Url::parse("https://example.test/lazy-main-world").unwrap();
    let page = runtime
        .create_html_page_from_response(
            url.clone(),
            url,
            None,
            false,
            0,
            200,
            vec![("content-type".to_owned(), b"text/html".to_vec())],
            &loader,
            crate::RendererWebStorageHandles::ephemeral(),
            html.to_owned(),
            options,
        )
        .await
        .unwrap()
        .0;
    (runtime, page)
}

async fn assert_main_world_count(page: &RendererPageHandle, expected: u64) {
    let usage = runtime_heap_usage_for_test(page).await;
    assert_eq!(usage["numberOfNativeContexts"], expected, "{usage}");
    assert_eq!(
        usage["moli"]["runtime"]["inspectorContextRegistrationCount"], expected,
        "{usage}"
    );
}

async fn evaluate_lazy_test(page: &RendererPageHandle, source: &str) -> serde_json::Value {
    let (reply, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: source.to_owned(),
            await_promise: false,
        })
        .await
        .unwrap();
    renderer_json_value(reply).expect("evaluation returns a JSON value")
}

#[tokio::test(flavor = "multi_thread")]
async fn native_environment_updates_survive_main_world_activation() {
    let (_runtime, mut page) = lazy_test_page(
        "<!doctype html><p>Native environment before script</p>",
        Default::default(),
    )
    .await;
    assert_main_world_count(&page, 0).await;
    for command in [
        RendererPageCommand::SetNavigatorOverrides(moli_page_types::NavigatorOverrides {
            online: Some(false),
            max_touch_points: Some(7),
            ..Default::default()
        }),
        RendererPageCommand::SetIdleOverride(Some(moli_page_types::EmulatedIdleOverride {
            is_user_active: false,
            is_screen_unlocked: false,
        })),
        RendererPageCommand::SetEmulatedMedia(moli_page_types::EmulatedMediaOverrides {
            media: Some("print".to_owned()),
            color_scheme: Some("dark".to_owned()),
            ..Default::default()
        }),
    ] {
        let (reply, _) = page.run_async_command(command).await.unwrap();
        assert!(matches!(reply, RendererPageReply::Unit));
        assert_main_world_count(&page, 0).await;
    }
    let expression = "JSON.stringify({online:navigator.onLine, touch:navigator.maxTouchPoints, print:matchMedia('print').matches, dark:matchMedia('(prefers-color-scheme: dark)').matches})";
    assert_eq!(
        evaluate_lazy_test(&page, expression).await,
        r#"{"online":false,"touch":7,"print":true,"dark":true}"#
    );
    assert_main_world_count(&page, 1).await;

    page.run_async_command(RendererPageCommand::SetNavigatorOverrides(
        moli_page_types::NavigatorOverrides {
            online: Some(true),
            max_touch_points: Some(2),
            ..Default::default()
        },
    ))
    .await
    .unwrap();
    assert_eq!(
        evaluate_lazy_test(&page, expression).await,
        r#"{"online":true,"touch":2,"print":true,"dark":true}"#
    );
    assert_main_world_count(&page, 1).await;
    page.close_async().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn unobserved_inline_attributes_and_native_toggle_tasks_keep_main_world_lazy() {
    for html in [
        "<!doctype html><button onclick='globalThis.unusedHandler=1'>Native button</button>",
        "<!doctype html><p onclick='function ('>Uncompiled malformed source</p>",
        "<!doctype html><div onmadeupevent='throw 1'>Unknown event attribute</div>",
        "<!doctype html><div onload='globalThis.unusedHandler=1'>Unrelated load target</div>",
        "<!doctype html><body onerror='globalThis.unusedHandler=1'>Unused error handler",
        "<!doctype html><details open><summary>Native details</summary>Native toggle</details>",
    ] {
        let (_runtime, mut page) = lazy_test_page(html, Default::default()).await;
        assert_main_world_count(&page, 0).await;
        page.close_async().await.unwrap();
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn parser_details_group_exclusivity_is_independent_of_realm_activation() {
    for name in ["group", " ", ""] {
        for prefix in ["", "<script>void 0</script>"] {
            let html = format!(
                "<!doctype html>{prefix}\
                 <details id='a' name='{name}' open><summary>A</summary>A content</details>\
                 <details id='b' name='{name}' open><summary>B</summary>B content</details>"
            );
            let (_runtime, mut page) = lazy_test_page(&html, Default::default()).await;
            // Inspect the native state before any evaluation or Runtime agent:
            // named groups need the full insertion pipeline, empty names do not.
            let expected_contexts = u64::from(!name.is_empty() || !prefix.is_empty());
            assert_main_world_count(&page, expected_contexts).await;
            assert_eq!(
                evaluate_lazy_test(
                    &page,
                    "JSON.stringify([document.querySelector('#a').open, document.querySelector('#b').open])",
                )
                .await,
                serde_json::Value::String(serde_json::json!([true, name.is_empty()]).to_string()),
                "name={name:?}, prefix={prefix:?}"
            );
            assert_main_world_count(&page, 1).await;
            page.close_async().await.unwrap();
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn native_input_activates_the_live_document_and_its_inline_handler_once() {
    let runtime = initialize_layout_test_runtime();
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut page = create_test_html_page(
        &runtime,
        &loader,
        url::Url::parse("https://example.test/lazy-native-input").unwrap(),
        "<!doctype html><button style='position:absolute;left:10px;top:10px;width:200px;height:80px' onclick='globalThis.hits=(globalThis.hits||0)+1;globalThis.receiver=this===event.currentTarget;globalThis.trusted=event.isTrusted'>Native button</button>",
    ).await;
    assert_main_world_count(&page, 0).await;
    capture_screenshot_for_renderer_page(&page).await;
    assert_main_world_count(&page, 0).await;
    for event in ["mousemove", "mousedown", "mouseup"] {
        let (reply, _) = page
            .run_async_command(RendererPageCommand::DispatchMouseEventAtPoint {
                x: 30.0,
                y: 30.0,
                event_name: event.to_owned(),
                button: if event == "mousemove" { -1 } else { 0 },
                buttons: Some(if event == "mousedown" { 1 } else { 0 }),
                click_count: 1,
                delta_x: 0.0,
                delta_y: 0.0,
                pointer: RendererPointerEventProperties::default(),
                modifiers: 0,
                intercept_drags: false,
            })
            .await
            .unwrap();
        assert!(matches!(reply, RendererPageReply::InputDispatchOutcome(_)));
    }
    assert_main_world_count(&page, 1).await;
    assert_eq!(
        evaluate_lazy_test(&page, "JSON.stringify({hits,receiver,trusted})").await,
        r#"{"hits":1,"receiver":true,"trusted":true}"#
    );
    page.close_async().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn cold_runtime_binding_install_initializes_main_world() {
    for execution_context_id in [None, Some(0)] {
        let (_runtime, mut page) = lazy_test_page(
            "<!doctype html><p>Native binding installation</p>",
            Default::default(),
        )
        .await;
        assert_main_world_count(&page, 0).await;
        let (reply, _) = page
            .run_async_command(RendererPageCommand::InstallRuntimeBinding {
                name: "coldBinding".to_owned(),
                execution_context_name: None,
                execution_context_id,
            })
            .await
            .expect("native binding installation must activate the default realm");
        assert!(matches!(reply, RendererPageReply::Unit));
        assert_main_world_count(&page, 1).await;
        assert_eq!(
            evaluate_lazy_test(&page, "typeof coldBinding").await,
            "function"
        );
        let (reply, _) = page
            .run_async_command(RendererPageCommand::RemoveRuntimeBinding(
                "coldBinding".to_owned(),
            ))
            .await
            .expect("a materialized binding must remain removable");
        assert!(matches!(reply, RendererPageReply::Unit));
        assert_eq!(
            evaluate_lazy_test(&page, "typeof coldBinding").await,
            "undefined"
        );
        page.close_async().await.unwrap();
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn cold_runtime_binding_removal_keeps_main_world_deferred() {
    for command in [
        RendererPageCommand::RemoveRuntimeBinding("coldBinding".to_owned()),
        RendererPageCommand::RemoveDefaultRuntimeBinding("coldBinding".to_owned()),
    ] {
        let (_runtime, mut page) = lazy_test_page(
            "<!doctype html><p>Native binding removal</p>",
            Default::default(),
        )
        .await;
        assert_main_world_count(&page, 0).await;
        let (reply, _) = page
            .run_async_command(command)
            .await
            .expect("removing a binding without a realm must complete natively");
        assert!(matches!(reply, RendererPageReply::Unit));
        assert_main_world_count(&page, 0).await;
        assert_eq!(
            evaluate_lazy_test(&page, "typeof coldBinding").await,
            "undefined"
        );
        assert_main_world_count(&page, 1).await;
        page.close_async().await.unwrap();
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn cold_runtime_binding_protocol_removal_clears_pending_world_registration() {
    for remove in [false, true] {
        let (_runtime, mut page) = lazy_test_page(
            "<!doctype html><p>Pending named-world binding</p>",
            Default::default(),
        )
        .await;
        let binding = crate::protocol_types::RuntimeBindingRegistration {
            devtools_session: None,
            name: "pendingBinding".to_owned(),
            execution_context_name: Some("pending-binding-world".to_owned()),
        };
        set_runtime_binding_state_for_test(
            &page,
            None,
            vec![
                binding
                    .clone()
                    .with_devtools_session(DevToolsSessionKey::Primary),
            ],
            vec![binding.clone()],
        )
        .await
        .unwrap();
        assert_main_world_count(&page, 0).await;
        if remove {
            page.run_async_command(RendererPageCommand::runtime_binding_events(
                None, binding, true,
            ))
            .await
            .expect("protocol removal must clear a registration before its world exists");
        }
        let execution_context_id =
            create_isolated_world_runtime_activity_for_test(&page, None, "pending-binding-world")
                .await
                .unwrap();
        let (reply, _) = page
            .run_async_command(RendererPageCommand::EvaluateExpressionInExecutionContext {
                execution_context_id,
                expression: "typeof pendingBinding".to_owned(),
                await_promise: false,
            })
            .await
            .unwrap();
        assert_eq!(
            renderer_json_value(reply),
            Some(serde_json::json!(if remove {
                "undefined"
            } else {
                "function"
            })),
            "a removed registration must not be restored when its world is created"
        );
        page.close_async().await.unwrap();
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn scriptless_document_can_close_without_initializing_main_world() {
    for html in [
        "<!doctype html><p>Never executes script</p>",
        "<!doctype html><script type='application/ld+json'>{\"name\":\"Native data block\"}</script><p>Never executes script</p>",
    ] {
        let (_runtime, mut page) = lazy_test_page(html, Default::default()).await;
        assert_main_world_count(&page, 0).await;
        page.close_async().await.unwrap();
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn lazy_main_world_preserves_document_timing_without_replaying_load() {
    let before = crate::context_bootstrap::unix_epoch_millis();
    let (_runtime, mut page) = lazy_test_page(
        "<!doctype html><title>Before JS</title><p>Complete</p>",
        Default::default(),
    )
    .await;
    let loaded = crate::context_bootstrap::unix_epoch_millis();
    assert_main_world_count(&page, 0).await;
    let value = evaluate_lazy_test(&page, r#"JSON.stringify({
        title: document.title, readyState: document.readyState,
        origin: performance.timeOrigin, timing: performance.getEntriesByType('navigation')[0].toJSON(),
        historyLength: history.length
    })"#).await;
    let value: serde_json::Value = serde_json::from_str(value.as_str().unwrap()).unwrap();
    assert_eq!(value["title"], "Before JS");
    assert_eq!(value["readyState"], "complete");
    let origin = value["origin"].as_f64().unwrap();
    assert!(origin >= before && origin <= loaded, "{value}");
    let timing = &value["timing"];
    let dcl_start = timing["domContentLoadedEventStart"].as_f64().unwrap();
    let dcl_end = timing["domContentLoadedEventEnd"].as_f64().unwrap();
    let load_start = timing["loadEventStart"].as_f64().unwrap();
    let load_end = timing["loadEventEnd"].as_f64().unwrap();
    assert!(
        0.0 < dcl_start && dcl_start <= dcl_end && dcl_end <= load_start && load_start <= load_end,
        "{value}"
    );
    assert_eq!(value["historyLength"], 1);
    assert_eq!(evaluate_lazy_test(&page, "globalThis.lazyLoadCount = 0; addEventListener('load', () => lazyLoadCount++); lazyLoadCount").await, 0);
    assert_eq!(evaluate_lazy_test(&page, "lazyLoadCount").await, 0);
    assert_main_world_count(&page, 1).await;
    page.close_async().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn first_parser_script_and_inline_load_handler_initialize_main_world() {
    for html in [
        "<!doctype html><script>globalThis.firstScriptCount = (globalThis.firstScriptCount || 0) + 1</script>",
        "<!doctype html><script defer src='data:text/javascript,globalThis.firstScriptCount=1'></script>",
        "<!doctype html><script type='module'>globalThis.firstScriptCount=1</script>",
        "<!doctype html><body onload='globalThis.firstScriptCount=1'><p>Inline handler</p>",
    ] {
        let (_runtime, mut page) = lazy_test_page(html, Default::default()).await;
        assert_main_world_count(&page, 1).await;
        assert_eq!(
            evaluate_lazy_test(&page, "firstScriptCount").await,
            1,
            "{html}"
        );
        page.close_async().await.unwrap();
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn document_start_script_initializes_main_world_before_parser_script() {
    let (_runtime, mut page) = lazy_test_page(
        "<!doctype html><script>globalThis.observedPreload = firstScriptCount</script>",
        crate::RendererDocumentOptions {
            document_start_scripts: vec![crate::DocumentStartScript {
                registry_key: None,
                devtools_session: None,
                source: "globalThis.firstScriptCount = 1".to_owned(),
                world_name: None,
                has_bidi_channel_argument: false,
                bidi_channel_handoffs: Vec::new(),
            }],
            ..Default::default()
        },
    )
    .await;
    assert_eq!(evaluate_lazy_test(&page, "observedPreload").await, 1);
    assert_main_world_count(&page, 1).await;
    page.close_async().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn streaming_scriptless_document_keeps_main_world_lazy() {
    let runtime = JsRuntime::initialize();
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let url = url::Url::parse("https://example.test/lazy-stream").unwrap();
    let (completion_tx, completion_rx) = tokio::sync::oneshot::channel();
    let (body_tx, raw_body) = crate::ExternalRawDocumentBodyStream::channel(completion_rx);
    let producer = tokio::spawn(async move {
        body_tx
            .send(b"<!doctype html><title>Streamed</title><main>First".to_vec())
            .await
            .unwrap();
        body_tx.send(b" and last</main>".to_vec()).await.unwrap();
        drop(body_tx);
        completion_tx.send(Ok(())).unwrap();
    });
    let (mut page, _, _, _, _) = runtime
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
            PageVmInitStage::Load,
            crate::RendererReplyBoundary::Stage,
            RendererTopLevelNavigationDispatch::FollowInStandaloneAdapter,
            RendererNavigationReplyPolicy::FollowBeforeReply,
            None,
            None,
            Default::default(),
        )
        .await
        .unwrap();
    producer.await.unwrap();
    assert_main_world_count(&page, 0).await;
    assert_eq!(
        evaluate_lazy_test(&page, "document.title + ':' + document.body.textContent").await,
        "Streamed:First and last"
    );
    page.close_async().await.unwrap();
}
