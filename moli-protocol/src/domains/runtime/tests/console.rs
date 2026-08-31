use super::*;

#[tokio::test(flavor = "multi_thread")]
async fn runtime_discard_console_entries_suppresses_buffered_runtime_events() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(
        &mut ctx,
        "<!doctype html><script>console.warn('boot warning')</script>",
    )
    .await;

    ctx.process_async(json!({"id": 206_810, "method": "Runtime.discardConsoleEntries"}))
        .await;
    ctx.expect_result(206_810, json!({}), None);

    let execution_context_id =
        enable_runtime_and_take_execution_context_id_async(&mut ctx, 206_811).await;
    assert!(
        !ctx.sent
            .iter()
            .any(|message| message["method"] == json!("Runtime.consoleAPICalled")),
        "discardConsoleEntries should suppress buffered console entries: {:?}",
        ctx.sent
    );
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 206_812,
        "method": "Runtime.evaluate",
        "params": {
            "expression": "console.warn('after discard')"
        }
    }))
    .await;

    let response = take_response_by_id(&mut ctx, 206_812);
    assert_eq!(response["result"]["result"]["type"], json!("undefined"));
    ctx.expect_event(
        "Runtime.consoleAPICalled",
        Some(&json!({
            "type": "warning",
            "args": [
                {
                    "type": "string",
                    "value": "after discard"
                }
            ],
            "executionContextId": execution_context_id,
        })),
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn runtime_discard_console_entries_is_page_target_local() {
    let mut ctx = TestContext::new();
    let background_target = crate::conn::PageTargetHost::with_url(
        "TID-background".to_owned(),
        Some("SID-background".to_owned()),
        "about:blank".to_owned(),
    );

    let mut browser_context = BrowserContext::new("BID-1".to_owned());
    browser_context.set_active_target_id("TID-active");
    browser_context.attach_active_session("SID-active");
    browser_context.insert_page_target_host(background_target);
    ctx.conn.browser_context = Some(browser_context);
    ctx.install_navigation_fixture_for_session_owner(
        "data:text/html,<script>console.log('active-discard-peer')</script>",
        Some("SID-active"),
    )
    .await;
    ctx.install_navigation_fixture_for_session_owner(
        "data:text/html,<script>console.log('background-discard-owner')</script>",
        Some("SID-background"),
    )
    .await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 206_815,
        "method": "Runtime.discardConsoleEntries",
        "sessionId": "SID-background"
    }))
    .await;
    let discard = take_response_by_id(&mut ctx, 206_815);
    assert_eq!(discard["sessionId"], json!("SID-background"));
    assert_eq!(discard["result"], json!({}));
    assert_eq!(
        ctx.conn
            .browser_context
            .as_ref()
            .expect("browser context should exist")
            .active_target_id(),
        Some("TID-active"),
        "background Runtime.discardConsoleEntries must not promote the target"
    );

    ctx.sent.clear();
    ctx.process_async(json!({
        "id": 206_816,
        "method": "Runtime.enable",
        "sessionId": "SID-background"
    }))
    .await;
    assert_eq!(take_response_by_id(&mut ctx, 206_816)["result"], json!({}));
    assert!(
        ctx.sent.iter().all(|message| {
            message["method"] != json!("Runtime.consoleAPICalled")
                || message["sessionId"] != json!("SID-background")
        }),
        "discardConsoleEntries should suppress only the addressed background target replay: {:?}",
        ctx.sent
    );

    ctx.sent.clear();
    ctx.process_async(json!({
        "id": 206_817,
        "method": "Runtime.enable",
        "sessionId": "SID-active"
    }))
    .await;
    assert_eq!(take_response_by_id(&mut ctx, 206_817)["result"], json!({}));
    let active_console = ctx.take_first_matching("active Runtime console replay", |message| {
        message["method"] == json!("Runtime.consoleAPICalled")
            && message["sessionId"] == json!("SID-active")
            && message["params"]["args"].as_array().is_some_and(|args| {
                args.iter()
                    .any(|arg| arg["value"] == json!("active-discard-peer"))
            })
    });
    assert_eq!(active_console["sessionId"], json!("SID-active"));
    assert!(
        ctx.sent.iter().all(|message| {
            message["method"] != json!("Runtime.consoleAPICalled")
                || message["params"]["args"].as_array().is_none_or(|args| {
                    !args
                        .iter()
                        .any(|arg| arg["value"] == json!("background-discard-owner"))
                })
        }),
        "active target Runtime.enable must not replay the discarded background console entry: {:?}",
        ctx.sent
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn runtime_get_heap_usage_returns_v8_heap_sizes() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(
        &mut ctx,
        "<!doctype html><script>globalThis.__heapProbe = new Array(64).fill('x')</script>",
    )
    .await;

    ctx.process_async(json!({"id": 206_813, "method": "Runtime.getHeapUsage"}))
        .await;

    let response = take_response_by_id(&mut ctx, 206_813);
    let used_size = response["result"]["usedSize"]
        .as_u64()
        .expect("usedSize should be a CDP number");
    let total_size = response["result"]["totalSize"]
        .as_u64()
        .expect("totalSize should be a CDP number");
    let _embedder_heap_used_size = response["result"]["embedderHeapUsedSize"]
        .as_u64()
        .expect("embedderHeapUsedSize should be a CDP number");
    let _backing_storage_size = response["result"]["backingStorageSize"]
        .as_u64()
        .expect("backingStorageSize should be a CDP number");
    assert!(
        used_size > 0,
        "usedSize should come from the live V8 isolate"
    );
    assert!(
        total_size >= used_size,
        "totalSize should be at least usedSize: {response}"
    );
    assert!(
        response["result"].get("moli").is_none(),
        "Runtime.getHeapUsage should keep Chromium/V8 inspector shape; Moli counters live in HeapProfiler.moliDiagnostics: {response}"
    );
}
#[tokio::test(flavor = "multi_thread")]
async fn heap_profiler_collect_garbage_succeeds_on_loaded_page() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(
        &mut ctx,
        "<!doctype html><script>globalThis.__heapProbe = new Array(1024).fill({value:'x'})</script>",
    )
    .await;

    ctx.process_async(json!({"id": 206_814, "method": "HeapProfiler.collectGarbage"}))
        .await;

    let response = wait_for_response_by_id_async(&mut ctx, None, 206_814).await;
    assert_eq!(response["result"], json!({}));
}

#[tokio::test(flavor = "multi_thread")]
async fn heap_profiler_agent_commands_dispatch_through_v8_on_loaded_page() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(
        &mut ctx,
        "<!doctype html><script>globalThis.__heapProbe = new Array(1024).fill({value:'x'})</script>",
    )
    .await;

    for (id, method) in [
        (206_821_u64, "HeapProfiler.enable"),
        (206_822_u64, "HeapProfiler.collectGarbage"),
        (206_823_u64, "HeapProfiler.disable"),
    ] {
        let raw = json!({
            "id": id,
            "method": method
        })
        .to_string();
        let step = ctx.conn.start_command_dispatch(&raw);
        assert!(
            matches!(&step, CdpCommandTaskStep::Pending(_)),
            "{method} should dispatch to V8 HeapProfiler agent instead of completing as a protocol-side stub"
        );

        let (messages, scheduler_events) =
            complete_command_task_step_for_test(&mut ctx, step).await;
        assert!(
            scheduler_events.is_empty(),
            "{method} should not enqueue scheduler work: {scheduler_events:?}"
        );
        let response = match messages.iter().find(|message| message["id"] == json!(id)) {
            Some(response) => response.clone(),
            None => wait_for_response_by_id_async(&mut ctx, None, id).await,
        };
        assert_eq!(
            response["result"],
            json!({}),
            "{method} should return V8 HeapProfiler agent success: {response:?}"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn heap_profiler_sampling_commands_dispatch_through_v8_on_loaded_page() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(
        &mut ctx,
        "<!doctype html><script>globalThis.__heapProbe = []</script>",
    )
    .await;

    ctx.process_async(json!({
        "id": 206_824,
        "method": "HeapProfiler.startSampling",
        "params": { "samplingInterval": 1024, "stackDepth": 32 }
    }))
    .await;
    assert_eq!(take_response_by_id(&mut ctx, 206_824)["result"], json!({}));

    ctx.process_async(json!({
        "id": 206_825,
        "method": "Runtime.evaluate",
        "params": {
            "expression": "for (let i = 0; i < 200; i++) globalThis.__heapProbe.push({i, value: 'heap-' + i}); 'done'"
        }
    }))
    .await;
    assert_eq!(
        take_response_by_id(&mut ctx, 206_825)["result"]["result"]["value"],
        json!("done")
    );

    ctx.process_async(json!({
        "id": 206_826,
        "method": "HeapProfiler.getSamplingProfile"
    }))
    .await;
    let profile = take_response_by_id(&mut ctx, 206_826);
    assert!(
        profile["result"]["profile"]["head"]["callFrame"]["functionName"].is_string(),
        "HeapProfiler.getSamplingProfile should return V8 sampling profile shape: {profile:?}"
    );
    assert!(
        profile["result"]["profile"]["samples"].is_array(),
        "HeapProfiler.getSamplingProfile should include a samples array: {profile:?}"
    );

    ctx.process_async(json!({
        "id": 206_827,
        "method": "HeapProfiler.stopSampling"
    }))
    .await;
    let stopped = take_response_by_id(&mut ctx, 206_827);
    assert!(
        stopped["result"]["profile"]["head"]["callFrame"]["functionName"].is_string(),
        "HeapProfiler.stopSampling should return V8 sampling profile shape: {stopped:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn failed_heap_profiler_sampling_command_does_not_create_typed_projection() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(&mut ctx, "<!doctype html><body>heap failure</body>").await;

    let raw = json!({
        "id": 206_828,
        "method": "HeapProfiler.startSampling",
        "params": { "samplingInterval": -1 }
    })
    .to_string();
    let step = ctx.conn.start_command_dispatch(&raw);
    assert!(
        matches!(&step, CdpCommandTaskStep::Pending(_)),
        "invalid sampling options should still reach the V8 HeapProfiler agent"
    );
    let (messages, scheduler_events) = complete_command_task_step_for_test(&mut ctx, step).await;
    assert!(scheduler_events.is_empty());
    let response = messages
        .iter()
        .find(|message| message["id"] == json!(206_828))
        .expect("V8 should answer the invalid startSampling request");
    assert!(
        response["error"]["message"].is_string(),
        "V8 should reject a negative sampling interval: {messages:?}"
    );

    ctx.process_async(json!({
        "id": 206_829,
        "method": "HeapProfiler.moliDiagnostics"
    }))
    .await;
    let diagnostics = take_response_by_id(&mut ctx, 206_829);
    let runtime = &diagnostics["result"]["activeBrowserContext"]["runtimeSession"];
    assert!(
        runtime.get("heapProfilerSampling").is_none()
            && runtime.get("rendererInspectorSessionRetained").is_none(),
        "diagnostics must not expose writable typed HeapProfiler state: {diagnostics:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn heap_profiler_sampling_and_tracking_are_restored_on_replacement_page_isolate() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(
        &mut ctx,
        "<!doctype html><script>globalThis.__heapBefore = []</script>",
    )
    .await;

    for command in [
        json!({"id": 206_840, "method": "HeapProfiler.enable"}),
        json!({
            "id": 206_841,
            "method": "HeapProfiler.startTrackingHeapObjects",
            "params": {"trackAllocations": false}
        }),
        json!({
            "id": 206_842,
            "method": "HeapProfiler.startSampling",
            "params": {
                "samplingInterval": 1024,
                "stackDepth": 32,
                "includeObjectsCollectedByMajorGC": true,
                "includeObjectsCollectedByMinorGC": false
            }
        }),
    ] {
        let id = command["id"].as_u64().expect("command id");
        ctx.process_async(command).await;
        assert_eq!(take_response_by_id(&mut ctx, id)["result"], json!({}));
    }

    ctx.process_async(json!({
        "id": 206_843,
        "method": "Page.navigate",
        "params": {
            "url": "data:text/html,<!doctype html><script>globalThis.__heapAfter = new Array(256).fill({value:'after'})</script>"
        }
    }))
    .await;
    let navigate = take_response_by_id(&mut ctx, 206_843);
    assert!(
        navigate["result"]["frameId"].is_string(),
        "navigation should restore both HeapProfiler modes on the replacement isolate: {navigate:?}"
    );

    ctx.process_async(json!({
        "id": 206_844,
        "method": "HeapProfiler.getSamplingProfile"
    }))
    .await;
    let profile = take_response_by_id(&mut ctx, 206_844);
    assert!(
        profile["result"]["profile"]["samples"].is_array(),
        "sampling must remain active in the replacement isolate: {profile:?}"
    );

    ctx.process_async(json!({
        "id": 206_845,
        "method": "Runtime.evaluate",
        "params": {
            "expression": "globalThis.__heapAfter.push(...Array.from({length: 4096}, (_, i) => ({i, value: 'tracked-' + i}))); globalThis.__heapAfter.length",
            "returnByValue": true
        }
    }))
    .await;
    assert!(
        take_response_by_id(&mut ctx, 206_845)["result"]["result"]["value"]
            .as_u64()
            .is_some_and(|length| length > 4096),
        "replacement document should allocate tracked heap objects"
    );
    ctx.process_async(json!({
        "id": 206_846,
        "method": "HeapProfiler.collectGarbage"
    }))
    .await;
    assert_eq!(
        wait_for_response_by_id_async(&mut ctx, None, 206_846).await["result"],
        json!({})
    );

    ctx.process_async(json!({
        "id": 206_847,
        "method": "HeapProfiler.stopTrackingHeapObjects",
        "params": { "reportProgress": false }
    }))
    .await;
    assert_eq!(
        wait_for_response_by_id_async(&mut ctx, None, 206_847).await["result"],
        json!({}),
        "tracking must remain active until explicitly stopped after replacement"
    );
    wait_until_message(
        &mut ctx,
        None,
        "restored HeapProfiler tracking event",
        |message| {
            matches!(
                message["method"].as_str(),
                Some("HeapProfiler.heapStatsUpdate" | "HeapProfiler.lastSeenObjectId")
            )
        },
    )
    .await;

    ctx.process_async(json!({
        "id": 206_848,
        "method": "HeapProfiler.disable"
    }))
    .await;
    assert_eq!(take_response_by_id(&mut ctx, 206_848)["result"], json!({}));
}

#[tokio::test(flavor = "multi_thread")]
async fn deferred_heap_profiler_state_follows_renderer_owned_document_navigation() {
    async fn after_page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html; charset=utf-8")],
            "<!doctype html><title>after</title><body>after</body>",
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let after_url = format!("http://{addr}/after");
    let start_after_url = after_url.clone();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route(
                    "/start",
                    get(move || {
                        let after_url = start_after_url.clone();
                        async move {
                            let after_url = serde_json::to_string(&after_url)
                                .expect("destination URL should serialize");
                            (
                                [(CONTENT_TYPE.as_str(), "text/html; charset=utf-8")],
                                format!(
                                    "<!doctype html><title>start</title><body>start<script>globalThis.__navigateAfterHeapSetup = () => setTimeout(() => location.href = {after_url}, 0);</script></body>"
                                ),
                            )
                        }
                    }),
                )
                .route("/after", get(after_page)),
        )
        .await
        .unwrap();
    });

    let start_url = format!("http://{addr}/start");
    let mut ctx = TestContext::new();
    with_loaded_http_document_async(
        &mut ctx,
        &start_url,
        "SID-heap-renderer-navigation",
        "TID-heap-renderer-navigation",
    )
    .await;
    ctx.conn
        .browser_context
        .as_mut()
        .expect("browser context should exist")
        .set_target_url(start_url);

    ctx.process_async(json!({
        "id": 206_847,
        "method": "HeapProfiler.startSampling",
        "params": { "samplingInterval": 1024, "stackDepth": 32 }
    }))
    .await;
    assert_eq!(take_response_by_id(&mut ctx, 206_847)["result"], json!({}));
    let inspector_state = &ctx
        .conn
        .browser_context
        .as_ref()
        .expect("browser context")
        .active_page_state()
        .devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
        .inspector_session_state;
    assert!(
        inspector_state.v8_state.is_some(),
        "successful HeapProfiler commands must persist an opaque V8 state cookie"
    );

    ctx.process_async(json!({
        "id": 206_848,
        "method": "Runtime.evaluate",
        "params": { "expression": "__navigateAfterHeapSetup()" }
    }))
    .await;
    assert_eq!(
        take_response_by_id(&mut ctx, 206_848)["result"]["result"]["type"],
        json!("number")
    );
    wait_until_message(
        &mut ctx,
        Some("SID-heap-renderer-navigation"),
        "renderer-owned HeapProfiler replacement navigation",
        |message| {
            message["method"] == json!("Page.frameNavigated")
                && message["params"]["frame"]["url"] == json!(after_url)
        },
    )
    .await;

    ctx.process_async(json!({
        "id": 206_849,
        "method": "HeapProfiler.getSamplingProfile"
    }))
    .await;
    let profile = take_response_by_id(&mut ctx, 206_849);
    assert!(
        profile["result"]["profile"]["samples"].is_array(),
        "deferred HeapProfiler success must update the current PageVM restore state before its renderer-owned replacement navigation: {profile:?}"
    );

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn heap_profiler_heap_object_commands_dispatch_to_v8_agent() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(&mut ctx, "<!doctype html><body></body>").await;

    ctx.process_async(json!({
        "id": 206_828,
        "method": "Runtime.evaluate",
        "params": {
            "expression": "globalThis.__heapObjectProbe = {heap: 42}; globalThis.__heapObjectProbe",
            "objectGroup": "heap-object-source"
        }
    }))
    .await;
    let evaluated = take_response_by_id(&mut ctx, 206_828);
    let source_object_id = evaluated["result"]["result"]["objectId"]
        .as_str()
        .unwrap_or_else(|| {
            panic!("Runtime.evaluate should return a heap object handle: {evaluated:?}")
        })
        .to_owned();

    ctx.process_async(json!({
        "id": 206_829,
        "method": "HeapProfiler.getHeapObjectId",
        "params": { "objectId": source_object_id }
    }))
    .await;
    let heap_id_response = take_response_by_id(&mut ctx, 206_829);
    let heap_snapshot_object_id = heap_id_response["result"]["heapSnapshotObjectId"]
        .as_str()
        .unwrap_or_else(|| {
            panic!(
                "HeapProfiler.getHeapObjectId should return a heap object id: {heap_id_response:?}"
            )
        })
        .to_owned();

    ctx.process_async(json!({
        "id": 206_830,
        "method": "HeapProfiler.addInspectedHeapObject",
        "params": { "heapObjectId": "not-a-heap-id" }
    }))
    .await;
    let inspected_response = take_response_by_id(&mut ctx, 206_830);
    assert_eq!(
        inspected_response["error"]["message"],
        json!("Invalid heap snapshot object id"),
        "HeapProfiler.addInspectedHeapObject should be handled by V8 HeapProfiler agent validation: {inspected_response:?}"
    );

    ctx.process_async(json!({
        "id": 206_831,
        "method": "HeapProfiler.getObjectByHeapObjectId",
        "params": {
            "objectId": heap_snapshot_object_id,
            "objectGroup": "heap-object-result"
        }
    }))
    .await;
    let object_response = take_response_by_id(&mut ctx, 206_831);
    assert_eq!(
        object_response["error"]["message"],
        json!("Object is not available"),
        "HeapProfiler.getObjectByHeapObjectId should reach V8 HeapProfiler agent instead of failing as UnknownMethod: {object_response:?}"
    );
}
#[tokio::test]
async fn heap_profiler_moli_diagnostics_reports_connection_state_without_loaded_page() {
    let mut ctx = TestContext::new();

    ctx.process_async(json!({"id": 206_816, "method": "HeapProfiler.moliDiagnostics"}))
        .await;

    let response = take_response_by_id(&mut ctx, 206_816);
    let connection = &response["result"]["connection"];
    let isolate_scope = &response["result"]["isolateScope"];
    assert_eq!(
        connection["hasActiveBrowserContext"],
        json!(false),
        "diagnostics should not require a loaded target: {response:?}"
    );
    assert_eq!(
        connection["pageNavigationEngineCount"],
        json!(0),
        "fresh diagnostics should expose page engine count: {response:?}"
    );
    let scheduler = &response["result"]["scheduler"];
    assert_eq!(
        scheduler["pendingSchedulerEventCount"],
        json!(0),
        "fresh diagnostics should expose scheduler state: {response:?}"
    );
    assert_eq!(
        scheduler["recentActivityTraceCount"],
        json!(0),
        "fresh diagnostics should expose an empty activity trace ring: {response:?}"
    );
    assert_eq!(
        isolate_scope["runtimeGetHeapUsageV8HeapScope"],
        json!("page-vm-document-isolate"),
        "diagnostics should label Runtime.getHeapUsage heap scope: {response:?}"
    );
    assert_eq!(
        isolate_scope["runtimeGetHeapUsageV8HeapIsTargetLocal"],
        json!(true),
        "diagnostics should make PageVM-local heap stats explicit: {response:?}"
    );
    assert_eq!(
        isolate_scope["runtimeGetHeapUsageMoliCountersScope"],
        json!("target-document"),
        "diagnostics should label Moli counter scope separately from V8 heap scope: {response:?}"
    );
    assert_eq!(
        isolate_scope["runtimeCollectGarbageScope"],
        json!("page-vm-document-isolate"),
        "diagnostics should label collectGarbage scope: {response:?}"
    );
    assert_eq!(
        isolate_scope["v8ForegroundTaskWakeScope"],
        json!("page-vm-document-isolate"),
        "diagnostics should label V8 foreground task wake scope: {response:?}"
    );
    assert_eq!(
        isolate_scope["v8ForegroundTaskWakeContextGroupIdAvailable"],
        json!(false),
        "V8 foreground task wakes should not claim a context-group id: {response:?}"
    );
    assert_eq!(
        isolate_scope["v8ForegroundTaskWakeInternalPolicy"],
        json!("page-runtime-queue-and-owner-page-tick"),
        "diagnostics should expose the page-specific internal wake policy: {response:?}"
    );
    assert_eq!(
        isolate_scope["v8ForegroundTaskWakeExternalPolicy"],
        json!("page-owner-runtime-wake"),
        "diagnostics should expose the external wake policy: {response:?}"
    );
    assert_eq!(
        isolate_scope["estimatedWorkerIsolateCount"],
        json!(0),
        "fresh diagnostics should not invent worker isolates without a running worker: {response:?}"
    );
    assert_eq!(
        isolate_scope["estimatedLiveV8IsolateCount"],
        json!(0),
        "fresh diagnostics should not report a live V8 isolate before a document or worker exists: {response:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn heap_profiler_moli_diagnostics_labels_profiler_renderer_agent_ownership() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(&mut ctx, "<!doctype html><body></body>").await;

    ctx.process_async(json!({"id": 206_819, "method": "HeapProfiler.moliDiagnostics"}))
        .await;

    let response = take_response_by_id(&mut ctx, 206_819);
    let runtime_session = &response["result"]["activeBrowserContext"]["runtimeSession"];
    assert_eq!(
        runtime_session["profilerCommandStateSource"],
        json!("renderer-v8-inspector-agent"),
        "Profiler command live-state should be owned by the renderer V8 inspector agent: {response:?}"
    );
    assert!(
        runtime_session
            .get("profilerProjectionStateSource")
            .is_none()
            && runtime_session.get("profilerEnabled").is_none(),
        "diagnostics must not expose a second writable Profiler projection: {response:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn heap_profiler_moli_diagnostics_reports_dedicated_worker_isolates() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(&mut ctx, "<!doctype html><body></body>").await;

    ctx.process_async(json!({
        "id": 206_818,
        "method": "Runtime.evaluate",
        "params": {
            "expression": r#"
                (() => {
                    globalThis.__lmWorkerDiagnosticsReady = false;
                    const worker = new Worker("data:text/javascript,postMessage('ready')");
                    worker.onmessage = () => {
                        globalThis.__lmWorkerDiagnosticsReady = true;
                    };
                    globalThis.__lmWorkerDiagnosticsWorker = worker;
                })()
            "#
        }
    }))
    .await;
    let response = take_response_by_id(&mut ctx, 206_818);
    assert_eq!(response["result"]["result"]["type"], json!("undefined"));

    let mut worker_ready = false;
    for _ in 0..64 {
        ctx.process_async(json!({
            "id": 206_819,
            "method": "Runtime.evaluate",
            "params": {
                "expression": "globalThis.__lmWorkerDiagnosticsReady === true",
                "returnByValue": true
            }
        }))
        .await;
        let ready_response = take_response_by_id(&mut ctx, 206_819);
        if ready_response["result"]["result"]["value"] == json!(true) {
            worker_ready = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert!(
        worker_ready,
        "dedicated worker diagnostics probe should become ready"
    );

    ctx.process_async(json!({"id": 206_820, "method": "HeapProfiler.moliDiagnostics"}))
        .await;

    let response = take_response_by_id(&mut ctx, 206_820);
    let isolate_scope = &response["result"]["isolateScope"];
    assert_eq!(
        isolate_scope["dedicatedWorkerLoadingCount"],
        json!(0),
        "diagnostics should not count the ready worker as loading: {response:?}"
    );
    assert_eq!(
        isolate_scope["dedicatedWorkerRunningWorkerIsolateCount"],
        json!(1),
        "diagnostics should aggregate page-owned dedicated worker isolates: {response:?}"
    );
    assert_eq!(
        isolate_scope["estimatedWorkerIsolateCount"],
        json!(1),
        "worker isolate total should include dedicated workers after page snapshots complete: {response:?}"
    );
    assert_eq!(
        isolate_scope["estimatedLiveV8IsolateCount"],
        json!(2),
        "live V8 isolate total should include the renderer document isolate and the dedicated worker isolate: {response:?}"
    );
    assert_eq!(
        isolate_scope["dedicatedWorkerDiagnosticsFailedPageSnapshotCount"],
        json!(0),
        "diagnostics should snapshot the loaded page successfully: {response:?}"
    );
}

#[tokio::test]
async fn heap_profiler_moli_reset_idle_engine_only_resets_without_loaded_page() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(&mut ctx, "<!doctype html><p>loaded</p>").await;

    ctx.process_async(json!({"id": 206_817, "method": "HeapProfiler.moliResetIdleEngine"}))
        .await;

    let loaded_response = take_response_by_id(&mut ctx, 206_817);
    assert_eq!(
        loaded_response["result"]["reset"],
        json!(false),
        "loaded targets should not be reset by the idle-engine diagnostic: {loaded_response:?}"
    );

    ctx.conn
        .browser_context
        .as_mut()
        .expect("browser context")
        .remove_active_page_target_async()
        .await;

    ctx.process_async(json!({"id": 206_819, "method": "HeapProfiler.moliResetIdleEngine"}))
        .await;

    let idle_response = take_response_by_id(&mut ctx, 206_819);
    assert_eq!(
        idle_response["result"]["reset"],
        json!(true),
        "closed target should leave the engine eligible for idle reset: {idle_response:?}"
    );
}
#[tokio::test(flavor = "multi_thread")]
async fn runtime_console_api_called_is_incremental_after_evaluate() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(&mut ctx, "<!doctype html><body></body>").await;
    let execution_context_id =
        enable_runtime_and_take_execution_context_id_async(&mut ctx, 20_682).await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 20_683,
        "method": "Runtime.evaluate",
        "params": {
            "expression": "console.error('later error')"
        }
    }))
    .await;

    let response = take_response_by_id(&mut ctx, 20_683);
    assert_eq!(response["result"]["result"]["type"], json!("undefined"));
    ctx.expect_event(
        "Runtime.consoleAPICalled",
        Some(&json!({
            "type": "error",
            "args": [
                {
                    "type": "string",
                    "value": "later error"
                }
            ],
            "executionContextId": execution_context_id,
        })),
    );

    ctx.process_async(json!({
        "id": 20_684,
        "method": "Runtime.evaluate",
        "params": {
            "expression": "1 + 1"
        }
    }))
    .await;
    let response = take_response_by_id(&mut ctx, 20_684);
    assert_eq!(response["result"]["result"]["value"], json!(2));
    assert!(
        !ctx.sent
            .iter()
            .any(|message| message["method"] == json!("Runtime.consoleAPICalled")),
        "Runtime.consoleAPICalled should not replay old console messages: {:?}",
        ctx.sent
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn runtime_disable_stops_v8_runtime_console_api_events() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(&mut ctx, "<!doctype html><body></body>").await;
    let _ = enable_runtime_and_take_execution_context_id_async(&mut ctx, 206_828).await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 206_829,
        "method": "Runtime.disable"
    }))
    .await;
    let disable = take_response_by_id(&mut ctx, 206_829);
    assert_eq!(disable["result"], json!({}));
    assert!(
        !ctx.sent
            .iter()
            .any(|message| message["method"] == json!("Runtime.consoleAPICalled")),
        "Runtime.disable should not emit Runtime console events: {:?}",
        ctx.sent
    );
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 206_830,
        "method": "Runtime.evaluate",
        "params": {
            "expression": "console.log('after runtime disable'); 7",
            "returnByValue": true
        }
    }))
    .await;
    let response = take_response_by_id(&mut ctx, 206_830);
    assert_eq!(response["result"]["result"]["value"], json!(7));
    assert!(
        !ctx.sent
            .iter()
            .any(|message| message["method"] == json!("Runtime.consoleAPICalled")),
        "disabled V8 Runtime agent must not continue emitting Runtime.consoleAPICalled: {:?}",
        ctx.sent
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn runtime_console_api_called_continues_after_isolated_world_console() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(&mut ctx, "<!doctype html><body></body>").await;
    let bc = ctx
        .conn
        .browser_context
        .as_mut()
        .expect("browser context should exist");
    bc.set_active_target_id("TID-1");
    bc.attach_active_session("SID-1");
    let default_context_id =
        enable_runtime_and_take_execution_context_id_async(&mut ctx, 206_820).await;
    let isolated_context_id = create_isolated_world_async(&mut ctx, 206_821, "utility").await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 206_822,
        "method": "Runtime.evaluate",
        "params": {
            "contextId": isolated_context_id,
            "expression": "console.log('isolated first')"
        }
    }))
    .await;
    let response = take_response_by_id(&mut ctx, 206_822);
    assert_eq!(response["result"]["result"]["type"], json!("undefined"));
    wait_until_message(
        &mut ctx,
        None,
        "isolated Runtime.consoleAPICalled",
        |message| {
            message["method"] == json!("Runtime.consoleAPICalled")
                && message["params"]["executionContextId"] == json!(isolated_context_id)
                && message["params"]["args"][0]["value"] == json!("isolated first")
        },
    )
    .await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 206_823,
        "method": "Runtime.evaluate",
        "params": {
            "expression": "console.log('default after isolated')"
        }
    }))
    .await;
    let response = take_response_by_id(&mut ctx, 206_823);
    assert_eq!(response["result"]["result"]["type"], json!("undefined"));
    wait_until_message(
        &mut ctx,
        None,
        "default Runtime.consoleAPICalled after isolated console",
        |message| {
            message["method"] == json!("Runtime.consoleAPICalled")
                && message["params"]["executionContextId"] == json!(default_context_id)
                && message["params"]["args"][0]["value"] == json!("default after isolated")
        },
    )
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn runtime_console_context_id_ignores_user_tampered_context_token_global() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(&mut ctx, "<!doctype html><body></body>").await;
    let default_context_id =
        enable_runtime_and_take_execution_context_id_async(&mut ctx, 206_824).await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 206_825,
        "method": "Runtime.evaluate",
        "params": {
            "expression": r#"
                Object.defineProperty(globalThis, "__moliRuntimeObservableContextToken", {
                    value: 0,
                    configurable: true
                });
                console.log('tamper-resistant console context');
            "#
        }
    }))
    .await;
    let response = take_response_by_id(&mut ctx, 206_825);
    assert_eq!(response["result"]["result"]["type"], json!("undefined"));
    wait_until_message(
        &mut ctx,
        None,
        "tamper-resistant Runtime.consoleAPICalled",
        |message| {
            message["method"] == json!("Runtime.consoleAPICalled")
                && message["params"]["executionContextId"] == json!(default_context_id)
                && message["params"]["args"][0]["value"]
                    == json!("tamper-resistant console context")
        },
    )
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn runtime_console_api_called_ignores_user_tampered_console_buffers() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(&mut ctx, "<!doctype html><body></body>").await;
    let default_context_id =
        enable_runtime_and_take_execution_context_id_async(&mut ctx, 206_826).await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 206_827,
        "method": "Runtime.evaluate",
        "params": {
            "expression": r#"
                Object.defineProperty(globalThis, "__moliConsole", {
                    value: null,
                    configurable: true
                });
                Object.defineProperty(globalThis, "__moliConsoleDetails", {
                    value: "not an array",
                    configurable: true
                });
                console.log('slot-backed console buffer');
            "#
        }
    }))
    .await;
    let response = take_response_by_id(&mut ctx, 206_827);
    assert_eq!(response["result"]["result"]["type"], json!("undefined"));
    wait_until_message(
        &mut ctx,
        None,
        "slot-backed Runtime.consoleAPICalled",
        |message| {
            message["method"] == json!("Runtime.consoleAPICalled")
                && message["params"]["executionContextId"] == json!(default_context_id)
                && message["params"]["args"][0]["value"] == json!("slot-backed console buffer")
        },
    )
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn runtime_timer_publication_emits_console_api_called_without_followup_command() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(&mut ctx, "<!doctype html><body></body>").await;
    let _ = enable_runtime_and_take_execution_context_id_async(&mut ctx, 20_685).await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 20_686,
        "method": "Runtime.evaluate",
        "params": {
            "expression": "setTimeout(() => console.log('timer observable'), 20)"
        }
    }))
    .await;
    let response = take_response_by_id(&mut ctx, 20_686);
    assert_eq!(response["result"]["result"]["type"], json!("number"));

    wait_until_message(
        &mut ctx,
        None,
        "timer Runtime.consoleAPICalled",
        |message| {
            message["method"] == json!("Runtime.consoleAPICalled")
                && message["params"]["type"] == json!("log")
                && message["params"]["args"][0]["value"] == json!("timer observable")
        },
    )
    .await;
}
#[tokio::test(flavor = "multi_thread")]
async fn runtime_timer_cross_document_navigation_with_history_api_emits_full_context_commit() {
    async fn auth_page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html; charset=utf-8")],
            "<!doctype html><html><title>auth</title><body>auth<script>history.replaceState({auth: true}, '', location.href);</script></body></html>",
        )
    }

    let auth_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let auth_addr = auth_listener.local_addr().unwrap();
    let auth_url = format!("http://{auth_addr}/auth");
    let auth_server = tokio::spawn(async move {
        axum::serve(auth_listener, Router::new().route("/auth", get(auth_page)))
            .await
            .unwrap();
    });

    let start_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let start_addr = start_listener.local_addr().unwrap();
    let start_auth_url = auth_url.clone();
    let start_server = tokio::spawn(async move {
        axum::serve(
            start_listener,
            Router::new().route(
                "/start",
                get(move || {
                    let auth_url = start_auth_url.clone();
                    async move {
                        (
                            [(CONTENT_TYPE.as_str(), "text/html; charset=utf-8")],
                            format!(
                                "<!doctype html><html><title>start</title><body>start<script>setTimeout(() => location.href = '{}', 20);</script></body></html>",
                                auth_url
                            ),
                        )
                    }
                }),
            ),
        )
        .await
        .unwrap();
    });

    let start_url = format!("http://{start_addr}/start");
    let mut ctx = TestContext::new();
    with_loaded_http_document_async(&mut ctx, &start_url, "SID-1", "TID-1").await;
    ctx.conn
        .browser_context
        .as_mut()
        .expect("browser context should exist")
        .set_target_url(start_url);
    ctx.enable_background_navigation_scheduler_for_test();
    let _ = enable_runtime_and_take_execution_context_id_async(&mut ctx, 20_700).await;
    ctx.sent.clear();

    tokio::task::LocalSet::new()
        .run_until(async {
            wait_until_message(
                &mut ctx,
                Some("SID-1"),
                "cross-document frameNavigated after timer navigation",
                |message| {
                    message["method"] == json!("Page.frameNavigated")
                        && message["params"]["frame"]["url"] == json!(auth_url)
                },
            )
            .await;

            let frame_pos = ctx
                .sent
                .iter()
                .position(|message| {
                    message["method"] == json!("Page.frameNavigated")
                        && message["params"]["frame"]["url"] == json!(auth_url)
                })
                .expect("frameNavigated should still be queued");
            let same_document_pos = ctx.sent.iter().position(|message| {
                message["method"] == json!("Page.navigatedWithinDocument")
                    && message["params"]["url"] == json!(auth_url)
            });
            assert!(
                same_document_pos.is_none_or(|pos| pos > frame_pos),
                "cross-document load must not be surfaced as same-document before the full commit: {:?}",
                ctx.sent
            );
            assert!(
                ctx.sent
                    .iter()
                    .any(|message| message["method"]
                        == json!("Runtime.executionContextsCleared")),
                "cross-document commit should clear old Runtime contexts: {:?}",
                ctx.sent
            );
            assert!(
                ctx.sent.iter().any(|message| {
                    message["method"] == json!("Runtime.executionContextCreated")
                        && message["params"]["context"]["name"] == json!(auth_url)
                        && message["params"]["context"]["auxData"]["frameId"] == json!("TID-1")
                }),
                "cross-document commit should create a Runtime context for the new page: {:?}",
                ctx.sent
            );
        })
        .await;

    start_server.abort();
    auth_server.abort();
}
#[tokio::test(flavor = "multi_thread")]
async fn runtime_console_api_called_preserves_basic_argument_shapes() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(&mut ctx, "<!doctype html><body></body>").await;
    let execution_context_id =
        enable_runtime_and_take_execution_context_id_async(&mut ctx, 206_840).await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 206_841,
        "method": "Runtime.evaluate",
        "params": {
            "expression": "console.log('alpha', 42, true, null, undefined, { answer: 42 }, [1, 2], NaN, Infinity, -Infinity, -0, 1n)"
        }
    }))
    .await;

    let response = take_response_by_id(&mut ctx, 206_841);
    assert_eq!(response["result"]["result"]["type"], json!("undefined"));
    wait_until_message(
        &mut ctx,
        None,
        "Runtime.consoleAPICalled with argument shapes",
        |message| {
            message["method"] == json!("Runtime.consoleAPICalled")
                && message["params"]["executionContextId"] == json!(execution_context_id)
        },
    )
    .await;
    let event = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Runtime.consoleAPICalled")
                && message["params"]["executionContextId"] == json!(execution_context_id)
        })
        .expect("consoleAPICalled event should be recorded");
    let args = event["params"]["args"]
        .as_array()
        .expect("consoleAPICalled args should be an array");
    assert_eq!(args.len(), 12);
    assert_eq!(args[0], json!({"type": "string", "value": "alpha"}));
    assert_eq!(args[1]["type"], json!("number"));
    assert_eq!(args[1]["value"], json!(42));
    assert_eq!(args[2]["type"], json!("boolean"));
    assert_eq!(args[2]["value"], json!(true));
    assert_eq!(
        args[3],
        json!({"type": "object", "subtype": "null", "value": null})
    );
    assert_eq!(args[4], json!({"type": "undefined"}));
    assert_eq!(args[5]["type"], json!("object"));
    assert!(
        args[5]["objectId"].is_string(),
        "object console argument should be a V8 remote object: {event}"
    );
    assert!(
        args[5]["description"]
            .as_str()
            .is_some_and(|description| description.contains("Object")),
        "object console argument should include a V8 description: {event}"
    );
    assert_eq!(args[6]["type"], json!("object"));
    assert_eq!(args[6]["subtype"], json!("array"));
    assert!(
        args[6]["objectId"].is_string(),
        "array console argument should be a V8 remote object: {event}"
    );
    assert!(
        args[6]["description"]
            .as_str()
            .is_some_and(|description| description.contains("Array")),
        "array console argument should include a V8 description: {event}"
    );
    assert_eq!(args[7]["type"], json!("number"));
    assert_eq!(args[7]["unserializableValue"], json!("NaN"));
    assert_eq!(args[8]["type"], json!("number"));
    assert_eq!(args[8]["unserializableValue"], json!("Infinity"));
    assert_eq!(args[9]["type"], json!("number"));
    assert_eq!(args[9]["unserializableValue"], json!("-Infinity"));
    assert_eq!(args[10]["type"], json!("number"));
    assert_eq!(args[10]["unserializableValue"], json!("-0"));
    assert_eq!(args[11]["type"], json!("bigint"));
    assert_eq!(args[11]["unserializableValue"], json!("1n"));
}
#[tokio::test(flavor = "multi_thread")]
async fn runtime_console_api_called_includes_basic_stack_trace() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(&mut ctx, "<!doctype html><body></body>").await;
    let execution_context_id =
        enable_runtime_and_take_execution_context_id_async(&mut ctx, 206_842).await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 206_843,
        "method": "Runtime.evaluate",
        "params": {
            "expression": r#"
                function moliConsoleStackOuter() {
                    moliConsoleStackInner();
                }
                function moliConsoleStackInner() {
                    console.warn('stacked warning');
                }
                moliConsoleStackOuter();
            "#
        }
    }))
    .await;

    let response = take_response_by_id(&mut ctx, 206_843);
    assert_eq!(response["result"]["result"]["type"], json!("undefined"));
    wait_until_message(
        &mut ctx,
        None,
        "Runtime.consoleAPICalled with stack trace",
        |message| {
            message["method"] == json!("Runtime.consoleAPICalled")
                && message["params"]["executionContextId"] == json!(execution_context_id)
        },
    )
    .await;
    let event = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Runtime.consoleAPICalled")
                && message["params"]["executionContextId"] == json!(execution_context_id)
        })
        .expect("consoleAPICalled event should be recorded");
    let call_frames = event["params"]["stackTrace"]["callFrames"]
        .as_array()
        .expect("consoleAPICalled should include stackTrace.callFrames");
    assert!(
        call_frames
            .iter()
            .any(|frame| frame["functionName"] == json!("moliConsoleStackInner")),
        "stack trace should include the console call site: {event}"
    );
    assert!(
        call_frames
            .iter()
            .any(|frame| frame["functionName"] == json!("moliConsoleStackOuter")),
        "stack trace should include the caller frame: {event}"
    );
    assert!(
        call_frames.iter().all(|frame| frame["lineNumber"].is_u64()
            && frame["columnNumber"].is_u64()
            && frame["url"].is_string()),
        "stack frames should have CDP-compatible location fields: {event}"
    );
}
#[tokio::test(flavor = "multi_thread")]
async fn runtime_exception_thrown_emits_timer_callback_warning() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(&mut ctx, "<!doctype html><body></body>").await;
    let execution_context_id =
        enable_runtime_and_take_execution_context_id_async(&mut ctx, 20_685).await;
    ctx.process_async(json!({
        "id": 20_686,
        "method": "Runtime.evaluate",
        "params": {
            "expression": "setTimeout(function(){ throw new Error('timer boom') }, 0)"
        }
    }))
    .await;
    let evaluate = take_response_by_id(&mut ctx, 20_686);
    assert!(evaluate.get("error").is_none(), "{evaluate:?}");

    wait_until_message(
        &mut ctx,
        None,
        "Runtime.exceptionThrown for timer callback",
        |message| {
            message["method"] == json!("Runtime.exceptionThrown")
                && message["params"]["exceptionDetails"]["executionContextId"]
                    == json!(execution_context_id)
                && message["params"]["exceptionDetails"]["text"]
                    .as_str()
                    .is_some_and(|text| text.contains("timer boom"))
        },
    )
    .await;
}
#[tokio::test]
async fn runtime_discard_console_entries_advances_background_owner_without_promotion() {
    let mut ctx = TestContext::new();
    with_loaded_runtime_frontend_enabled_background_target_async(
        &mut ctx,
        "TID-active",
        "SID-active",
        "TID-background",
        "SID-background",
        "<script>console.log('owner-discard')</script>",
    )
    .await;

    let queue_console_entries = {
        let runtime_slot = ctx
            .conn
            .runtime_session_owner_slot_mut(Some("SID-background"))
            .expect("background runtime slot should exist");
        runtime_slot.ingest_owner_page_observable_output_updates();
        runtime_slot
            .observable_output_queue_snapshot()
            .expect("background observable queue should exist")
            .observable_output_items
            .iter()
            .filter(|item| {
                matches!(
                    item,
                    moli_core::page::ScriptObservableOutputItem::ConsoleMessage(_)
                )
            })
            .count()
    };
    assert_eq!(
        queue_console_entries, 1,
        "background owner observable queue should have one console message"
    );

    ctx.process_async(json!({
        "id": 534,
        "method": "Runtime.discardConsoleEntries",
        "sessionId": "SID-background"
    }))
    .await;
    let response = take_response_by_id(&mut ctx, 534);
    assert_eq!(response["result"], json!({}));
    let browser_context = ctx
        .conn
        .browser_context
        .as_ref()
        .expect("browser context should exist");
    assert_eq!(
        browser_context.active_target_id(),
        Some("TID-active"),
        "background Runtime.discardConsoleEntries should not promote the target"
    );
    assert_eq!(
        browser_context
            .parked_target_owner_state_or_default("TID-background")
            .runtime_observable_state
            .emitted_console_entries(),
        queue_console_entries,
        "discardConsoleEntries should advance the background owner observable cursor"
    );
}
#[tokio::test(flavor = "multi_thread")]
async fn background_runtime_get_heap_usage_reads_owner_page_without_promotion() {
    let mut ctx = TestContext::new();
    with_loaded_runtime_frontend_enabled_background_target_async(
        &mut ctx,
        "TID-active",
        "SID-active",
        "TID-background",
        "SID-background",
        "<script>globalThis.__backgroundHeapProbe = new Array(64).fill('background')</script>",
    )
    .await;

    ctx.process_async(json!({
        "id": 532,
        "method": "Runtime.getHeapUsage",
        "sessionId": "SID-background"
    }))
    .await;
    let response = take_response_by_id(&mut ctx, 532);
    let used_size = response["result"]["usedSize"]
        .as_u64()
        .unwrap_or_else(|| panic!("Runtime.getHeapUsage should return usedSize: {response:?}"));
    let total_size = response["result"]["totalSize"]
        .as_u64()
        .unwrap_or_else(|| panic!("Runtime.getHeapUsage should return totalSize: {response:?}"));
    assert!(used_size > 0, "usedSize should come from a live owner page");
    assert!(
        total_size >= used_size,
        "totalSize should be at least usedSize: {response:?}"
    );
    assert_eq!(
        response["sessionId"],
        json!("SID-background"),
        "direct response should remain scoped to the background session"
    );
    assert_eq!(
        ctx.conn
            .browser_context
            .as_ref()
            .and_then(|browser_context| browser_context.active_target_id()),
        Some("TID-active"),
        "background Runtime.getHeapUsage should not promote the target"
    );
}
