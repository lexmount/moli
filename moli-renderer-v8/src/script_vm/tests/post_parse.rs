use super::*;
use crate::dynamic_script_owner::DynamicScriptRunnable;

fn dynamic_script_owner_id(raw: u64) -> crate::dynamic_script_owner::DynamicScriptOwnerId {
    crate::dynamic_script_owner::DynamicScriptOwnerId::from_u64(raw)
}

fn take_main_document_runtime_action(
    queue: &crate::page_task_queue::PageTaskQueueTestHarness,
) -> Option<RendererPageMainDocumentRuntimeAction> {
    queue
        .task_sources()
        .take_main_document_runtime_for_executor_test()
        .map(|task| task.into_action())
}

fn prepared_post_parse_script(position: usize, mode: ScriptMode) -> PreparedScript {
    let attribute = match mode {
        ScriptMode::Async => " async",
        ScriptMode::Defer => " defer",
        ScriptMode::Normal => "",
        other => panic!("unsupported test script mode: {other:?}"),
    };
    let document = HtmlParser.parse(
            Url::parse("https://example.com/").expect("test url"),
            format!(
                "<!doctype html><html><head><script{attribute} src=\"/script-{position}.js\"></script></head><body></body></html>"
            ),
        );
    let script = document
        .script_handles()
        .first()
        .copied()
        .expect("script handle");
    let classification = classify_parser_script(&document, script).expect("script classification");
    let final_url = document.final_url_clone().expect("document final url");
    let document_base_url = document
        .document_base_url_clone()
        .expect("document base url");
    match build_prepared_script(
        &classification,
        final_url,
        document_base_url,
        script,
        position,
    ) {
        PrepareScriptOutcome::Prepared(script) => *script,
        _ => panic!("expected prepared post-parse script"),
    }
}

fn test_post_parse_lifecycle_driver_for(
    stage: crate::renderer::PageVmInitStage,
) -> PostParseLifecycleDriver {
    PostParseLifecycleDriver::new(
        stage,
        PostParseLifecycleRound {
            queue_stats: PostParseLifecycleQueueStats::default(),
            phase_started: Instant::now(),
        },
    )
}

fn ready_dynamic_runtime_script(position: usize) -> PreparedScript {
    PreparedScript {
        position,
        node_id: NodeId::new(position + 1),
        kind: ScriptKind::Classic,
        mode: ScriptMode::Async,
        source_kind: ScriptSourceKind::External,
        fetch_metadata: crate::planning::ScriptFetchMetadata::default(),
        source: ScriptSource::External,
        url: Url::parse(&format!("https://example.com/dynamic-{position}.js")).unwrap(),
        base_url: Url::parse(&format!("https://example.com/dynamic-{position}.js")).unwrap(),
        initiator_url: Url::parse("https://example.com/").unwrap(),
        host_script_handle: None,
    }
}

#[tokio::test]
async fn successful_external_classic_script_drains_microtasks_before_completion() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_test_vm_with_loader("https://example.com/", &loader);
    let script = ready_dynamic_runtime_script(6);

    let outcome = vm
        .execute_loaded_prepared_script_source(
            &script,
            r#"
            globalThis.__classicScriptOrder = ["body"];
            queueMicrotask(() => globalThis.__classicScriptOrder.push("microtask"));
            "#,
            None,
        )
        .await
        .expect("external classic script should complete");

    assert!(matches!(
        outcome,
        crate::script_vm::LoadedScriptExecutionOutcome::Completed(
            crate::script_vm::PreparedScriptBodyActivity::Entered
        )
    ));
    assert_eq!(
        vm.eval("JSON.stringify(globalThis.__classicScriptOrder)")
            .expect("classic script microtask should be observable at completion"),
        r#"["body","microtask"]"#,
    );
}

#[tokio::test]
async fn classic_script_exception_reports_window_error_then_completes() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_test_vm_with_loader("https://example.com/", &loader);
    vm.eval(
        r#"
        globalThis.__classicScriptOrder = [];
        globalThis.__classicScriptError = null;
        window.onerror = (message, source, line, column, error) => {
          globalThis.__classicScriptOrder.push("global-error");
          globalThis.__classicScriptError = {
            message,
            source,
            linePositive: typeof line === "number" && line > 0,
            columnPositive: typeof column === "number" && column > 0,
            exactPrimitive: error === 7,
          };
          return true;
        };
        "installed";
        "#,
    )
    .expect("window error observer should install");

    let mut script = ready_dynamic_runtime_script(7);
    script.url = Url::parse("data:,throw 7").expect("data script URL");
    script.base_url = script.url.clone();
    let outcome = vm
        .execute_loaded_prepared_script_source(
            &script,
            r#"
            globalThis.__classicScriptOrder.push("body");
            queueMicrotask(() => globalThis.__classicScriptOrder.push("microtask"));
            throw 7;
            "#,
            None,
        )
        .await
        .expect("a page-thrown exception should not fail classic script loading");

    assert!(matches!(
        outcome,
        crate::script_vm::LoadedScriptExecutionOutcome::Completed(
            crate::script_vm::PreparedScriptBodyActivity::Entered
        )
    ));
    assert_eq!(
        vm.eval(
            r#"
            JSON.stringify({
              order: globalThis.__classicScriptOrder,
              error: globalThis.__classicScriptError,
            })
            "#,
        )
        .expect("classic script error report should remain observable"),
        r#"{"order":["body","global-error","microtask"],"error":{"message":"Uncaught 7","source":"data:,throw 7","linePositive":true,"columnPositive":true,"exactPrimitive":true}}"#,
    );
}

fn is_document_script_execution_work(
    work: &PostParsePageOwnedWork,
    lane: crate::document_script_scheduler::DocumentScriptExecutionLane,
    position: usize,
) -> bool {
    let work = match work {
        PostParsePageOwnedWork::DocumentScript(work)
        | PostParsePageOwnedWork::DocumentScriptWithStylesheetSnapshot { work, .. } => work,
        PostParsePageOwnedWork::Lifecycle(_) => return false,
    };
    matches!(
        work.as_ref(),
        crate::document_script_scheduler::PageOwnedDocumentScriptWork::Script {
            lane: actual_lane,
            script,
            ..
        } if *actual_lane == lane && script.position == position
    )
}

fn pop_post_parse_page_task(page_task_queue: &mut PageTaskQueue) -> Option<PageTask> {
    page_task_queue
        .post_parse_pop_front()
        .and_then(PostParsePageOwnedWork::into_page_task)
}

fn post_parse_lifecycle_work(work: PostParseLifecycleWork) -> PostParsePageOwnedWork {
    PostParsePageOwnedWork::lifecycle_work(work)
}

fn post_parse_lifecycle_action(work: PostParseLifecycleWork) -> PostParseProcessingAction {
    PostParseProcessingAction::from_page_owned_work(post_parse_lifecycle_work(work))
}

fn post_parse_document_script_action(
    lane: crate::document_script_scheduler::DocumentScriptExecutionLane,
    script: PreparedScript,
) -> PostParseProcessingAction {
    PostParseProcessingAction::from_page_owned_work(post_parse_document_script_work(lane, script))
}

fn post_parse_document_script_work(
    lane: crate::document_script_scheduler::DocumentScriptExecutionLane,
    script: PreparedScript,
) -> PostParsePageOwnedWork {
    PostParsePageOwnedWork::document_script_work(
        crate::document_script_scheduler::PageOwnedDocumentScriptWork::script(lane, script),
    )
}

fn post_parse_owner_step_for_lifecycle_work(
    work: PostParseLifecycleWork,
) -> PostParseOwnerDriverStep {
    PostParseOwnerDriverStep::Ready(Box::new(DocumentProcessingAction::PostParsePageOwnedWork(
        Box::new(post_parse_lifecycle_work(work)),
    )))
}

fn ready_dynamic_runtime_module_script(position: usize, node_id: NodeId) -> PreparedScript {
    PreparedScript {
        position,
        node_id,
        kind: ScriptKind::Module,
        mode: ScriptMode::ModuleInOrder,
        source_kind: ScriptSourceKind::External,
        fetch_metadata: crate::planning::ScriptFetchMetadata::default(),
        source: ScriptSource::Loaded(
            "import './dep.js'; globalThis.dynamicModuleRan = true;".to_owned(),
        ),
        url: Url::parse(&format!("https://example.com/dynamic-module-{position}.js")).unwrap(),
        base_url: Url::parse(&format!("https://example.com/dynamic-module-{position}.js")).unwrap(),
        initiator_url: Url::parse("https://example.com/").unwrap(),
        host_script_handle: Some(format!("runtime-module-{position}")),
    }
}

#[test]
fn csp_request_treats_missing_dynamic_script_node_as_not_parser_inserted() {
    let vm = new_storage_test_vm("https://app.test/page.html");
    let script = ready_dynamic_runtime_script(1);

    let request = vm.content_security_policy_script_element_request(&script);

    assert!(
        !request.parser_inserted,
        "an unknown DOM node should not make a dynamic script parser-inserted"
    );
}

#[test]
fn external_script_redirect_final_url_obeys_script_src_csp() {
    let mut vm = new_storage_test_vm("https://app.test/page.html");
    vm.set_response_content_security_policies(&["script-src 'self'".to_owned()]);
    vm.eval(
        r#"
        globalThis.__scriptCspEvents = [];
        document.addEventListener("securitypolicyviolation", event => {
            globalThis.__scriptCspEvents.push({
                blockedURI: event.blockedURI,
                effectiveDirective: event.effectiveDirective,
                disposition: event.disposition,
                instance: event instanceof SecurityPolicyViolationEvent,
            });
        });
        "#,
    )
    .expect("install CSP listener");
    let script = PreparedScript {
        position: 1,
        node_id: NodeId::new(2),
        kind: ScriptKind::Classic,
        mode: ScriptMode::Async,
        source_kind: ScriptSourceKind::External,
        fetch_metadata: crate::planning::ScriptFetchMetadata::default(),
        source: ScriptSource::External,
        url: Url::parse("https://app.test/redirect.js").unwrap(),
        base_url: Url::parse("https://app.test/redirect.js").unwrap(),
        initiator_url: Url::parse("https://app.test/page.html").unwrap(),
        host_script_handle: None,
    };
    let final_url = Url::parse("https://cdn.test/final.js").unwrap();
    let response = Ok(crate::types::NavigationResponse::from_head_and_text_body(
        moli_fetch::ResponseHead {
            final_url: final_url.clone(),
            status: 200,
            headers: Vec::new(),
            request_cookie_report: None,
            cookie_set_reports: Vec::new(),
            redirected: true,
            redirect_chain: Vec::new(),
            from_cache: false,
            negotiated_http_version: None,
        },
        "globalThis.__redirectScriptRan = true;".to_owned(),
    ));

    let error = vm
        .enforce_external_script_redirect_csp(&script, Some(&response))
        .expect_err("redirected script final URL should be blocked by script-src");
    assert!(error.message().contains("Content Security Policy"));
    assert_eq!(
        drain_pre_domcontentloaded_non_script_page_tasks_for_test(&mut vm),
        1
    );
    assert_eq!(
        vm.eval("JSON.stringify(globalThis.__scriptCspEvents)")
            .expect("read CSP events"),
        format!(
            r#"[{{"blockedURI":"{}","effectiveDirective":"script-src-elem","disposition":"enforce","instance":true}}]"#,
            final_url
        )
    );
}

#[test]
fn module_graph_redirect_final_url_obeys_script_src_csp() {
    let mut vm = new_storage_test_vm("https://app.test/page.html");
    vm.set_response_content_security_report_only_policies(&["script-src 'self'".to_owned()]);
    vm.set_response_content_security_policies(&["script-src 'self'".to_owned()]);
    vm.eval(
        r#"
        globalThis.__moduleCspEvents = [];
        document.addEventListener("securitypolicyviolation", event => {
            globalThis.__moduleCspEvents.push({
                blockedURI: event.blockedURI,
                effectiveDirective: event.effectiveDirective,
                disposition: event.disposition,
                instance: event instanceof SecurityPolicyViolationEvent,
            });
        });
        "#,
    )
    .expect("install module CSP listener");
    let final_url = Url::parse("https://cdn.test/dependency.js").unwrap();
    let fetched_source = crate::module_runtime::ModuleGraphFetchedSource::new(
        final_url.clone(),
        true,
        crate::module_runtime::ModuleSource::text("export default 1;".to_owned()),
    );

    let error = vm
        .module_graph_fetched_source_or_csp_error(
            7,
            fetched_source,
            &crate::module_runtime::ModuleFetchMetadata::default(),
        )
        .expect_err("redirected module final URL should be blocked by script-src");
    assert!(error.message().contains("Content Security Policy"));
    assert_eq!(
        drain_pre_domcontentloaded_non_script_page_tasks_for_test(&mut vm),
        2
    );
    assert_eq!(
        vm.eval("JSON.stringify(globalThis.__moduleCspEvents)")
            .expect("read module CSP events"),
        format!(
            r#"[{{"blockedURI":"{}","effectiveDirective":"script-src-elem","disposition":"report","instance":true}},{{"blockedURI":"{}","effectiveDirective":"script-src-elem","disposition":"enforce","instance":true}}]"#,
            final_url, final_url
        )
    );
}

#[tokio::test]
async fn external_module_prepared_script_hides_root_graph_fetch_in_owner_state() {
    let _js_runtime = crate::JsRuntime::initialize();
    let document = HtmlParser.parse(
        Url::parse("https://example.com/page.html").expect("test url"),
        "<!doctype html><html><head><script type=\"module\" src=\"/entry\"></script></head></html>"
            .to_owned(),
    );
    let script_handle = document
        .script_handles()
        .first()
        .copied()
        .expect("module script handle");
    let classification =
        classify_parser_script(&document, script_handle).expect("script classification");
    let final_url = document.final_url_clone().expect("document final url");
    let document_base_url = document
        .document_base_url_clone()
        .expect("document base url");
    let mut prepared = match build_prepared_script(
        &classification,
        final_url,
        document_base_url,
        script_handle,
        0,
    ) {
        PrepareScriptOutcome::Prepared(script) => *script,
        _ => panic!("expected prepared external module script"),
    };
    assert!(matches!(prepared.source, ScriptSource::External));

    let page_task_queue = crate::page_task_queue::PageTaskQueueTestHarness::new();
    let post_domcontentloaded_page_task_sender =
        page_task_queue.owner_attached_runtime_page_task_sender_for_test();
    let script_event_parser_boundary_sender = page_task_queue.parser_boundary_sender();
    let mut vm = ScriptVmDefaultWorldBootstrap::standalone_from_dom_host_for_test(
        DomHost::from_dom(document),
        post_domcontentloaded_page_task_sender,
        script_event_parser_boundary_sender,
    )
    .expect("script vm bootstrap")
    .finish()
    .expect("script vm finish");
    vm.bind_prepared_script_handle_if_needed(&mut prepared, ScriptHandleSource::ParserOwned);

    let task_owner = vm
        .current_main_document_task_owner()
        .expect("main document task owner");
    assert!(
        vm.claim_main_parser_deferred_script(
            task_owner,
            prepared.clone(),
            None,
            None,
            Default::default(),
        )
        .expect("external module acceptance should start its graph")
    );
    let root_key = crate::module_runtime::ModuleMapKey::java_script(prepared.url.clone());
    let entry = vm
        .document_runtime
        .native_module_entry_id(&root_key)
        .expect("root graph fetch should reserve module map entry");
    assert_eq!(
        vm.document_runtime.native_module_entry_state(entry),
        crate::module_runtime::ModuleMapEntryState::Fetching
    );
    let parser_owner = crate::module_script_continuation::MainParserDocumentOwner::new(task_owner);
    let pending_script_id =
        crate::document_script_scheduler::ParserPendingScriptId::new(parser_owner, &prepared);
    assert!(
        vm.document_runtime
            .parser_module_document_scripts()
            .has_module_script(pending_script_id),
        "graph start must remain attached to its preparation-time PendingScript"
    );
    let marker = vm
        .seal_main_parser_deferred_scripts(task_owner)
        .expect("parser EOF should arm the accepted module-defer queue");
    assert_eq!(
        marker
            .as_lifecycle_work()
            .expect("parser-deferred marker should be lifecycle work")
            .main_parser_deferred_script_count(),
        1
    );
    assert!(
        vm.document_runtime
            .parser_module_document_scripts()
            .has_after_parsing_script(parser_owner),
        "parser EOF must expose the accepted script through parser order"
    );
}

#[tokio::test]
async fn page_timer_turn_catches_callback_exception_variants_and_continues() {
    let mut vm = new_storage_test_vm("https://example.com/");
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    vm.eval(
        r#"
        globalThis.__timerEvents = [];
        globalThis.__timerReportedErrors = [];
        globalThis.__timerErrorObject = new Error("timer error object");
        addEventListener("error", event => {
            globalThis.__timerReportedErrors.push(
                event.error === globalThis.__timerErrorObject
                    ? "same-error-object"
                    : String(event.error)
            );
        });
        setTimeout(() => {
            globalThis.__timerEvents.push("error-before");
            throw globalThis.__timerErrorObject;
        }, 0);
        setTimeout(() => {
            globalThis.__timerEvents.push("primitive-before");
            throw "timer primitive";
        }, 0);
        setTimeout(() => {
            globalThis.__timerEvents.push("after");
        }, 0);
        "scheduled";
        "#,
    )
    .expect("timer setup");

    for _ in 0..3 {
        assert!(
            vm.run_next_due_timer_callback_for_test(&loader)
                .await
                .expect("timer turn")
        );
    }

    assert_eq!(
        vm.eval("JSON.stringify(globalThis.__timerEvents)")
            .expect("events"),
        r#"["error-before","primitive-before","after"]"#
    );
    assert_eq!(
        vm.eval("JSON.stringify(globalThis.__timerReportedErrors)")
            .expect("reported timer errors"),
        r#"["same-error-object","timer primitive"]"#
    );
    let runtime_warnings = vm.runtime_observable_lifecycle_errors_for_testing();
    assert!(
        runtime_warnings
            .iter()
            .any(|warning| warning.contains("timer callback dispatch failed")
                && warning.contains("timer error object")),
        "runtime warnings: {:?}",
        runtime_warnings
    );
    assert!(
        runtime_warnings
            .iter()
            .any(|warning| warning.contains("timer callback dispatch failed")
                && warning.contains("timer primitive")),
        "runtime warnings: {:?}",
        runtime_warnings
    );
}

#[tokio::test]
async fn window_timer_uses_webidl_callback_function_semantics() {
    let mut vm = new_storage_test_vm("https://window-timer-webidl-callback.test/");
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    vm.eval(
        r#"
        const frame = document.createElement("iframe");
        (document.body || document.documentElement || document).appendChild(frame);
        globalThis.__windowTimerCallbackFrame = frame;
        "#,
    )
    .expect("window timer callback-realm setup");
    materialize_single_child_default_realm_for_test(&mut vm, "window timer callback-realm setup");

    vm.eval(
        r#"
        (() => {
          const child = __windowTimerCallbackFrame.contentWindow;
          globalThis.__windowTimerCallbackFacts = null;
          globalThis.__windowTimerProxyCalls = 0;
          const callback = child.Function(`
            return new Proxy(
              function(value) {
                "use strict";
                parent.__windowTimerCallbackFacts = {
                  callbackRealm:
                    globalThis === parent.__windowTimerCallbackFrame.contentWindow,
                  receiverIsTargetWindow: this === parent,
                  argumentCount: arguments.length,
                  value,
                  proxyCalls: parent.__windowTimerProxyCalls
                };
              },
              {
                apply(target, receiver, argumentsList) {
                  parent.__windowTimerProxyCalls++;
                  return Reflect.apply(target, receiver, argumentsList);
                }
              }
            );
          `)();
          setTimeout(callback, 0, "extra-argument");
          return "scheduled";
        })()
        "#,
    )
    .expect("Window timer Web IDL callback should schedule");

    vm.advance_timers_until_deadline_for_test(&loader)
        .await
        .expect("Window timer Web IDL callback should drain");
    assert_eq!(
        vm.eval("JSON.stringify(__windowTimerCallbackFacts)")
            .expect("Window timer Web IDL callback facts"),
        r#"{"callbackRealm":true,"receiverIsTargetWindow":true,"argumentCount":1,"value":"extra-argument","proxyCalls":1}"#
    );
}

#[tokio::test]
async fn request_animation_frame_callbacks_share_one_timestamp_per_batch() {
    let mut vm = new_storage_test_vm("https://example.com/");
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    vm.eval(
        r#"
        globalThis.__animationFrameTimestamps = [];
        globalThis.__animationFrameRequestTime = performance.now();
        requestAnimationFrame(timestamp => {
            globalThis.__animationFrameTimestamps.push(timestamp);
            requestAnimationFrame(nextTimestamp => {
                globalThis.__animationFrameTimestamps.push(nextTimestamp);
            });
        });
        requestAnimationFrame(timestamp => {
            globalThis.__animationFrameTimestamps.push(timestamp);
        });
        "scheduled";
        "#,
    )
    .expect("animation frame setup");

    vm.advance_timers_until_deadline_for_test(&loader)
        .await
        .expect("animation frame callbacks");

    assert_eq!(
        vm.eval(
            r#"
            JSON.stringify({
                count: globalThis.__animationFrameTimestamps.length,
                sameBatch:
                    globalThis.__animationFrameTimestamps[0] ===
                    globalThis.__animationFrameTimestamps[1],
                nextBatchIsLater:
                    globalThis.__animationFrameTimestamps[2] >
                    globalThis.__animationFrameTimestamps[0],
                usesPerformanceTimeline:
                    globalThis.__animationFrameTimestamps[0] >=
                        globalThis.__animationFrameRequestTime &&
                    globalThis.__animationFrameTimestamps[0] < performance.timeOrigin,
            })
            "#,
        )
        .expect("animation frame result"),
        r#"{"count":3,"sameBatch":true,"nextBatchIsLater":true,"usesPerformanceTimeline":true}"#
    );
}

#[tokio::test]
async fn request_animation_frame_reports_exceptions_to_the_callback_realm() {
    let mut vm = new_storage_test_vm("https://example.com/");
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    vm.eval(
        r#"
        for (let index = 0; index < 3; index++) {
            (document.body || document.documentElement || document)
                .appendChild(document.createElement("iframe"));
        }
        globalThis.__animationFrameErrorWindows = [];
        frames[0].onerror = () => __animationFrameErrorWindows.push("frame0");
        frames[1].onerror = () => __animationFrameErrorWindows.push("frame1");
        frames[2].onerror = () => __animationFrameErrorWindows.push("frame2");
        frames[0].requestAnimationFrame(
            new frames[1].Function(
                `throw new parent.frames[2].Error("animation frame error");`
            )
        );
        "scheduled";
        "#,
    )
    .expect("cross-realm animation frame setup");

    vm.advance_timers_until_deadline_for_test(&loader)
        .await
        .expect("cross-realm animation frame callback");

    assert_eq!(
        vm.eval("JSON.stringify(globalThis.__animationFrameErrorWindows)")
            .expect("reported animation frame errors"),
        r#"["frame1"]"#
    );
}

#[tokio::test]
async fn request_animation_frame_uses_webidl_callback_function_semantics() {
    let mut vm = new_storage_test_vm("https://animation-frame-webidl-callback.test/");
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    vm.eval(
        r#"
        const frame = document.createElement("iframe");
        (document.body || document.documentElement || document).appendChild(frame);
        globalThis.__animationFrameCallbackFrame = frame;
        "#,
    )
    .expect("animation frame callback-realm setup");
    materialize_single_child_default_realm_for_test(
        &mut vm,
        "animation frame callback-realm setup",
    );

    vm.eval(
        r#"
        (() => {
          const child = __animationFrameCallbackFrame.contentWindow;
          globalThis.__animationFrameCallbackFacts = null;
          globalThis.__animationFrameProxyCalls = 0;
          const callback = child.Function(`
            return new Proxy(
              function(timestamp) {
                "use strict";
                parent.__animationFrameCallbackFacts = {
                  callbackRealm:
                    globalThis === parent.__animationFrameCallbackFrame.contentWindow,
                  receiverUndefined: this === undefined,
                  argumentCount: arguments.length,
                  timestampFinite: Number.isFinite(timestamp),
                  proxyCalls: parent.__animationFrameProxyCalls
                };
              },
              {
                apply(target, receiver, argumentsList) {
                  parent.__animationFrameProxyCalls++;
                  if (receiver !== undefined)
                    throw new Error("animation frame callback receiver was not undefined");
                  return Reflect.apply(target, receiver, argumentsList);
                }
              }
            );
          `)();
          requestAnimationFrame(callback);
          return "scheduled";
        })()
        "#,
    )
    .expect("animation frame Web IDL callback should schedule");

    vm.advance_timers_until_deadline_for_test(&loader)
        .await
        .expect("animation frame Web IDL callback should drain");
    assert_eq!(
        vm.eval("JSON.stringify(__animationFrameCallbackFacts)")
            .expect("animation frame Web IDL callback facts"),
        r#"{"callbackRealm":true,"receiverUndefined":true,"argumentCount":1,"timestampFinite":true,"proxyCalls":1}"#
    );
}

#[tokio::test]
async fn request_animation_frame_retires_with_its_callback_realm() {
    let mut vm = new_storage_test_vm("https://animation-frame-callback-retirement.test/");
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    vm.eval(
        r#"
        const frame = document.createElement("iframe");
        (document.body || document.documentElement || document).appendChild(frame);
        globalThis.__retiredAnimationFrameCallbackFrame = frame;
        globalThis.__retiredAnimationFrameCallbackRan = false;
        "#,
    )
    .expect("animation frame callback retirement setup");
    materialize_single_child_default_realm_for_test(
        &mut vm,
        "animation frame callback retirement setup",
    );

    vm.eval(
        r#"
        (() => {
          const child = __retiredAnimationFrameCallbackFrame.contentWindow;
          requestAnimationFrame(child.Function(
            `parent.__retiredAnimationFrameCallbackRan = true;`
          ));
          __retiredAnimationFrameCallbackFrame.remove();
          return "scheduled";
        })()
        "#,
    )
    .expect("retired animation frame callback should schedule");

    vm.advance_timers_until_deadline_for_test(&loader)
        .await
        .expect("retired animation frame callback should leave the timer source quiescent");
    assert_eq!(
        vm.eval("String(__retiredAnimationFrameCallbackRan)")
            .expect("retired animation frame callback result"),
        "false"
    );
}

#[tokio::test]
async fn window_timer_stringifies_non_function_handler_before_queueing() {
    let mut vm = new_storage_test_vm("https://example.com/");
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    vm.eval(
        r#"
        globalThis.__timerLog = "";
        function logger(value) {
          globalThis.__timerLog += value + " ";
        }
        setTimeout({ toString() {
          setTimeout("logger('ONE')", 0);
          return "logger('TWO')";
        } }, 0);
        "scheduled";
        "#,
    )
    .expect("timer setup");

    for _ in 0..2 {
        assert!(
            vm.run_next_due_timer_callback_for_test(&loader)
                .await
                .expect("timer turn")
        );
    }

    assert_eq!(
        vm.eval("globalThis.__timerLog").expect("timer log"),
        "ONE TWO "
    );
}

#[test]
fn window_string_timers_apply_trusted_types_default_policy() {
    let mut vm = new_storage_test_vm("https://trusted-types-window-timers.test/");
    vm.set_response_content_security_policies(&["require-trusted-types-for 'script'".to_owned()]);

    vm.eval(
        r#"
        globalThis.__trustedTimerLog = [];
        globalThis.__trustedTimerPolicyCalls = [];
        trustedTypes.createPolicy("default", {
          createScript(source, type, sink) {
            __trustedTimerPolicyCalls.push([source, type, sink]);
            if (source === "timeout-token") {
              return "__trustedTimerLog.push('timeout')";
            }
            return "__trustedTimerLog.push('interval'); clearInterval(__trustedIntervalId)";
          }
        });
        setTimeout("timeout-token", 0);
        globalThis.__trustedIntervalId = setInterval("interval-token", 0);
        "scheduled";
        "#,
    )
    .expect("trusted timer setup should evaluate");

    for timer_name in ["timeout", "interval"] {
        assert!(
            matches!(
                vm.run_next_timeout_for_test()
                    .unwrap_or_else(|error| panic!("{timer_name} should run: {error}")),
                crate::host::HostTimeoutRunResult::Consumed
            ),
            "{timer_name} callback should execute successfully"
        );
    }

    assert_eq!(
        vm.eval("JSON.stringify({calls: __trustedTimerPolicyCalls, log: __trustedTimerLog})")
            .expect("trusted timer result should evaluate"),
        r#"{"calls":[["timeout-token","TrustedScript","Window setTimeout"],["interval-token","TrustedScript","Window setInterval"]],"log":["timeout","interval"]}"#
    );
}

#[test]
fn window_string_timers_apply_eval_csp_before_scheduling() {
    let mut vm = new_storage_test_vm("https://string-timer-csp.test/page.html");
    vm.set_response_content_security_policies(&["script-src 'self' 'unsafe-inline'".to_owned()]);
    vm.eval(
        r#"
        globalThis.__stringTimerRuns = [];
        globalThis.__stringTimerCspEvents = [];
        document.addEventListener("securitypolicyviolation", event => {
          __stringTimerCspEvents.push({
            blockedURI: event.blockedURI,
            effectiveDirective: event.effectiveDirective,
            disposition: event.disposition,
          });
        });
        globalThis.__stringTimerTimeout = setTimeout(
          "__stringTimerRuns.push('timeout')",
          0
        );
        globalThis.__stringTimerInterval = setInterval(
          "__stringTimerRuns.push('interval')",
          0
        );
        "scheduled";
        "#,
    )
    .expect("blocked string timer installation should not throw");

    assert!(matches!(
        vm.run_next_timeout_for_test()
            .expect("blocked string timers should leave the scheduler idle"),
        crate::host::HostTimeoutRunResult::Idle
    ));
    assert_eq!(
        drain_pre_domcontentloaded_non_script_page_tasks_for_test(&mut vm),
        2
    );
    assert_eq!(
        vm.eval(
            "JSON.stringify({timeoutId: __stringTimerTimeout, intervalId: __stringTimerInterval, runs: __stringTimerRuns, events: __stringTimerCspEvents})"
        )
            .expect("string timer CSP outcome should be observable"),
        r#"{"timeoutId":0,"intervalId":0,"runs":[],"events":[{"blockedURI":"eval","effectiveDirective":"script-src","disposition":"enforce"},{"blockedURI":"eval","effectiveDirective":"script-src","disposition":"enforce"}]}"#
    );
}

#[test]
fn window_string_timer_report_only_eval_csp_reports_and_runs() {
    let mut vm = new_storage_test_vm("https://string-timer-csp-report-only.test/page.html");
    vm.set_response_content_security_report_only_policies(&["script-src 'self'".to_owned()]);
    vm.eval(
        r#"
        globalThis.__reportOnlyStringTimerRan = false;
        globalThis.__reportOnlyStringTimerEvents = [];
        document.addEventListener("securitypolicyviolation", event => {
          __reportOnlyStringTimerEvents.push({
            blockedURI: event.blockedURI,
            effectiveDirective: event.effectiveDirective,
            disposition: event.disposition,
          });
        });
        setTimeout("__reportOnlyStringTimerRan = true", 0);
        "scheduled";
        "#,
    )
    .expect("report-only string timer should schedule");

    assert!(matches!(
        vm.run_next_timeout_for_test()
            .expect("report-only string timer should run"),
        crate::host::HostTimeoutRunResult::Consumed
    ));
    assert_eq!(
        drain_pre_domcontentloaded_non_script_page_tasks_for_test(&mut vm),
        1
    );
    assert_eq!(
        vm.eval(
            "JSON.stringify({ran: __reportOnlyStringTimerRan, events: __reportOnlyStringTimerEvents})"
        )
        .expect("report-only string timer result should be observable"),
        r#"{"ran":true,"events":[{"blockedURI":"eval","effectiveDirective":"script-src","disposition":"report"}]}"#
    );
}

#[test]
fn window_string_timer_unsafe_eval_runs_without_reporting() {
    let mut vm = new_storage_test_vm("https://string-timer-csp-allowed.test/page.html");
    vm.set_response_content_security_policies(&["script-src 'self' 'unsafe-eval'".to_owned()]);
    vm.eval(
        r#"
        globalThis.__allowedStringTimerRan = false;
        globalThis.__allowedStringTimerEvents = 0;
        document.addEventListener("securitypolicyviolation", () => {
          __allowedStringTimerEvents++;
        });
        setTimeout("__allowedStringTimerRan = true", 0);
        "scheduled";
        "#,
    )
    .expect("allowed string timer should schedule");

    assert!(matches!(
        vm.run_next_timeout_for_test()
            .expect("allowed string timer should run"),
        crate::host::HostTimeoutRunResult::Consumed
    ));
    assert_eq!(
        drain_pre_domcontentloaded_non_script_page_tasks_for_test(&mut vm),
        0
    );
    assert_eq!(
        vm.eval(
            "JSON.stringify({ran: __allowedStringTimerRan, events: __allowedStringTimerEvents})"
        )
        .expect("allowed string timer result should be observable"),
        r#"{"ran":true,"events":0}"#
    );
}

#[test]
fn window_string_timer_honors_trusted_types_eval_with_enforced_require() {
    let mut vm = new_storage_test_vm("https://string-timer-trusted-types-eval.test/page.html");
    vm.set_response_content_security_policies(&[
        "script-src 'trusted-types-eval'; require-trusted-types-for 'script'".to_owned(),
    ]);
    vm.eval(
        r#"
        globalThis.__trustedTypesEvalTimerRan = false;
        globalThis.__trustedTypesEvalTimerEvents = 0;
        trustedTypes.createPolicy("default", { createScript: source => source });
        document.addEventListener("securitypolicyviolation", () => {
          __trustedTypesEvalTimerEvents++;
        });
        setTimeout("__trustedTypesEvalTimerRan = true", 0);
        "scheduled";
        "#,
    )
    .expect("trusted-types-eval string timer should schedule");

    assert!(matches!(
        vm.run_next_timeout_for_test()
            .expect("trusted-types-eval string timer should run"),
        crate::host::HostTimeoutRunResult::Consumed
    ));
    assert_eq!(
        drain_pre_domcontentloaded_non_script_page_tasks_for_test(&mut vm),
        0
    );
    assert_eq!(
        vm.eval(
            "JSON.stringify({ran: __trustedTypesEvalTimerRan, events: __trustedTypesEvalTimerEvents})"
        )
        .expect("trusted-types-eval timer result should be observable"),
        r#"{"ran":true,"events":0}"#
    );
}

#[tokio::test]
async fn window_timer_delay_uses_webidl_long_conversion() {
    let mut vm = new_storage_test_vm("https://example.com/");
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    vm.eval(
        r#"
        globalThis.__timeoutFired = false;
        globalThis.__intervalFired = false;
        let intervalId = setInterval(() => {
          globalThis.__intervalFired = true;
          clearInterval(intervalId);
        }, Math.pow(2, 32));
        setTimeout(() => {
          globalThis.__timeoutFired = true;
        }, Math.pow(2, 32));
        "scheduled";
        "#,
    )
    .expect("timer setup");

    for _ in 0..2 {
        assert!(
            vm.run_next_due_timer_callback_for_test(&loader)
                .await
                .expect("timer turn")
        );
    }

    assert_eq!(
        vm.eval("[globalThis.__timeoutFired, globalThis.__intervalFired].join(',')")
            .expect("timer flags"),
        "true,true"
    );
}

#[tokio::test]
async fn page_timer_turn_propagates_host_driver_failure() {
    let mut vm = new_storage_test_vm("https://example.com/");
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    vm.fail_next_timeout_for_testing("host timeout driver failed for test");

    let error = match vm.run_next_due_timer_callback_for_test(&loader).await {
        Ok(_) => panic!("host timeout driver failure should propagate"),
        Err(error) => error,
    };
    assert!(
        error
            .to_string()
            .contains("host timeout driver failed for test"),
        "{error:?}"
    );
    let runtime_warnings = vm.runtime_observable_lifecycle_errors_for_testing();
    assert!(
        runtime_warnings.is_empty(),
        "runtime warnings: {runtime_warnings:?}"
    );
}

#[tokio::test]
async fn post_parse_round_injects_lifecycle_boundary_tasks_around_trailing_work() {
    let _js_runtime = crate::JsRuntime::initialize();
    let document = HtmlParser.parse(
        Url::parse("https://example.com/").unwrap(),
        "<!doctype html><html><head></head><body></body></html>".to_owned(),
    );
    let mut page_task_queue = crate::page_task_queue::PageTaskQueueTestHarness::new();
    let post_domcontentloaded_page_task_sender =
        page_task_queue.owner_attached_runtime_page_task_sender_for_test();
    let script_event_parser_boundary_sender = page_task_queue.parser_boundary_sender();
    let mut vm = ScriptVmDefaultWorldBootstrap::standalone_from_dom_host_for_test(
        DomHost::from_dom(document),
        post_domcontentloaded_page_task_sender,
        script_event_parser_boundary_sender,
    )
    .expect("script vm bootstrap")
    .finish()
    .expect("script vm finish");
    let mut report = ScriptExecutionReport::default();
    let detached_run = ScriptRun::skipped(
        NodeId::new(99),
        ScriptKind::Classic,
        ScriptMode::Async,
        ScriptSourceKind::External,
        Url::parse("https://example.com/detached.js").unwrap(),
        ScriptSkipReason::NotInMainDocument,
    );

    let _driver = vm
        .start_post_parse_lifecycle_round(
            crate::renderer::PageVmInitStage::Load,
            &mut page_task_queue,
            &mut report,
            vec![post_parse_lifecycle_work(
                PostParseLifecycleWork::RecordDetachedPostParseRuns(vec![detached_run]),
            )],
        )
        .await;

    assert!(
        page_task_queue
            .post_parse_pop_front()
            .is_some_and(|work| work.is_main_document_interactive_task())
    );
    assert!(matches!(
        pop_post_parse_page_task(&mut page_task_queue),
        Some(PageTask::DispatchDomContentLoaded)
    ));
    assert!(matches!(
        pop_post_parse_page_task(&mut page_task_queue),
        Some(PageTask::RecordDetachedPostParseRuns(_))
    ));
    assert!(matches!(
        pop_post_parse_page_task(&mut page_task_queue),
        Some(PageTask::DispatchWindowLoad)
    ));
    assert!(page_task_queue.is_empty());
}

#[test]
fn deferred_page_owned_tasks_preserve_original_lane_when_drained() {
    let mut deferred = DeferredPageTaskState::default();
    deferred.enter_scope();

    assert_eq!(
        deferred.enqueue_or_defer(
            DeferredPageTask::page_owned_work(
                post_parse_document_script_work(
                    crate::document_script_scheduler::DocumentScriptExecutionLane::AsyncPhase,
                    prepared_post_parse_script(40, ScriptMode::Async),
                ),
                DeferredPageTaskLane::PreDomContentLoaded,
            )
            .expect("pre-DCL page-owned work should be deferrable"),
            |_| unreachable!("deferred task should not enqueue immediately"),
        ),
        FollowupPageTaskDisposition::Deferred
    );
    assert_eq!(
        deferred.enqueue_or_defer(
            DeferredPageTask::page_owned_work(
                post_parse_document_script_work(
                    crate::document_script_scheduler::DocumentScriptExecutionLane::AsyncPhase,
                    prepared_post_parse_script(41, ScriptMode::Async),
                ),
                DeferredPageTaskLane::PostDomContentLoaded,
            )
            .expect("post-DCL page-owned work should be deferrable"),
            |_| unreachable!("deferred task should not enqueue immediately"),
        ),
        FollowupPageTaskDisposition::Deferred
    );

    deferred.exit_scope();

    let mut drained = Vec::new();
    deferred.drain_into(|task| drained.push(task));

    assert_eq!(drained.len(), 2);
    assert_eq!(drained[0].lane, DeferredPageTaskLane::PreDomContentLoaded);
    assert!(is_document_script_execution_work(
        drained[0].page_owned_work_ref(),
        crate::document_script_scheduler::DocumentScriptExecutionLane::AsyncPhase,
        40,
    ));
    assert_eq!(drained[1].lane, DeferredPageTaskLane::PostDomContentLoaded);
    assert!(is_document_script_execution_work(
        drained[1].page_owned_work_ref(),
        crate::document_script_scheduler::DocumentScriptExecutionLane::AsyncPhase,
        41,
    ));
}

#[test]
fn deferred_pre_domcontentloaded_lane_stores_explicit_page_owned_work() {
    let mut vm = new_storage_test_vm("https://example.com/");

    vm.document_runtime.deferred_page_tasks_mut().enter_scope();
    assert_eq!(
        vm.enqueue_page_owned_work_or_defer(
            post_parse_document_script_work(
                crate::document_script_scheduler::DocumentScriptExecutionLane::AsyncPhase,
                prepared_post_parse_script(42, ScriptMode::Async),
            ),
            DeferredPageTaskLane::PreDomContentLoaded,
        )
        .expect("pre-DCL page task should defer"),
        FollowupPageTaskDisposition::Deferred
    );

    let mut drained = Vec::new();
    vm.document_runtime
        .deferred_page_tasks_mut()
        .drain_into(|deferred| drained.push(deferred));

    assert_eq!(drained.len(), 1);
    assert_eq!(drained[0].lane, DeferredPageTaskLane::PreDomContentLoaded);
    assert!(
        is_document_script_execution_work(
            drained[0].page_owned_work_ref(),
            crate::document_script_scheduler::DocumentScriptExecutionLane::AsyncPhase,
            42,
        ),
        "pre-DCL deferred tasks should be projected before entering the deferred queue"
    );
}

#[test]
fn deferred_post_domcontentloaded_script_lane_stores_explicit_page_owned_work() {
    let mut vm = new_storage_test_vm("https://example.com/");

    vm.document_runtime.deferred_page_tasks_mut().enter_scope();
    assert_eq!(
        vm.enqueue_page_owned_work_or_defer(
            post_parse_document_script_work(
                crate::document_script_scheduler::DocumentScriptExecutionLane::AsyncPhase,
                prepared_post_parse_script(43, ScriptMode::Async),
            ),
            DeferredPageTaskLane::PostDomContentLoaded,
        )
        .expect("post-DCL script page task should defer"),
        FollowupPageTaskDisposition::Deferred
    );

    let mut drained = Vec::new();
    vm.document_runtime
        .deferred_page_tasks_mut()
        .drain_into(|deferred| drained.push(deferred));

    assert_eq!(drained.len(), 1);
    assert_eq!(drained[0].lane, DeferredPageTaskLane::PostDomContentLoaded);
    assert!(
        is_document_script_execution_work(
            drained[0].page_owned_work_ref(),
            crate::document_script_scheduler::DocumentScriptExecutionLane::AsyncPhase,
            43,
        ),
        "post-DCL deferred script tasks should not rely on the page-task sender bridge"
    );
}

#[test]
fn document_open_synchronously_clears_document_owned_runtime_script_work() {
    let mut vm = new_storage_test_vm("https://example.com/");

    vm.document_runtime
        .runtime_script_work_mut()
        .dynamic_scripts
        .requeue_ready_script_front(
            dynamic_script_owner_id(1),
            ready_dynamic_runtime_script(1),
            None,
        );
    vm.document_runtime
        .runtime_script_work_mut()
        .pause_for_deferred_page_tasks(RuntimeScriptWorkPauseKind::StablePageTurnContinuation);

    vm.document_runtime.deferred_page_tasks_mut().enter_scope();
    assert_eq!(
        vm.document_runtime
            .deferred_page_tasks_mut()
            .enqueue_or_defer(
                DeferredPageTask::page_owned_work(
                    post_parse_document_script_work(
                        crate::document_script_scheduler::DocumentScriptExecutionLane::AsyncPhase,
                        prepared_post_parse_script(44, ScriptMode::Async),
                    ),
                    DeferredPageTaskLane::PostDomContentLoaded,
                )
                .expect("post-DCL work should be deferrable"),
                |_| unreachable!("task should be deferred before document replacement"),
            ),
        FollowupPageTaskDisposition::Deferred
    );
    assert!(
        vm.document_runtime
            .runtime_script_work_mut()
            .has_pending_work()
    );

    vm.document_runtime.open_document();

    assert!(
        !vm.document_runtime
            .runtime_script_work_mut()
            .has_pending_work()
    );
    assert!(
        !vm.document_runtime
            .runtime_script_work_mut()
            .is_paused_for_deferred_page_tasks()
    );
    let mut drained = Vec::new();
    vm.document_runtime
        .deferred_page_tasks_mut()
        .drain_into(|deferred| drained.push(deferred));
    assert!(drained.is_empty());
}

#[tokio::test]
async fn document_replacement_round_restart_does_not_duplicate_live_parser_owned_script() {
    let mut vm = new_storage_test_vm("https://replacement-restart.test/page.html");
    vm.eval(
        r#"
document.open();
document.write(`<!doctype html><script id="replacement-script">globalThis.__replacementRestartRuns = (globalThis.__replacementRestartRuns || 0) + 1;</script>`);
document.close();
"#,
    )
    .expect("document replacement should parse");
    assert_eq!(
        vm.eval("String(globalThis.__replacementRestartRuns)")
            .expect("replacement script count should be readable"),
        "1",
        "the live replacement parser executes an inline classic script synchronously"
    );
    let mut page_task_queue = crate::page_task_queue::PageTaskQueueTestHarness::new();
    let mut report = ScriptExecutionReport::default();

    assert!(
        vm.restart_post_parse_lifecycle_round_if_invalidated(&mut page_task_queue, &mut report,)
            .await,
        "replacement should restart the post-parse owner round"
    );
    assert!(
        vm.document_runtime
            .pop_parser_owned_pre_domcontentloaded_action()
            .is_none()
    );
    assert_eq!(
        vm.eval("String(globalThis.__replacementRestartRuns)")
            .expect("replacement script count should remain readable"),
        "1",
        "restarting the owner round must not replay a script already consumed by the live parser"
    );
}

#[tokio::test]
async fn completed_load_restarts_replacement_before_runtime_followup_publication() {
    let mut vm = new_storage_test_vm("https://replacement-load-restart.test/page.html");
    vm.eval(
        r#"
document.open();
document.write(`<!doctype html><script id="replacement-script">globalThis.__replacementLoadRestartRuns = (globalThis.__replacementLoadRestartRuns || 0) + 1;</script>`);
document.close();
"#,
    )
    .expect("post-load document replacement should parse");
    assert_eq!(
        vm.eval("String(globalThis.__replacementLoadRestartRuns)")
            .expect("replacement script count should be readable"),
        "1"
    );
    let mut page_task_queue = crate::page_task_queue::PageTaskQueueTestHarness::new();
    let mut report = ScriptExecutionReport::default();
    let completed_load = PostParsePageOwnedTask {
        work: None,
        completion: PostParseTaskCompletion {
            token: PostParseTaskExecutionToken {
                boundary_completion: Some(PostParseLifecycleCompletionAction::ReturnAtStage(
                    "load",
                )),
                invalidation_policy: PostParseTaskInvalidationPolicy::RestartIfInvalidated,
                requires_runtime_followup_publication: true,
            },
        },
    };

    let advance = vm
        .finish_completed_post_parse_page_owned_task_or_continue(
            &mut page_task_queue,
            &mut report,
            Some(completed_load),
        )
        .await
        .expect("replacement completion should restart the owner round");

    assert!(
        advance.is_none(),
        "the retired document's load boundary must not complete the replacement round"
    );
    assert!(
        vm.document_runtime
            .pop_parser_owned_pre_domcontentloaded_action()
            .is_none()
    );
    assert_eq!(
        vm.eval("String(globalThis.__replacementLoadRestartRuns)")
            .expect("replacement script count should remain readable"),
        "1",
        "a retired load completion must not replay replacement parser work"
    );
}

#[test]
fn followup_lanes_track_script_semantics() {
    assert_eq!(
        crate::host::HostScriptScheduler::followup_lane_for_script(
            ScriptHandleSource::Unknown,
            ScriptMode::Normal
        ),
        DeferredPageTaskLane::PreDomContentLoaded
    );
    assert_eq!(
        crate::host::HostScriptScheduler::followup_lane_for_script(
            ScriptHandleSource::ParserOwned,
            ScriptMode::Normal
        ),
        DeferredPageTaskLane::ParserBoundary
    );
    assert_eq!(
        crate::host::HostScriptScheduler::followup_lane_for_script(
            ScriptHandleSource::DocumentWriteOwned,
            ScriptMode::Normal,
        ),
        DeferredPageTaskLane::PreDomContentLoaded
    );
    assert_eq!(
        crate::host::HostScriptScheduler::followup_lane_for_script(
            ScriptHandleSource::ParserOwned,
            ScriptMode::Defer
        ),
        DeferredPageTaskLane::PreDomContentLoaded
    );
    assert_eq!(
        crate::host::HostScriptScheduler::followup_lane_for_script(
            ScriptHandleSource::Unknown,
            ScriptMode::Defer
        ),
        DeferredPageTaskLane::PreDomContentLoaded
    );
    assert_eq!(
        crate::host::HostScriptScheduler::followup_lane_for_script(
            ScriptHandleSource::DocumentWriteOwned,
            ScriptMode::ModuleDefer,
        ),
        DeferredPageTaskLane::PreDomContentLoaded
    );
    assert_eq!(
        crate::host::HostScriptScheduler::followup_lane_for_script(
            ScriptHandleSource::DocumentWriteOwned,
            ScriptMode::InOrder,
        ),
        DeferredPageTaskLane::PreDomContentLoaded
    );
    assert_eq!(
        crate::host::HostScriptScheduler::followup_lane_for_script(
            ScriptHandleSource::ParserOwned,
            ScriptMode::Async
        ),
        DeferredPageTaskLane::PostDomContentLoaded
    );
    assert_eq!(
        crate::host::HostScriptScheduler::followup_lane_for_script(
            ScriptHandleSource::DocumentWriteOwned,
            ScriptMode::ModuleInOrder,
        ),
        DeferredPageTaskLane::PreDomContentLoaded
    );
    assert_eq!(
        crate::host::HostScriptScheduler::followup_lane_for_script(
            ScriptHandleSource::DocumentWriteOwned,
            ScriptMode::ImportMapInOrder,
        ),
        DeferredPageTaskLane::PreDomContentLoaded
    );
    assert_eq!(
        crate::host::HostScriptScheduler::followup_lane_for_script(
            ScriptHandleSource::RuntimeOwned,
            ScriptMode::Normal
        ),
        DeferredPageTaskLane::PostDomContentLoaded
    );
    assert_eq!(
        crate::host::HostScriptScheduler::followup_lane_for_script(
            ScriptHandleSource::RuntimeOwned,
            ScriptMode::Defer
        ),
        DeferredPageTaskLane::PostDomContentLoaded
    );
    assert_eq!(
        crate::host::HostScriptScheduler::followup_lane_for_script(
            ScriptHandleSource::RuntimeOwned,
            ScriptMode::Async
        ),
        DeferredPageTaskLane::PostDomContentLoaded
    );
}

#[test]
fn storage_set_item_stringifies_null_key() {
    let mut vm = new_storage_test_vm("https://storage-null-key.test/");

    let result = vm
            .eval(
                "(() => { localStorage.clear(); localStorage.setItem(null, 'value'); return `${localStorage.getItem('null')}|${localStorage.length}`; })()",
            )
            .expect("storage script should execute");

    assert_eq!(result, "value|1");
}

#[test]
fn storage_get_item_stringifies_undefined_key() {
    let mut vm = new_storage_test_vm("https://storage-undefined-key.test/");

    let result = vm
            .eval(
                "(() => { localStorage.clear(); localStorage.setItem('undefined', 'value'); return String(localStorage.getItem(undefined)); })()",
            )
            .expect("storage script should execute");

    assert_eq!(result, "value");
}

#[test]
fn storage_remove_item_stringifies_null_key() {
    let mut vm = new_storage_test_vm("https://storage-remove-null-key.test/");

    let result = vm
            .eval(
                "(() => { localStorage.clear(); localStorage.setItem('null', 'value'); localStorage.removeItem(null); return String(localStorage.length); })()",
            )
            .expect("storage script should execute");

    assert_eq!(result, "0");
}

#[test]
fn storage_methods_parse_webidl_arguments() {
    let mut vm = new_storage_test_vm("https://storage-webidl-args.test/");

    let result = vm
        .eval(
            r#"
(() => {
  localStorage.clear();
  function probe(callback) {
    try {
      return callback();
    } catch (error) {
      return 'throw:' + error.name;
    }
  }
  localStorage.setItem(null, undefined);
  return [
    localStorage.getItem('null'),
    localStorage.key(0),
    localStorage.key(-1),
    probe(() => localStorage.getItem(Symbol())),
    probe(() => localStorage.setItem('symbol', Symbol())),
    probe(() => localStorage.removeItem(Symbol())),
    localStorage.length,
    localStorage.getItem('symbol')
  ].join('|');
})()
"#,
        )
        .expect("storage WebIDL argument conversion should execute");

    assert_eq!(
        result,
        "undefined|null||throw:TypeError|throw:TypeError|throw:TypeError|1|"
    );
}

#[test]
fn storage_prototype_methods_are_declared_operations() {
    let mut vm = new_storage_test_vm("https://storage-prototype-methods.test/");

    let result = vm
        .eval(
            r#"
(() => {
  localStorage.clear();
  sessionStorage.clear();
  const names = ["getItem", "setItem", "removeItem", "clear", "key"];
  const descriptors = names.map(name => {
    const descriptor = Object.getOwnPropertyDescriptor(Storage.prototype, name);
    return [
      name,
      typeof descriptor.value,
      descriptor.value.name,
      descriptor.value.length,
      descriptor.enumerable,
      descriptor.writable,
      descriptor.configurable,
      Object.prototype.hasOwnProperty.call(localStorage, name),
      Object.prototype.hasOwnProperty.call(sessionStorage, name)
    ].join(":");
  });
  const lengthDescriptor = Object.getOwnPropertyDescriptor(Storage.prototype, "length");
  const lengthShape = [
    "length",
    typeof lengthDescriptor.get,
    lengthDescriptor.get.name,
    lengthDescriptor.get.length,
    typeof lengthDescriptor.set,
    lengthDescriptor.enumerable,
    lengthDescriptor.configurable,
    Object.prototype.hasOwnProperty.call(localStorage, "length"),
    Object.prototype.hasOwnProperty.call(sessionStorage, "length")
  ].join(":");
  localStorage.setItem("alpha", "one");
  sessionStorage.setItem("beta", "two");
  const behavior = [
    localStorage.getItem("alpha"),
    sessionStorage.getItem("beta"),
    localStorage.key(0),
    sessionStorage.key(0),
    localStorage.removeItem("alpha"),
    String(localStorage.getItem("alpha")),
    sessionStorage.clear(),
    sessionStorage.length
  ].join(":");
  return descriptors.concat(lengthShape).join("|") + "|" + behavior + "|" + Object.keys(Storage.prototype).join(",");
})()
"#,
        )
        .expect("storage prototype method descriptor test should execute");

    assert_eq!(
        result,
        "getItem:function:getItem:1:true:true:true:false:false|setItem:function:setItem:2:true:true:true:false:false|removeItem:function:removeItem:1:true:true:true:false:false|clear:function:clear:0:true:true:true:false:false|key:function:key:1:true:true:true:false:false|length:function:get length:0:undefined:true:true:false:false|one:two:alpha:beta::null::0|getItem,setItem,removeItem,clear,key,length"
    );
}

#[test]
fn storage_window_object_cache_ignores_reflection_and_spoofing() {
    let mut vm = new_storage_test_vm("https://storage-window-cache-private-slots.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const slots = [
    "__moliWindowLocalStorage",
    "__moliWindowSessionStorage"
  ];
  const internalNames = () => Object.getOwnPropertyNames(globalThis)
    .filter(name => slots.includes(name))
    .sort()
    .join(",");
  const beforeAccess = internalNames();
  const firstLocal = localStorage;
  const firstSession = sessionStorage;
  firstLocal.clear();
  firstSession.clear();
  firstLocal.setItem("cache", "local");
  firstSession.setItem("cache", "session");
  const afterAccess = internalNames();
  Object.defineProperty(globalThis, "__moliWindowLocalStorage", {
    value: { spoof: "local" },
    configurable: true,
    writable: true
  });
  Object.defineProperty(globalThis, "__moliWindowSessionStorage", {
    value: { spoof: "session" },
    configurable: true,
    writable: true
  });
  try {
    const afterSpoof = internalNames();
    const secondLocal = localStorage;
    const secondSession = sessionStorage;
    return JSON.stringify({
      beforeAccess,
      afterAccess,
      afterSpoof,
      publicSpoof: [
        globalThis.__moliWindowLocalStorage.spoof,
        globalThis.__moliWindowSessionStorage.spoof
      ].join(","),
      sameObjects: [firstLocal === secondLocal, firstSession === secondSession].join(","),
      values: [secondLocal.getItem("cache"), secondSession.getItem("cache")].join(",")
    });
  } finally {
    delete globalThis.__moliWindowLocalStorage;
    delete globalThis.__moliWindowSessionStorage;
    firstLocal.clear();
    firstSession.clear();
  }
})()
"#,
        )
        .expect("storage window cache private slot test should execute");

    assert_eq!(
        result,
        r#"{"beforeAccess":"","afterAccess":"","afterSpoof":"__moliWindowLocalStorage,__moliWindowSessionStorage","publicSpoof":"local,session","sameObjects":"true,true","values":"local,session"}"#
    );
}

#[test]
fn storage_symbol_interceptor_values_throw_without_mutating_store() {
    let mut vm = new_storage_test_vm("https://storage-symbol-value.test/");

    let result = vm
        .eval(
            r#"
(() => {
  localStorage.clear();
  let setThrew = false;
  let defineThrew = false;
  try {
    localStorage.symbolValue = Symbol("value");
  } catch (error) {
    setThrew = error instanceof TypeError;
  }
  try {
    Object.defineProperty(localStorage, "definedSymbol", { value: Symbol("value") });
  } catch (error) {
    defineThrew = error instanceof TypeError;
  }
  return JSON.stringify({
    setThrew,
    defineThrew,
    length: localStorage.length,
    symbolValue: localStorage.getItem("symbolValue"),
    definedSymbol: localStorage.getItem("definedSymbol")
  });
})()
"#,
        )
        .expect("storage Symbol conversion test should execute");

    assert_eq!(
        result,
        r#"{"setThrew":true,"defineThrew":true,"length":0,"symbolValue":null,"definedSymbol":null}"#
    );
}

#[test]
fn storage_property_access_respects_prototype_shadowing_for_indexed_keys() {
    let mut vm = new_storage_test_vm("https://storage-prototype-index-shadow.test/");

    let result = vm
        .eval(
            r#"
(() => {
  function probeWptShape(name, key) {
    const proto = "proto";
    Storage.prototype[key] = proto;
    try {
      const storage = window[name];
      storage.clear();
      const before = storage[key];
      const assignment = (storage[key] = "next");
      const after = storage[key];
      const item = storage.getItem(String(key));
      const own = Object.getOwnPropertyDescriptor(storage, key);
      return [name, String(key), before, assignment, after, item, String(own === undefined)].join(":");
    } finally {
      delete Storage.prototype[key];
      window[name].clear();
    }
  }
  function probeValue(key, proto) {
    Storage.prototype[key] = proto;
    try {
      return probe(key);
    } finally {
      delete Storage.prototype[key];
      localStorage.clear();
    }
  }
  function probeAccessor(key, descriptor) {
    Object.defineProperty(Storage.prototype, key, {
      ...descriptor,
      configurable: true
    });
    try {
      return probe(key);
    } finally {
      delete Storage.prototype[key];
      localStorage.clear();
    }
  }
  function probe(key) {
    localStorage.clear();
    localStorage.setItem(String(key), "existing");
    const before = localStorage[key];
    const assignment = (localStorage[key] = "next");
    const after = localStorage[key];
    const item = localStorage.getItem(String(key));
    const own = Object.getOwnPropertyDescriptor(localStorage, key);
    return [String(key), before, assignment, after, item, String(own === undefined)].join(":");
  }
  return [
    probeWptShape("localStorage", 9),
    probeWptShape("sessionStorage", 9),
    probeValue(9, "proto-9"),
    probeValue("x", "proto-x"),
    probeAccessor(10, {
      get() { return "getter-10"; },
      set() { throw new Error("prototype setter should not run"); }
    })
  ].join("|");
})()
"#,
        )
        .expect("storage prototype shadowing test should execute");

    assert_eq!(
        result,
        "localStorage:9:proto:next:proto:next:true|sessionStorage:9:proto:next:proto:next:true|9:proto-9:next:proto-9:next:true|x:proto-x:next:proto-x:next:true|10:getter-10:next:getter-10:next:true"
    );
}

#[test]
fn storage_declared_descriptor_objects_preserve_reflection_shape() {
    let mut vm = new_storage_test_vm("https://storage-descriptor-shape.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const cacheSlot = "__moliStoragePrototypeIndexedDescriptors";
  const cacheSlotNames = () => Object.getOwnPropertyNames(globalThis)
    .filter(name => name === cacheSlot)
    .join(",");
  localStorage.clear();
  localStorage.setItem("alpha", "stored");
  const initialGlobalNames = cacheSlotNames();
  const valueDescriptor = Object.getOwnPropertyDescriptor(localStorage, "alpha");
  Object.defineProperty(Storage.prototype, 12, {
    get() { return "getter-12"; },
    set(value) { this.setItem("setter-12", value); },
    enumerable: false,
    configurable: true
  });
  try {
    const afterCacheGlobalNames = cacheSlotNames();
    const accessorDescriptor = Object.getOwnPropertyDescriptor(Storage.prototype, 12);
    Object.defineProperty(globalThis, cacheSlot, {
      value: { 12: { value: "spoofed" } },
      configurable: true,
      writable: true
    });
    const afterSpoofGlobalNames = cacheSlotNames();
    const spoofedDescriptor = Object.getOwnPropertyDescriptor(Storage.prototype, 12);
    return JSON.stringify({
      initialGlobalNames,
      afterCacheGlobalNames,
      afterSpoofGlobalNames,
      valueDescriptorKeys: Object.keys(valueDescriptor),
      value: valueDescriptor.value,
      writable: valueDescriptor.writable,
      enumerable: valueDescriptor.enumerable,
      configurable: valueDescriptor.configurable,
      accessorDescriptorKeys: Object.keys(accessorDescriptor),
      getType: typeof accessorDescriptor.get,
      setType: typeof accessorDescriptor.set,
      accessorEnumerable: accessorDescriptor.enumerable,
      accessorConfigurable: accessorDescriptor.configurable,
      accessorHasValue: Object.prototype.hasOwnProperty.call(accessorDescriptor, "value"),
      accessorHasWritable: Object.prototype.hasOwnProperty.call(accessorDescriptor, "writable"),
      spoofedGetType: typeof spoofedDescriptor.get,
      spoofedSetType: typeof spoofedDescriptor.set,
      spoofedHasValue: Object.prototype.hasOwnProperty.call(spoofedDescriptor, "value")
    });
  } finally {
    delete Storage.prototype[12];
    delete globalThis[cacheSlot];
    localStorage.clear();
  }
})()
"#,
        )
        .expect("storage descriptor reflection test should execute");

    assert_eq!(
        result,
        r#"{"initialGlobalNames":"","afterCacheGlobalNames":"","afterSpoofGlobalNames":"__moliStoragePrototypeIndexedDescriptors","valueDescriptorKeys":["value","writable","enumerable","configurable"],"value":"stored","writable":true,"enumerable":true,"configurable":true,"accessorDescriptorKeys":["get","set","enumerable","configurable"],"getType":"function","setType":"function","accessorEnumerable":false,"accessorConfigurable":true,"accessorHasValue":false,"accessorHasWritable":false,"spoofedGetType":"function","spoofedSetType":"function","spoofedHasValue":false}"#
    );
}

#[test]
fn storage_named_query_respects_prototype_shadowing() {
    let mut vm = new_storage_test_vm("https://storage-named-query-shadow.test/");

    let result = vm
        .eval(
            r#"
(() => {
  function probe(name) {
    const storage = window[name];
    storage.clear();
    storage.setItem("getItem", "stored");
    const descriptor = Object.getOwnPropertyDescriptor(storage, "getItem");
    const hasOwn = Object.prototype.hasOwnProperty.call(storage, "getItem");
    const value = Storage.prototype.getItem.call(storage, "getItem");
    return [
      name,
      typeof storage.getItem,
      value,
      String(descriptor === undefined),
      String(hasOwn)
    ].join(":");
  }
  return [probe("localStorage"), probe("sessionStorage")].join("|");
})()
"#,
        )
        .expect("storage named query shadowing test should execute");

    assert_eq!(
        result,
        "localStorage:function:stored:true:false|sessionStorage:function:stored:true:false"
    );
}

#[test]
fn form_data_symbol_filename_throws_without_mutating_entries() {
    let mut vm = new_storage_test_vm("https://formdata-symbol-filename.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const formData = new FormData();
  let appendThrew = false;
  let setThrew = false;
  try {
    formData.append("file", new Blob(["x"]), Symbol("name"));
  } catch (error) {
    appendThrew = error instanceof TypeError;
  }
  formData.append("keep", "value");
  try {
    formData.set("keep", new Blob(["y"]), Symbol("name"));
  } catch (error) {
    setThrew = error instanceof TypeError;
  }
  return JSON.stringify({
    appendThrew,
    setThrew,
    keys: Array.from(formData.keys()),
    keep: formData.get("keep")
  });
})()
"#,
        )
        .expect("FormData Symbol filename conversion test should execute");

    assert_eq!(
        result,
        r#"{"appendThrew":true,"setThrew":true,"keys":["keep"],"keep":"value"}"#
    );
}

#[test]
fn form_data_constructor_parses_webidl_arguments() {
    let mut vm = new_storage_test_vm("https://formdata-constructor-webidl.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const probe = callback => {
    try {
      const value = callback();
      return value === undefined ? 'undefined' : String(value);
    } catch (error) {
      return `throw:${error && error.name}`;
    }
  };
  const form = document.createElement('form');
  const input = document.createElement('input');
  input.name = 'field';
  input.value = 'value';
  form.appendChild(input);
  const submit = document.createElement('button');
  submit.type = 'submit';
  submit.name = 'go';
  submit.value = 'yes';
  form.appendChild(submit);

  const empty = Array.from(new FormData(undefined)).length;
  const fromForm = Array.from(new FormData(form, null)).join(',');
  const withSubmitter = Array.from(new FormData(form, submit)).join(',');

  return [
    empty,
    fromForm,
    withSubmitter,
    probe(() => new FormData(null)),
    probe(() => new FormData(Symbol('form'))),
    probe(() => new FormData(form, Symbol('submitter')))
  ].join('|');
})()
            "#,
        )
        .expect("FormData constructor WebIDL probe should run");

    assert_eq!(
        result,
        "0|field,value|field,value,go,yes|throw:TypeError|throw:TypeError|throw:TypeError"
    );
}

#[test]
fn form_data_image_submitter_entries_follow_tree_order() {
    let mut vm = new_storage_test_vm("https://formdata-image-submitter-order.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const form = document.createElement('form');
  form.innerHTML =
    '<input name="n1" value="v1">' +
    '<input type="image" name="image">' +
    '<input name="n3" value="v3">';
  const image = form.querySelector('[type="image"]');
  return JSON.stringify({
    listedControls: form.elements.length,
    entries: Array.from(new FormData(form, image)),
  });
})()
            "#,
        )
        .expect("image submitter FormData entries should serialize");

    assert_eq!(
        result,
        r#"{"listedControls":2,"entries":[["n1","v1"],["image.x","0"],["image.y","0"],["n3","v3"]]}"#
    );
}

#[test]
fn form_data_entries_ignore_slot_property_spoofing() {
    let mut vm = new_storage_test_vm("https://formdata-prototype-slot-spoof.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const formData = new FormData();
  formData.append("real", "value");
  const ownSlotsBefore = Object.getOwnPropertyNames(formData)
    .filter(name => name.startsWith("__moliFormData"))
    .sort();
  Object.defineProperty(FormData.prototype, "__moliFormDataEntries", {
    value: [["proto-spoof", "bad"]],
    configurable: true
  });
  Object.defineProperty(formData, "__moliFormDataEntries", {
    value: [["own-spoof", "bad"]],
    configurable: true
  });
  const ownSpoofWasWritten = Object.prototype.hasOwnProperty.call(
    formData,
    "__moliFormDataEntries"
  );
  const beforeDelete = Array.from(formData);
  const deletedOwnSpoof = delete formData.__moliFormDataEntries;
  const afterDelete = Array.from(formData);
  formData.append("second", "value2");
  return JSON.stringify({
    ownSlotsBefore,
    ownSpoofWasWritten,
    beforeDelete,
    deletedOwnSpoof,
    afterDelete,
    afterAppend: Array.from(formData),
    protoSpoof: formData.get("proto-spoof"),
    ownSpoof: formData.get("own-spoof")
  });
})()
            "#,
        )
        .expect("FormData slot spoofing probe should run");

    assert_eq!(
        result,
        r#"{"ownSlotsBefore":[],"ownSpoofWasWritten":true,"beforeDelete":[["real","value"]],"deletedOwnSpoof":true,"afterDelete":[["real","value"]],"afterAppend":[["real","value"],["second","value2"]],"protoSpoof":null,"ownSpoof":null}"#
    );
}

#[test]
fn form_data_reads_parsed_control_state() {
    let mut vm = new_storage_test_vm("https://formdata-parsed-controls.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const form = document.createElement('form');
  form.innerHTML =
    '<input name="foo" value="bara">' +
    '<textarea name="area">bar</textarea>' +
    '<input name="nokeygen" value="barb"><keygen>';
  return Array.from(new FormData(form))
    .map(([name, value]) => `${name}=${value}`)
    .join('&');
})()
            "#,
        )
        .expect("parsed FormData values should serialize");

    assert_eq!(result, "foo=bara&area=bar&nokeygen=barb");
}

#[test]
fn local_storage_reuses_explicit_partition_handles_for_same_origin_pages() {
    let storage = crate::RendererWebStorageHandles::ephemeral();
    let mut first = new_storage_test_vm("https://storage-share.test/a");
    let mut second = new_storage_test_vm("https://storage-share.test/b");
    first.set_web_storage_handles(&storage);
    second.set_web_storage_handles(&storage);

    first
        .eval("localStorage.clear(); localStorage.setItem('k', 'v');")
        .expect("first page should write localStorage");
    let result = second
        .eval("localStorage.getItem('k')")
        .expect("second page should read localStorage");

    assert_eq!(result, "v");
}

#[test]
fn local_storage_does_not_cross_partition_handle_boundaries_for_same_origin_pages() {
    let mut first = new_storage_test_vm("https://storage-isolated.test/a");
    let mut second = new_storage_test_vm("https://storage-isolated.test/b");
    first.set_web_storage_handles(&crate::RendererWebStorageHandles::ephemeral());
    second.set_web_storage_handles(&crate::RendererWebStorageHandles::ephemeral());

    first
        .eval("localStorage.clear(); localStorage.setItem('k', 'v');")
        .expect("first page should write localStorage");
    let result = second
        .eval("String(localStorage.getItem('k'))")
        .expect("second page should read localStorage");

    assert_eq!(result, "null");
}

#[test]
fn session_storage_reuses_explicit_browsing_context_handles_for_same_origin_pages() {
    let storage = crate::RendererWebStorageHandles::ephemeral();
    let mut first = new_storage_test_vm("https://session-share.test/a");
    let mut second = new_storage_test_vm("https://session-share.test/b");
    first.set_web_storage_handles(&storage);
    second.set_web_storage_handles(&storage);

    first
        .eval("sessionStorage.clear(); sessionStorage.setItem('k', 'v');")
        .expect("first page should write sessionStorage");
    let result = second
        .eval("sessionStorage.getItem('k')")
        .expect("second page should read sessionStorage");

    assert_eq!(result, "v");
}

#[test]
fn session_storage_does_not_cross_browsing_context_handle_boundaries() {
    let mut first = new_storage_test_vm("https://session-isolated.test/a");
    let mut second = new_storage_test_vm("https://session-isolated.test/b");
    first.set_web_storage_handles(&crate::RendererWebStorageHandles::ephemeral());
    second.set_web_storage_handles(&crate::RendererWebStorageHandles::ephemeral());

    first
        .eval("sessionStorage.clear(); sessionStorage.setItem('k', 'v');")
        .expect("first page should write sessionStorage");
    let result = second
        .eval("String(sessionStorage.getItem('k'))")
        .expect("second page should read sessionStorage");

    assert_eq!(result, "null");
}

#[tokio::test]
async fn poll_next_post_parse_driver_step_requests_wait_when_owner_progress_is_pending_but_not_ready()
 {
    let _js_runtime = crate::JsRuntime::initialize();
    let document = HtmlParser.parse(
            Url::parse("https://example.com/").unwrap(),
            "<!doctype html><html><head><link rel=stylesheet href='/app.css'></head><body></body></html>"
                .to_owned(),
        );
    let mut page_task_queue = crate::page_task_queue::PageTaskQueueTestHarness::new();
    let post_domcontentloaded_page_task_sender =
        page_task_queue.owner_attached_runtime_page_task_sender_for_test();
    let script_event_parser_boundary_sender = page_task_queue.parser_boundary_sender();
    let mut vm = ScriptVmDefaultWorldBootstrap::standalone_from_dom_host_for_test(
        DomHost::from_dom(document),
        post_domcontentloaded_page_task_sender,
        script_event_parser_boundary_sender,
    )
    .expect("script vm bootstrap")
    .finish()
    .expect("script vm finish");
    let snapshot = vm.document_runtime.snapshot_document();
    let head = snapshot.document_head_handle().expect("head handle");
    let link = snapshot
        .child_nodes(head)
        .expect("head children")
        .into_iter()
        .find(|handle| {
            snapshot
                .node(*handle)
                .and_then(Node::as_element)
                .is_some_and(|element| element.is_html_element("link"))
        })
        .expect("stylesheet link");
    vm.document_runtime
        .enqueue_pending_connected_style_load_for_test(link);
    vm.document_runtime.note_dom_content_loaded_dispatched();
    page_task_queue.extend_post_parse_work([post_parse_lifecycle_work(
        PostParseLifecycleWork::test_window_load(),
    )]);

    let step = vm.poll_next_post_parse_driver_step(&mut page_task_queue);

    assert!(matches!(step, PostParseDriverStep::AwaitProgress));
}

#[tokio::test]
async fn poll_next_post_parse_driver_step_prefers_owner_action_over_runtime_continuation() {
    let _js_runtime = crate::JsRuntime::initialize();
    let document = HtmlParser.parse(
            Url::parse("https://example.com/").unwrap(),
            "<!doctype html><html><head><link rel=stylesheet href='/app.css'></head><body></body></html>"
                .to_owned(),
        );
    let mut page_task_queue = crate::page_task_queue::PageTaskQueueTestHarness::new();
    let post_domcontentloaded_page_task_sender =
        page_task_queue.owner_attached_runtime_page_task_sender_for_test();
    let script_event_parser_boundary_sender = page_task_queue.parser_boundary_sender();
    let mut vm = ScriptVmDefaultWorldBootstrap::standalone_from_dom_host_for_test(
        DomHost::from_dom(document),
        post_domcontentloaded_page_task_sender,
        script_event_parser_boundary_sender,
    )
    .expect("script vm bootstrap")
    .finish()
    .expect("script vm finish");
    let snapshot = vm.document_runtime.snapshot_document();
    let head = snapshot.document_head_handle().expect("head handle");
    let link = snapshot
        .child_nodes(head)
        .expect("head children")
        .into_iter()
        .find(|handle| {
            snapshot
                .node(*handle)
                .and_then(Node::as_element)
                .is_some_and(|element| element.is_html_element("link"))
        })
        .expect("stylesheet link");
    vm.document_runtime
        .enqueue_ready_connected_style_load_for_test(link);
    vm.document_runtime.note_dom_content_loaded_dispatched();
    vm.enqueue_test_pending_runtime_source_load();
    vm.arm_runtime_script_work_continuation_if_needed();

    let step = vm.poll_next_post_parse_driver_step(&mut page_task_queue);

    assert!(matches!(
        step,
        PostParseDriverStep::Ready(action)
            if matches!(
                action.as_ref(),
                ReadyPostParseAction::Processing(processing)
                    if matches!(
                        processing.as_ref(),
                        PostParseProcessingAction {
                            work,
                            ..
                        } if matches!(
                            work.as_lifecycle_work(),
                            Some(crate::page_task_queue::PostParseLifecycleWork::DispatchConnectedStyleLoad(ready))
                                if ready.owner() == link
                        )
                    )
            )
    ));
}

#[tokio::test]
async fn poll_next_post_parse_driver_step_requests_wait_for_runtime_backlog_without_owner_progress()
{
    let _js_runtime = crate::JsRuntime::initialize();
    let document = HtmlParser.parse(
        Url::parse("https://example.com/").unwrap(),
        "<!doctype html><html><head></head><body></body></html>".to_owned(),
    );
    let mut page_task_queue = crate::page_task_queue::PageTaskQueueTestHarness::new();
    let post_domcontentloaded_page_task_sender =
        page_task_queue.owner_attached_runtime_page_task_sender_for_test();
    let script_event_parser_boundary_sender = page_task_queue.parser_boundary_sender();
    let mut vm = ScriptVmDefaultWorldBootstrap::standalone_from_dom_host_for_test(
        DomHost::from_dom(document),
        post_domcontentloaded_page_task_sender,
        script_event_parser_boundary_sender,
    )
    .expect("script vm bootstrap")
    .finish()
    .expect("script vm finish");
    let mut loading_async_script = ready_dynamic_runtime_script(9);
    loading_async_script.source_kind = ScriptSourceKind::External;
    loading_async_script.source = crate::planning::ScriptSource::External;
    vm.document_runtime.note_dom_content_loaded_dispatched();
    vm.document_runtime
        .runtime_script_work_mut()
        .dynamic_scripts
        .enqueue_loading_script_for_test(loading_async_script);
    vm.arm_runtime_script_work_continuation_if_needed();

    let step = vm.poll_next_post_parse_driver_step(&mut page_task_queue);

    assert!(matches!(step, PostParseDriverStep::AwaitProgress));
}

#[tokio::test]
async fn pre_domcontentloaded_runtime_source_wait_yields_to_stable_page_continuation() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_test_vm_with_loader("https://example.com/", &loader);
    let script_node = vm.document_runtime.dom_host_mut().create_element("script");
    let mut script = vm.test_pending_runtime_source_load_script();
    script.node_id = script_node;
    script.mode = ScriptMode::InOrder;
    vm.bind_prepared_script_handle_if_needed(&mut script, ScriptHandleSource::DocumentWriteOwned);
    let handle = script
        .host_script_handle
        .as_deref()
        .expect("external document.write script must have an exact host handle");
    assert_eq!(
        vm.document_runtime.script_handle_followup_lane(handle),
        Some(DeferredPageTaskLane::PreDomContentLoaded)
    );
    vm.document_runtime
        .runtime_script_work_mut()
        .dynamic_scripts
        .enqueue_loading_script_for_test(script);

    let mut page_task_queue = crate::page_task_queue::PageTaskQueueTestHarness::new();
    let mut report = ScriptExecutionReport::default();
    let step = tokio::time::timeout(
        std::time::Duration::from_secs(1),
        vm.next_post_parse_processing_step(&loader, &mut page_task_queue, &mut report),
    )
    .await
    .expect("post-parse runtime source wait must not retain the executor")
    .expect("post-parse runtime source wait should remain valid");

    assert!(matches!(
        step,
        crate::script_vm::PostParseProcessingStep::AwaitProgress
    ));
    assert_eq!(
        vm.document_runtime.runtime_script_work_mut().pause_kind(),
        Some(RuntimeScriptWorkPauseKind::StablePageTurnContinuation),
        "the unresolved producer must retain an exact stable continuation route"
    );
    assert!(
        vm._page_task_residence_for_executor_test
            .as_ref()
            .expect("standalone fixture must retain its production Page source")
            .task_sources()
            .take_main_document_runtime_for_executor_test()
            .is_none(),
        "waiting for a source must not manufacture a runnable continuation"
    );
}

#[test]
fn post_parse_lifecycle_driver_targets_page_vm_stage_boundaries() {
    assert_eq!(
        PostParseLifecycleDriver::target_boundary_for_stage(
            crate::renderer::PageVmInitStage::DomContentLoaded,
        ),
        PostParseStageBoundary::DomContentLoaded
    );
    assert_eq!(
        PostParseLifecycleDriver::target_boundary_for_stage(crate::renderer::PageVmInitStage::Load,),
        PostParseStageBoundary::WindowLoad
    );
}

#[test]
fn post_parse_task_execution_captures_only_the_requested_lifecycle_boundary() {
    let dcl_execution =
        test_post_parse_lifecycle_driver_for(crate::renderer::PageVmInitStage::DomContentLoaded)
            .task_execution_for_action(post_parse_lifecycle_action(
                PostParseLifecycleWork::test_domcontentloaded(),
            ));
    assert!(matches!(
        dcl_execution.token.boundary_completion,
        Some(PostParseLifecycleCompletionAction::ReturnAtStage(
            "DOMContentLoaded"
        ))
    ));

    let dcl_while_waiting_for_load =
        test_post_parse_lifecycle_driver_for(crate::renderer::PageVmInitStage::Load)
            .task_execution_for_action(post_parse_lifecycle_action(
                PostParseLifecycleWork::test_domcontentloaded(),
            ));
    assert!(
        dcl_while_waiting_for_load
            .token
            .boundary_completion
            .is_none()
    );

    let load_execution =
        test_post_parse_lifecycle_driver_for(crate::renderer::PageVmInitStage::Load)
            .task_execution_for_action(post_parse_lifecycle_action(
                PostParseLifecycleWork::test_window_load(),
            ));
    assert!(matches!(
        load_execution.token.boundary_completion,
        Some(PostParseLifecycleCompletionAction::ReturnAtStage("Load"))
    ));
}

#[test]
fn post_parse_lifecycle_driver_marks_only_host_event_tasks_for_runtime_followup_publication() {
    let driver = test_post_parse_lifecycle_driver_for(crate::renderer::PageVmInitStage::Load);

    let lifecycle_execution = driver.task_execution_for_action(post_parse_lifecycle_action(
        PostParseLifecycleWork::test_window_load(),
    ));
    assert!(
        lifecycle_execution
            .token
            .requires_runtime_followup_publication,
        "lifecycle event tasks must publish runtime-produced follow-up work"
    );

    let script_execution = driver.task_execution_for_action(post_parse_document_script_action(
        crate::document_script_scheduler::DocumentScriptExecutionLane::AsyncPhase,
        prepared_post_parse_script(15, ScriptMode::Async),
    ));
    assert!(
        !script_execution.token.requires_runtime_followup_publication,
        "script-carrying page tasks already run through the script execution path"
    );
}

#[test]
fn post_parse_lifecycle_driver_task_execution_preserves_explicit_invalidation_policy() {
    let driver = test_post_parse_lifecycle_driver_for(crate::renderer::PageVmInitStage::Load);

    let connected_style_execution =
        driver.task_execution_for_action(PostParseProcessingAction::without_invalidation_restart(
            crate::page_task_queue::PostParseLifecycleWork::DispatchConnectedStyleLoad(
                crate::document_runtime::ReadyConnectedStyleLoad::for_owner(
                    DomHandle::new(7),
                    true,
                    crate::document_runtime::ConnectedStyleEventElementKind::Link,
                ),
            ),
        ));
    assert!(matches!(
        connected_style_execution.token.invalidation_policy,
        PostParseTaskInvalidationPolicy::KeepCurrentTask
    ));

    let lifecycle_execution = driver.task_execution_for_action(post_parse_lifecycle_action(
        PostParseLifecycleWork::test_window_load(),
    ));
    assert!(matches!(
        lifecycle_execution.token.invalidation_policy,
        PostParseTaskInvalidationPolicy::RestartIfInvalidated
    ));
}

#[test]
fn post_parse_selector_keeps_window_load_ahead_of_generic_runtime_backlog() {
    let selected = select_post_parse_driver_step(
        post_parse_owner_step_for_lifecycle_work(PostParseLifecycleWork::test_window_load()),
        PostParseRuntimeDriverStep::PendingBacklog,
    );

    assert!(matches!(
        selected,
        PostParseDriverStep::Ready(action) if matches!(
            action.as_ref(),
            ReadyPostParseAction::Processing(action) if action.work.is_window_load_task()
        )
    ));
}

#[test]
fn post_parse_lifecycle_driver_idle_completion_preserves_round_stats() {
    let driver = PostParseLifecycleDriver {
        round: PostParseLifecycleRound {
            queue_stats: PostParseLifecycleQueueStats {
                defer_count: 2,
                async_count: 3,
                detached_count: 4,
            },
            phase_started: Instant::now(),
        },
        target_boundary: PostParseStageBoundary::WindowLoad,
    };

    let PostParseLifecycleCompletionAction::Finalize(finalization) =
        driver.idle_completion_action()
    else {
        panic!("idle driver completion should finalize the post-parse round");
    };

    assert_eq!(finalization.defer_count(), 2);
    assert_eq!(finalization.async_count(), 3);
    assert_eq!(finalization.detached_count(), 4);
}

#[tokio::test]
async fn next_post_parse_lifecycle_advance_from_driver_returns_complete_when_driver_is_idle() {
    let _js_runtime = crate::JsRuntime::initialize();
    let document = HtmlParser.parse(
        Url::parse("https://example.com/").unwrap(),
        "<!doctype html><html><head></head><body></body></html>".to_owned(),
    );
    let mut page_task_queue = crate::page_task_queue::PageTaskQueueTestHarness::new();
    let post_domcontentloaded_page_task_sender =
        page_task_queue.owner_attached_runtime_page_task_sender_for_test();
    let script_event_parser_boundary_sender = page_task_queue.parser_boundary_sender();
    let mut vm = ScriptVmDefaultWorldBootstrap::standalone_from_dom_host_for_test(
        DomHost::from_dom(document),
        post_domcontentloaded_page_task_sender,
        script_event_parser_boundary_sender,
    )
    .expect("script vm bootstrap")
    .finish()
    .expect("script vm finish");
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut report = ScriptExecutionReport::default();
    let driver = PostParseLifecycleDriver {
        round: PostParseLifecycleRound {
            queue_stats: PostParseLifecycleQueueStats::default(),
            phase_started: Instant::now(),
        },
        target_boundary: PostParseStageBoundary::WindowLoad,
    };

    let advance = vm
        .next_post_parse_lifecycle_advance_from_driver(
            &loader,
            &mut page_task_queue,
            &mut report,
            driver,
        )
        .await
        .expect("driver helper should succeed");

    assert!(matches!(
        advance,
        PostParseLifecycleAdvance::Complete(PostParseLifecycleCompletionAction::Finalize(_))
    ));
}

#[tokio::test]
async fn next_post_parse_lifecycle_advance_from_driver_returns_page_owned_task_for_ready_processing_action()
 {
    let _js_runtime = crate::JsRuntime::initialize();
    let document = HtmlParser.parse(
        Url::parse("https://example.com/").unwrap(),
        "<!doctype html><html><head></head><body></body></html>".to_owned(),
    );
    let mut page_task_queue = crate::page_task_queue::PageTaskQueueTestHarness::new();
    let post_domcontentloaded_page_task_sender =
        page_task_queue.owner_attached_runtime_page_task_sender_for_test();
    let script_event_parser_boundary_sender = page_task_queue.parser_boundary_sender();
    let mut vm = ScriptVmDefaultWorldBootstrap::standalone_from_dom_host_for_test(
        DomHost::from_dom(document),
        post_domcontentloaded_page_task_sender,
        script_event_parser_boundary_sender,
    )
    .expect("script vm bootstrap")
    .finish()
    .expect("script vm finish");
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut report = ScriptExecutionReport::default();
    let driver = PostParseLifecycleDriver {
        round: PostParseLifecycleRound {
            queue_stats: PostParseLifecycleQueueStats::default(),
            phase_started: Instant::now(),
        },
        target_boundary: PostParseStageBoundary::WindowLoad,
    };

    vm.document_runtime.note_dom_content_loaded_dispatched();
    page_task_queue.extend_post_parse_work([post_parse_document_script_work(
        crate::document_script_scheduler::DocumentScriptExecutionLane::AsyncPhase,
        prepared_post_parse_script(11, ScriptMode::Async),
    )]);

    let advance = vm
        .next_post_parse_lifecycle_advance_from_driver(
            &loader,
            &mut page_task_queue,
            &mut report,
            driver,
        )
        .await
        .expect("driver helper should succeed");

    let PostParseLifecycleAdvance::PageOwnedTask(mut task) = advance else {
        panic!("ready processing action should become a page-owned task");
    };
    assert!(is_document_script_execution_work(
        &task.take_work_for_execution(),
        crate::document_script_scheduler::DocumentScriptExecutionLane::AsyncPhase,
        11,
    ));
}

#[tokio::test]
async fn finish_completed_post_parse_page_owned_task_or_continue_returns_boundary_completion() {
    let _js_runtime = crate::JsRuntime::initialize();
    let document = HtmlParser.parse(
        Url::parse("https://example.com/").unwrap(),
        "<!doctype html><html><head></head><body></body></html>".to_owned(),
    );
    let page_task_queue = crate::page_task_queue::PageTaskQueueTestHarness::new();
    let post_domcontentloaded_page_task_sender =
        page_task_queue.owner_attached_runtime_page_task_sender_for_test();
    let script_event_parser_boundary_sender = page_task_queue.parser_boundary_sender();
    let mut vm = ScriptVmDefaultWorldBootstrap::standalone_from_dom_host_for_test(
        DomHost::from_dom(document),
        post_domcontentloaded_page_task_sender,
        script_event_parser_boundary_sender,
    )
    .expect("script vm bootstrap")
    .finish()
    .expect("script vm finish");
    let mut page_task_queue = crate::page_task_queue::PageTaskQueueTestHarness::new();
    let mut report = ScriptExecutionReport::default();
    let completed_task = PostParsePageOwnedTask {
        work: None,
        completion: PostParseTaskCompletion {
            token: PostParseTaskExecutionToken {
                boundary_completion: Some(PostParseLifecycleCompletionAction::ReturnAtStage(
                    "DOMContentLoaded",
                )),
                invalidation_policy: PostParseTaskInvalidationPolicy::KeepCurrentTask,
                requires_runtime_followup_publication: false,
            },
        },
    };

    let advance = vm
        .finish_completed_post_parse_page_owned_task_or_continue(
            &mut page_task_queue,
            &mut report,
            Some(completed_task),
        )
        .await
        .expect("completion helper should succeed");

    assert!(matches!(
        advance,
        Some(PostParseLifecycleAdvance::Complete(
            PostParseLifecycleCompletionAction::ReturnAtStage("DOMContentLoaded")
        ))
    ));
}

#[tokio::test]
async fn advance_post_parse_lifecycle_returns_completion_from_completed_boundary_task() {
    let _js_runtime = crate::JsRuntime::initialize();
    let document = HtmlParser.parse(
        Url::parse("https://example.com/").unwrap(),
        "<!doctype html><html><head></head><body></body></html>".to_owned(),
    );
    let mut page_task_queue = crate::page_task_queue::PageTaskQueueTestHarness::new();
    let post_domcontentloaded_page_task_sender =
        page_task_queue.owner_attached_runtime_page_task_sender_for_test();
    let script_event_parser_boundary_sender = page_task_queue.parser_boundary_sender();
    let mut vm = ScriptVmDefaultWorldBootstrap::standalone_from_dom_host_for_test(
        DomHost::from_dom(document),
        post_domcontentloaded_page_task_sender,
        script_event_parser_boundary_sender,
    )
    .expect("script vm bootstrap")
    .finish()
    .expect("script vm finish");
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut report = ScriptExecutionReport::default();
    let driver = PostParseLifecycleDriver {
        round: PostParseLifecycleRound {
            queue_stats: PostParseLifecycleQueueStats::default(),
            phase_started: Instant::now(),
        },
        target_boundary: PostParseStageBoundary::DomContentLoaded,
    };
    let completed_task = PostParsePageOwnedTask {
        work: None,
        completion: PostParseTaskCompletion {
            token: PostParseTaskExecutionToken {
                boundary_completion: Some(PostParseLifecycleCompletionAction::ReturnAtStage(
                    "DOMContentLoaded",
                )),
                invalidation_policy: PostParseTaskInvalidationPolicy::KeepCurrentTask,
                requires_runtime_followup_publication: false,
            },
        },
    };

    let advance = vm
        .advance_post_parse_lifecycle(
            &loader,
            &mut page_task_queue,
            &mut report,
            driver,
            Some(completed_task),
        )
        .await
        .expect("driver should succeed");

    assert!(matches!(
        advance,
        PostParseLifecycleAdvance::Complete(PostParseLifecycleCompletionAction::ReturnAtStage(
            "DOMContentLoaded"
        ))
    ));
}

#[tokio::test]
async fn advance_post_parse_lifecycle_continues_to_driver_after_non_boundary_completion() {
    let _js_runtime = crate::JsRuntime::initialize();
    let document = HtmlParser.parse(
        Url::parse("https://example.com/").unwrap(),
        "<!doctype html><html><head></head><body></body></html>".to_owned(),
    );
    let mut page_task_queue = crate::page_task_queue::PageTaskQueueTestHarness::new();
    let post_domcontentloaded_page_task_sender =
        page_task_queue.owner_attached_runtime_page_task_sender_for_test();
    let script_event_parser_boundary_sender = page_task_queue.parser_boundary_sender();
    let mut vm = ScriptVmDefaultWorldBootstrap::standalone_from_dom_host_for_test(
        DomHost::from_dom(document),
        post_domcontentloaded_page_task_sender,
        script_event_parser_boundary_sender,
    )
    .expect("script vm bootstrap")
    .finish()
    .expect("script vm finish");
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut report = ScriptExecutionReport::default();
    let driver = PostParseLifecycleDriver {
        round: PostParseLifecycleRound {
            queue_stats: PostParseLifecycleQueueStats::default(),
            phase_started: Instant::now(),
        },
        target_boundary: PostParseStageBoundary::WindowLoad,
    };
    let completed_task = PostParsePageOwnedTask {
        work: None,
        completion: PostParseTaskCompletion {
            token: PostParseTaskExecutionToken {
                boundary_completion: None,
                invalidation_policy: PostParseTaskInvalidationPolicy::KeepCurrentTask,
                requires_runtime_followup_publication: false,
            },
        },
    };

    vm.document_runtime.note_dom_content_loaded_dispatched();
    page_task_queue.extend_post_parse_work([post_parse_document_script_work(
        crate::document_script_scheduler::DocumentScriptExecutionLane::AsyncPhase,
        prepared_post_parse_script(13, ScriptMode::Async),
    )]);

    let advance = vm
        .advance_post_parse_lifecycle(
            &loader,
            &mut page_task_queue,
            &mut report,
            driver,
            Some(completed_task),
        )
        .await
        .expect("driver should succeed");

    let PostParseLifecycleAdvance::PageOwnedTask(mut task) = advance else {
        panic!("non-boundary completion should fall through to the driver");
    };
    assert!(is_document_script_execution_work(
        &task.take_work_for_execution(),
        crate::document_script_scheduler::DocumentScriptExecutionLane::AsyncPhase,
        13,
    ));
}

#[tokio::test]
async fn advance_post_parse_lifecycle_restarts_invalidated_round_after_completed_task_before_returning_task()
 {
    let _js_runtime = crate::JsRuntime::initialize();
    let document = HtmlParser.parse(
        Url::parse("https://example.com/").unwrap(),
        "<!doctype html><html><head></head><body></body></html>".to_owned(),
    );
    let mut page_task_queue = crate::page_task_queue::PageTaskQueueTestHarness::new();
    let post_domcontentloaded_page_task_sender =
        page_task_queue.owner_attached_runtime_page_task_sender_for_test();
    let script_event_parser_boundary_sender = page_task_queue.parser_boundary_sender();
    let mut vm = ScriptVmDefaultWorldBootstrap::standalone_from_dom_host_for_test(
        DomHost::from_dom(document),
        post_domcontentloaded_page_task_sender,
        script_event_parser_boundary_sender,
    )
    .expect("script vm bootstrap")
    .finish()
    .expect("script vm finish");
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut report = ScriptExecutionReport::default();
    let driver = PostParseLifecycleDriver {
        round: PostParseLifecycleRound {
            queue_stats: PostParseLifecycleQueueStats::default(),
            phase_started: Instant::now(),
        },
        target_boundary: PostParseStageBoundary::WindowLoad,
    };
    let completed_task = PostParsePageOwnedTask {
        work: None,
        completion: PostParseTaskCompletion {
            token: PostParseTaskExecutionToken {
                boundary_completion: None,
                invalidation_policy: PostParseTaskInvalidationPolicy::RestartIfInvalidated,
                requires_runtime_followup_publication: false,
            },
        },
    };

    vm.document_runtime.open_document();

    let advance = vm
        .advance_post_parse_lifecycle(
            &loader,
            &mut page_task_queue,
            &mut report,
            driver,
            Some(completed_task),
        )
        .await
        .expect("driver should succeed");

    let PostParseLifecycleAdvance::PageOwnedTask(mut task) = advance else {
        panic!("restarted round should continue with rebuilt document lifecycle work");
    };
    assert!(
        task.take_work_for_execution()
            .is_main_document_interactive_task(),
        "a restarted round must begin with the replacement interactive transition"
    );
}

#[tokio::test]
async fn advance_post_parse_lifecycle_restarts_invalidated_round_before_boundary_completion() {
    let _js_runtime = crate::JsRuntime::initialize();
    let document = HtmlParser.parse(
        Url::parse("https://example.com/").unwrap(),
        "<!doctype html><html><head></head><body></body></html>".to_owned(),
    );
    let mut page_task_queue = crate::page_task_queue::PageTaskQueueTestHarness::new();
    let post_domcontentloaded_page_task_sender =
        page_task_queue.owner_attached_runtime_page_task_sender_for_test();
    let script_event_parser_boundary_sender = page_task_queue.parser_boundary_sender();
    let mut vm = ScriptVmDefaultWorldBootstrap::standalone_from_dom_host_for_test(
        DomHost::from_dom(document),
        post_domcontentloaded_page_task_sender,
        script_event_parser_boundary_sender,
    )
    .expect("script vm bootstrap")
    .finish()
    .expect("script vm finish");
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut report = ScriptExecutionReport::default();
    let driver =
        test_post_parse_lifecycle_driver_for(crate::renderer::PageVmInitStage::DomContentLoaded);
    let completed_task = PostParsePageOwnedTask {
        work: None,
        completion: PostParseTaskCompletion {
            token: PostParseTaskExecutionToken {
                boundary_completion: Some(PostParseLifecycleCompletionAction::ReturnAtStage(
                    "DOMContentLoaded",
                )),
                invalidation_policy: PostParseTaskInvalidationPolicy::RestartIfInvalidated,
                requires_runtime_followup_publication: false,
            },
        },
    };

    vm.document_runtime.open_document();

    let advance = vm
        .advance_post_parse_lifecycle(
            &loader,
            &mut page_task_queue,
            &mut report,
            driver,
            Some(completed_task),
        )
        .await
        .expect("driver should succeed");

    let PostParseLifecycleAdvance::PageOwnedTask(mut task) = advance else {
        panic!("invalidated round must restart before returning boundary completion");
    };
    assert!(
        task.take_work_for_execution()
            .is_main_document_interactive_task(),
        "invalidation must restart at the replacement interactive transition before the old boundary"
    );
}

#[tokio::test]
async fn prepare_post_parse_task_execution_keeps_non_restartable_task_after_invalidation() {
    let mut vm = new_storage_test_vm("https://example.com/");
    let mut page_task_queue = crate::page_task_queue::PageTaskQueueTestHarness::new();
    let mut report = ScriptExecutionReport::default();
    let execution = test_post_parse_lifecycle_driver_for(crate::renderer::PageVmInitStage::Load)
        .task_execution_for_action(PostParseProcessingAction::without_invalidation_restart(
            crate::page_task_queue::PostParseLifecycleWork::DispatchConnectedStyleLoad(
                crate::document_runtime::ReadyConnectedStyleLoad::for_owner(
                    DomHandle::new(7),
                    true,
                    crate::document_runtime::ConnectedStyleEventElementKind::Link,
                ),
            ),
        ));

    vm.document_runtime.open_document();

    let restarted = vm
        .prepare_post_parse_task_execution(&mut page_task_queue, &mut report, execution.token)
        .await;

    assert!(
        !restarted,
        "non-restartable connected style load tasks must survive a stale schedule"
    );
    assert!(
        vm.document_runtime.take_post_parse_schedule_invalidated(),
        "keeping the current task must not consume the pending schedule invalidation"
    );
}

#[tokio::test]
async fn prepare_post_parse_task_execution_restarts_restartable_task_after_invalidation() {
    let mut vm = new_storage_test_vm("https://example.com/");
    let mut page_task_queue = crate::page_task_queue::PageTaskQueueTestHarness::new();
    let mut report = ScriptExecutionReport::default();
    let execution =
        test_post_parse_lifecycle_driver_for(crate::renderer::PageVmInitStage::DomContentLoaded)
            .task_execution_for_action(post_parse_lifecycle_action(
                PostParseLifecycleWork::test_domcontentloaded(),
            ));

    vm.document_runtime.open_document();

    let restarted = vm
        .prepare_post_parse_task_execution(&mut page_task_queue, &mut report, execution.token)
        .await;

    assert!(
        restarted,
        "restartable lifecycle tasks must yield to a freshly invalidated post-parse round"
    );
    assert!(
        !vm.document_runtime.take_post_parse_schedule_invalidated(),
        "restarting must consume the pending schedule invalidation"
    );
    assert!(
        page_task_queue
            .post_parse_front()
            .is_some_and(PostParsePageOwnedWork::is_main_document_interactive_task),
        "restartable lifecycle work must rebuild from the replacement interactive transition"
    );
}

#[test]
fn poll_next_post_parse_driver_step_returns_idle_without_owner_or_runtime_work() {
    let _js_runtime = crate::JsRuntime::initialize();
    let document = HtmlParser.parse(
        Url::parse("https://example.com/").unwrap(),
        "<!doctype html><html><head></head><body></body></html>".to_owned(),
    );
    let mut page_task_queue = crate::page_task_queue::PageTaskQueueTestHarness::new();
    let post_domcontentloaded_page_task_sender =
        page_task_queue.owner_attached_runtime_page_task_sender_for_test();
    let script_event_parser_boundary_sender = page_task_queue.parser_boundary_sender();
    let mut vm = ScriptVmDefaultWorldBootstrap::standalone_from_dom_host_for_test(
        DomHost::from_dom(document),
        post_domcontentloaded_page_task_sender,
        script_event_parser_boundary_sender,
    )
    .expect("script vm bootstrap")
    .finish()
    .expect("script vm finish");
    let step = vm.poll_next_post_parse_driver_step(&mut page_task_queue);

    assert!(matches!(step, PostParseDriverStep::Idle));
}

#[test]
fn runtime_script_work_pause_tracks_internal_pause_kind() {
    let mut work = RuntimeScriptWorkState::default();
    work.dynamic_scripts.requeue_ready_script_front(
        dynamic_script_owner_id(1),
        ready_dynamic_runtime_script(1),
        None,
    );

    work.pause_for_deferred_page_tasks(RuntimeScriptWorkPauseKind::StablePageTurnContinuation);

    assert!(work.is_paused_for_deferred_page_tasks());
    assert_eq!(
        work.pause_kind(),
        Some(RuntimeScriptWorkPauseKind::StablePageTurnContinuation)
    );

    work.resume_after_deferred_page_tasks();

    assert!(!work.is_paused_for_deferred_page_tasks());
    assert_eq!(work.pause_kind(), None);
}

#[test]
fn selected_post_parse_action_settlement_publishes_concrete_runtime_followup() {
    let _js_runtime = crate::JsRuntime::initialize();
    let document = HtmlParser.parse(
        Url::parse("https://example.com/").unwrap(),
        "<!doctype html><html><head></head><body></body></html>".to_owned(),
    );
    let mut page_task_queue = crate::page_task_queue::PageTaskQueueTestHarness::new();
    let post_domcontentloaded_page_task_sender =
        page_task_queue.owner_attached_runtime_page_task_sender_for_test();
    let script_event_parser_boundary_sender = page_task_queue.parser_boundary_sender();
    let mut vm = ScriptVmDefaultWorldBootstrap::standalone_from_dom_host_for_test(
        DomHost::from_dom(document),
        post_domcontentloaded_page_task_sender,
        script_event_parser_boundary_sender,
    )
    .expect("script vm bootstrap")
    .finish()
    .expect("script vm finish");

    vm.document_runtime
        .runtime_script_work_mut()
        .dynamic_scripts
        .requeue_ready_script_front(
            dynamic_script_owner_id(1),
            ready_dynamic_runtime_script(1),
            None,
        );
    vm.publish_post_parse_action_followups(&mut page_task_queue);

    assert_eq!(
        take_main_document_runtime_action(&page_task_queue).map(|action| action.kind()),
        Some(PageMainDocumentRuntimeActionKind::RuntimeScriptContinuation),
        "runnable runtime progress must publish a concrete production task"
    );
}

#[test]
fn selected_post_parse_action_settlement_arms_stable_continuation_state() {
    let _js_runtime = crate::JsRuntime::initialize();
    let document = HtmlParser.parse(
        Url::parse("https://example.com/").unwrap(),
        "<!doctype html><html><head></head><body></body></html>".to_owned(),
    );
    let mut page_task_queue = crate::page_task_queue::PageTaskQueueTestHarness::new();
    let post_domcontentloaded_page_task_sender =
        page_task_queue.owner_attached_runtime_page_task_sender_for_test();
    let script_event_parser_boundary_sender = page_task_queue.parser_boundary_sender();
    let mut vm = ScriptVmDefaultWorldBootstrap::standalone_from_dom_host_for_test(
        DomHost::from_dom(document),
        post_domcontentloaded_page_task_sender,
        script_event_parser_boundary_sender,
    )
    .expect("script vm bootstrap")
    .finish()
    .expect("script vm finish");
    vm.document_runtime.note_dom_content_loaded_dispatched();
    vm.document_runtime
        .runtime_script_work_mut()
        .dynamic_scripts
        .requeue_ready_script_front(
            dynamic_script_owner_id(1),
            ready_dynamic_runtime_script(1),
            None,
        );

    vm.publish_post_parse_action_followups(&mut page_task_queue);

    assert!(
        vm.document_runtime
            .runtime_script_work_mut()
            .is_paused_for_deferred_page_tasks(),
        "runtime work should be re-armed on the stable continuation state"
    );
    assert_eq!(
        vm.document_runtime.runtime_script_work_mut().pause_kind(),
        Some(RuntimeScriptWorkPauseKind::StablePageTurnContinuation)
    );
    assert_eq!(
        take_main_document_runtime_action(&page_task_queue).map(|action| action.kind()),
        Some(PageMainDocumentRuntimeActionKind::RuntimeScriptContinuation),
        "settlement must admit the ready continuation to the production source"
    );
}

#[test]
fn runtime_owned_module_source_ready_before_domcontentloaded_publishes_task_without_starting_graph_inline()
 {
    let _js_runtime = crate::JsRuntime::initialize();
    let document = HtmlParser.parse(
        Url::parse("https://example.com/").unwrap(),
        "<!doctype html><html><head></head><body></body></html>".to_owned(),
    );
    let mut page_task_queue = crate::page_task_queue::PageTaskQueueTestHarness::new();
    let post_domcontentloaded_page_task_sender =
        page_task_queue.owner_attached_runtime_page_task_sender_for_test();
    let script_event_parser_boundary_sender = page_task_queue.parser_boundary_sender();
    let mut vm = ScriptVmDefaultWorldBootstrap::standalone_from_dom_host_for_test(
        DomHost::from_dom(document),
        post_domcontentloaded_page_task_sender,
        script_event_parser_boundary_sender,
    )
    .expect("script vm bootstrap")
    .finish()
    .expect("script vm finish");
    let body = vm
        .document_runtime
        .snapshot_document()
        .document_body_handle()
        .expect("body should exist");
    let script_node = vm.document_runtime.dom_host_mut().create_element("script");
    assert!(
        vm.document_runtime
            .dom_host_mut()
            .append_child(body, script_node),
        "runtime-owned module script node should attach"
    );
    vm.document_runtime
        .bind_runtime_owned_script_handle_for_node(script_node, "runtime-module-7");
    vm.document_runtime
        .runtime_script_work_mut()
        .dynamic_scripts
        .requeue_ready_script_front(
            dynamic_script_owner_id(7),
            ready_dynamic_runtime_module_script(7, script_node),
            None,
        );
    vm.publish_post_parse_action_followups(&mut page_task_queue);

    assert_eq!(
        take_main_document_runtime_action(&page_task_queue).map(|action| action.kind()),
        Some(PageMainDocumentRuntimeActionKind::RuntimeScriptContinuation),
        "source-ready runtime module must publish one concrete scheduler task before DOMContentLoaded"
    );
    assert!(
        !vm.has_pending_runtime_owned_module_script_graph(),
        "admission must not start the module graph outside its selected Page turn"
    );
    assert!(
        vm.document_runtime
            .runtime_script_work_mut()
            .dynamic_scripts
            .next_runnable_script()
            .is_some(),
        "the selected runtime continuation owns consumption of the runnable module"
    );
}

#[test]
fn explicitly_domcontentloaded_gated_runtime_head_is_not_admitted_early() {
    let _js_runtime = crate::JsRuntime::initialize();
    let document = HtmlParser.parse(
        Url::parse("https://example.com/").unwrap(),
        "<!doctype html><html><head></head><body></body></html>".to_owned(),
    );
    let mut page_task_queue = crate::page_task_queue::PageTaskQueueTestHarness::new();
    let post_domcontentloaded_page_task_sender =
        page_task_queue.owner_attached_runtime_page_task_sender_for_test();
    let script_event_parser_boundary_sender = page_task_queue.parser_boundary_sender();
    let mut vm = ScriptVmDefaultWorldBootstrap::standalone_from_dom_host_for_test(
        DomHost::from_dom(document),
        post_domcontentloaded_page_task_sender,
        script_event_parser_boundary_sender,
    )
    .expect("script vm bootstrap")
    .finish()
    .expect("script vm finish");
    let body = vm
        .document_runtime
        .snapshot_document()
        .document_body_handle()
        .expect("body should exist");
    let script_node = vm.document_runtime.dom_host_mut().create_element("script");
    assert!(
        vm.document_runtime
            .dom_host_mut()
            .append_child(body, script_node),
        "gated script node should attach"
    );
    let handle = vm
        .document_runtime
        .bind_runtime_owned_script_handle_for_node(script_node, "explicitly-dcl-gated-script");
    vm.document_runtime
        .set_script_handle_waits_until_dom_content_loaded(&handle);
    let mut script = ready_dynamic_runtime_script(9);
    script.node_id = script_node;
    script.host_script_handle = Some(handle);
    vm.document_runtime
        .runtime_script_work_mut()
        .dynamic_scripts
        .requeue_ready_script_front(dynamic_script_owner_id(9), script, None);

    vm.publish_post_parse_action_followups(&mut page_task_queue);

    assert!(
        take_main_document_runtime_action(&page_task_queue).is_none(),
        "an explicitly gated head must remain durable without becoming runnable before DCL"
    );
    assert!(
        vm.document_runtime
            .runtime_script_work_mut()
            .dynamic_scripts
            .has_immediately_runnable_work(),
        "the denied head must remain in its authoritative owner"
    );

    vm.document_runtime.note_dom_content_loaded_dispatched();
    vm.publish_post_parse_action_followups(&mut page_task_queue);

    assert_eq!(
        take_main_document_runtime_action(&page_task_queue).map(|action| action.kind()),
        Some(PageMainDocumentRuntimeActionKind::RuntimeScriptContinuation),
        "the same durable head must become admitted once its DCL gate opens"
    );
}

#[tokio::test]
async fn reentrant_runtime_admission_survives_page_task_claim_in_stable_authority() {
    let _js_runtime = crate::JsRuntime::initialize();
    let document = HtmlParser.parse(
        Url::parse("https://example.com/").unwrap(),
        "<!doctype html><html><head></head><body></body></html>".to_owned(),
    );
    let page_task_queue = crate::page_task_queue::PageTaskQueueTestHarness::new();
    let post_domcontentloaded_page_task_sender =
        page_task_queue.owner_attached_runtime_page_task_sender_for_test();
    let script_event_parser_boundary_sender = page_task_queue.parser_boundary_sender();
    let mut vm = ScriptVmDefaultWorldBootstrap::standalone_from_dom_host_for_test(
        DomHost::from_dom(document),
        post_domcontentloaded_page_task_sender,
        script_event_parser_boundary_sender,
    )
    .expect("script vm bootstrap")
    .finish()
    .expect("script vm finish");
    let body = vm
        .document_runtime
        .snapshot_document()
        .document_body_handle()
        .expect("body should exist");
    let script_node = vm.document_runtime.dom_host_mut().create_element("script");
    assert!(
        vm.document_runtime
            .dom_host_mut()
            .append_child(body, script_node),
        "runtime-owned classic script node should attach"
    );
    vm.document_runtime
        .bind_runtime_owned_script_handle_for_node(script_node, "runtime-classic-8");
    let mut script = ready_dynamic_runtime_script(8);
    script.node_id = script_node;
    script.host_script_handle = Some("runtime-classic-8".to_owned());
    script.source_kind = ScriptSourceKind::Inline;
    script.source = ScriptSource::Inline("globalThis.runtimeClassicRan = true;".to_owned());
    let owner = vm
        .current_main_document_task_owner()
        .expect("main document owner");
    let lease = vm
        ._context_host
        .borrow_mut()
        .acquire_current_main_document_script_load_delay(
            owner,
            crate::frame_owner_model::MainDocumentScriptLoadDelayKind::Classic,
        )
        .expect("runtime classic script should acquire an exact load-delay lease");
    let document_loader = vm
        .current_main_document_resource_loader()
        .expect("standalone VM should retain its Document resource authority");
    let loader = document_loader.request_client().clone();
    let task_runner = document_loader.task_runner();
    vm.admit_main_document_runtime_script_task(
        &loader,
        task_runner.clone(),
        crate::host::RuntimeScriptAdmission::new(
            crate::host::RuntimeScriptAdmissionPayload::Script(script),
            lease,
        ),
    );
    let second_script_node = vm.document_runtime.dom_host_mut().create_element("script");
    assert!(
        vm.document_runtime
            .dom_host_mut()
            .append_child(body, second_script_node)
    );
    let second_handle = vm
        .document_runtime
        .bind_runtime_owned_script_handle_for_node(second_script_node, "runtime-classic-9");
    let mut second_script = ready_dynamic_runtime_script(9);
    second_script.node_id = second_script_node;
    second_script.host_script_handle = Some(second_handle);
    second_script.source_kind = ScriptSourceKind::Inline;
    second_script.source =
        ScriptSource::Inline("globalThis.runtimeClassicSecondRan = true;".to_owned());
    let second_lease = vm
        ._context_host
        .borrow_mut()
        .acquire_current_main_document_script_load_delay(
            owner,
            crate::frame_owner_model::MainDocumentScriptLoadDelayKind::Classic,
        )
        .expect("second runtime classic script should acquire its own exact lease");
    let mut reentrant_admission = Some(crate::host::RuntimeScriptAdmission::new(
        crate::host::RuntimeScriptAdmissionPayload::Script(second_script),
        second_lease,
    ));
    vm.document_runtime.note_dom_content_loaded_dispatched();
    let runtime_script_work = vm.document_runtime.runtime_script_work_handle();

    let mut emitted = Vec::new();
    assert!(
        vm.emit_ready_runtime_page_owned_work(|work| {
            runtime_script_work
                .borrow_mut()
                .dynamic_scripts
                .enqueue_admission(
                    &loader,
                    task_runner.clone(),
                    reentrant_admission
                        .take()
                        .expect("enqueue callback should admit the second script once"),
                    None,
                    None,
                );
            emitted.push(work);
        })
        .is_some()
    );

    assert_eq!(emitted.len(), 1);
    let Some(PostParsePageOwnedWork::DocumentScript(work)) = emitted.pop() else {
        panic!("runtime classic script should become concrete page-owned work");
    };
    let crate::document_script_scheduler::PageOwnedDocumentScriptWork::Script {
        lane: crate::document_script_scheduler::DocumentScriptExecutionLane::AsyncPhase,
        script,
        runtime_script_claim: Some(claim),
        source_network_result: None,
        load_delay_binding: None,
    } = *work
    else {
        panic!("runtime classic page task should carry its exact terminal claim");
    };
    assert_eq!(script.position, 8);
    assert_eq!(claim.owner(), owner);
    let first_script_owner_id = claim.id();

    assert_eq!(
        vm._context_host
            .borrow()
            .current_main_document_has_async_script_load_delay(owner),
        Some(true),
        "claiming page work must leave the task-owned lease active"
    );
    let _ = vm.finish_claimed_runtime_owned_script_success_body(claim, &script);
    assert!(
        runtime_script_work
            .borrow_mut()
            .dynamic_scripts
            .finish_script_terminal(first_script_owner_id)
            .is_none(),
        "a claimed terminal cannot settle the same script through the owner a second time"
    );
    assert_eq!(
        vm._context_host
            .borrow()
            .current_main_document_has_async_script_load_delay(owner),
        Some(true),
        "settling one task-owned lease must not release the reentrant script's lease"
    );
    assert!(
        !runtime_script_work.borrow_mut().dynamic_scripts.is_idle(),
        "the reentrant script must remain in the same stable runtime authority"
    );

    let mut emitted = Vec::new();
    assert!(
        vm.emit_ready_runtime_page_owned_work(|work| {
            emitted.push(work);
        })
        .is_some()
    );
    let Some(PostParsePageOwnedWork::DocumentScript(work)) = emitted.pop() else {
        panic!("second runtime classic script should become concrete page-owned work");
    };
    let crate::document_script_scheduler::PageOwnedDocumentScriptWork::Script {
        script,
        runtime_script_claim: Some(claim),
        ..
    } = *work
    else {
        panic!("second runtime classic page task should carry its exact terminal claim");
    };
    vm.cancel_claimed_runtime_owned_script_load_delay_body(claim, &script);
    assert_eq!(
        vm._context_host
            .borrow()
            .current_main_document_has_async_script_load_delay(owner),
        Some(false),
        "the last task-owned lease must settle without looking it up in runtime authority"
    );
    assert!(
        runtime_script_work.borrow_mut().dynamic_scripts.is_idle(),
        "both claimed scripts should leave the stable runtime authority idle"
    );
}

#[test]
fn script_terminal_event_body_defers_listener_reaction_to_task_completion() {
    let _js_runtime = crate::JsRuntime::initialize();
    let document = HtmlParser.parse(
        Url::parse("https://example.com/").unwrap(),
        "<!doctype html><html><head></head><body></body></html>".to_owned(),
    );
    let page_task_queue = crate::page_task_queue::PageTaskQueueTestHarness::new();
    let post_domcontentloaded_page_task_sender =
        page_task_queue.owner_attached_runtime_page_task_sender_for_test();
    let script_event_parser_boundary_sender = page_task_queue.parser_boundary_sender();
    let mut vm = ScriptVmDefaultWorldBootstrap::standalone_from_dom_host_for_test(
        DomHost::from_dom(document),
        post_domcontentloaded_page_task_sender,
        script_event_parser_boundary_sender,
    )
    .expect("script vm bootstrap")
    .finish()
    .expect("script vm finish");
    let body = vm
        .document_runtime
        .snapshot_document()
        .document_body_handle()
        .expect("body should exist");
    let script_node = vm.document_runtime.dom_host_mut().create_element("script");
    assert!(
        vm.document_runtime
            .dom_host_mut()
            .append_child(body, script_node)
    );
    assert!(vm.document_runtime.dom_host_mut().set_attribute(
        script_node,
        "id",
        "runtime-terminal-script"
    ));
    assert!(vm.document_runtime.dom_host_mut().set_attribute(
        script_node,
        "src",
        "/runtime-terminal.js"
    ));
    let handle = vm
        .document_runtime
        .bind_runtime_owned_script_handle_for_node(script_node, "runtime-binding-9");
    vm.eval(
        r#"
        globalThis.__runtimeTerminalOrder = [];
        document.getElementById("runtime-terminal-script").onload = () => {
          __runtimeTerminalOrder.push("load");
          queueMicrotask(() => __runtimeTerminalOrder.push("load-microtask"));
        };
        "installed";
        "#,
    )
    .expect("runtime terminal listener should install");

    vm.dispatch_script_event_body_best_effort(&crate::host::ScriptEventTask::new(
        crate::host::ScriptEventKind::Load,
        handle,
    ));
    assert_eq!(
        vm.eval_without_microtask_checkpoint_for_test("__runtimeTerminalOrder.join('|')")
            .expect("terminal body order should be readable without a checkpoint"),
        "load",
        "the terminal body must not perform the enclosing task-end checkpoint"
    );
    vm.perform_script_task_checkpoint(None)
        .expect("selected task completion checkpoint should run");
    assert_eq!(
        vm.eval_without_microtask_checkpoint_for_test("__runtimeTerminalOrder.join('|')")
            .expect("terminal completion order should be readable"),
        "load|load-microtask"
    );
    assert!(
        take_main_document_runtime_action(&page_task_queue).is_none(),
        "a terminal event body must not manufacture a second task"
    );
}

#[test]
fn runtime_dynamic_script_terminal_consumes_its_accepted_document_lease_inline() {
    let _js_runtime = crate::JsRuntime::initialize();
    let document = HtmlParser.parse(
        Url::parse("https://example.com/").unwrap(),
        "<!doctype html><html><head></head><body></body></html>".to_owned(),
    );
    let page_task_queue = crate::page_task_queue::PageTaskQueueTestHarness::new();
    let post_domcontentloaded_page_task_sender =
        page_task_queue.owner_attached_runtime_page_task_sender_for_test();
    let script_event_parser_boundary_sender = page_task_queue.parser_boundary_sender();
    let mut vm = ScriptVmDefaultWorldBootstrap::standalone_from_dom_host_for_test(
        DomHost::from_dom(document),
        post_domcontentloaded_page_task_sender,
        script_event_parser_boundary_sender,
    )
    .expect("script vm bootstrap")
    .finish()
    .expect("script vm finish");
    let body = vm
        .document_runtime
        .snapshot_document()
        .document_body_handle()
        .expect("body should exist");
    let script_node = vm.document_runtime.dom_host_mut().create_element("script");
    assert!(
        vm.document_runtime
            .dom_host_mut()
            .append_child(body, script_node)
    );
    let handle = vm
        .document_runtime
        .bind_runtime_owned_script_handle_for_node(script_node, "runtime-binding-9");
    let mut script = ready_dynamic_runtime_script(9);
    script.node_id = script_node;
    script.host_script_handle = Some(handle);
    script.kind = ScriptKind::Module;
    script.source_kind = ScriptSourceKind::Inline;
    script.source = ScriptSource::Inline("globalThis.__dynamicBindingRan = true;".to_owned());

    let owner = vm
        .current_main_document_task_owner()
        .expect("main document owner");
    let document_loader = vm
        .current_main_document_resource_loader()
        .expect("standalone VM should retain its Document resource authority");
    let loader = document_loader.request_client().clone();
    let task_runner = document_loader.task_runner();
    let lease = vm
        ._context_host
        .borrow_mut()
        .acquire_current_main_document_script_load_delay(
            owner,
            crate::frame_owner_model::MainDocumentScriptLoadDelayKind::Module,
        )
        .expect("dynamic script should acquire the current main Document lease");
    vm.document_runtime.note_dom_content_loaded_dispatched();
    vm.admit_main_document_runtime_script_task(
        &loader,
        task_runner,
        crate::host::RuntimeScriptAdmission::new(
            crate::host::RuntimeScriptAdmissionPayload::Script(script),
            lease,
        ),
    );
    assert_eq!(
        vm._context_host
            .borrow()
            .current_main_document_has_async_script_load_delay(owner),
        Some(true),
        "admission must retain lifecycle ownership before execution"
    );

    let DynamicScriptRunnable::Execute { id, script, .. } = vm
        .document_runtime
        .runtime_script_work_mut()
        .dynamic_scripts
        .next_runnable_script()
        .expect("inline dynamic script should be ready")
    else {
        panic!("expected dynamic script execution work");
    };
    let lease = vm
        .document_runtime
        .runtime_script_work_mut()
        .dynamic_scripts
        .finish_script_terminal(id)
        .expect("selected terminal must retain its acceptance-time lease");
    vm.apply_runtime_script_success_terminal(&script, lease);
    assert_eq!(
        vm._context_host
            .borrow()
            .current_main_document_has_async_script_load_delay(owner),
        Some(false),
        "the selected terminal must release its exact Document lease"
    );
    assert!(
        vm.document_runtime
            .runtime_script_work_mut()
            .dynamic_scripts
            .finish_script_terminal(id)
            .is_none(),
        "the same terminal cannot consume its lease twice"
    );
    while let Some(action) = take_main_document_runtime_action(&page_task_queue) {
        if let RendererPageMainDocumentRuntimeAction::ExecuteReadyPostParseWork(work) = action {
            assert!(
                !matches!(
                    work.into_post_parse_work(),
                    PostParsePageOwnedWork::Lifecycle(work)
                        if matches!(
                            work.as_ref(),
                            PostParseLifecycleWork::SettleMainDocumentScriptLoadDelay(_)
                        )
                ),
                "runtime terminal settlement must not create a second hidden lifecycle task"
            );
        }
    }
}

#[test]
fn runtime_owned_inline_importmap_bypasses_dcl_gate() {
    let _js_runtime = crate::JsRuntime::initialize();
    let document = HtmlParser.parse(
        Url::parse("https://example.com/").unwrap(),
        "<!doctype html><html><head></head><body></body></html>".to_owned(),
    );
    let page_task_queue = crate::page_task_queue::PageTaskQueueTestHarness::new();
    let post_domcontentloaded_page_task_sender =
        page_task_queue.owner_attached_runtime_page_task_sender_for_test();
    let script_event_parser_boundary_sender = page_task_queue.parser_boundary_sender();
    let mut vm = ScriptVmDefaultWorldBootstrap::standalone_from_dom_host_for_test(
        DomHost::from_dom(document),
        post_domcontentloaded_page_task_sender,
        script_event_parser_boundary_sender,
    )
    .expect("script vm bootstrap")
    .finish()
    .expect("script vm finish");
    let body = vm
        .document_runtime
        .snapshot_document()
        .document_body_handle()
        .expect("body should exist");
    let script_node = vm.document_runtime.dom_host_mut().create_element("script");
    assert!(
        vm.document_runtime
            .dom_host_mut()
            .append_child(body, script_node),
        "runtime-owned import map script node should attach"
    );
    let handle = vm
        .document_runtime
        .bind_runtime_owned_script_handle_for_node(script_node, "runtime-importmap-1");
    vm.document_runtime
        .set_script_handle_waits_until_dom_content_loaded(&handle);

    let script = PreparedScript {
        position: 1,
        node_id: script_node,
        kind: ScriptKind::ImportMap,
        mode: ScriptMode::ImportMapInOrder,
        source_kind: ScriptSourceKind::Inline,
        fetch_metadata: crate::planning::ScriptFetchMetadata::default(),
        source: ScriptSource::Inline("{\"imports\":{}}".to_owned()),
        url: Url::parse("https://example.com/").unwrap(),
        base_url: Url::parse("https://example.com/").unwrap(),
        initiator_url: Url::parse("https://example.com/").unwrap(),
        host_script_handle: Some(handle),
    };

    assert!(
        dynamic_script_execute_is_runnable_before_dom_content_loaded(&vm.document_runtime, &script),
        "dynamic inline import maps must register in the current flush"
    );
    assert!(
        !vm.prepared_script_uses_runtime_owned_page_task_execution(&script),
        "dynamic inline import maps should not wait for the runtime-owned page-task lane"
    );
}

#[test]
fn followup_task_boundary_preserves_pending_dynamic_source_residence() {
    let _js_runtime = crate::JsRuntime::initialize();
    let document = HtmlParser.parse(
        Url::parse("https://example.com/").unwrap(),
        "<!doctype html><html><head></head><body></body></html>".to_owned(),
    );
    let page_task_queue = crate::page_task_queue::PageTaskQueueTestHarness::new();
    let post_domcontentloaded_page_task_sender =
        page_task_queue.owner_attached_runtime_page_task_sender_for_test();
    let script_event_parser_boundary_sender = page_task_queue.parser_boundary_sender();
    let mut vm = ScriptVmDefaultWorldBootstrap::standalone_from_dom_host_for_test(
        DomHost::from_dom(document),
        post_domcontentloaded_page_task_sender,
        script_event_parser_boundary_sender,
    )
    .expect("script vm bootstrap")
    .finish()
    .expect("script vm finish");
    vm.enqueue_test_pending_runtime_source_load();

    assert!(vm.pause_runtime_script_work_at_followup_task_boundary(
        FollowupPageTaskDisposition::Enqueued
    ));
    assert_eq!(
        vm.document_runtime
            .runtime_script_work()
            .dynamic_scripts
            .pending_source_load_count_for_test(),
        1,
        "the explicit boundary must not manufacture or consume a source terminal"
    );
    assert!(
        vm.document_runtime
            .runtime_script_work_mut()
            .is_paused_for_deferred_page_tasks(),
        "the pending producer must retain a stable continuation residence"
    );
}

#[tokio::test]
async fn queued_pre_domcontentloaded_runtime_tasks_are_backfilled_ahead_of_domcontentloaded() {
    let _js_runtime = crate::JsRuntime::initialize();
    let document = HtmlParser.parse(
        Url::parse("https://example.com/").unwrap(),
        "<!doctype html><html><head></head><body></body></html>".to_owned(),
    );
    let mut page_task_queue = crate::page_task_queue::PageTaskQueueTestHarness::new();
    let post_domcontentloaded_page_task_sender =
        page_task_queue.owner_attached_runtime_page_task_sender_for_test();
    let script_event_parser_boundary_sender = page_task_queue.parser_boundary_sender();
    let mut vm = ScriptVmDefaultWorldBootstrap::standalone_from_dom_host_for_test(
        DomHost::from_dom(document),
        post_domcontentloaded_page_task_sender,
        script_event_parser_boundary_sender,
    )
    .expect("script vm bootstrap")
    .finish()
    .expect("script vm finish");
    let mut report = ScriptExecutionReport::default();

    let _driver = vm
        .start_post_parse_lifecycle_round(
            crate::renderer::PageVmInitStage::Load,
            &mut page_task_queue,
            &mut report,
            vec![],
        )
        .await;
    vm.document_runtime
        .enqueue_parser_owned_pre_domcontentloaded_page_owned_work(post_parse_lifecycle_work(
            PostParseLifecycleWork::RecordDetachedPostParseRuns(Vec::new()),
        ));

    assert!(matches!(
        vm.document_runtime
            .poll_next_post_parse_owner_driver_step(&mut page_task_queue, false),
        PostParseOwnerDriverStep::Ready(action)
            if matches!(
                action.as_ref(),
                DocumentProcessingAction::PostParsePageOwnedWork(work)
                    if matches!(
                        work.as_page_task(),
                        Some(PageTask::RecordDetachedPostParseRuns(_))
                    )
            )
    ));
    assert!(
        page_task_queue
            .post_parse_pop_front()
            .is_some_and(|work| work.is_main_document_interactive_task())
    );
    assert!(matches!(
        pop_post_parse_page_task(&mut page_task_queue),
        Some(PageTask::DispatchDomContentLoaded)
    ));
}

#[test]
fn selected_post_parse_action_keeps_pending_dynamic_source_out_of_pre_dcl_lane_after_dcl() {
    let _js_runtime = crate::JsRuntime::initialize();
    let document = HtmlParser.parse(
        Url::parse("https://example.com/").unwrap(),
        "<!doctype html><html><head></head><body></body></html>".to_owned(),
    );
    let mut page_task_queue = crate::page_task_queue::PageTaskQueueTestHarness::new();
    let post_domcontentloaded_page_task_sender =
        page_task_queue.owner_attached_runtime_page_task_sender_for_test();
    let script_event_parser_boundary_sender = page_task_queue.parser_boundary_sender();
    let mut vm = ScriptVmDefaultWorldBootstrap::standalone_from_dom_host_for_test(
        DomHost::from_dom(document),
        post_domcontentloaded_page_task_sender,
        script_event_parser_boundary_sender,
    )
    .expect("script vm bootstrap")
    .finish()
    .expect("script vm finish");
    vm.document_runtime.note_dom_content_loaded_dispatched();
    vm.enqueue_test_pending_runtime_source_load();

    vm.publish_post_parse_action_followups(&mut page_task_queue);

    assert!(
        !vm.document_runtime
            .has_parser_owned_pre_domcontentloaded_page_tasks(),
        "a pending dynamic source load must never manufacture parser-owned work"
    );
    assert!(
        vm.document_runtime
            .runtime_script_work_mut()
            .is_paused_for_deferred_page_tasks(),
        "the pending dynamic producer should stay on its runtime continuation state"
    );
    assert_eq!(
        vm.document_runtime.runtime_script_work_mut().pause_kind(),
        Some(RuntimeScriptWorkPauseKind::StablePageTurnContinuation)
    );
    assert!(
        take_main_document_runtime_action(&page_task_queue).is_none(),
        "an unresolved source must not manufacture a runnable continuation before its producer completes"
    );
}

#[test]
fn post_parse_owner_readiness_does_not_block_window_load_for_generic_post_dcl_pause_state() {
    let _js_runtime = crate::JsRuntime::initialize();
    let document = HtmlParser.parse(
        Url::parse("https://example.com/").unwrap(),
        "<!doctype html><html><head></head><body></body></html>".to_owned(),
    );
    let mut page_task_queue = crate::page_task_queue::PageTaskQueueTestHarness::new();
    let post_domcontentloaded_page_task_sender =
        page_task_queue.owner_attached_runtime_page_task_sender_for_test();
    let script_event_parser_boundary_sender = page_task_queue.parser_boundary_sender();
    let mut vm = ScriptVmDefaultWorldBootstrap::standalone_from_dom_host_for_test(
        DomHost::from_dom(document),
        post_domcontentloaded_page_task_sender,
        script_event_parser_boundary_sender,
    )
    .expect("script vm bootstrap")
    .finish()
    .expect("script vm finish");
    vm.document_runtime.note_dom_content_loaded_dispatched();
    vm.enqueue_test_pending_runtime_source_load();
    vm.arm_runtime_script_work_continuation_if_needed();
    page_task_queue.extend_post_parse_work([post_parse_lifecycle_work(
        PostParseLifecycleWork::test_window_load(),
    )]);

    let has_post_domcontentloaded_runtime_backlog =
        vm.has_post_domcontentloaded_runtime_backlog(&mut page_task_queue);
    assert!(has_post_domcontentloaded_runtime_backlog);
    let readiness = vm
        .document_runtime
        .post_parse_owner_readiness(&mut page_task_queue, false);

    assert!(!readiness.blocks_page_task_pop);
    assert!(!readiness.has_pending_progress_source);
}

#[test]
fn document_write_owned_inline_normal_defers_already_started_until_execution_without_handle() {
    let _js_runtime = crate::JsRuntime::initialize();
    let document = HtmlParser.parse(
        Url::parse("https://example.com/").unwrap(),
        "<!doctype html><html><head><script>window.test = 1;</script></head><body></body></html>"
            .to_owned(),
    );
    let script = document
        .script_handles()
        .first()
        .copied()
        .expect("inline script handle");
    let page_task_queue = crate::page_task_queue::PageTaskQueueTestHarness::new();
    let post_domcontentloaded_page_task_sender =
        page_task_queue.owner_attached_runtime_page_task_sender_for_test();
    let script_event_parser_boundary_sender = page_task_queue.parser_boundary_sender();
    let mut vm = ScriptVmDefaultWorldBootstrap::standalone_from_dom_host_for_test(
        DomHost::from_dom(document.clone()),
        post_domcontentloaded_page_task_sender,
        script_event_parser_boundary_sender,
    )
    .expect("script vm bootstrap")
    .finish()
    .expect("script vm finish");
    let mut prepared = PreparedScript {
        position: 0,
        node_id: NodeId::new(script.index()),
        kind: ScriptKind::Classic,
        mode: ScriptMode::Normal,
        source_kind: ScriptSourceKind::Inline,
        fetch_metadata: crate::planning::ScriptFetchMetadata::default(),
        source: ScriptSource::Inline("window.test = 1;".to_owned()),
        url: Url::parse("https://example.com/inline.js").unwrap(),
        base_url: Url::parse("https://example.com/inline.js").unwrap(),
        initiator_url: Url::parse("https://example.com/").unwrap(),
        host_script_handle: None,
    };

    assert!(
        !vm.document_runtime
            .snapshot_document()
            .node(script)
            .and_then(|node| node.as_element())
            .is_some_and(|element| element.script_already_started())
    );

    vm.bind_prepared_script_handle_if_needed(&mut prepared, ScriptHandleSource::DocumentWriteOwned);

    assert!(
        prepared.host_script_handle.is_none(),
        "inline classic normal still executes without a document-write host handle"
    );
    assert!(
        !vm.document_runtime
            .snapshot_document()
            .node(script)
            .and_then(|node| node.as_element())
            .is_some_and(|element| element.script_already_started()),
        "document.write-owned inline classic normal scripts are marked already-started when execution begins, not when planned"
    );
}

#[test]
fn script_inner_text_exposes_inline_json_payload() {
    let _js_runtime = crate::JsRuntime::initialize();
    let document = HtmlParser.parse(
            Url::parse("https://example.com/").unwrap(),
            "<!doctype html><html><body><script id=\"RENDER_DATA\" type=\"application/json\">%7B%22data%22%3A1%7D</script></body></html>"
                .to_owned(),
        );
    let page_task_queue = crate::page_task_queue::PageTaskQueueTestHarness::new();
    let post_domcontentloaded_page_task_sender =
        page_task_queue.owner_attached_runtime_page_task_sender_for_test();
    let script_event_parser_boundary_sender = page_task_queue.parser_boundary_sender();
    let mut vm = ScriptVmDefaultWorldBootstrap::standalone_from_dom_host_for_test(
        DomHost::from_dom(document),
        post_domcontentloaded_page_task_sender,
        script_event_parser_boundary_sender,
    )
    .expect("script vm bootstrap")
    .finish()
    .expect("script vm finish");

    let inner_text = vm
        .eval("document.getElementById('RENDER_DATA').innerText")
        .expect("script innerText should be readable");

    assert_eq!(inner_text, "%7B%22data%22%3A1%7D");
}

#[test]
fn indexed_db_globals_are_present() {
    let mut vm = new_storage_test_vm("https://indexeddb-globals.test/");

    let result = vm
            .eval(
                "(() => { const ev = new IDBVersionChangeEvent('upgradeneeded', { oldVersion: 1, newVersion: 2 }); return [typeof indexedDB, typeof indexedDB.open, typeof IDBIndex, typeof IDBCursor, typeof IDBCursorWithValue, typeof IDBKeyRange, typeof IDBVersionChangeEvent, `${ev.oldVersion}/${ev.newVersion}`].join('|'); })()",
            )
            .expect("indexeddb globals should exist");

    assert_eq!(
        result,
        "object|function|function|function|function|function|function|1/2"
    );
}

#[test]
fn indexed_db_declared_methods_have_webidl_operation_descriptors() {
    let mut vm = new_storage_test_vm("https://indexeddb-descriptors.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const methods = [
                [IDBFactory.prototype, "open", 2],
                [IDBFactory.prototype, "deleteDatabase", 1],
                [IDBFactory.prototype, "databases", 0],
                [IDBFactory.prototype, "cmp", 2],
                [IDBFactory.prototype, "databases", 0],
                [IDBDatabase.prototype, "createObjectStore", 2],
                [IDBDatabase.prototype, "deleteObjectStore", 1],
                [IDBDatabase.prototype, "transaction", 2],
                [IDBDatabase.prototype, "close", 0],
                [IDBDatabase.prototype, "addEventListener", 2],
                [IDBDatabase.prototype, "removeEventListener", 2],
                [IDBDatabase.prototype, "dispatchEvent", 1],
                [IDBTransaction.prototype, "objectStore", 1],
                [IDBTransaction.prototype, "abort", 0],
                [IDBTransaction.prototype, "commit", 0],
                [IDBTransaction.prototype, "addEventListener", 2],
                [IDBTransaction.prototype, "removeEventListener", 2],
                [IDBTransaction.prototype, "dispatchEvent", 1],
                [IDBRequest.prototype, "addEventListener", 2],
                [IDBRequest.prototype, "removeEventListener", 2],
                [IDBRequest.prototype, "dispatchEvent", 1],
                [IDBObjectStore.prototype, "get", 1],
                [IDBObjectStore.prototype, "getAll", 2],
                [IDBObjectStore.prototype, "getKey", 1],
                [IDBObjectStore.prototype, "getAllKeys", 2],
                [IDBObjectStore.prototype, "count", 1],
                [IDBObjectStore.prototype, "put", 2],
                [IDBObjectStore.prototype, "add", 2],
                [IDBObjectStore.prototype, "delete", 1],
                [IDBObjectStore.prototype, "clear", 0],
                [IDBObjectStore.prototype, "createIndex", 3],
                [IDBObjectStore.prototype, "index", 1],
                [IDBObjectStore.prototype, "deleteIndex", 1],
                [IDBObjectStore.prototype, "openCursor", 2],
                [IDBObjectStore.prototype, "openKeyCursor", 2],
                [IDBIndex.prototype, "get", 1],
                [IDBIndex.prototype, "getKey", 1],
                [IDBIndex.prototype, "getAll", 2],
                [IDBIndex.prototype, "getAllKeys", 2],
                [IDBIndex.prototype, "count", 1],
                [IDBIndex.prototype, "openCursor", 2],
                [IDBIndex.prototype, "openKeyCursor", 2],
                [IDBCursor.prototype, "advance", 1],
                [IDBCursor.prototype, "continue", 1],
                [IDBCursor.prototype, "continuePrimaryKey", 2],
                [IDBCursor.prototype, "update", 1],
                [IDBCursor.prototype, "delete", 0],
                [IDBKeyRange.prototype, "includes", 1],
                [IDBKeyRange, "only", 1],
                [IDBKeyRange, "bound", 4],
                [IDBKeyRange, "lowerBound", 2],
                [IDBKeyRange, "upperBound", 2],
              ];
              for (const [target, name, length] of methods) {
                const descriptor = Object.getOwnPropertyDescriptor(target, name);
                if (!descriptor || typeof descriptor.value !== "function") {
                  throw new Error(`${name} should be a declared function`);
                }
                if (descriptor.value.length !== length ||
                    descriptor.value.name !== name ||
                    descriptor.writable !== true ||
                    descriptor.enumerable !== true ||
                    descriptor.configurable !== true) {
                  throw new Error(`${name} descriptor should match Web IDL operation shape`);
                }
              }
              return "ok";
            })()
            "#,
        )
        .expect("indexeddb declared method descriptors should evaluate");

    assert_eq!(result, "ok");
}
