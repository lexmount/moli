use super::*;

#[test]
fn dynamic_import_invalid_attribute_key_rejects_with_type_error() {
    let mut vm = new_storage_test_vm("https://dynamic-import-attributes.test/page.html");

    vm.exec(
        r#"
        globalThis.__invalidDynamicImportAttribute = "pending";
        import("data:text/javascript,export%20default%201", { with: { foo: "bar" } })
          .then(() => {
            globalThis.__invalidDynamicImportAttribute = "unexpected";
          }, (error) => {
            globalThis.__invalidDynamicImportAttribute = JSON.stringify({
              name: error && error.name,
              type: error instanceof TypeError,
              message: String(error && error.message),
            });
          });
        "#,
        None,
    )
    .expect("dynamic import rejection setup should run");

    assert_eq!(
        vm.eval("globalThis.__invalidDynamicImportAttribute")
            .expect("dynamic import rejection should be observable"),
        r#"{"name":"TypeError","type":true,"message":"Invalid attribute key \"foo\"."}"#
    );
}
#[test]
fn string_timer_dynamic_import_keeps_captured_incumbent_script_base_url() {
    let mut vm = new_storage_test_vm("https://dynamic-import-timer.test/page/index.html");
    let script_url =
        Url::parse("https://dynamic-import-timer.test/scripts/nested/entry.js").unwrap();

    vm.exec(
        r#"setTimeout("import('../dependency.js')", 0);"#,
        Some(&script_url),
    )
    .expect("external script should queue a string timer");
    vm.exec(
        r#"
        const html = document.createElement('html');
        const head = document.createElement('head');
        html.append(head);
        document.append(html);
        const base = document.createElement('base');
        base.href = 'https://changed.example.test/assets/';
        head.append(base);
        "#,
        None,
    )
    .expect("document base mutation should run before the timer");
    assert!(matches!(
        vm.run_next_timeout_for_test()
            .expect("string timer should run"),
        crate::host::HostTimeoutRunResult::Consumed
    ));

    let request = vm
        .document_runtime
        .take_next_native_dynamic_module_import()
        .expect("timer source should queue a dynamic import")
        .into_dynamic_import_request();
    assert_eq!(request.specifier(), "../dependency.js");
    assert_eq!(request.base_url(), &script_url);
}
#[test]
fn reflected_event_handler_dynamic_import_uses_document_base_url() {
    let mut vm = new_storage_test_vm("https://dynamic-import-handler.test/page/index.html");
    let script_url =
        Url::parse("https://dynamic-import-handler.test/scripts/nested/entry.js").unwrap();

    vm.exec(
        r#"
const html = document.createElement('html');
const head = document.createElement('head');
const body = document.createElement('body');
html.append(head, body);
document.appendChild(html);
const base = document.createElement('base');
base.href = '../assets/';
head.appendChild(base);
const target = document.createElement('div');
target.setAttribute('onclick', "import('./dependency.js')");
body.appendChild(target);
target.onclick();
"#,
        Some(&script_url),
    )
    .expect("external script should invoke the reflected event handler");

    let request = vm
        .document_runtime
        .take_next_native_dynamic_module_import()
        .expect("event handler source should queue a dynamic import")
        .into_dynamic_import_request();
    assert_eq!(request.specifier(), "./dependency.js");
    assert_eq!(
        request.base_url().as_str(),
        "https://dynamic-import-handler.test/assets/"
    );
}
#[test]
fn script_turn_watchdog_terminates_runaway_script_and_recovers_isolate() {
    let _watchdog_timeout =
        crate::v8_execution_watchdog::V8ExecutionWatchdog::override_timeout_for_test(
            crate::v8_execution_watchdog::V8ExecutionWatchdogKind::ScriptTurn,
            std::time::Duration::from_millis(500),
        );
    let mut vm = new_parsed_test_vm("https://example.test/", "<!doctype html><body></body>");
    let started = Instant::now();
    let error = vm
        .exec("for (;;) {}", None)
        .expect_err("runaway script should be terminated");

    assert!(
        started.elapsed() < std::time::Duration::from_secs(4),
        "script watchdog should terminate the turn promptly"
    );
    assert!(
        error.to_string().contains("script execution exceeded"),
        "unexpected watchdog error: {error}"
    );
    assert_eq!(
        vm.eval("String(1 + 1)")
            .expect("isolate should remain usable after termination"),
        "2"
    );
}
#[test]
fn microtask_checkpoint_watchdog_terminates_runaway_queue_and_recovers_isolate() {
    let _watchdog_timeout =
        crate::v8_execution_watchdog::V8ExecutionWatchdog::override_timeout_for_test(
            crate::v8_execution_watchdog::V8ExecutionWatchdogKind::ScriptTurn,
            std::time::Duration::from_millis(500),
        );
    let mut vm = new_parsed_test_vm("https://example.test/", "<!doctype html><body></body>");
    let started = Instant::now();
    let error = vm
        .exec(
            "Promise.resolve().then(function loop() { queueMicrotask(loop); });",
            None,
        )
        .expect_err("runaway microtask checkpoint should be terminated");

    assert!(
        started.elapsed() < std::time::Duration::from_secs(4),
        "microtask watchdog should terminate the checkpoint promptly"
    );
    assert!(
        error.to_string().contains("microtask checkpoint exceeded"),
        "unexpected watchdog error: {error}"
    );
    assert_eq!(
        vm.eval("String(2 + 2)")
            .expect("isolate should remain usable after microtask termination"),
        "4"
    );
}
#[test]
fn runtime_await_promise_sync_result_survives_queued_allocation_gc() {
    let mut vm = new_parsed_test_vm(
        "https://runtime-await-promise-queued-gc.test/",
        "<!doctype html><body></body>",
    );

    // This callback is installed only in this test realm. It makes the race
    // deterministic: the page microtask runs after Inspector has converted the
    // synchronous string to an awaitable promise, but before Inspector's
    // reaction publishes the Runtime.evaluate response.
    let context_ptr: *const v8::Global<v8::Context> = &vm.page_default_context;
    vm.renderer_document_isolate
        .clone()
        .with_entered_renderer_document_isolate(|isolate| {
            let scope = std::pin::pin!(v8::HandleScope::new(isolate));
            let scope = &mut scope.init();
            let context = unsafe { v8::Local::new(scope, &*context_ptr) };
            let scope = &mut v8::ContextScope::new(scope, context);
            let callback = v8::Function::new(scope, force_gc_and_report_inspector_policy_for_test)
                .expect("test GC callback");
            let key = v8::String::new(scope, "__moliForceGcAndReportInspectorPolicyForTest")
                .expect("test GC callback key");
            assert!(
                context
                    .global(scope)
                    .set(scope, key.into(), callback.into())
                    == Some(true),
                "test GC callback should install"
            );
            Ok(())
        })
        .expect("test GC callback should install in the page realm");

    let messages = vm
        .dispatch_inspector_protocol_message(
            &serde_json::json!({
                "id": 91,
                "method": "Runtime.evaluate",
                "params": {
                    "expression": r#"(() => {
                        queueMicrotask(() => {
                            const values = [];
                            for (let index = 0; index < 5000; index += 1) {
                                values.push({ index, text: "x".repeat(128) });
                            }
                            globalThis.__queuedAllocationCount = values.length;
                            globalThis.__inspectorPolicyWasScoped =
                                __moliForceGcAndReportInspectorPolicyForTest();
                        });
                        return "sync-result";
                    })()"#,
                    "awaitPromise": true,
                    "returnByValue": true,
                }
            })
            .to_string(),
        )
        .expect("Runtime.evaluate dispatch");
    let response = messages
        .iter()
        .find(|message| message["id"] == serde_json::json!(91))
        .unwrap_or_else(|| panic!("Runtime.evaluate response missing: {messages:#?}"));
    assert!(
        response.get("error").is_none(),
        "a synchronous awaitPromise result must remain reachable through the Inspector checkpoint: {response:#?}"
    );
    assert_eq!(
        response["result"]["result"]["value"],
        serde_json::json!("sync-result")
    );
    assert_eq!(
        vm.eval("String(globalThis.__queuedAllocationCount)")
            .expect("queued allocation marker"),
        "5000",
        "the page microtask must run in the Runtime command checkpoint"
    );
    assert_eq!(
        vm.eval("String(globalThis.__inspectorPolicyWasScoped)")
            .expect("Inspector policy marker"),
        "true",
        "the queued page microtask must run inside Inspector's scoped checkpoint"
    );
    vm.renderer_document_isolate
        .clone()
        .with_entered_renderer_document_isolate(|isolate| {
            assert_eq!(
                isolate.get_microtasks_policy(),
                v8::MicrotasksPolicy::Explicit,
                "the document isolate must leave the Runtime dispatch with its owner policy restored"
            );
            Ok(())
        })
        .expect("inspect restored microtasks policy");

    let messages = vm
        .dispatch_inspector_protocol_message(
            &serde_json::json!({
                "id": 92,
                "method": "Runtime.evaluate",
                "params": {
                    "expression": "__moliForceGcAndReportInspectorPolicyForTest()",
                    "returnByValue": true,
                }
            })
            .to_string(),
        )
        .expect("non-await Runtime.evaluate dispatch");
    let response = messages
        .iter()
        .find(|message| message["id"] == serde_json::json!(92))
        .unwrap_or_else(|| panic!("non-await Runtime.evaluate response missing: {messages:#?}"));
    assert_eq!(
        response["result"]["result"]["value"],
        serde_json::json!(true),
        "non-await Inspector commands must use the same scoped dispatch boundary"
    );
    vm.renderer_document_isolate
        .clone()
        .with_entered_renderer_document_isolate(|isolate| {
            assert_eq!(
                isolate.get_microtasks_policy(),
                v8::MicrotasksPolicy::Explicit,
                "a non-await Inspector command must restore the document owner policy"
            );
            Ok(())
        })
        .expect("inspect policy after non-await command");
}
#[tokio::test]
async fn timer_callback_watchdog_terminates_runaway_timer_and_recovers_isolate() {
    let _watchdog_timeout =
        crate::v8_execution_watchdog::V8ExecutionWatchdog::override_timeout_for_test(
            crate::v8_execution_watchdog::V8ExecutionWatchdogKind::TimerCallback,
            std::time::Duration::from_millis(500),
        );
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_parsed_test_vm("https://example.test/", "<!doctype html><body></body>");
    vm.exec(
        "setTimeout(() => { for (;;) {} }, 0); window.__afterTimer = 1;",
        None,
    )
    .expect("timer setup should run");

    let started = Instant::now();
    vm.advance_timers_until_deadline_for_test(&loader)
        .await
        .expect("runaway timer should be reported without poisoning the isolate");

    assert!(
        started.elapsed() < std::time::Duration::from_secs(4),
        "timer watchdog should terminate the callback promptly"
    );
    assert_eq!(
        vm.eval("String(window.__afterTimer + 1)")
            .expect("isolate should remain usable after timer termination"),
        "2"
    );
}
