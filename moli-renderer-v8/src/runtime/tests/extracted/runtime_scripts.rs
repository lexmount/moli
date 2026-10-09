use super::*;

#[tokio::test(flavor = "multi_thread")]
async fn typed_attributes_resolve_scoped_references_against_live_nodes() {
    use super::{RendererDocumentNodeAttributesResolution as Attributes, RendererDomNodeReference};

    async fn attributes(
        page: &super::RendererPageHandle,
        reference: RendererDomNodeReference,
    ) -> Attributes {
        let (reply, _) = page
            .run_async_command(RendererPageCommand::DocumentNodeAttributes { reference })
            .await
            .expect("typed attributes query completes");
        let RendererPageReply::DocumentNodeAttributesResolution(result) = reply else {
            panic!("expected typed attribute resolution");
        };
        result
    }

    let runtime = JsRuntime::initialize();
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default())
        .expect("default resource request client");
    let page = create_test_html_page(
        &runtime,
        &loader,
        url::Url::parse("https://example.test/attributes").unwrap(),
        "<!doctype html><html><body><input id='probe' data-state='before'></body></html>",
    )
    .await;
    let (reply, _) = page
        .run_async_command(RendererPageCommand::DocumentQuerySelectorForDocument {
            inspector_session_id: Some("attributes-a".to_owned()),
            include_whitespace: false,
            selector: "#probe".to_owned(),
            multiple: false,
        })
        .await
        .expect("bind the frontend node in session A");
    let RendererPageReply::DocumentQuerySelectorResolution(
        RendererDocumentQuerySelectorResolution::Found(nodes),
    ) = reply
    else {
        panic!("expected probe node");
    };
    let node = nodes[0];
    let frontend = RendererDomNodeReference::FrontendNodeId {
        inspector_session_id: Some("attributes-a".to_owned()),
        frontend_node_id: node.frontend_node_id,
    };
    assert_eq!(
        attributes(&page, frontend.clone()).await,
        Attributes::Found(vec![
            ("id".to_owned(), "probe".to_owned()),
            ("data-state".to_owned(), "before".to_owned()),
        ])
    );
    assert_eq!(
        attributes(
            &page,
            RendererDomNodeReference::FrontendNodeId {
                inspector_session_id: Some("attributes-b".to_owned()),
                frontend_node_id: node.frontend_node_id,
            },
        )
        .await,
        Attributes::MissingNode,
        "a frontend node ID must not inherit another session's binding"
    );
    page.run_async_command(RendererPageCommand::EvaluateExpression {
        expression: "document.querySelector('#probe').setAttribute('data-state', 'after')"
            .to_owned(),
        await_promise: false,
    })
    .await
    .expect("mutate the live node");
    let backend = RendererDomNodeReference::BackendNodeId(node.backend_node_id);
    assert_eq!(
        attributes(&page, frontend.clone()).await,
        attributes(&page, backend.clone()).await,
        "frontend and backend references must read the same live node"
    );
    assert!(matches!(
        attributes(&page, backend.clone()).await,
        Attributes::Found(values) if values.contains(&("data-state".to_owned(), "after".to_owned()))
    ));
    page.run_async_command(RendererPageCommand::DiscardDomAgentFrontendBindings {
        inspector_session_id: Some("attributes-a".to_owned()),
    })
    .await
    .expect("discard session A's frontend bindings");
    assert_eq!(attributes(&page, frontend).await, Attributes::MissingNode);

    let other_page = create_test_html_page(
        &runtime,
        &loader,
        url::Url::parse("https://example.test/other-attributes").unwrap(),
        "<!doctype html><html><body><input id='other'></body></html>",
    )
    .await;
    assert_eq!(
        attributes(&other_page, backend).await,
        Attributes::MissingNode,
        "backend references must belong to the queried Page document"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn outer_html_document_command_includes_only_author_shadow_roots() {
    let runtime = JsRuntime::initialize();
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default())
        .expect("default resource request client");
    let url = url::Url::parse("https://example.test/outer-html-shadow").unwrap();
    let page = create_test_html_page(
        &runtime,
        &loader,
        url,
        concat!(
            "<!doctype html><html><body>",
            "<x-host id='host'><template shadowrootmode='closed'>",
            "<span>shadow</span></template>light</x-host>",
            "<input id='control'>",
            "</body></html>"
        ),
    )
    .await;

    let ordinary = outer_html_for_renderer_document(&page, false).await;
    let serialize_html = serialize_html_for_renderer_page(&page).await;
    assert_eq!(ordinary, serialize_html);
    assert!(!ordinary.contains("shadowrootmode"));
    assert!(!ordinary.contains("shadow"));

    let including_shadow = outer_html_for_renderer_document(&page, true).await;
    assert!(including_shadow.contains(concat!(
        "<x-host id=\"host\"><template shadowrootmode=\"closed\">",
        "<span>shadow</span></template>light</x-host>"
    )));
    assert_eq!(
        including_shadow
            .matches("<template shadowrootmode=")
            .count(),
        1
    );
    assert!(including_shadow.contains("<input id=\"control\">"));
    assert!(!including_shadow.contains("<input id=\"control\"><template"));
}
#[tokio::test(flavor = "multi_thread")]
async fn bounded_viewport_clip_keeps_root_controls_while_page_clip_omits_them() {
    let runtime = initialize_layout_test_runtime();
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default())
        .expect("default resource request client");
    let url = url::Url::parse("https://example.test/viewport-control-capture").unwrap();
    let page = create_test_html_page(
        &runtime,
        &loader,
        url,
        concat!(
            "<!doctype html><style>",
            "html,body{margin:0}",
            "html{scrollbar-color:rgb(255,0,0) rgb(0,0,255)}",
            "#content{width:200px;height:200px}",
            "</style><div id='content'></div>",
        ),
    )
    .await;
    let viewport = crate::protocol_types::ViewportSurface {
        inner_width: 100,
        inner_height: 100,
        outer_width: 100,
        outer_height: 100,
        device_pixel_ratio: 1.0,
        screen_width: 100,
        screen_height: 100,
        screen_avail_width: 100,
        screen_avail_height: 100,

        ..Default::default()
    };
    page.run_async_command(RendererPageCommand::SetViewportSurface(Some(viewport)))
        .await
        .expect("viewport should update");
    let clip = super::RendererScreenshotClip {
        x: 0.0,
        y: 0.0,
        width: 100.0,
        height: 100.0,
        scale: 1.0,
    };

    let viewport_clip = capture_screenshot_with_request(
        &page,
        super::RendererCaptureScreenshotRequest {
            base_background_color: [255; 4],
            purpose: super::RendererScreenshotPurpose::Screenshot,
            format: super::RendererScreenshotFormat::Png,
            quality: 100,
            region: super::RendererScreenshotRegion::ViewportClip(clip),
            optimize_for_speed: false,
            max_width: None,
            max_height: None,
        },
    )
    .await;
    assert_eq!(
        decoded_png_pixel(&viewport_clip.bytes, 95, 30),
        [255, 0, 0, 255],
        "a bounded live-viewport capture includes its fixed root thumb"
    );
    assert_eq!(
        decoded_png_pixel(&viewport_clip.bytes, 92, 8),
        [255, 0, 0, 255],
        "the root control paints Chromium's up-arrow glyph"
    );
    assert_eq!(
        decoded_png_pixel(&viewport_clip.bytes, 87, 8),
        [0, 0, 255, 255],
        "the arrow remains centered inside its author-colored button"
    );

    let page_clip = capture_screenshot_with_request(
        &page,
        super::RendererCaptureScreenshotRequest {
            base_background_color: [255; 4],
            purpose: super::RendererScreenshotPurpose::Screenshot,
            format: super::RendererScreenshotFormat::Png,
            quality: 100,
            region: super::RendererScreenshotRegion::PageClip(clip),
            optimize_for_speed: false,
            max_width: None,
            max_height: None,
        },
    )
    .await;
    assert_eq!(
        decoded_png_pixel(&page_clip.bytes, 95, 30),
        [255, 255, 255, 255],
        "a capture-beyond-viewport page clip omits compositor controls"
    );
}
#[test]
fn page_ids_are_unique_across_threads() {
    let runtime = JsRuntime::initialize();
    let renderer_owner = runtime.renderer_owner_handle();
    let ids = Arc::new(Mutex::new(Vec::new()));
    let mut workers = Vec::new();

    for _ in 0..4 {
        let renderer_owner = renderer_owner.clone();
        let ids = ids.clone();
        workers.push(std::thread::spawn(move || {
            let id = renderer_owner.allocate_page_id().as_u64();
            ids.lock().push(id);
        }));
    }

    for worker in workers {
        worker.join().expect("worker should finish");
    }

    let ids = ids.lock();
    assert_eq!(ids.len(), 4);
    let unique = ids.iter().copied().collect::<HashSet<_>>();
    assert_eq!(unique.len(), 4);
}
#[tokio::test(flavor = "multi_thread")]
async fn dropping_page_handle_runs_detached_owner_cleanup() {
    let runtime = JsRuntime::initialize();
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default())
        .expect("default resource request client");
    let url = url::Url::parse("https://example.test/detached-drop-cleanup").unwrap();
    let page = create_test_html_page(&runtime, &loader, url, "<!doctype html>").await;
    let testing = RendererPageTestingHandle::new_for_testing(&page);

    testing
        .owner_slot_async()
        .await
        .expect("page should initially occupy an owner slot");
    drop(page);

    let owner_slot_after_drop =
        tokio::time::timeout(Duration::from_secs(1), testing.owner_slot_async())
            .await
            .expect("detached remove-page command should not stall");
    assert!(
        owner_slot_after_drop.is_err(),
        "dropping the page handle should remove its owner slot"
    );
}
#[tokio::test(flavor = "multi_thread")]
async fn javascript_location_assignment_executes_renderer_owned_task_on_delegate_pages() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let url = url::Url::parse("https://javascript-location-ghost.test/start.html").unwrap();
    let mut page = create_test_html_page_with_navigation_dispatch(
        &runtime,
        &loader,
        url,
        "<!doctype html><title>start</title>",
        RendererTopLevelNavigationDispatch::DelegateToBrowser,
    )
    .await;

    let (set_reply, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"
globalThis.__javascriptLocationDone = new Promise(resolve => {
  globalThis.__resolveJavascriptLocation = resolve;
});
location.href = "javascript:document.title = 'LOC-RAN'; globalThis.__resolveJavascriptLocation('ran'); void 0";
"set"
"#
            .to_owned(),
            await_promise: false,
        })
        .await
        .expect("javascript: location assignment should complete");
    assert_eq!(
        renderer_json_value(set_reply),
        Some(serde_json::json!("set"))
    );

    let (href_reply, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: "location.href".to_owned(),
            await_promise: false,
        })
        .await
        .expect("location.href readback should complete");
    assert_eq!(
        renderer_json_value(href_reply),
        Some(serde_json::json!(
            "https://javascript-location-ghost.test/start.html"
        )),
        "assigning a javascript: URL must not ghost location.href"
    );

    let (task_reply, _) = tokio::time::timeout(
        Duration::from_secs(2),
        page.run_async_command(RendererPageCommand::EvaluateExpression {
            expression: "globalThis.__javascriptLocationDone".to_owned(),
            await_promise: true,
        }),
    )
    .await
    .expect("renderer-owned javascript: task should settle its promise")
    .expect("javascript: task promise should evaluate");
    assert_eq!(
        renderer_json_value(task_reply),
        Some(serde_json::json!("ran")),
        "the javascript: task must run after the assigning command"
    );

    let (title_reply, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: "document.title".to_owned(),
            await_promise: false,
        })
        .await
        .expect("document.title readback should complete");
    assert_eq!(
        renderer_json_value(title_reply),
        Some(serde_json::json!("LOC-RAN"))
    );

    page.close_async()
        .await
        .expect("javascript location ghost page should close");
}
#[tokio::test(flavor = "current_thread")]
async fn parser_failed_custom_element_construction_uses_unknown_element_surface() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let url = url::Url::parse("https://example.test/parser-failed-custom-element").unwrap();
    let html = r#"<!doctype html>
<body>
<script>
window.onerror = () => true;
globalThis.__ReturnsText = class extends HTMLElement {
  constructor() {
    super();
    return document.createTextNode("text");
  }
};
customElements.define("wpt-parser-returns-text", globalThis.__ReturnsText);
globalThis.__ReturnsObject = class extends HTMLElement {
  constructor() {
    super();
    return {};
  }
};
customElements.define("wpt-parser-returns-object", globalThis.__ReturnsObject);
globalThis.__LacksSuper = class extends HTMLElement {
  constructor() {}
};
customElements.define("wpt-parser-lacks-super", globalThis.__LacksSuper);
globalThis.__ThrowsElement = class extends HTMLElement {
  constructor() {
    throw new Error("boom");
  }
};
customElements.define("wpt-parser-throws", globalThis.__ThrowsElement);
</script>
<wpt-parser-returns-text></wpt-parser-returns-text>
<wpt-parser-returns-object></wpt-parser-returns-object>
<wpt-parser-lacks-super></wpt-parser-lacks-super>
<wpt-parser-throws></wpt-parser-throws>
</body>"#;
    let mut page = create_test_html_page(&runtime, &loader, url, html).await;

    let (reply, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"
(() => {
  function summarize(selector, constructor) {
    const element = document.querySelector(selector);
    return [
      element instanceof HTMLElement,
      element instanceof HTMLUnknownElement,
      element instanceof constructor
    ].join(":");
  }
  return [
    summarize("wpt-parser-returns-text", globalThis.__ReturnsText),
    summarize("wpt-parser-returns-object", globalThis.__ReturnsObject),
    summarize("wpt-parser-lacks-super", globalThis.__LacksSuper),
    summarize("wpt-parser-throws", globalThis.__ThrowsElement)
  ].join("|");
})()
"#
            .to_owned(),
            await_promise: false,
        })
        .await
        .expect("parser failed custom element fallback surface should evaluate");

    assert_eq!(
        renderer_json_value(reply),
        Some(serde_json::json!(
            "true:true:false|true:true:false|true:true:false|true:true:false"
        ))
    );
    page.close_async()
        .await
        .expect("parser failed custom element page should close");
}
#[tokio::test(flavor = "current_thread")]
async fn parser_custom_element_document_write_microtasks_wait_for_outer_script() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let url =
        url::Url::parse("https://example.test/parser-custom-element-write-microtasks").unwrap();
    let html = r#"<!doctype html><body>
<script>
window.onerror = () => true;
globalThis.constructorMicrotaskLog = [];
class WrittenElement extends HTMLElement {
  constructor() {
    super();
    constructorMicrotaskLog.push("constructor");
    Promise.resolve().then(() => {
      constructorMicrotaskLog.push("microtask");
      this.setAttribute("data-constructed", "yes");
    });
  }
}
customElements.define("microtask-written-element", WrittenElement);
document.write("<microtask-written-element></microtask-written-element>");
constructorMicrotaskLog.push("after-write");
constructorMicrotaskLog.push(document.querySelector("microtask-written-element") instanceof WrittenElement);
</script>
<script>constructorMicrotaskLog.push("following-script");</script>
</body>"#;
    let mut page = create_test_html_page(&runtime, &loader, url, html).await;

    let (reply, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: "JSON.stringify({log: constructorMicrotaskLog, value: document.querySelector('microtask-written-element').getAttribute('data-constructed')})".to_owned(),
            await_promise: false,
        })
        .await
        .expect("parser document.write microtask order should evaluate");

    assert_eq!(
        renderer_json_value(reply),
        Some(serde_json::json!(
            r#"{"log":["constructor","after-write",true,"microtask","following-script"],"value":"yes"}"#
        ))
    );
    page.close_async()
        .await
        .expect("parser document.write microtask page should close");
}
#[tokio::test(flavor = "current_thread")]
async fn parser_custom_element_microtask_mutation_fails_before_validation() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let url =
        url::Url::parse("https://example.test/parser-custom-element-microtask-failure").unwrap();
    let html = r#"<!doctype html>
<body>
<script>
window.onerror = () => true;
globalThis.__ParserMicrotaskMutates = class extends HTMLElement {
  constructor() {
    super();
    Promise.resolve().then(() => this.setAttribute("attribute", "value"));
  }
};
customElements.define("wpt-parser-microtask-mutates", globalThis.__ParserMicrotaskMutates);
</script>
<wpt-parser-microtask-mutates></wpt-parser-microtask-mutates>
</body>"#;
    let mut page = create_test_html_page(&runtime, &loader, url, html).await;

    let (reply, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"
(() => {
  const element = document.querySelector("wpt-parser-microtask-mutates");
  return [
    element.hasAttribute("attribute"),
    element instanceof HTMLUnknownElement,
    element instanceof globalThis.__ParserMicrotaskMutates
  ].join(":");
})()
"#
            .to_owned(),
            await_promise: false,
        })
        .await
        .expect("parser microtask mutation fallback should evaluate");

    assert_eq!(
        renderer_json_value(reply),
        Some(serde_json::json!("false:true:false"))
    );
    page.close_async()
        .await
        .expect("parser microtask mutation page should close");
}
#[tokio::test(flavor = "multi_thread")]
async fn runtime_binding_replay_cannot_consume_same_id_frontend_deferred_response() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let page_url = url::Url::parse("https://example.test/inspector-internal-id-collision").unwrap();
    let mut page = create_test_html_page(
        &runtime,
        &loader,
        page_url,
        "<!doctype html><body>inspector internal id collision</body>",
    )
    .await;

    let colliding_call_id = 900_100_000;
    let (response_tx, mut response_rx) = oneshot::channel();
    let (dispatch, _) = page
        .run_async_command(
            RendererPageCommand::dispatch_runtime_protocol_message_with_deferred_response(
                None,
                serde_json::json!({
                    "id": colliding_call_id,
                    "method": "Runtime.evaluate",
                    "params": {
                        "expression": "new Promise(resolve => { globalThis.__resolveInspectorCollision = resolve; })",
                        "awaitPromise": true,
                        "returnByValue": true,
                    },
                })
                .to_string(),
                RendererRuntimeInspectorResponseSender::new(
                    colliding_call_id,
                    response_tx,
                ),
            ),
        )
        .await
        .expect("frontend awaitPromise should remain deferred");
    assert!(matches!(
        dispatch,
        RendererPageReply::RuntimeInspectorProtocolMessages(ref messages) if messages.is_empty()
    ));

    let binding = crate::protocol_types::RuntimeBindingRegistration {
        devtools_session: None,
        name: "internalCollisionBinding".to_owned(),
        execution_context_name: None,
    };
    set_runtime_binding_state_for_test(&page, None, Vec::new(), vec![binding])
        .await
        .expect("runtime binding state should become pending for replay");
    let replay_trigger_messages = dispatch_runtime_protocol_for_test(
        &page,
        serde_json::json!({
            "id": 710_100,
            "method": "Runtime.evaluate",
            "params": {
                "expression": "typeof internalCollisionBinding",
                "returnByValue": true,
            },
        }),
    )
    .await
    .expect("a later frontend dispatch should replay the new binding");
    assert_eq!(
        runtime_protocol_response_by_id(&replay_trigger_messages, 710_100)
            .expect("replay trigger response")["result"]["result"]["value"],
        serde_json::json!("function")
    );
    assert!(
        matches!(
            response_rx.try_recv(),
            Err(tokio::sync::oneshot::error::TryRecvError::Empty)
        ),
        "the internal Runtime.addBinding response must not complete the same-id frontend await"
    );

    let (resolved, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: "__resolveInspectorCollision('frontend-result'); 'resolved'".to_owned(),
            await_promise: false,
        })
        .await
        .expect("the page should resolve the original frontend promise");
    assert_eq!(
        renderer_json_value(resolved),
        Some(serde_json::json!("resolved"))
    );
    let completion = tokio::time::timeout(Duration::from_secs(2), &mut response_rx)
        .await
        .expect("the frontend promise response publication should not stall")
        .expect("the frontend promise response should retain its callback owner");
    assert_eq!(completion.call_id, colliding_call_id);
    let response = completion
        .output
        .protocol_response(colliding_call_id)
        .expect("frontend completion should contain its protocol response");
    assert_eq!(
        response["result"]["result"]["value"],
        serde_json::json!("frontend-result")
    );

    page.close_async()
        .await
        .expect("internal collision test page should close");
}
#[tokio::test(flavor = "multi_thread")]
async fn page_document_isolate_completes_real_v8_foreground_task() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let page_url = url::Url::parse("https://example.test/shared-v8-foreground-task").unwrap();

    let (page, _, _, _creation_artifacts, pending_download) = runtime
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
            "<!doctype html><body>shared v8 foreground task</body>".to_owned(),
            crate::RendererDocumentOptions {
                ..Default::default()
            },
        )
        .await
        .expect("shared-isolate page should load");
    assert!(pending_download.is_none());

    let (observed, _) = page
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
        .expect("wasm compilation should finish through the Page isolate foreground-task route");
    assert_eq!(
        renderer_json_value(observed),
        Some(serde_json::json!("compiled")),
        "the Page isolate must remain routable while V8 completes foreground work"
    );
}
#[tokio::test(flavor = "multi_thread")]
async fn child_default_runtime_evaluate_pending_await_promise_does_not_leak_internal_token() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let page_url = url::Url::parse("https://example.test/child-runtime-await-promise").unwrap();
    let mut page = create_test_html_page(
        &runtime,
        &loader,
        page_url,
        r#"<!doctype html><body><iframe srcdoc="<body>pending promise child</body>"></iframe></body>"#,
    )
    .await;

    let child_context_ids = child_default_context_ids_for_test(&page)
        .await
        .expect("child context events should replay");
    assert_eq!(
        child_context_ids.len(),
        1,
        "page should expose exactly one child default context"
    );
    let child_context_id = child_context_ids[0];

    let evaluate = dispatch_runtime_protocol_with_context_resolution_for_test(
        &page,
        "evaluate",
        serde_json::json!({
            "id": 67,
            "method": "Runtime.evaluate",
            "params": {
                "contextId": child_context_id,
                "expression": "new Promise(() => {})",
                "awaitPromise": true,
                "returnByValue": true
            }
        }),
    )
    .await
    .expect("child Runtime.evaluate awaitPromise should dispatch");
    let response =
        runtime_protocol_response_by_id(&evaluate, 67).expect("child awaitPromise response");
    let messages_json =
        serde_json::to_string(&evaluate).expect("Runtime.evaluate messages should serialize");
    assert!(
        !messages_json.contains("__moliAwaitPromiseToken"),
        "child Runtime.evaluate awaitPromise leaked internal polling token: {evaluate:?}"
    );
    assert!(
        !messages_json.contains("__moliPendingPromise"),
        "child Runtime.evaluate awaitPromise leaked internal pending marker: {evaluate:?}"
    );
    assert!(
        response.get("error").is_some() || response["result"]["exceptionDetails"].is_object(),
        "pending child Runtime.evaluate awaitPromise must fail closed instead of returning an internal success payload: {response:?}"
    );

    page.close_async()
        .await
        .expect("child runtime awaitPromise page should close");
}
#[tokio::test(flavor = "multi_thread")]
async fn runtime_enable_waits_for_queued_child_realm_before_reporting_contexts() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let page_url =
        url::Url::parse("https://example.test/runtime-enable-child-barrier").expect("test url");
    let mut page = create_test_html_page(
        &runtime,
        &loader,
        page_url,
        "<!doctype html><body>child realm barrier</body>",
    )
    .await;

    // Queue Runtime.enable before yielding. The Page scheduler admits the
    // child-realm wake after the setup command, then permits one already-ready
    // command to overtake that Page turn. This makes the production race
    // deterministic: Runtime.enable must park behind the exact-Document realm
    // task instead of reporting an incomplete current-context inventory.
    let setup = page
        .enqueue_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"(() => {
  const frame = document.createElement("iframe");
  frame.id = "runtime-enable-barrier-child";
  document.body.appendChild(frame);
  void frame.contentWindow.Function;
  return "queued";
})()"#
                .to_owned(),
            await_promise: false,
        })
        .expect("child realm setup command should enqueue");
    let enable = page
        .enqueue_async_command(RendererPageCommand::runtime_enable_events(Some(
            "SID-runtime-enable-child-barrier".to_owned(),
        )))
        .expect("Runtime.enable command should enqueue behind child realm setup");

    let setup = setup
        .wait()
        .await
        .expect("child realm setup command should complete");
    let (setup, _) = setup.into_completion_and_predecessor();
    let (setup_reply, _, _) = setup.into_parts();
    assert_eq!(
        renderer_json_value(setup_reply),
        Some(serde_json::json!("queued"))
    );

    let enable = enable
        .wait()
        .await
        .expect("Runtime.enable should resume after child realm materialization");
    let (enable, _) = enable.into_completion_and_predecessor();
    let (enable_reply, _, _) = enable.into_parts();
    let RendererPageReply::RuntimeInspectorProtocolMessages(output) = enable_reply else {
        panic!("Runtime.enable should return inspector protocol messages");
    };
    let messages = output
        .into_messages()
        .into_iter()
        .map(runtime_inspector_message_protocol_message_for_test)
        .collect::<Vec<_>>();
    let child_context = messages.iter().find(|message| {
        message["method"] == serde_json::json!("Runtime.executionContextCreated")
            && message["params"]["context"]["auxData"]["isDefault"] == serde_json::json!(true)
            && message["params"]["context"]["auxData"]["frameId"].is_string()
    });
    assert!(
        child_context.is_some(),
        "Runtime.enable must report the child context created by its materialization prerequisite: {messages:?}"
    );

    page.close_async()
        .await
        .expect("Runtime.enable child barrier page should close");
}
#[tokio::test(flavor = "multi_thread")]
async fn runtime_enable_events_for_new_inspector_session_replays_existing_isolated_worlds() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let page_url =
        url::Url::parse("https://example.test/runtime-enable-new-session").expect("test url");

    let (page, _, _, _creation_artifacts, pending_download) = runtime
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
            "<!doctype html><body>new inspector session</body>".to_owned(),
            crate::RendererDocumentOptions {
                ..Default::default()
            },
        )
        .await
        .expect("new-session runtime replay test page should load");
    assert!(pending_download.is_none());

    let world_context_id = create_isolated_world_for_test(&page, "new-session-utility")
        .await
        .expect("isolated world should be created before the inspector session enables Runtime");

    let events = runtime_enable_events_for_inspector_session_for_test(
        &page,
        Some("SID-new-runtime-session"),
    )
    .await
    .expect("new inspector session Runtime.enable should run");
    let context_ids = runtime_execution_context_ids(&events);
    assert!(
        context_ids.contains(&world_context_id),
        "new inspector session Runtime.enable should replay existing isolated world: {events:?}"
    );
}
#[tokio::test(flavor = "multi_thread")]
async fn runtime_enable_events_include_renderer_root_frame_id_for_default_context() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let page_url =
        url::Url::parse("https://example.test/runtime-enable-root-frame").expect("test url");
    let root_frame_id = "TID-runtime-enable-root-frame";

    let (mut page, _, _, _creation_artifacts, pending_download) = runtime
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
            "<!doctype html><body>root frame id</body>".to_owned(),
            crate::RendererDocumentOptions {
                root_frame_id: Some(root_frame_id.to_owned()),
                ..Default::default()
            },
        )
        .await
        .expect("root-frame test page should load");
    assert!(pending_download.is_none());

    let events = runtime_enable_events_for_test(&page)
        .await
        .expect("Runtime.enable replay should run");
    let default_contexts = events
        .iter()
        .filter(|message| {
            message.get("method") == Some(&serde_json::json!("Runtime.executionContextCreated"))
                && message["params"]["context"]["auxData"]["isDefault"] == serde_json::json!(true)
                && message["params"]["context"]["auxData"]["type"] == serde_json::json!("default")
        })
        .collect::<Vec<_>>();

    assert_eq!(
        default_contexts.len(),
        1,
        "Runtime.enable should expose exactly one top-level default context: {events:?}"
    );
    assert_eq!(
        default_contexts[0]["params"]["context"]["auxData"]["frameId"],
        serde_json::json!(root_frame_id),
        "renderer Runtime.enable output should carry the root frame id before protocol emission"
    );
    assert_eq!(
        default_contexts[0]["params"]["context"]["origin"],
        serde_json::json!("https://example.test"),
        "renderer Runtime.enable output should carry the document security origin"
    );

    page.close_async()
        .await
        .expect("root-frame test page should close");
}
#[tokio::test(flavor = "multi_thread")]
async fn set_runtime_binding_state_updates_renderer_inspector_session_store() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let url = url::Url::parse("https://example.test/renderer-runtime-binding-state").unwrap();
    let mut page = create_test_html_page(
        &runtime,
        &loader,
        url,
        "<!doctype html><body>renderer runtime binding state</body>",
    )
    .await;
    let binding = crate::protocol_types::RuntimeBindingRegistration {
        devtools_session: None,
        name: "rendererSessionStoredBinding".to_owned(),
        execution_context_name: None,
    };

    set_runtime_binding_state_for_test(&page, None, vec![binding.clone()], vec![binding])
        .await
        .expect("renderer runtime binding state should update");
    runtime_enable_events_for_test(&page)
        .await
        .expect("Runtime.enable should replay renderer-stored bindings");

    let (binding_type, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"typeof rendererSessionStoredBinding"#.to_owned(),
            await_promise: false,
        })
        .await
        .expect("renderer-stored binding type should evaluate");
    assert_eq!(
        renderer_json_value(binding_type),
        Some(serde_json::json!("function")),
        "Runtime.enable should replay bindings from renderer inspector session state"
    );

    page.close_async()
        .await
        .expect("renderer binding state test page should close");

    let clearing_url =
        url::Url::parse("https://example.test/renderer-runtime-binding-state-cleared").unwrap();
    let mut clearing_page = create_test_html_page(
        &runtime,
        &loader,
        clearing_url,
        "<!doctype html><body>renderer runtime binding state cleared</body>",
    )
    .await;
    let scoped_binding = crate::protocol_types::RuntimeBindingRegistration {
        devtools_session: None,
        name: "rendererSessionClearedBinding".to_owned(),
        execution_context_name: Some("cleared-binding-world".to_owned()),
    };

    set_runtime_binding_state_for_test(
        &clearing_page,
        None,
        vec![scoped_binding.clone()],
        vec![scoped_binding],
    )
    .await
    .expect("renderer runtime binding state should accept pending named-world binding");
    set_runtime_binding_state_for_test(&clearing_page, None, Vec::new(), Vec::new())
        .await
        .expect("renderer runtime binding state should clear pending named-world binding");
    runtime_enable_events_for_test(&clearing_page)
        .await
        .expect("Runtime.enable should run with cleared renderer session state");
    let cleared_world_context_id =
        create_isolated_world_for_test(&clearing_page, "cleared-binding-world")
            .await
            .expect("cleared binding world should be created");
    let (cleared_binding_type, _) = clearing_page
        .run_async_command(RendererPageCommand::EvaluateExpressionInExecutionContext {
            execution_context_id: cleared_world_context_id,
            expression: r#"typeof rendererSessionClearedBinding"#.to_owned(),
            await_promise: false,
        })
        .await
        .expect("cleared renderer-stored binding type should evaluate");
    assert_eq!(
        renderer_json_value(cleared_binding_type),
        Some(serde_json::json!("undefined")),
        "cleared renderer inspector session state must not replay stale named-world bindings"
    );

    clearing_page
        .close_async()
        .await
        .expect("renderer binding clearing test page should close");
}
#[tokio::test(flavor = "multi_thread")]
async fn apply_runtime_protocol_state_keeps_session_binding_replay_scoped() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let url =
        url::Url::parse("https://example.test/runtime-protocol-state-session-bindings").unwrap();
    let mut page = create_test_html_page(
        &runtime,
        &loader,
        url,
        "<!doctype html><body>runtime protocol state session bindings</body>",
    )
    .await;
    let stored_only_binding = crate::protocol_types::RuntimeBindingRegistration {
        devtools_session: None,
        name: "storedOnlySessionBinding".to_owned(),
        execution_context_name: Some("stored-only-world".to_owned()),
    };

    let (reply, _) = page
        .run_async_command(RendererPageCommand::apply_runtime_protocol_state(
            Some("SID-primary".to_owned()),
            Vec::new(),
            Vec::new(),
            vec![stored_only_binding],
            Vec::new(),
        ))
        .await
        .expect("runtime protocol state should apply");
    assert!(
        matches!(reply, RendererPageReply::Unit),
        "expected ApplyRuntimeProtocolState to return unit reply"
    );

    let context_id = create_isolated_world_runtime_activity_for_test(
        &page,
        Some("SID-primary"),
        "stored-only-world",
    )
    .await
    .expect("runtime activity should create stored-only world");
    let (binding_type, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpressionInExecutionContext {
            execution_context_id: context_id,
            expression: "typeof storedOnlySessionBinding".to_owned(),
            await_promise: false,
        })
        .await
        .expect("stored-only binding type should evaluate");
    assert_eq!(
        renderer_json_value(binding_type),
        Some(serde_json::json!("undefined")),
        "ApplyRuntimeProtocolState must not copy page-level stored bindings into the current inspector session replay store"
    );

    page.close_async()
        .await
        .expect("renderer protocol-state session binding test page should close");
}
#[tokio::test(flavor = "multi_thread")]
async fn inspector_output_flushes_v8_state_for_commands_and_notifications() {
    let runtime = JsRuntime::initialize();
    let (output_tx, mut output_rx) = renderer_external_activity_test_channel();
    runtime.set_renderer_output_transport_sender(output_tx);
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let url = url::Url::parse("https://example.test/inspector-state-output").unwrap();
    let mut page = create_test_html_page(
        &runtime,
        &loader,
        url,
        "<!doctype html><body>inspector state output</body>",
    )
    .await;

    let (enable_messages, enable_output) = dispatch_runtime_protocol_with_output_for_test(
        &page,
        serde_json::json!({
            "id": 701,
            "method": "Runtime.enable",
        }),
    )
    .await
    .expect("Runtime.enable should dispatch");
    assert!(
        runtime_protocol_response_by_id(&enable_messages, 701)
            .is_some_and(|message| message.get("error").is_none()),
        "Runtime.enable should return a successful response: {enable_messages:?}"
    );
    let command_state = enable_output
        .v8_state_update()
        .expect("a V8 command response should flush the latest session state");
    assert!(
        !command_state.is_empty(),
        "Runtime.enable should produce a non-empty V8 state cookie"
    );
    assert!(
        output_rx.drain().iter().all(|publication| {
            publication.records().iter().all(|record| {
                !matches!(
                    record.item(),
                    RendererOutputItem::Observation(
                        RendererProtocolObservation::RuntimeInspector(batch)
                    ) if batch.messages.iter().any(|message| matches!(
                        message,
                        RendererRuntimeInspectorMessage::Protocol(message)
                            if message.get("id").and_then(serde_json::Value::as_i64) == Some(701)
                    ))
                )
            })
        }),
        "a synchronous Runtime response must not leak into the live notification stream"
    );

    page.run_async_command(RendererPageCommand::EvaluateExpression {
        expression: "console.log('state-notification-marker')".to_owned(),
        await_promise: false,
    })
    .await
    .expect("console evaluation should complete");
    assert!(
        output_rx.drain().iter().any(|publication| {
            publication.records().iter().any(|record| matches!(
                record.item(),
                RendererOutputItem::Observation(RendererProtocolObservation::RuntimeInspector(batch))
                    if batch.v8_state_update
                        .as_ref()
                        .is_some_and(|state| !state.is_empty())
                        && batch.messages.iter().any(|message| matches!(
                            message,
                            RendererRuntimeInspectorMessage::Protocol(message)
                                if message.get("method")
                                    == Some(&serde_json::json!("Runtime.consoleAPICalled"))
                        ))
            ))
        }),
        "the producing turn's concrete Runtime notification must carry the latest V8 state"
    );

    page.close_async()
        .await
        .expect("inspector state output page should close");
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn owner_command_fences_exact_cursor_when_later_output_is_already_published() {
    let runtime = JsRuntime::initialize();
    let (output_tx, mut output_rx) = renderer_external_activity_test_channel();
    runtime.set_renderer_output_transport_sender(output_tx);
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let url = url::Url::parse("https://example.test/concurrent-command-output").unwrap();
    let mut page =
        create_test_html_page(&runtime, &loader, url, "<!doctype html><body>before</body>").await;

    let (enable_messages, _) = dispatch_runtime_protocol_with_output_for_test(
        &page,
        serde_json::json!({
            "id": 711,
            "method": "Runtime.enable",
        }),
    )
    .await
    .expect("Runtime.enable should dispatch");
    assert!(
        runtime_protocol_response_by_id(&enable_messages, 711)
            .is_some_and(|message| message.get("error").is_none()),
        "Runtime.enable should succeed before the command-output race: {enable_messages:?}"
    );
    output_rx.drain();

    runtime.publish_next_command_output_before_owner_settlement_for_testing();
    let pending_evaluate = page
        .enqueue_async_command(RendererPageCommand::EvaluateExpression {
            expression: "console.log('concurrent-command-output'); 'done'".to_owned(),
            await_promise: false,
        })
        .expect("Runtime.evaluate should enqueue");
    let concurrent_publication = tokio::time::timeout(Duration::from_secs(3), output_rx.recv())
        .await
        .expect("the test publication should not hang")
        .expect("the test hook should settle the pending owner-command records");
    assert!(
        concurrent_publication
            .records()
            .iter()
            .any(|record| matches!(
                record.item(),
                RendererOutputItem::Observation(RendererProtocolObservation::RuntimeInspector(batch))
                    if batch.messages.iter().any(|message| matches!(
                        message,
                        RendererRuntimeInspectorMessage::Protocol(message)
                            if message.get("method")
                                == Some(&serde_json::json!("Runtime.consoleAPICalled"))
                    ))
            )),
        "the early publication must own the Runtime notification emitted by the owner command"
    );
    let trailing_publication = tokio::time::timeout(Duration::from_secs(3), output_rx.recv())
        .await
        .expect("the trailing test publication should not hang")
        .expect("final owner settlement should publish a later independent batch");
    assert!(
        trailing_publication.records().iter().any(|record| matches!(
            record.item(),
            RendererOutputItem::Observation(RendererProtocolObservation::RuntimeLifecycleError {
                text,
                execution_context_id: None,
            }) if text == "test trailing output publication"
        )),
        "final Page settlement should publish the independent trailing test output"
    );
    assert_eq!(
        trailing_publication.cursor().sequence(),
        concurrent_publication.cursor().sequence() + 1,
        "the independent output must follow the command publication"
    );

    let evaluate_output = tokio::time::timeout(Duration::from_secs(3), pending_evaluate.wait())
        .await
        .expect("owner command should not hang after the IO publication")
        .expect("owner command should complete after its records were settled by IO");
    assert_eq!(
        evaluate_output
            .renderer_output_predecessor()
            .expect("owner response must retain the concurrently settled prefix")
            .cursor(),
        concurrent_publication.cursor()
    );
    assert_ne!(
        evaluate_output
            .renderer_output_predecessor()
            .expect("owner response must retain an exact output prefix")
            .cursor(),
        trailing_publication.cursor(),
        "the response fence must not drift to output published after its command batch"
    );

    page.close_async()
        .await
        .expect("concurrent command output page should close");
}
#[tokio::test(flavor = "multi_thread")]
async fn page_diagnostics_snapshot_is_read_only_for_current_inspector_output() {
    let runtime = JsRuntime::initialize();
    let (output_tx, mut output_rx) = renderer_external_activity_test_channel();
    runtime.set_renderer_output_transport_sender(output_tx);
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let url = url::Url::parse("https://example.test/exact-document-inspector-snapshot").unwrap();
    let mut page = create_test_html_page(
        &runtime,
        &loader,
        url,
        "<!doctype html><body>exact document inspector snapshot</body>",
    )
    .await;

    let (identity_reply, _) = page
        .run_async_command(RendererPageCommand::PageDiagnosticsSnapshot)
        .await
        .expect("initial page diagnostics snapshot should complete");
    let RendererPageReply::PageDiagnosticsSnapshot(identity_snapshot) = identity_reply else {
        panic!("page diagnostics command should return an activity snapshot");
    };
    let current_document = identity_snapshot
        .document_lifecycle_identity()
        .expect("snapshot should carry the current Document identity");
    while output_rx.try_recv().is_ok() {}

    dispatch_runtime_protocol_with_output_for_test(
        &page,
        serde_json::json!({
            "id": 702,
            "method": "Runtime.enable",
        }),
    )
    .await
    .expect("Runtime.enable should dispatch");
    while output_rx.try_recv().is_ok() {}
    page.enqueue_async_command(RendererPageCommand::EvaluateExpression {
        expression: "console.log('exact-document-snapshot-marker')".to_owned(),
        await_promise: false,
    })
    .expect("console evaluation should enqueue")
    .wait()
    .await
    .expect("console evaluation should complete");
    assert!(
        output_rx.drain().iter().any(|publication| {
            publication.records().iter().any(|record| matches!(
                record.item(),
                RendererOutputItem::Observation(RendererProtocolObservation::RuntimeInspector(batch))
                    if batch.messages.iter().any(|message| matches!(
                        message,
                        RendererRuntimeInspectorMessage::Protocol(message)
                            if message.get("method")
                                == Some(&serde_json::json!("Runtime.consoleAPICalled"))
                    ))
            ))
        }),
        "console output must be frozen in the producing turn's concrete publication"
    );

    let (snapshot_reply, _) = page
        .run_async_command(RendererPageCommand::PageDiagnosticsSnapshot)
        .await
        .expect("read-only activity snapshot should complete");
    let RendererPageReply::PageDiagnosticsSnapshot(snapshot) = snapshot_reply else {
        panic!("activity command should return a snapshot");
    };
    assert_eq!(
        snapshot.document_lifecycle_identity(),
        Some(current_document)
    );
    assert_eq!(
        snapshot.diagnostics.pending_inspector_messages, 0,
        "published Inspector messages must not remain as a diagnostics-owned output queue"
    );
    assert!(
        snapshot
            .runtime_observable_source()
            .is_some_and(|source| source.source_items().iter().any(|item| matches!(
                item,
                super::RendererRuntimeObservableSourceItem::ConsoleMessage { message, .. }
                    if message.message == "log: exact-document-snapshot-marker"
            ))),
        "diagnostics should expose read-only source state without owning the protocol publication"
    );

    page.close_async()
        .await
        .expect("read-only Inspector diagnostics page should close");
}
#[tokio::test(flavor = "multi_thread")]
async fn failed_stateful_inspector_command_preserves_v8_state() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let url = url::Url::parse("https://example.test/inspector-failed-state").unwrap();
    let mut page = create_test_html_page(
        &runtime,
        &loader,
        url,
        "<!doctype html><body>failed inspector state</body>",
    )
    .await;

    for request in [
        serde_json::json!({"id": 711, "method": "Profiler.enable"}),
        serde_json::json!({
            "id": 712,
            "method": "Profiler.setSamplingInterval",
            "params": {"interval": 937},
        }),
        serde_json::json!({"id": 713, "method": "Profiler.start"}),
    ] {
        let response_id = request["id"].as_i64().expect("numeric request id");
        let (messages, _) = dispatch_runtime_protocol_with_output_for_test(&page, request)
            .await
            .expect("Profiler setup command should dispatch");
        assert!(
            runtime_protocol_response_by_id(&messages, response_id)
                .is_some_and(|message| message.get("error").is_none()),
            "Profiler setup command {response_id} should succeed: {messages:?}"
        );
    }
    let (_, before_failed_command) = dispatch_runtime_protocol_with_output_for_test(
        &page,
        serde_json::json!({"id": 714, "method": "Runtime.enable"}),
    )
    .await
    .expect("state checkpoint command should dispatch");
    let before_failed_command = before_failed_command
        .v8_state_update()
        .cloned()
        .expect("state checkpoint should flush a cookie");

    let (failed_messages, failed_output) = dispatch_runtime_protocol_with_output_for_test(
        &page,
        serde_json::json!({
            "id": 715,
            "method": "Profiler.setSamplingInterval",
            "params": {"interval": 123},
        }),
    )
    .await
    .expect("invalid stateful command should still return a protocol response");
    assert!(
        runtime_protocol_response_by_id(&failed_messages, 715)
            .is_some_and(|message| message.get("error").is_some()),
        "changing the sampling interval while profiling should fail: {failed_messages:?}"
    );
    assert_eq!(
        failed_output.v8_state_update(),
        Some(&before_failed_command),
        "a failed stateful command must not advance the opaque V8 state"
    );

    page.close_async()
        .await
        .expect("failed inspector state page should close");
}
#[tokio::test(flavor = "multi_thread")]
async fn stored_document_start_script_remove_uses_registry_key_namespace() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let url = url::Url::parse("https://example.test/stored-preload-registry-key").unwrap();

    let (mut page, _, _, _creation_artifacts, download) = runtime
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
            "<!doctype html><body>stored preload key namespace</body>".to_owned(),
            crate::RendererDocumentOptions {
                ..Default::default()
            },
        )
        .await
        .expect("registry-key stored-preload page should load");
    assert!(download.is_none());

    set_stored_document_start_scripts_for_test(
        &page,
        vec![
            crate::DocumentStartScript {
                registry_key: Some("default:1".to_owned()),
                devtools_session: None,
                source: r#"globalThis.__defaultPreload = "default";"#.to_owned(),
                world_name: Some("registry-key-world".to_owned()),
                has_bidi_channel_argument: false,
                bidi_channel_handoffs: Vec::new(),
            },
            crate::DocumentStartScript {
                registry_key: Some("target:TID-1:1".to_owned()),
                devtools_session: None,
                source: r#"globalThis.__targetPreload = "target";"#.to_owned(),
                world_name: Some("registry-key-world".to_owned()),
                has_bidi_channel_argument: false,
                bidi_channel_handoffs: Vec::new(),
            },
        ],
    )
    .await
    .expect("stored document-start scripts should install");

    let (remove_reply, _) = page
        .run_async_command(RendererPageCommand::RemoveDocumentStartScriptByRegistryKey(
            "target:TID-1:1".to_owned(),
        ))
        .await
        .expect("registry-key remove should run");
    assert!(matches!(remove_reply, RendererPageReply::Unit));

    let replacement_url = "data:text/html;charset=utf-8,%3C!doctype%20html%3E%3Cbody%3Eregistry%20key%20replacement%3C/body%3E";
    let (navigation_reply, _) = page
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
        .expect("registry-key stored-preload page should navigate");
    assert_eq!(
        renderer_json_value(navigation_reply),
        Some(serde_json::json!("navigating"))
    );

    let context_id = create_isolated_world_for_test(&page, "registry-key-world")
        .await
        .expect("registry-key isolated world should be created");
    let (value, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpressionInExecutionContext {
            execution_context_id: context_id,
            expression: r#"JSON.stringify({
                defaultValue: globalThis.__defaultPreload,
                targetValue: globalThis.__targetPreload ?? "absent"
            })"#
            .to_owned(),
            await_promise: false,
        })
        .await
        .expect("registry-key isolated world value should evaluate");
    assert_eq!(
        renderer_json_value(value),
        Some(serde_json::json!(
            r#"{"defaultValue":"default","targetValue":"absent"}"#
        ))
    );

    page.close_async()
        .await
        .expect("registry-key stored-preload page should close");
}
#[tokio::test(flavor = "multi_thread")]
async fn webcrypto_checkpoint_reconciles_document_replacement_before_restoring_page_residence() {
    let runtime = JsRuntime::initialize();
    let (activity_wake_tx, mut activity_wake_rx) = renderer_external_activity_test_channel();
    runtime.set_renderer_output_transport_sender(activity_wake_tx);
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let page_url = url::Url::parse("https://example.test/webcrypto-checkpoint-document-open")
        .expect("page URL");
    let mut page = create_test_html_page(
        &runtime,
        &loader,
        page_url,
        "<!doctype html><body>initial WebCrypto document</body>",
    )
    .await;
    let document_open_session = "SID-document-open-agent";
    runtime_enable_events_for_inspector_session_for_test(&page, Some(document_open_session))
        .await
        .expect("attached Runtime session should attach before document.open");
    let inspector_before = runtime_heap_usage_for_test(&page).await;
    let inspector_before = &inspector_before["moli"]["runtime"];
    let context_group_before = inspector_before["inspectorContextGroupId"].clone();
    let session_count_before = inspector_before["inspectorSessionCount"].clone();
    let registration_count_before = inspector_before["inspectorContextRegistrationCount"].clone();
    assert_eq!(session_count_before, serde_json::json!(2));
    assert_eq!(registration_count_before, serde_json::json!(1));

    let _ = page
        .run_async_command(RendererPageCommand::TakeDocumentLifecycleEvents)
        .await
        .expect("initial lifecycle events should be drainable");
    while activity_wake_rx.try_recv().is_ok() {}

    let (scheduled, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"(() => {
  crypto.subtle.digest("SHA-256", new TextEncoder().encode("replace-document"))
    .then(() => {
      document.open();
      document.write('<main id="webcrypto-checkpoint-replacement">replacement</main>');
      document.close();
    });
  return "scheduled";
})()"#
                .to_owned(),
            await_promise: false,
        })
        .await
        .expect("WebCrypto replacement reaction should schedule");
    assert_eq!(
        renderer_json_value(scheduled),
        Some(serde_json::json!("scheduled"))
    );

    let replacement_events = tokio::time::timeout(
        Duration::from_secs(2),
        recv_page_lifecycle_until(
            &mut activity_wake_rx,
            &page,
            RendererDocumentLifecycleMilestone::Load,
        ),
    )
    .await
    .expect("the WebCrypto task-end checkpoint should install and schedule replacement lifecycle");
    assert!(replacement_events.iter().any(|event| matches!(
        event.kind,
        RendererDocumentLifecycleEventKind::Milestone(
            RendererDocumentLifecycleMilestone::DomContentLoaded
        )
    )));

    let (replacement, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression:
                "document.querySelector('#webcrypto-checkpoint-replacement')?.textContent ?? 'missing'"
                    .to_owned(),
            await_promise: false,
        })
        .await
        .expect("replacement DOM should evaluate");
    assert_eq!(
        renderer_json_value(replacement),
        Some(serde_json::json!("replacement"))
    );
    let inspector_after = runtime_heap_usage_for_test(&page).await;
    let inspector_after = &inspector_after["moli"]["runtime"];
    assert_eq!(
        inspector_after["inspectorContextGroupId"], context_group_before,
        "same-Page document.open must preserve the local-root context group"
    );
    assert_eq!(
        inspector_after["inspectorSessionCount"], session_count_before,
        "same-Page document.open must not detach frontend V8 sessions"
    );
    assert_eq!(
        inspector_after["inspectorContextRegistrationCount"], registration_count_before,
        "same-Page document.open must preserve the existing Window context registration"
    );
    assert_eq!(
        inspector_after["inspectorSessionRegistryOwner"],
        serde_json::json!("renderer-devtools-agent")
    );
    runtime_enable_events_for_inspector_session_for_test(&page, Some(document_open_session))
        .await
        .expect("the existing attached Runtime session should remain dispatchable");

    page.close_async()
        .await
        .expect("WebCrypto checkpoint replacement page should close");
}
#[tokio::test(flavor = "multi_thread")]
async fn broadcast_channel_checkpoint_reconciles_replacement_before_page_restore() {
    let runtime = JsRuntime::initialize();
    let (activity_wake_tx, mut activity_wake_rx) = renderer_external_activity_test_channel();
    runtime.set_renderer_output_transport_sender(activity_wake_tx);
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let page_url = url::Url::parse("https://example.test/broadcast-checkpoint-document-open")
        .expect("page URL");
    let mut page = create_test_html_page(
        &runtime,
        &loader,
        page_url,
        "<!doctype html><body>initial BroadcastChannel document</body>",
    )
    .await;

    let _ = page
        .run_async_command(RendererPageCommand::TakeDocumentLifecycleEvents)
        .await
        .expect("initial lifecycle events should be drainable");
    while activity_wake_rx.try_recv().is_ok() {}

    let (scheduled, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"(() => {
  globalThis.__broadcastCheckpointReceiver =
    new BroadcastChannel("broadcast-checkpoint-replacement");
  __broadcastCheckpointReceiver.onmessage = () => {
    Promise.resolve().then(() => {
      document.open();
      document.write('<main id="broadcast-checkpoint-replacement">replacement</main>');
      document.close();
    });
  };
  globalThis.__broadcastCheckpointSender =
    new BroadcastChannel("broadcast-checkpoint-replacement");
  __broadcastCheckpointSender.postMessage("replace");
  return "scheduled";
})()"#
                .to_owned(),
            await_promise: false,
        })
        .await
        .expect("BroadcastChannel replacement reaction should schedule");
    assert_eq!(
        renderer_json_value(scheduled),
        Some(serde_json::json!("scheduled"))
    );

    let replacement_events = tokio::time::timeout(
        Duration::from_secs(2),
        recv_page_lifecycle_until(
            &mut activity_wake_rx,
            &page,
            RendererDocumentLifecycleMilestone::Load,
        ),
    )
    .await
    .expect("the BroadcastChannel task-end checkpoint should install replacement lifecycle");
    assert!(replacement_events.iter().any(|event| matches!(
        event.kind,
        RendererDocumentLifecycleEventKind::Milestone(
            RendererDocumentLifecycleMilestone::DomContentLoaded
        )
    )));

    let (replacement, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression:
                "document.querySelector('#broadcast-checkpoint-replacement')?.textContent ?? 'missing'"
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
        .expect("BroadcastChannel checkpoint replacement page should close");
}
#[tokio::test(flavor = "multi_thread")]
async fn storage_event_checkpoint_reconciles_replacement_before_page_restore() {
    let runtime = JsRuntime::initialize();
    let (activity_wake_tx, mut activity_wake_rx) = renderer_external_activity_test_channel();
    runtime.set_renderer_output_transport_sender(activity_wake_tx);
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let page_url = url::Url::parse("https://example.test/storage-event-checkpoint-document-open")
        .expect("page URL");
    let mut page = create_test_html_page(
        &runtime,
        &loader,
        page_url,
        "<!doctype html><body><iframe id=\"storage-source\"></iframe></body>",
    )
    .await;

    let _ = page
        .run_async_command(RendererPageCommand::TakeDocumentLifecycleEvents)
        .await
        .expect("initial lifecycle events should be drainable");
    while activity_wake_rx.try_recv().is_ok() {}

    let (scheduled, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"(() => {
  addEventListener("storage", () => {
    Promise.resolve().then(() => {
      document.open();
      document.write('<main id="storage-event-checkpoint-replacement">replacement</main>');
      document.close();
    });
  }, { once: true });
  document.getElementById("storage-source").contentWindow.localStorage
    .setItem("storage-event-checkpoint-key", "replace-document");
  return "scheduled";
})()"#
                .to_owned(),
            await_promise: false,
        })
        .await
        .expect("StorageEvent replacement reaction should schedule");
    assert_eq!(
        renderer_json_value(scheduled),
        Some(serde_json::json!("scheduled"))
    );

    let replacement_events = tokio::time::timeout(
        Duration::from_secs(2),
        recv_page_lifecycle_until(
            &mut activity_wake_rx,
            &page,
            RendererDocumentLifecycleMilestone::Load,
        ),
    )
    .await
    .expect(
        "the StorageEvent task-end checkpoint should install and schedule replacement lifecycle",
    );
    assert!(replacement_events.iter().any(|event| matches!(
        event.kind,
        RendererDocumentLifecycleEventKind::Milestone(
            RendererDocumentLifecycleMilestone::DomContentLoaded
        )
    )));

    let (replacement, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression:
                "document.querySelector('#storage-event-checkpoint-replacement')?.textContent ?? 'missing'"
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
        .expect("StorageEvent checkpoint replacement page should close");
}
#[tokio::test(flavor = "multi_thread")]
async fn hashchange_checkpoint_reconciles_replacement_before_page_restore() {
    let runtime = JsRuntime::initialize();
    let (activity_wake_tx, mut activity_wake_rx) = renderer_external_activity_test_channel();
    runtime.set_renderer_output_transport_sender(activity_wake_tx);
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let page_url = url::Url::parse("https://example.test/hashchange-checkpoint-document-open")
        .expect("page URL");
    let mut page = create_test_html_page(
        &runtime,
        &loader,
        page_url,
        "<!doctype html><body>initial hashchange document</body>",
    )
    .await;

    let _ = page
        .run_async_command(RendererPageCommand::TakeDocumentLifecycleEvents)
        .await
        .expect("initial lifecycle events should be drainable");
    while activity_wake_rx.try_recv().is_ok() {}

    let (scheduled, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r##"(() => {
  addEventListener("hashchange", () => {
    Promise.resolve().then(() => {
      document.open();
      document.write('<main id="hashchange-checkpoint-replacement">replacement</main>');
      document.close();
    });
  }, { once: true });
  location.hash = "#replace";
  return "scheduled";
})()"##
                .to_owned(),
            await_promise: false,
        })
        .await
        .expect("hashchange replacement reaction should schedule");
    assert_eq!(
        renderer_json_value(scheduled),
        Some(serde_json::json!("scheduled"))
    );

    let replacement_events = tokio::time::timeout(
        Duration::from_secs(2),
        recv_page_lifecycle_until(
            &mut activity_wake_rx,
            &page,
            RendererDocumentLifecycleMilestone::Load,
        ),
    )
    .await
    .expect("the hashchange task-end checkpoint should install replacement lifecycle");
    assert!(replacement_events.iter().any(|event| matches!(
        event.kind,
        RendererDocumentLifecycleEventKind::Milestone(
            RendererDocumentLifecycleMilestone::DomContentLoaded
        )
    )));

    let (replacement, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression:
                "document.querySelector('#hashchange-checkpoint-replacement')?.textContent ?? 'missing'"
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
        .expect("hashchange checkpoint replacement page should close");
}
#[tokio::test(flavor = "multi_thread")]
async fn element_toggle_checkpoint_reconciles_replacement_before_page_restore() {
    let runtime = JsRuntime::initialize();
    let (activity_wake_tx, mut activity_wake_rx) = renderer_external_activity_test_channel();
    runtime.set_renderer_output_transport_sender(activity_wake_tx);
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let page_url = url::Url::parse("https://example.test/element-toggle-checkpoint-document-open")
        .expect("page URL");
    let mut page = create_test_html_page(
        &runtime,
        &loader,
        page_url,
        r#"<!doctype html><body>
<details id="element-toggle-checkpoint"><summary>summary</summary></details>
</body>"#,
    )
    .await;

    let _ = page
        .run_async_command(RendererPageCommand::TakeDocumentLifecycleEvents)
        .await
        .expect("initial lifecycle events should be drainable");
    while activity_wake_rx.try_recv().is_ok() {}

    let (scheduled, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"(() => {
  const details = document.getElementById("element-toggle-checkpoint");
  details.addEventListener("toggle", () => {
    Promise.resolve().then(() => {
      document.open();
      document.write('<main id="element-toggle-replacement">replacement</main>');
      document.close();
    });
  }, { once: true });
  details.open = true;
  return "scheduled";
})()"#
                .to_owned(),
            await_promise: false,
        })
        .await
        .expect("element-toggle replacement reaction should schedule");
    assert_eq!(
        renderer_json_value(scheduled),
        Some(serde_json::json!("scheduled"))
    );

    let replacement_events = tokio::time::timeout(
        Duration::from_secs(2),
        recv_page_lifecycle_until(
            &mut activity_wake_rx,
            &page,
            RendererDocumentLifecycleMilestone::Load,
        ),
    )
    .await
    .expect("the element-toggle task-end checkpoint should install replacement lifecycle");
    assert!(replacement_events.iter().any(|event| matches!(
        event.kind,
        RendererDocumentLifecycleEventKind::Milestone(
            RendererDocumentLifecycleMilestone::DomContentLoaded
        )
    )));

    let (replacement, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression:
                "document.querySelector('#element-toggle-replacement')?.textContent ?? 'missing'"
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
        .expect("element-toggle checkpoint replacement page should close");
}
#[tokio::test(flavor = "multi_thread")]
async fn image_load_checkpoint_reconciles_replacement_before_page_restore() {
    let runtime = JsRuntime::initialize();
    let (activity_wake_tx, mut activity_wake_rx) = renderer_external_activity_test_channel();
    runtime.set_renderer_output_transport_sender(activity_wake_tx);
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let page_url =
        url::Url::parse("https://example.test/image-checkpoint-document-open").expect("page URL");
    let mut page =
        create_test_html_page(&runtime, &loader, page_url, "<!doctype html><body></body>").await;

    let _ = page
        .run_async_command(RendererPageCommand::TakeDocumentLifecycleEvents)
        .await
        .expect("initial lifecycle events should be drainable");
    while activity_wake_rx.try_recv().is_ok() {}

    let (scheduled, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"(() => {
  const image = new Image();
  image.addEventListener("load", () => {
    Promise.resolve().then(() => {
      document.open();
      document.write('<main id="image-replacement">replacement</main>');
      document.close();
    });
  }, { once: true });
  document.body.appendChild(image);
  image.src = "/not-fetched-by-policy.png";
  return "scheduled";
})()"#
                .to_owned(),
            await_promise: false,
        })
        .await
        .expect("image replacement reaction should schedule");
    assert_eq!(
        renderer_json_value(scheduled),
        Some(serde_json::json!("scheduled"))
    );

    let replacement_events = tokio::time::timeout(
        Duration::from_secs(2),
        recv_page_lifecycle_until(
            &mut activity_wake_rx,
            &page,
            RendererDocumentLifecycleMilestone::Load,
        ),
    )
    .await
    .expect("the image task-end checkpoint should install replacement lifecycle");
    assert!(replacement_events.iter().any(|event| matches!(
        event.kind,
        RendererDocumentLifecycleEventKind::Milestone(
            RendererDocumentLifecycleMilestone::DomContentLoaded
        )
    )));

    let (replacement, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: "document.querySelector('#image-replacement')?.textContent ?? 'missing'"
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
        .expect("image checkpoint replacement page should close");
}
#[tokio::test(flavor = "multi_thread")]
async fn connected_style_checkpoint_reconciles_replacement_before_page_restore() {
    let runtime = JsRuntime::initialize();
    let (activity_wake_tx, mut activity_wake_rx) = renderer_external_activity_test_channel();
    runtime.set_renderer_output_transport_sender(activity_wake_tx);
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let page_url =
        url::Url::parse("https://example.test/style-checkpoint-document-open").expect("page URL");
    let mut page =
        create_test_html_page(&runtime, &loader, page_url, "<!doctype html><body></body>").await;

    let _ = page
        .run_async_command(RendererPageCommand::TakeDocumentLifecycleEvents)
        .await
        .expect("initial lifecycle events should be drainable");
    while activity_wake_rx.try_recv().is_ok() {}

    let (scheduled, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"(() => {
  const style = document.createElement("style");
  style.textContent = "body { color: teal; }";
  style.addEventListener("load", () => {
    Promise.resolve().then(() => {
      document.open();
      document.write('<main id="style-replacement">replacement</main>');
      document.close();
    });
  }, { once: true });
  document.head.appendChild(style);
  return "scheduled";
})()"#
                .to_owned(),
            await_promise: false,
        })
        .await
        .expect("connected-style replacement reaction should schedule");
    assert_eq!(
        renderer_json_value(scheduled),
        Some(serde_json::json!("scheduled"))
    );

    let replacement_events = tokio::time::timeout(
        Duration::from_secs(2),
        recv_page_lifecycle_until(
            &mut activity_wake_rx,
            &page,
            RendererDocumentLifecycleMilestone::Load,
        ),
    )
    .await
    .expect("the connected-style task-end checkpoint should install replacement lifecycle");
    assert!(replacement_events.iter().any(|event| matches!(
        event.kind,
        RendererDocumentLifecycleEventKind::Milestone(
            RendererDocumentLifecycleMilestone::DomContentLoaded
        )
    )));

    let (replacement, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: "document.querySelector('#style-replacement')?.textContent ?? 'missing'"
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
        .expect("connected-style checkpoint replacement page should close");
}
#[tokio::test(flavor = "multi_thread")]
async fn connected_style_microtasks_run_before_the_delayed_window_load_task() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let page_url = url::Url::parse("https://example.test/style-microtask-before-window-load")
        .expect("page URL");
    let mut page = create_test_html_page(
        &runtime,
        &loader,
        page_url,
        concat!(
            "<!doctype html><head>",
            "<script>",
            "globalThis.__styleLoadOrder = [];",
            "window.addEventListener('load', () => __styleLoadOrder.push('window-load'));",
            "</script>",
            "<style id='ordered-style'>body { color: olive; }</style>",
            "<script>",
            "document.getElementById('ordered-style').addEventListener('load', () => {",
            "  __styleLoadOrder.push('style-load');",
            "  Promise.resolve().then(() => __styleLoadOrder.push('style-microtask'));",
            "});",
            "</script>",
            "</head><body></body>",
        ),
    )
    .await;

    let (order, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: "__styleLoadOrder.join('|')".to_owned(),
            await_promise: false,
        })
        .await
        .expect("connected-style/window load order should evaluate");
    assert_eq!(
        renderer_json_value(order),
        Some(serde_json::json!("style-load|style-microtask|window-load")),
        "the element event task must release its delay before returning, while its task-end checkpoint still runs before the later window-load task"
    );

    page.close_async()
        .await
        .expect("connected-style load-order page should close");
}
#[tokio::test(flavor = "multi_thread")]
async fn user_interaction_checkpoint_reconciles_replacement_before_page_restore() {
    let runtime = JsRuntime::initialize();
    let (activity_wake_tx, mut activity_wake_rx) = renderer_external_activity_test_channel();
    runtime.set_renderer_output_transport_sender(activity_wake_tx);
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let page_url =
        url::Url::parse("https://example.test/user-interaction-checkpoint-document-open")
            .expect("page URL");
    let mut page = create_test_html_page(
        &runtime,
        &loader,
        page_url,
        "<!doctype html><body>initial user-interaction document</body>",
    )
    .await;

    let _ = page
        .run_async_command(RendererPageCommand::TakeDocumentLifecycleEvents)
        .await
        .expect("initial lifecycle events should be drainable");
    while activity_wake_rx.try_recv().is_ok() {}

    let (scheduled, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"(() => {
  const dialog = document.createElement("dialog");
  dialog.addEventListener("close", () => {
    Promise.resolve().then(() => {
      document.open();
      document.write('<main id="user-interaction-checkpoint-replacement">replacement</main>');
      document.close();
    });
  });
  document.body.append(dialog);
  dialog.show();
  dialog.close();
  return "scheduled";
})()"#
                .to_owned(),
            await_promise: false,
        })
        .await
        .expect("user-interaction replacement reaction should schedule");
    assert_eq!(
        renderer_json_value(scheduled),
        Some(serde_json::json!("scheduled"))
    );

    let replacement_events = tokio::time::timeout(
        Duration::from_secs(2),
        recv_page_lifecycle_until(
            &mut activity_wake_rx,
            &page,
            RendererDocumentLifecycleMilestone::Load,
        ),
    )
    .await
    .expect(
        "the user-interaction task-end checkpoint should install and schedule replacement lifecycle",
    );
    assert!(replacement_events.iter().any(|event| matches!(
        event.kind,
        RendererDocumentLifecycleEventKind::Milestone(
            RendererDocumentLifecycleMilestone::DomContentLoaded
        )
    )));

    let (replacement, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression:
                "document.querySelector('#user-interaction-checkpoint-replacement')?.textContent ?? 'missing'"
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
        .expect("user-interaction checkpoint replacement page should close");
}
#[tokio::test(flavor = "multi_thread")]
async fn window_message_checkpoint_reconciles_document_replacement_before_restoring_page_residence()
{
    let runtime = JsRuntime::initialize();
    let (activity_wake_tx, mut activity_wake_rx) = renderer_external_activity_test_channel();
    runtime.set_renderer_output_transport_sender(activity_wake_tx);
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let page_url = url::Url::parse("https://example.test/window-message-checkpoint-document-open")
        .expect("page URL");
    let mut page = create_test_html_page(
        &runtime,
        &loader,
        page_url,
        "<!doctype html><body>initial Window.postMessage document</body>",
    )
    .await;

    let _ = page
        .run_async_command(RendererPageCommand::TakeDocumentLifecycleEvents)
        .await
        .expect("initial lifecycle events should be drainable");
    while activity_wake_rx.try_recv().is_ok() {}

    let (scheduled, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"(() => {
  onmessage = () => {
    Promise.resolve().then(() => {
      document.open();
      document.write('<main id="window-message-checkpoint-replacement">replacement</main>');
      document.close();
    });
  };
  postMessage("replace-document", "*");
  return "scheduled";
})()"#
                .to_owned(),
            await_promise: false,
        })
        .await
        .expect("Window.postMessage replacement reaction should schedule");
    assert_eq!(
        renderer_json_value(scheduled),
        Some(serde_json::json!("scheduled"))
    );

    let replacement_events = tokio::time::timeout(
        Duration::from_secs(2),
        recv_page_lifecycle_until(
            &mut activity_wake_rx,
            &page,
            RendererDocumentLifecycleMilestone::Load,
        ),
    )
    .await
    .expect(
        "the Window.postMessage task-end checkpoint should install and schedule replacement lifecycle",
    );
    assert!(replacement_events.iter().any(|event| matches!(
        event.kind,
        RendererDocumentLifecycleEventKind::Milestone(
            RendererDocumentLifecycleMilestone::DomContentLoaded
        )
    )));

    let (replacement, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression:
                "document.querySelector('#window-message-checkpoint-replacement')?.textContent ?? 'missing'"
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
        .expect("Window.postMessage checkpoint replacement page should close");
}
#[tokio::test(flavor = "multi_thread")]
async fn message_port_checkpoint_reconciles_document_replacement_before_restoring_page_residence() {
    let runtime = JsRuntime::initialize();
    let (activity_wake_tx, mut activity_wake_rx) = renderer_external_activity_test_channel();
    runtime.set_renderer_output_transport_sender(activity_wake_tx);
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let page_url = url::Url::parse("https://example.test/message-port-checkpoint-document-open")
        .expect("page URL");
    let mut page = create_test_html_page(
        &runtime,
        &loader,
        page_url,
        "<!doctype html><body>initial MessagePort document</body>",
    )
    .await;

    let _ = page
        .run_async_command(RendererPageCommand::TakeDocumentLifecycleEvents)
        .await
        .expect("initial lifecycle events should be drainable");
    while activity_wake_rx.try_recv().is_ok() {}

    let (scheduled, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"(() => {
  globalThis.__lmCheckpointMessagePortChannel = new MessageChannel();
  const { port1, port2 } = __lmCheckpointMessagePortChannel;
  port1.onmessage = () => {
    Promise.resolve().then(() => {
      document.open();
      document.write('<main id="message-port-checkpoint-replacement">replacement</main>');
      document.close();
    });
  };
  port2.postMessage("replace-document");
  return "scheduled";
})()"#
                .to_owned(),
            await_promise: false,
        })
        .await
        .expect("MessagePort replacement reaction should schedule");
    assert_eq!(
        renderer_json_value(scheduled),
        Some(serde_json::json!("scheduled"))
    );

    let replacement_events = tokio::time::timeout(
        Duration::from_secs(2),
        recv_page_lifecycle_until(
            &mut activity_wake_rx,
            &page,
            RendererDocumentLifecycleMilestone::Load,
        ),
    )
    .await
    .expect(
        "the MessagePort task-end checkpoint should install and schedule replacement lifecycle",
    );
    assert!(replacement_events.iter().any(|event| matches!(
        event.kind,
        RendererDocumentLifecycleEventKind::Milestone(
            RendererDocumentLifecycleMilestone::DomContentLoaded
        )
    )));

    let (replacement, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression:
                "document.querySelector('#message-port-checkpoint-replacement')?.textContent ?? 'missing'"
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
        .expect("MessagePort checkpoint replacement page should close");
}
#[tokio::test(flavor = "multi_thread")]
async fn timer_checkpoint_reconciles_replacement_before_page_restore() {
    let runtime = JsRuntime::initialize();
    let (activity_wake_tx, mut activity_wake_rx) = renderer_external_activity_test_channel();
    runtime.set_renderer_output_transport_sender(activity_wake_tx);
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let page_url =
        url::Url::parse("https://example.test/timer-checkpoint-document-open").expect("page URL");
    let mut page = create_test_html_page(
        &runtime,
        &loader,
        page_url,
        "<!doctype html><body>initial timer document</body>",
    )
    .await;

    let _ = page
        .run_async_command(RendererPageCommand::TakeDocumentLifecycleEvents)
        .await
        .expect("initial lifecycle events should be drainable");
    while activity_wake_rx.try_recv().is_ok() {}

    let (scheduled, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"(() => {
  setTimeout(() => {
    Promise.resolve().then(() => {
      document.open();
      document.write('<main id="timer-checkpoint-replacement">replacement</main>');
      document.close();
    });
  }, 0);
  return "scheduled";
})()"#
                .to_owned(),
            await_promise: false,
        })
        .await
        .expect("timer replacement reaction should schedule");
    assert_eq!(
        renderer_json_value(scheduled),
        Some(serde_json::json!("scheduled"))
    );

    let replacement_events = tokio::time::timeout(
        Duration::from_secs(2),
        recv_page_lifecycle_until(
            &mut activity_wake_rx,
            &page,
            RendererDocumentLifecycleMilestone::Load,
        ),
    )
    .await
    .expect("the timer task-end checkpoint should install and schedule replacement lifecycle");
    assert!(replacement_events.iter().any(|event| matches!(
        event.kind,
        RendererDocumentLifecycleEventKind::Milestone(
            RendererDocumentLifecycleMilestone::DomContentLoaded
        )
    )));

    let (replacement, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression:
                "document.querySelector('#timer-checkpoint-replacement')?.textContent ?? 'missing'"
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
        .expect("timer checkpoint replacement page should close");
}
#[tokio::test(flavor = "multi_thread")]
async fn owner_runtime_expression_await_uses_page_wake_or_timer_deadline() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let page_url =
        url::Url::parse("https://example.test/runtime-expression-await").expect("page url");
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
            "<!doctype html><body>runtime expression await</body>".to_owned(),
            crate::RendererDocumentOptions {
                ..Default::default()
            },
        )
        .await
        .expect("page should load");
    assert!(pending_download.is_none());

    let (observed, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"new Promise(resolve => {
  setTimeout(() => resolve("owner-await-timer"), 25);
})"#
            .to_owned(),
            await_promise: true,
        })
        .await
        .expect("owner runtime expression await should settle from page timer");
    assert_eq!(
        renderer_json_value(observed),
        Some(serde_json::json!("owner-await-timer"))
    );

    let (globals, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"JSON.stringify(Object.getOwnPropertyNames(globalThis).sort())"#
                .to_owned(),
            await_promise: false,
        })
        .await
        .expect("global property snapshot should evaluate");
    let globals = renderer_json_value(globals)
        .and_then(|value| value.as_str().map(str::to_owned))
        .expect("global property snapshot should be a string");
    assert!(
        !globals.contains("__lmAwaitPromise"),
        "page-level await must not leave legacy global token properties: {globals}"
    );
    assert!(
        !globals.contains("__moliAwaitPromiseToken"),
        "page-level await must not expose legacy await token payloads: {globals}"
    );
    assert!(
        !globals.contains("__moliCompleteRuntimeExpressionAwait"),
        "page-level await must not expose an internal completion binding: {globals}"
    );
}
#[tokio::test(flavor = "multi_thread")]
async fn script_snapshot_ignores_window_indexed_child_frame_globals() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let page_url =
        url::Url::parse("https://example.test/snapshot-child-frame-index").expect("page url");
    let mut page = create_test_html_page(
        &runtime,
        &loader,
        page_url,
        r#"<!doctype html>
<iframe srcdoc="<p>child</p>"></iframe>
<script>globalThis.__lm_snapshot_marker = "parent";</script>"#,
    )
    .await;

    let snapshot = RendererPageTestingHandle::new_for_testing(&page)
        .current_page_state_async()
        .await
        .expect("snapshot should refresh");
    let globals = snapshot.script_execution.globals();

    assert_eq!(
        globals.get("__lm_snapshot_marker"),
        Some(&crate::types::JsValueSnapshot::String("parent".to_owned()))
    );
    assert!(
        !globals.contains_key("0"),
        "Window child-frame indexed properties must not be reported as script globals: {globals:?}"
    );

    page.close_async()
        .await
        .expect("snapshot child frame test page should close");
}
#[tokio::test(flavor = "multi_thread")]
async fn script_snapshot_does_not_stringify_unsupported_globals() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let page_url =
        url::Url::parse("https://example.test/snapshot-unsupported-globals").expect("page url");
    let mut page = create_test_html_page(
        &runtime,
        &loader,
        page_url,
        r#"<!doctype html>
<script>
globalThis.__lm_snapshot_to_string_calls = 0;
globalThis.__lm_snapshot_object = {
  toString() {
    globalThis.__lm_snapshot_to_string_calls += 1;
    return "side-effect";
  }
};
globalThis.__lm_snapshot_large_array = new Array(25000).fill("large-item-value");
const __lm_snapshot_revocable_array = Proxy.revocable([], {});
globalThis.__lm_snapshot_revoked_array = __lm_snapshot_revocable_array.proxy;
__lm_snapshot_revocable_array.revoke();
</script>"#,
    )
    .await;

    let snapshot = RendererPageTestingHandle::new_for_testing(&page)
        .current_page_state_async()
        .await
        .expect("snapshot should refresh");
    let globals = snapshot.script_execution.globals();

    assert_eq!(
        globals.get("__lm_snapshot_object"),
        Some(&crate::types::JsValueSnapshot::Unsupported(
            "[object]".to_owned()
        ))
    );
    assert_eq!(
        globals.get("__lm_snapshot_large_array"),
        Some(&crate::types::JsValueSnapshot::Unsupported(
            "[array]".to_owned()
        ))
    );
    assert_eq!(
        globals.get("__lm_snapshot_revoked_array"),
        Some(&crate::types::JsValueSnapshot::Unsupported(
            "[object]".to_owned()
        ))
    );
    assert_eq!(
        globals.get("__lm_snapshot_to_string_calls"),
        Some(&crate::types::JsValueSnapshot::Number(0.0)),
        "snapshot must not run user-defined toString while describing unsupported globals"
    );

    page.close_async()
        .await
        .expect("unsupported globals snapshot test page should close");
}
#[tokio::test(flavor = "multi_thread")]
async fn native_terminals_publish_before_waiters_are_polled() {
    use super::{RendererCdpCall, RendererNativeOperation, RendererNativeProtocolResponse};
    use moli_page_types::{DevToolsSessionKey, RendererAgentAttachmentId};
    let runtime = JsRuntime::initialize();
    let (output_tx, mut output_rx) = renderer_external_activity_test_channel();
    runtime.set_renderer_output_transport_sender(output_tx);
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut page = create_test_html_page(
        &runtime,
        &loader,
        url::Url::parse("https://example.test/native-terminals").unwrap(),
        "<!doctype html><title>Native terminals</title>",
    )
    .await;
    output_rx.drain();
    let attachment = RendererAgentAttachmentId::allocate();
    let mut pending = Vec::new();
    let mut guards = Vec::new();
    for command_id in [100, 101, 102] {
        let (command, guard) = RendererCdpCall::new(
            moli_page_types::FrontendCommandId::new(command_id),
            DevToolsSessionKey::Attached("session-a".to_owned()),
            attachment,
            RendererNativeOperation::new(RendererPageCommand::ChildFrameTreeSnapshot, |reply| {
                let RendererPageReply::ChildFrameTreeSnapshots(children) = reply.unwrap() else {
                    panic!("child frame tree snapshot reply");
                };
                RendererNativeProtocolResponse::success(serde_json::json!({
                    "childCount": children.len(),
                }))
            }),
        );
        if command_id == 100 {
            // Cancellation precedes admission, so no scheduling race can
            // decide whether this request may publish a terminal.
            drop(guard);
        } else {
            guards.push(guard);
        }
        pending.push((
            command_id,
            page.enqueue_protocol_command_in_inspector_session(
                RendererPageCommand::Native(command),
                Some("session-a".to_owned()),
            )
            .expect("enqueue native terminal"),
        ));
    }
    let later = page
        .enqueue_protocol_command_in_inspector_session(
            RendererPageCommand::EvaluateExpression {
                expression: "42".to_owned(),
                await_promise: false,
            },
            Some("session-a".to_owned()),
        )
        .expect("enqueue concrete same-session gate");
    // Intentionally poll the last command before either native waiter. A
    // receipt/decode handoff would deadlock here. The timeout only diagnoses
    // a blocked gate; no ordering assertion depends on an elapsed interval.
    let later = tokio::time::timeout(std::time::Duration::from_secs(3), later.wait())
        .await
        .expect("published terminals must not retain the Main lane")
        .expect("later evaluation completes");
    assert_eq!(
        renderer_json_value(later.into_reply_and_state().0),
        Some(serde_json::json!(42))
    );
    // Retire the stream before polling native waiters or publication receipts.
    // Already-published replies and their exact fences must survive teardown.
    page.close_async()
        .await
        .expect("close page before consuming native replies");
    let terminals = output_rx
        .drain()
        .into_iter()
        .flat_map(|publication| {
            let cursor = publication.cursor();
            publication
                .into_records()
                .into_iter()
                .filter_map(move |record| match record.into_parts().1 {
                    RendererOutputItem::NativeTerminal(terminal) => Some((cursor, terminal)),
                    _ => None,
                })
        })
        .collect::<Vec<_>>();
    assert_eq!(
        terminals
            .iter()
            .map(|(_, reply)| reply.command_id.get())
            .collect::<Vec<_>>(),
        [101, 102]
    );
    for ((cursor, terminal), guard) in terminals.iter().zip(guards) {
        assert_eq!(terminal.attachment_id, attachment);
        assert_eq!(terminal.session.wire_session_id(), Some("session-a"));
        assert_eq!(
            terminal.reply.result,
            Ok(serde_json::json!({ "childCount": 0 })),
        );
        // Even if an adapter loses its value channel, a committed publication
        // wins over cancellation and suppresses a second terminal error.
        assert_eq!(
            guard
                .cancel_or_published()
                .await
                .expect("publication already committed")
                .cursor(),
            *cursor
        );
    }
    // Decode in reverse order, with a scheduler yield between receives. It
    // cannot change the producer order already observed on the journal.
    while let Some((command_id, pending)) = pending.pop() {
        tokio::task::yield_now().await;
        if command_id == 100 {
            let Err(error) = pending.wait().await else {
                panic!("a canceled native response must not publish");
            };
            assert!(error.to_string().contains("native response was canceled"));
            continue;
        }
        let completion = pending
            .wait()
            .await
            .expect("receive native completion after page retirement");
        let terminal_cursor = terminals
            .iter()
            .find(|(_, terminal)| terminal.command_id.get() == command_id)
            .expect("native terminal was published before retirement")
            .0;
        assert_eq!(
            completion
                .renderer_output_predecessor()
                .expect("native terminal fence")
                .cursor(),
            terminal_cursor,
            "retirement must retain the exact committed terminal boundary",
        );
        let reply = completion.into_reply_and_state().0;
        assert!(matches!(reply, RendererPageReply::NativeCommandPublished));
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn projected_native_success_and_error_publish_without_adapter_receipt() {
    use super::{RendererCdpCall, RendererNativeOperation, RendererNativeProtocolResponse};
    use moli_page_types::{DevToolsSessionKey, FrontendCommandId, RendererAgentAttachmentId};
    let runtime = JsRuntime::initialize();
    let (output_tx, mut output_rx) = renderer_external_activity_test_channel();
    runtime.set_renderer_output_transport_sender(output_tx);
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut page = create_test_html_page(
        &runtime,
        &loader,
        url::Url::parse("https://example.test/native-projection").unwrap(),
        "<!doctype html><title>Native projection</title>",
    )
    .await;
    output_rx.drain();
    let attachment = RendererAgentAttachmentId::allocate();
    let mut waiters = Vec::new();
    let mut guards = Vec::new();
    for id in [1, 2] {
        let (command, guard) = RendererCdpCall::new(
            FrontendCommandId::new(id),
            DevToolsSessionKey::Attached("native".to_owned()),
            attachment,
            RendererNativeOperation::new(
                RendererPageCommand::ComputedStyleProperties {
                    reference:
                        crate::runtime::page_surface::RendererDomNodeReference::BackendNodeId(
                            u32::MAX,
                        ),
                },
                move |reply| {
                    assert!(matches!(
                        reply.unwrap(),
                        RendererPageReply::ComputedStyleProperties(None)
                    ));
                    if id == 1 {
                        RendererNativeProtocolResponse::success(serde_json::json!({"found": false}))
                    } else {
                        RendererNativeProtocolResponse::error(
                            -32000,
                            "Could not find node with given id",
                        )
                    }
                },
            ),
        );
        guards.push(guard);
        waiters.push(
            page.enqueue_protocol_command_in_inspector_session(
                RendererPageCommand::Native(command),
                Some("native".to_owned()),
            )
            .unwrap(),
        );
    }
    let gate = page
        .enqueue_protocol_command_in_inspector_session(
            RendererPageCommand::EvaluateExpression {
                expression: "42".to_owned(),
                await_promise: false,
            },
            Some("native".to_owned()),
        )
        .unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(3), gate.wait())
        .await
        .expect("native publication cannot wait for adapter receipt")
        .unwrap();
    let terminals = output_rx
        .drain()
        .into_iter()
        .flat_map(|publication| publication.into_records())
        .filter_map(|record| match record.into_parts().1 {
            RendererOutputItem::NativeTerminal(terminal) => Some(terminal),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        terminals
            .iter()
            .map(|terminal| terminal.command_id.get())
            .collect::<Vec<_>>(),
        [1, 2]
    );
    let first = &terminals[0].reply;
    assert_eq!(
        first.result.as_ref().unwrap(),
        &serde_json::json!({"found": false})
    );
    let second = &terminals[1].reply;
    assert_eq!(second.result.as_ref().unwrap_err().code, -32000);
    for guard in guards {
        assert!(guard.cancel_or_published().await.is_some());
    }
    for waiter in waiters.into_iter().rev() {
        tokio::task::yield_now().await;
        assert!(matches!(
            waiter.wait().await.unwrap().into_reply_and_state().0,
            RendererPageReply::NativeCommandPublished
        ));
    }
    page.close_async().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn unpolled_internal_reply_does_not_block_later_main_dispatch() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let mut page = create_test_html_page(
        &runtime,
        &loader,
        url::Url::parse("https://example.test/main-reply-handoff").expect("page url"),
        "<!doctype html><title>Main reply handoff</title>",
    )
    .await;

    let snapshot = page
        .enqueue_protocol_command_in_inspector_session(
            RendererPageCommand::ChildFrameTreeSnapshot,
            Some("session-a".to_owned()),
        )
        .expect("enqueue snapshot without polling its reply receiver");
    let later = page
        .enqueue_protocol_command_in_inspector_session(
            RendererPageCommand::EvaluateExpression {
                expression: "globalThis.handoffMarker = 42".to_owned(),
                await_promise: false,
            },
            Some("session-a".to_owned()),
        )
        .expect("enqueue later command in the same session");

    // Complete the later command without polling the earlier typed reply.
    // The timeout diagnoses a receiver gate; it is not an ordering assertion.
    let (later_reply, _) = tokio::time::timeout(std::time::Duration::from_secs(5), later.wait())
        .await
        .expect("internal reply polling must not gate Main dispatch")
        .expect("later command")
        .into_reply_and_state();
    assert_eq!(
        renderer_json_value(later_reply),
        Some(serde_json::json!(42))
    );
    let snapshot = snapshot
        .wait()
        .await
        .expect("receive retained snapshot reply");
    let (snapshot_reply, _) = snapshot.into_reply_and_state();
    assert!(matches!(
        snapshot_reply,
        RendererPageReply::ChildFrameTreeSnapshots(_)
    ));
    page.close_async().await.expect("close test page");
}

#[tokio::test(flavor = "multi_thread")]
async fn protocol_turn_keeps_page_facts_current_and_marks_globals_snapshot_dirty() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let page_url =
        url::Url::parse("https://example.test/globals-snapshot/start").expect("page url");
    let mut page = create_test_html_page(
        &runtime,
        &loader,
        page_url,
        r#"<!doctype html><title>before</title><script>
globalThis.__lm_snapshot_before = 1;
</script>"#,
    )
    .await;

    let initial = RendererPageTestingHandle::new_for_testing(&page)
        .current_page_state_async()
        .await
        .expect("initial page state");
    assert!(initial.script_execution.globals_are_fresh());
    assert_eq!(
        initial.script_execution.global("__lm_snapshot_before"),
        Some(&crate::types::JsValueSnapshot::Number(1.0))
    );

    let output = page
        .enqueue_protocol_command(RendererPageCommand::EvaluateExpression {
            expression: r#"(() => {
  delete globalThis.__lm_snapshot_before;
  globalThis.__lm_snapshot_after = 2;
  document.title = "after";
  history.pushState({}, "", "/globals-snapshot/after");
  return "updated";
})()"#
                .to_owned(),
            await_promise: false,
        })
        .expect("protocol command should enqueue")
        .wait()
        .await
        .expect("protocol command should finish");
    let protocol_state = output.completion().page_state();

    assert_eq!(
        protocol_state.script_execution.globals_snapshot_state(),
        crate::types::ScriptGlobalsSnapshotState::Dirty
    );
    assert!(protocol_state.script_execution.fresh_globals().is_none());
    assert_eq!(
        protocol_state
            .script_execution
            .global("__lm_snapshot_before"),
        Some(&crate::types::JsValueSnapshot::Number(1.0)),
        "dirty compatibility access must remain the last complete snapshot"
    );
    assert!(
        protocol_state
            .script_execution
            .global("__lm_snapshot_after")
            .is_none()
    );
    assert_eq!(protocol_state.document_title(), "after");
    assert_eq!(
        protocol_state.final_url().as_str(),
        "https://example.test/globals-snapshot/after"
    );

    let (_, refreshed) = page
        .run_async_command(RendererPageCommand::RefreshFullPageState)
        .await
        .expect("full report refresh should finish");
    assert!(refreshed.script_execution.globals_are_fresh());
    assert!(
        refreshed
            .script_execution
            .global("__lm_snapshot_before")
            .is_none()
    );
    assert_eq!(
        refreshed.script_execution.global("__lm_snapshot_after"),
        Some(&crate::types::JsValueSnapshot::Number(2.0))
    );

    page.close_async()
        .await
        .expect("globals freshness test page should close");
}
#[tokio::test(flavor = "multi_thread")]
async fn owner_page_creation_applies_document_write_terminal_from_stable_page_route() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let (base_url, server) = spawn_owner_wake_server_with_content_type(
        "/owner-document-write.js",
        "globalThis.__lm_owner_document_write_events.push('external');",
        "application/javascript",
        Duration::ZERO,
    )
    .await;
    let page_url = url::Url::parse(&format!("{base_url}/page")).expect("page url");
    let html = r#"<!doctype html><body><script>
globalThis.__lm_owner_document_write_events = ['inline-before'];
document.write(`<script src="/owner-document-write.js" onload="globalThis.__lm_owner_document_write_events.push('load')"><\/script><main id="owner-written-tail">written</main>`);
globalThis.__lm_owner_document_write_events.push('inline-after');
</script><p id="owner-parser-tail">parser</p></body>"#;

    let mut page = create_test_html_page(&runtime, &loader, page_url, html).await;
    server
        .await
        .expect("owner document.write script server should finish");

    let (reply, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"JSON.stringify({
  events: globalThis.__lm_owner_document_write_events,
  writtenTail: !!document.getElementById('owner-written-tail'),
  parserTail: !!document.getElementById('owner-parser-tail')
})"#
            .to_owned(),
            await_promise: false,
        })
        .await
        .expect("owner-routed document.write result should evaluate");
    assert_eq!(
        renderer_json_value(reply),
        Some(serde_json::json!(
            r#"{"events":["inline-before","inline-after","external","load"],"writtenTail":true,"parserTail":true}"#
        )),
        "the public Page creation path must admit, authorize, and apply the typed terminal before completing creation"
    );

    page.close_async()
        .await
        .expect("owner document.write page should close");
}
#[tokio::test(flavor = "multi_thread")]
async fn owner_page_creation_replays_ready_document_write_after_older_timer_turn() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let (base_url, request_seen, timer_effect_seen, release_response, server) =
        spawn_gated_resource_with_concurrent_effect(
            "/owner-document-write-after-timer.js",
            "globalThis.__lm_owner_document_write_timer_events.push('external');",
            "application/javascript",
            "/owner-document-write-timer-fired",
        )
        .await;
    let page_url = url::Url::parse(&format!("{base_url}/page")).expect("page url");
    let html = r#"<!doctype html><body><script>
globalThis.__lm_owner_document_write_timer_events = ['inline-before'];
setTimeout(() => {
  globalThis.__lm_owner_document_write_timer_events.push('timer');
  fetch('/owner-document-write-timer-fired');
}, 0);
document.write(`<script src="/owner-document-write-after-timer.js" onload="globalThis.__lm_owner_document_write_timer_events.push('load')"><\/script>`);
globalThis.__lm_owner_document_write_timer_events.push('inline-after');
</script></body>"#;

    let mut creation = Box::pin(create_test_html_page(&runtime, &loader, page_url, html));
    tokio::select! {
        seen = request_seen => {
            seen.expect("document.write request should reach the gated server");
        }
        _ = &mut creation => {
            panic!("page creation must remain parked while the script response is gated");
        }
    }
    tokio::time::timeout(Duration::from_secs(2), timer_effect_seen)
        .await
        .expect("the due timer must run while the resource response is still gated")
        .expect("timer effect signal should remain open");
    release_response
        .send(())
        .expect("document.write response should release after the timer turn");
    let mut page = creation.await;
    server
        .await
        .expect("owner timer/document.write script server should finish");
    let (reply, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: "JSON.stringify(globalThis.__lm_owner_document_write_timer_events)"
                .to_owned(),
            await_promise: false,
        })
        .await
        .expect("owner timer/document.write result should evaluate");
    assert_eq!(
        renderer_json_value(reply),
        Some(serde_json::json!(
            r#"["inline-before","inline-after","timer","external","load"]"#
        )),
        "a ready typed terminal must receive a fresh internal Page admission after the older timer wins the first turn"
    );

    page.close_async()
        .await
        .expect("owner timer/document.write page should close");
}
#[tokio::test(flavor = "multi_thread")]
async fn domcontentloaded_page_creation_reply_resumes_owner_to_load_without_external_work() {
    let runtime = JsRuntime::initialize();
    let (activity_wake_tx, mut activity_wake_rx) = renderer_external_activity_test_channel();
    runtime.set_renderer_output_transport_sender(activity_wake_tx);
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let page_url = url::Url::parse("https://example.test/dcl-reply-load-tail").expect("page url");
    let (completion_tx, completion_rx) = oneshot::channel();
    let (body_tx, raw_body) = ExternalRawDocumentBodyStream::channel(completion_rx);
    let producer = tokio::spawn(async move {
        body_tx
            .send(b"<!doctype html><body>ready</body>".to_vec())
            .await
            .expect("html chunk should send");
        drop(body_tx);
        completion_tx.send(Ok(())).expect("completion should send");
    });

    let (mut page, _, _, creation_artifacts, pending_download) = runtime
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
        .expect("page should reply at DOMContentLoaded");
    producer.await.expect("producer should finish");
    assert!(pending_download.is_none());
    assert!(
        creation_artifacts
            .lifecycle_snapshot
            .dom_content_loaded
            .is_some()
    );
    assert!(creation_artifacts.lifecycle_snapshot.load.is_none());

    let load_events = tokio::time::timeout(
        Duration::from_millis(500),
        recv_page_lifecycle_until(
            &mut activity_wake_rx,
            &page,
            RendererDocumentLifecycleMilestone::Load,
        ),
    )
    .await
    .expect("renderer owner should resume the DCL page reply through load");
    assert!(load_events.iter().any(|event| matches!(
        event.kind,
        RendererDocumentLifecycleEventKind::Milestone(RendererDocumentLifecycleMilestone::Load)
    )));

    page.close_async()
        .await
        .expect("DCL reply load-tail page should close");
}
#[tokio::test(flavor = "multi_thread")]
async fn document_commit_background_dcl_completion_resumes_owner_to_load() {
    let runtime = JsRuntime::initialize();
    let (activity_wake_tx, mut activity_wake_rx) = renderer_external_activity_test_channel();
    runtime.set_renderer_output_transport_sender(activity_wake_tx);
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let predecessor_url = url::Url::parse("about:blank").expect("predecessor page url");
    let mut predecessor =
        create_test_html_page(&runtime, &loader, predecessor_url, "<body>initial</body>").await;
    let page_url =
        url::Url::parse("https://example.test/document-commit-dcl-tail").expect("page url");
    let (completion_tx, completion_rx) = oneshot::channel();
    let (body_tx, raw_body) = ExternalRawDocumentBodyStream::channel(completion_rx);
    let producer = tokio::spawn(async move {
        body_tx
            .send(b"<!doctype html><body>ready</body>".to_vec())
            .await
            .expect("html chunk should send");
        drop(body_tx);
        completion_tx.send(Ok(())).expect("completion should send");
    });

    let (mut page, _, _, creation_artifacts, pending_download) = runtime
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
            RendererReplyBoundary::DocumentCommit,
            RendererTopLevelNavigationDispatch::DelegateToBrowser,
            RendererNavigationReplyPolicy::ReturnWithPendingNavigation,
            None,
            None,
            crate::RendererDocumentOptions {
                root_frame_id: Some("TID-1".to_owned()),
                ..Default::default()
            },
        )
        .await
        .expect("page should attach at document commit");
    producer.await.expect("producer should finish");
    assert!(pending_download.is_none());
    assert!(
        creation_artifacts
            .lifecycle_snapshot
            .dom_content_loaded
            .is_none()
    );
    assert!(creation_artifacts.lifecycle_snapshot.load.is_none());
    predecessor
        .close_async()
        .await
        .expect("predecessor page should close after replacement attach");
    page.take_committed_document_post_response_continuation()
        .expect("DocumentCommit should defer parser continuation")
        .release();

    let observed = tokio::time::timeout(
        Duration::from_millis(500),
        recv_page_lifecycle_until(
            &mut activity_wake_rx,
            &page,
            RendererDocumentLifecycleMilestone::Load,
        ),
    )
    .await
    .expect("document-commit background continuation should reach load");
    let milestones = observed
        .iter()
        .filter_map(|event| match event.kind {
            RendererDocumentLifecycleEventKind::Milestone(milestone) => Some(milestone),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        milestones,
        vec![
            RendererDocumentLifecycleMilestone::DomContentLoaded,
            RendererDocumentLifecycleMilestone::Load,
        ]
    );

    page.close_async()
        .await
        .expect("document-commit DCL-tail page should close");
}
#[tokio::test(flavor = "multi_thread")]
async fn document_title_observation_precedes_dcl_for_exact_document() {
    let runtime = JsRuntime::initialize();
    let (activity_wake_tx, mut activity_wake_rx) = renderer_external_activity_test_channel();
    runtime.set_renderer_output_transport_sender(activity_wake_tx);
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let page_url = url::Url::parse("https://example.test/title-before-dcl").expect("page url");
    let mut page = create_test_html_page_at_document_commit(
        &runtime,
        &loader,
        page_url,
        "<!doctype html><title>committed title</title><main>ready</main>",
    )
    .await;

    let (observed, title_identity, dcl_identity) =
        tokio::time::timeout(Duration::from_secs(2), async {
            let mut observed = Vec::new();
            let mut title_identity = None;
            loop {
                let publication = activity_wake_rx
                    .recv()
                    .await
                    .expect("renderer output channel should stay open");
                if !publication_is_for_page(&publication, &page) {
                    continue;
                }
                for record in publication.records() {
                    match record.item() {
                        RendererOutputItem::Observation(
                            RendererProtocolObservation::DocumentTitleChanged(change),
                        ) => {
                            assert_eq!(change.title, "committed title");
                            assert!(
                                title_identity.replace(change.source_document).is_none(),
                                "an unchanged title must not be published twice"
                            );
                            observed.push("title");
                        }
                        RendererOutputItem::Observation(
                            RendererProtocolObservation::DocumentLifecycle(event),
                        ) if event.kind
                            == RendererDocumentLifecycleEventKind::Milestone(
                                RendererDocumentLifecycleMilestone::DomContentLoaded,
                            ) =>
                        {
                            observed.push("dcl");
                            let identity = super::RendererDocumentLifecycleIdentity {
                                frame: event.frame,
                                document: event.document,
                                epoch: event.epoch,
                            };
                            return (observed, title_identity, identity);
                        }
                        _ => {}
                    }
                }
            }
        })
        .await
        .expect("title and DCL observations should be published");

    assert_eq!(observed, vec!["title", "dcl"]);
    assert_eq!(
        title_identity,
        Some(dcl_identity),
        "the title observation must be sourced from the exact DCL document"
    );

    page.close_async()
        .await
        .expect("title observation page should close");
}
#[tokio::test(flavor = "multi_thread")]
async fn document_commit_release_runs_parser_script_location_handoff_in_background() {
    let runtime = JsRuntime::initialize();
    let (activity_wake_tx, mut activity_wake_rx) = renderer_external_activity_test_channel();
    runtime.set_renderer_output_transport_sender(activity_wake_tx);
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let page_url =
        url::Url::parse("https://example.test/parser-location-source").expect("page url");
    let mut page = create_test_html_page_at_document_commit_with_navigation_dispatch(
        &runtime,
        &loader,
        page_url,
        r#"<!doctype html><script>location.href = "/final"</script>"#,
        RendererTopLevelNavigationDispatch::DelegateToBrowser,
        RendererNavigationReplyPolicy::ReturnWithPendingNavigation,
    )
    .await;

    tokio::time::timeout(Duration::from_secs(2), async {
        while let Some(publication) = activity_wake_rx.recv().await {
            if !publication_is_for_page(&publication, &page) {
                continue;
            }
            if publication.records().iter().any(|record| {
                matches!(
                    record.item(),
                    super::RendererOutputItem::OwnerAction(
                        super::RendererOwnerAction::TopLevelLocationNavigation(_)
                    )
                )
            }) {
                return;
            }
        }
        panic!("renderer output channel closed before parser location handoff");
    })
    .await
    .expect("released parser continuation should publish its location handoff");

    page.close_async()
        .await
        .expect("parser location handoff page should close");
}
#[tokio::test(flavor = "multi_thread")]
async fn command_turn_output_scope_is_removed_after_command_error() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let page_url = url::Url::parse("https://example.test/command-turn-error").expect("page url");
    let mut page = create_test_html_page(
        &runtime,
        &loader,
        page_url,
        "<!doctype html><body>initial</body>",
    )
    .await;

    let (invalid_response_tx, _invalid_response_rx) = oneshot::channel();
    let invalid = page
        .enqueue_async_command(
            RendererPageCommand::dispatch_runtime_protocol_message_with_context_resolution_and_deferred_response(
                None,
                "evaluate".to_owned(),
                "{".to_owned(),
                RendererRuntimeInspectorResponseSender::new(
                    710_220,
                    invalid_response_tx,
                ),
            ),
        )
        .expect("invalid Runtime.evaluate should enqueue")
        .wait()
        .await;
    assert!(
        invalid.is_err(),
        "malformed protocol JSON should fail the command"
    );

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
                        "expression": "42",
                        "returnByValue": true,
                    },
                })
                .to_string(),
                RendererRuntimeInspectorResponseSender::new(call_id, response_tx),
            ),
        )
        .expect("the command after an error should enqueue")
        .wait()
        .await
        .expect("the failed command must not leave an active command-turn output scope");
    assert_eq!(
        completion
            .runtime_inspector_output()
            .and_then(|output| output.protocol_response(call_id))
            .expect("the next Runtime command should retain its response")["result"]["result"]["value"],
        serde_json::json!(42)
    );

    page.close_async()
        .await
        .expect("command-turn error cleanup test page should close");
}
#[tokio::test(flavor = "multi_thread")]
async fn load_target_observer_remains_pending_after_domcontentloaded() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let (
        base_url,
        async_script_request_seen,
        domcontentloaded_request_seen,
        release_async_script_response,
        server,
    ) = spawn_owner_lifecycle_gated_async_server().await;
    let page_url = url::Url::parse(&format!("{base_url}/page")).expect("page url");
    let (completion_tx, completion_rx) = oneshot::channel();
    let (body_tx, raw_body) = ExternalRawDocumentBodyStream::channel(completion_rx);
    let producer = tokio::spawn(async move {
        body_tx
            .send(
                br#"<!doctype html><body>
<script async src="/async.js"></script>
<script>
document.addEventListener("DOMContentLoaded", () => {
  fetch("/domcontentloaded-seen");
}, { once: true });
</script>
</body>"#
                    .to_vec(),
            )
            .await
            .expect("html chunk should send");
        drop(body_tx);
        completion_tx.send(Ok(())).expect("completion should send");
    });

    let creation = runtime.create_streaming_raw_page_from_external_body(
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
    tokio::pin!(creation);

    tokio::select! {
        _result = &mut creation => {
            panic!("Load observer completed before its gated async script request")
        }
        seen = async_script_request_seen => {
            seen.expect("gated async script request channel should stay open");
        }
    }
    tokio::select! {
        _result = &mut creation => {
            panic!("Load observer completed before DOMContentLoaded was observable")
        }
        seen = domcontentloaded_request_seen => {
            seen.expect("DOMContentLoaded signal request channel should stay open");
        }
    }
    producer.await.expect("producer should finish");

    let (probe_tx, probe_rx) = oneshot::channel();
    probe_tx.send(()).expect("readiness probe should send");
    tokio::select! {
        biased;
        _result = &mut creation => {
            panic!("Load observer returned at DOMContentLoaded while async work was blocked")
        }
        _ = probe_rx => {}
    }

    release_async_script_response
        .send(())
        .expect("release gated async script response");
    let (mut page, _, _creation_diagnostics, creation_artifacts, pending_download) =
        tokio::time::timeout(Duration::from_secs(2), creation)
            .await
            .expect("Load observer should complete after the async script response")
            .expect("page should reach Load");
    assert!(pending_download.is_none());
    assert!(
        creation_artifacts
            .lifecycle_snapshot
            .dom_content_loaded
            .is_some()
    );
    assert!(creation_artifacts.lifecycle_snapshot.load.is_some());
    let (marker, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"globalThis.__lm_load_target_async_marker ?? "pending""#.to_owned(),
            await_promise: false,
        })
        .await
        .expect("async marker should evaluate");
    assert_eq!(
        renderer_json_value(marker),
        Some(serde_json::json!("executed"))
    );

    page.close_async()
        .await
        .expect("Load observer test page should close");
    server.await.expect("lifecycle gated server should finish");
}
#[test]
fn script_execution_domain_is_allowed_on_plain_current_thread_runtime() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime should build");
    let executor = JsLocalExecutor::new();

    runtime.block_on(async move {
        assert!(
            super::is_on_script_execution_domain_for(&executor),
            "plain current-thread runtime should remain a valid script execution fallback"
        );
    });
}
#[test]
fn script_execution_domain_is_allowed_on_matching_executor_lane() {
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
                assert!(
                    super::is_on_script_execution_domain_for(&executor_for_assert),
                    "matching executor lane should remain a valid script execution domain"
                );
            })
            .await;
    });
}
#[test]
fn script_execution_domain_is_allowed_on_scaffold_lane() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime should build");
    let local = tokio::task::LocalSet::new();
    let executor = JsLocalExecutor::new();

    local.block_on(&runtime, async move {
        scope_on_scaffold_js_local_executor(async move {
            assert!(
                super::is_on_script_execution_domain_for(&executor),
                "parse-time scaffold lane should remain a valid script execution domain"
            );
        })
        .await;
    });
}
#[test]
fn script_execution_domain_is_rejected_on_different_executor_lane() {
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
                assert!(
                    !super::is_on_script_execution_domain_for(&second_executor),
                    "a different executor lane should not count as this page's script execution domain"
                );
            })
            .await;
    });
}
#[test]
fn script_execution_lane_is_rejected_on_plain_current_thread_runtime() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime should build");
    let executor = JsLocalExecutor::new();

    runtime.block_on(async move {
        assert!(
            !is_on_script_execution_lane_for(&executor),
            "plain current-thread runtime fallback must not count as a lane-backed script execution domain"
        );
    });
}
#[test]
fn script_execution_lane_is_allowed_on_matching_executor_lane() {
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
                assert!(
                    is_on_script_execution_lane_for(&executor_for_assert),
                    "matching executor lane should count as a lane-backed script execution domain"
                );
            })
            .await;
    });
}
#[test]
fn script_execution_lane_is_allowed_on_scaffold_lane() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime should build");
    let local = tokio::task::LocalSet::new();
    let executor = JsLocalExecutor::new();

    local.block_on(&runtime, async move {
        scope_on_scaffold_js_local_executor(async move {
            assert!(
                is_on_script_execution_lane_for(&executor),
                "scaffold lane should count as a lane-backed script execution domain"
            );
        })
        .await;
    });
}
#[test]
fn script_execution_lane_is_rejected_on_different_executor_lane() {
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
                assert!(
                    !is_on_script_execution_lane_for(&second_executor),
                    "a different executor lane must not count as this page's lane-backed script execution domain"
                );
            })
            .await;
    });
}
#[test]
fn scaffold_lane_uses_distinct_script_and_runtime_access_paths() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime should build");
    let local = tokio::task::LocalSet::new();
    let executor = JsLocalExecutor::new();

    local.block_on(&runtime, async move {
        scope_on_scaffold_js_local_executor(async move {
            assert_eq!(
                super::script_execution_domain_path(&executor),
                super::ScriptExecutionDomainPath::DirectScaffoldLane,
                "parse-time scaffold should stay a script-execution domain"
            );
            assert_eq!(
                super::owner_local_runtime_access_path(&executor),
                super::OwnerLocalRuntimeAccessPath::ExecutorHop,
                "parse-time scaffold must not become an owner-local runtime direct path"
            );
        })
        .await;
    });
}
#[test]
fn current_thread_fallback_uses_distinct_direct_paths() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime should build");
    let executor = JsLocalExecutor::new();

    runtime.block_on(async move {
        assert_eq!(
            super::script_execution_domain_path(&executor),
            super::ScriptExecutionDomainPath::CurrentThreadFallback,
            "plain current-thread runtime should remain a script-execution fallback"
        );
        assert_eq!(
            super::script_execution_lane_path(&executor),
            super::ScriptExecutionLanePath::Inaccessible,
            "plain current-thread runtime fallback should no longer count as a lane-backed script execution path"
        );
        assert_eq!(
            super::owner_local_runtime_access_path(&executor),
            super::OwnerLocalRuntimeAccessPath::CurrentThreadFallback,
            "plain current-thread runtime should remain a current-thread owner-local runtime fallback"
        );
        assert_eq!(
            super::owner_local_runtime_entry_path(&executor),
            super::OwnerLocalRuntimeEntryPath::ExecutorHop,
            "plain current-thread runtime fallback should no longer count as a direct owner-local runtime entry path"
        );
    });
}
#[test]
fn current_thread_fallback_is_rejected_on_multithread_runtime() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .expect("multi-thread runtime should build");
    let executor = JsLocalExecutor::new();

    runtime.block_on(async move {
        assert!(
            matches!(
                executor.current_access_context(),
                crate::local_executor::JsLocalExecutorAccessContext::Outside
            ),
            "multi-thread runtimes should stay outside any direct current-thread fallback context"
        );
        assert_eq!(
            super::script_execution_domain_path(&executor),
            super::ScriptExecutionDomainPath::Inaccessible,
            "multi-thread runtimes should not count as direct script-execution fallbacks"
        );
        assert_eq!(
            super::owner_local_runtime_access_path(&executor),
            super::OwnerLocalRuntimeAccessPath::ExecutorHop,
            "multi-thread runtimes should not count as direct owner-local-runtime fallbacks"
        );
    });
}
#[test]
fn named_owner_execution_lane_is_rejected_on_plain_current_thread_runtime() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime should build");
    let executor = JsLocalExecutor::new();

    runtime.block_on(async move {
        assert!(
            !super::is_on_named_owner_execution_lane_for(&executor),
            "plain current-thread fallback must not count as a named owner lane"
        );
    });
}
#[test]
fn named_owner_execution_lane_is_allowed_on_matching_executor_lane() {
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
                assert!(
                    super::is_on_named_owner_execution_lane_for(&executor_for_assert),
                    "matching executor lane should count as the named owner lane"
                );
            })
            .await;
    });
}
#[test]
fn named_owner_execution_lane_is_rejected_on_scaffold_lane() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime should build");
    let local = tokio::task::LocalSet::new();
    let executor = JsLocalExecutor::new();

    local.block_on(&runtime, async move {
        scope_on_scaffold_js_local_executor(async move {
            assert!(
                !super::is_on_named_owner_execution_lane_for(&executor),
                "scaffold lane must not count as the named owner lane"
            );
        })
        .await;
    });
}
#[test]
fn named_owner_execution_lane_is_rejected_on_different_executor_lane() {
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
                assert!(
                    !super::is_on_named_owner_execution_lane_for(&second_executor),
                    "a different executor lane must not count as this page's named owner lane"
                );
            })
            .await;
    });
}
#[test]
fn parse_time_scaffold_lane_is_allowed_on_scaffold_lane() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime should build");
    let local = tokio::task::LocalSet::new();

    local.block_on(&runtime, async move {
        scope_on_scaffold_js_local_executor(async move {
            assert!(
                super::is_on_parse_time_scaffold_lane(),
                "parse-time scaffold lane should be recognized as itself"
            );
        })
        .await;
    });
}
#[test]
fn parse_time_scaffold_lane_is_rejected_on_plain_current_thread_runtime() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime should build");

    runtime.block_on(async move {
        assert!(
            !super::is_on_parse_time_scaffold_lane(),
            "plain current-thread fallback must not count as the parse-time scaffold lane"
        );
    });
}
#[test]
fn parse_time_scaffold_lane_is_rejected_on_named_executor_lane() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime should build");
    let local = tokio::task::LocalSet::new();
    let executor = JsLocalExecutor::new();

    local.block_on(&runtime, async move {
        executor
            .run(async move {
                assert!(
                    !super::is_on_parse_time_scaffold_lane(),
                    "named owner lanes must not count as the parse-time scaffold lane"
                );
            })
            .await;
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn pending_expression_publishes_console_output_before_settlement() {
    let runtime = JsRuntime::initialize();
    let (output_tx, mut output_rx) = renderer_external_activity_test_channel();
    runtime.set_renderer_output_transport_sender(output_tx);
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let url = url::Url::parse("https://example.test/pending-expression-output").unwrap();
    let mut page = create_test_html_page(
        &runtime,
        &loader,
        url,
        "<!doctype html><body>pending</body>",
    )
    .await;
    dispatch_runtime_protocol_with_output_for_test(
        &page,
        serde_json::json!({"id": 731, "method": "Runtime.enable"}),
    )
    .await
    .unwrap();
    output_rx.drain();
    let pending = page
        .enqueue_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"new Promise(resolve => {
                globalThis.__resolvePendingExpression = resolve;
                console.log('owner-output-before-await');
                setTimeout(() => console.log('owner-output-during-await'), 0);
            })"#
            .to_owned(),
            await_promise: true,
        })
        .expect("the pending expression should enqueue");
    let mut completion = Box::pin(pending.wait());
    let observed = tokio::time::timeout(Duration::from_secs(3), async {
        let mut observed = Vec::new();
        while observed.len() < 2 {
            let output = tokio::select! {
                result = &mut completion => panic!("the unresolved expression completed; error: {:?}", result.err()),
                output = output_rx.recv() => output.expect("the output stream should stay open"),
            };
            for record in output.records() {
                let RendererOutputItem::Observation(RendererProtocolObservation::RuntimeInspector(batch)) = record.item() else {
                    continue;
                };
                for message in &batch.messages {
                    let RendererRuntimeInspectorMessage::Protocol(message) = message else {
                        continue;
                    };
                    if message.get("method").and_then(serde_json::Value::as_str)
                        == Some("Runtime.consoleAPICalled")
                        && let Some(value) = message["params"]["args"][0]["value"].as_str()
                    {
                        observed.push(value.to_owned());
                    }
                }
            }
        }
        observed
    })
    .await
    .expect("console output must arrive before promise settlement");
    assert_eq!(
        observed,
        ["owner-output-before-await", "owner-output-during-await"]
    );

    tokio::time::timeout(
        Duration::from_secs(3),
        page.run_async_command(RendererPageCommand::EvaluateExpression {
            expression: "__resolvePendingExpression('done'); 'resolved'".to_owned(),
            await_promise: false,
        }),
    )
    .await
    .expect("a pending expression must allow another command to run")
    .expect("the resolver command should succeed");
    let output = tokio::time::timeout(Duration::from_secs(3), &mut completion)
        .await
        .expect("the expression should complete after its explicit resolution")
        .expect("the resolved expression should succeed");
    let (completion, _) = output.into_completion_and_predecessor();
    let (reply, _, _) = completion.into_parts();
    assert_eq!(renderer_json_value(reply), Some(serde_json::json!("done")));
    page.close_async()
        .await
        .expect("close pending-expression page");
}

#[tokio::test(flavor = "multi_thread")]
async fn navigation_follow_publishes_command_output_before_replacing_document() {
    let runtime = JsRuntime::initialize();
    let (output_tx, mut output_rx) = renderer_external_activity_test_channel();
    runtime.set_renderer_output_transport_sender(output_tx);
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let url = url::Url::parse("https://example.test/navigation-command-output").unwrap();
    let mut page =
        create_test_html_page(&runtime, &loader, url, "<!doctype html><body>source</body>").await;
    dispatch_runtime_protocol_with_output_for_test(
        &page,
        serde_json::json!({"id": 732, "method": "Runtime.enable"}),
    )
    .await
    .unwrap();
    page.run_async_command(RendererPageCommand::EvaluateExpression {
        expression: "document.title = 'source document'".to_owned(),
        await_promise: false,
    })
    .await
    .expect("the source Document title should publish");
    let initial_publications = output_rx.drain();
    let source_stream = initial_publications
        .last()
        .expect("Runtime.enable must publish the source Document's output")
        .cursor()
        .stream();
    let source_document = initial_publications
        .iter()
        .flat_map(|publication| publication.records())
        .find_map(|record| match record.item() {
            RendererOutputItem::Observation(RendererProtocolObservation::DocumentTitleChanged(
                change,
            )) if change.title == "source document" => Some(change.source_document),
            _ => None,
        })
        .expect("the source title observation must identify its exact Document");
    let replacement_url = "data:text/html;charset=utf-8,%3C!doctype%20html%3E%3Ctitle%3Ereplacement%3C/title%3E%3Cbody%3Ereplacement%3C/body%3E";
    let (reply, state) = tokio::time::timeout(
        Duration::from_secs(3),
        page.run_async_command(RendererPageCommand::EvaluateExpressionAndFollowPendingNavigation {
            expression: format!(
                "console.log('command-output-before-navigation'); location.href = {replacement_url:?}; 'navigating'"
            ),
            await_promise: false,
        }),
    )
    .await
    .expect("navigation follow should complete")
    .expect("the command should survive its navigation");
    assert_eq!(
        renderer_json_value(reply),
        Some(serde_json::json!("navigating"))
    );
    assert_eq!(state.document_title, "replacement");

    let publications = output_rx.drain();
    let source_output = publications
        .iter()
        .position(|publication| {
            publication.records().iter().any(|record| matches!(
                record.item(),
                RendererOutputItem::Observation(RendererProtocolObservation::RuntimeInspector(batch))
                    if batch.messages.iter().any(|message| matches!(
                        message,
                        RendererRuntimeInspectorMessage::Protocol(message)
                            if message["method"] == "Runtime.consoleAPICalled"
                                && message["params"]["args"][0]["value"] == "command-output-before-navigation"
                    ))
            ))
        })
        .expect("the source command's console notification must be retained");
    assert_eq!(publications[source_output].cursor().stream(), source_stream);
    let replacement_output = publications
        .iter()
        .position(|publication| {
            publication.records().iter().any(|record| matches!(
                record.item(),
                RendererOutputItem::Observation(RendererProtocolObservation::DocumentTitleChanged(change))
                    if change.title == "replacement" && change.source_document != source_document
            ))
        })
        .expect("the replacement title observation must identify its new Document");
    assert!(
        source_output < replacement_output,
        "command output must be published before replacing its source Document"
    );
    page.close_async()
        .await
        .expect("close navigation-output page");
}
