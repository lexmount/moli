use super::*;

fn record_resource_from_javascript_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let name = args.get(0).to_rust_string_lossy(scope);
    crate::context_bootstrap::record_resource_performance_entry(
        scope,
        crate::context_bootstrap::ResourcePerformanceEntry::without_network_result(
            name,
            "xmlhttprequest",
            None,
        ),
    );
    rv.set_undefined();
}

fn install_test_resource_recorder(vm: &mut ScriptVm) {
    vm.with_default_context_scope_and_checkpoint_for_test(|scope, _runtime_ptr| {
        let global = scope.get_current_context().global(scope);
        let function = v8::Function::builder(record_resource_from_javascript_callback)
            .build(scope)
            .ok_or_else(|| anyhow::anyhow!("failed to create resource timing test recorder"))?;
        let _ = global.define_own_property(
            scope,
            crate::util::v8str(scope, "__recordResourceTiming").into(),
            function.into(),
            v8::PropertyAttribute::DONT_ENUM,
        );
        Ok(())
    })
    .expect("resource timing test recorder should install");
}

fn record_failed_script_resource(vm: &mut ScriptVm, document_url: &Url, resource_index: usize) {
    let request_url = document_url
        .join(&format!("resource-{resource_index}.js"))
        .expect("resource URL");
    let result: std::result::Result<crate::types::NavigationResponse, String> =
        Err("synthetic resource failure".to_owned());
    vm.record_script_subresource_network_result(document_url.clone(), request_url, &result);
}

#[test]
fn resource_timing_ignores_non_http_resources() {
    let mut vm = new_storage_test_vm("https://resource-scheme.test/");
    install_test_resource_recorder(&mut vm);

    vm.eval(
        r#"
        __recordResourceTiming("data:image/gif;base64,R0lGODlhAQABAIAAAAUEBA==");
        __recordResourceTiming("blob:https://resource-scheme.test/id");
        __recordResourceTiming("https://resource-scheme.test/image.png");
        "#,
    )
    .expect("resource timing scheme records should evaluate");

    assert_eq!(
        vm.eval(
            r#"JSON.stringify(performance.getEntriesByType("resource").map(entry => entry.name))"#
        )
        .expect("resource timing scheme entries should evaluate"),
        r#"["https://resource-scheme.test/image.png"]"#
    );
}

#[test]
fn resource_timing_default_buffer_is_finite_and_fires_event_handler() {
    let document_url = Url::parse("https://resource-buffer-default.test/").expect("document URL");
    let mut vm = new_storage_test_vm(document_url.as_str());

    vm.eval(
        r#"
        globalThis.__resourceBufferFullCount = 0;
        performance.onresourcetimingbufferfull = () => ++__resourceBufferFullCount;
        "#,
    )
    .expect("resource buffer event handler should install");

    for index in 0..=250 {
        record_failed_script_resource(&mut vm, &document_url, index);
    }

    assert_eq!(
        vm.eval("performance.getEntriesByType('resource').length")
            .expect("resource entry count should evaluate"),
        "250"
    );
    assert_eq!(
        vm.eval("String(__resourceBufferFullCount)")
            .expect("pre-task event count should evaluate"),
        "0"
    );

    vm.run_next_timeout_for_test()
        .expect("resource buffer-full task should run");

    assert_eq!(
        vm.eval(
            "JSON.stringify({ count: __resourceBufferFullCount, entries: performance.getEntriesByType('resource').length })"
        )
        .expect("post-task resource buffer state should evaluate"),
        r#"{"count":1,"entries":250}"#
    );
}

#[test]
fn resource_timing_overflow_uses_secondary_buffer_and_ordered_event_handlers() {
    let document_url = Url::parse("https://resource-buffer-overflow.test/").expect("document URL");
    let mut vm = new_storage_test_vm(document_url.as_str());

    let surface = vm
        .eval(
            r#"
            (() => {
              globalThis.__resourceBufferEvents = [];
              globalThis.__resourceObserver = new PerformanceObserver(() => {});
              __resourceObserver.observe({ type: "resource" });
              performance.mark("keep-mark");
              performance.clearResourceTimings();
              performance.setResourceTimingBufferSize(1);
              performance.addEventListener("resourcetimingbufferfull", event => {
                __resourceBufferEvents.push([
                  "listener",
                  performance.getEntriesByType("resource").length,
                  event.target === performance,
                  event.bubbles,
                  event.cancelable,
                  event.isTrusted
                ].join(":"));
              });
              const handler = event => {
                __resourceBufferEvents.push([
                  "handler",
                  performance.getEntriesByType("resource").length,
                  event.target === performance
                ].join(":"));
                performance.clearResourceTimings();
              };
              performance.onresourcetimingbufferfull = handler;
              const clear = Object.getOwnPropertyDescriptor(
                Performance.prototype,
                "clearResourceTimings"
              );
              const resize = Object.getOwnPropertyDescriptor(
                Performance.prototype,
                "setResourceTimingBufferSize"
              );
              const onfull = Object.getOwnPropertyDescriptor(
                Performance.prototype,
                "onresourcetimingbufferfull"
              );
              return JSON.stringify({
                clear: [clear.value.name, clear.value.length, clear.enumerable].join(":"),
                resize: [resize.value.name, resize.value.length, resize.enumerable].join(":"),
                onfull: [
                  onfull.get.name,
                  onfull.set.name,
                  onfull.enumerable,
                  onfull.configurable
                ].join(":"),
                handlerRoundTrip: performance.onresourcetimingbufferfull === handler
              });
            })()
            "#,
        )
        .expect("resource timing buffer surface should evaluate");
    assert_eq!(
        surface,
        r#"{"clear":"clearResourceTimings:0:true","resize":"setResourceTimingBufferSize:1:true","onfull":"get onresourcetimingbufferfull:set onresourcetimingbufferfull:true:true","handlerRoundTrip":true}"#
    );

    record_failed_script_resource(&mut vm, &document_url, 0);
    record_failed_script_resource(&mut vm, &document_url, 1);

    let before_task = vm
        .eval(
            r#"
            JSON.stringify({
              primary: performance.getEntriesByType("resource").map(entry => entry.name),
              observed: __resourceObserver.takeRecords().map(entry => entry.name),
              events: __resourceBufferEvents,
              marks: performance.getEntriesByName("keep-mark", "mark").length
            })
            "#,
        )
        .expect("pre-task resource timing state should evaluate");
    assert_eq!(
        before_task,
        r#"{"primary":["https://resource-buffer-overflow.test/resource-0.js"],"observed":["https://resource-buffer-overflow.test/resource-0.js","https://resource-buffer-overflow.test/resource-1.js"],"events":[],"marks":1}"#
    );

    vm.run_next_timeout_for_test()
        .expect("resource buffer-full task should run");

    let after_task = vm
        .eval(
            r#"
            JSON.stringify({
              primary: performance.getEntriesByType("resource").map(entry => entry.name),
              events: __resourceBufferEvents,
              marks: performance.getEntriesByName("keep-mark", "mark").length
            })
            "#,
        )
        .expect("post-task resource timing state should evaluate");
    assert_eq!(
        after_task,
        r#"{"primary":["https://resource-buffer-overflow.test/resource-1.js"],"events":["listener:1:true:false:false:true","handler:1:true"],"marks":1}"#
    );
}

#[test]
fn resource_timing_growth_before_pending_task_copies_without_firing() {
    let document_url = Url::parse("https://resource-buffer-growth.test/").expect("document URL");
    let mut vm = new_storage_test_vm(document_url.as_str());

    vm.eval(
        r#"
        globalThis.__resourceBufferFullCount = 0;
        performance.addEventListener(
          "resourcetimingbufferfull",
          () => ++__resourceBufferFullCount
        );
        performance.clearResourceTimings();
        performance.setResourceTimingBufferSize(1);
        "#,
    )
    .expect("resource buffer growth setup should evaluate");

    record_failed_script_resource(&mut vm, &document_url, 0);
    record_failed_script_resource(&mut vm, &document_url, 1);
    assert_eq!(
        vm.eval(
            r#"
            performance.setResourceTimingBufferSize(2);
            String(performance.getEntriesByType("resource").length);
            "#,
        )
        .expect("resource buffer should resize before task"),
        "1"
    );

    vm.run_next_timeout_for_test()
        .expect("pending resource buffer task should run");

    assert_eq!(
        vm.eval(
            r#"
            performance.setResourceTimingBufferSize(1);
            JSON.stringify({
              entries: performance.getEntriesByType("resource").length,
              events: __resourceBufferFullCount
            });
            "#,
        )
        .expect("resource buffer growth result should evaluate"),
        r#"{"entries":2,"events":0}"#
    );
}

#[test]
fn resource_timing_clear_before_pending_task_keeps_secondary_entries() {
    let document_url = Url::parse("https://resource-buffer-clear.test/").expect("document URL");
    let mut vm = new_storage_test_vm(document_url.as_str());

    vm.eval(
        r#"
        globalThis.__resourceBufferFullCount = 0;
        performance.addEventListener(
          "resourcetimingbufferfull",
          () => ++__resourceBufferFullCount
        );
        performance.clearResourceTimings();
        performance.setResourceTimingBufferSize(1);
        "#,
    )
    .expect("resource buffer clear setup should evaluate");

    record_failed_script_resource(&mut vm, &document_url, 0);
    record_failed_script_resource(&mut vm, &document_url, 1);
    record_failed_script_resource(&mut vm, &document_url, 2);
    vm.eval(
        r#"
        performance.clearResourceTimings();
        performance.setResourceTimingBufferSize(3);
        "#,
    )
    .expect("primary resource buffer should clear and resize");
    record_failed_script_resource(&mut vm, &document_url, 3);

    assert_eq!(
        vm.eval("performance.getEntriesByType('resource').length")
            .expect("pre-task resource entry count should evaluate"),
        "0"
    );
    vm.run_next_timeout_for_test()
        .expect("pending resource buffer task should run");

    assert_eq!(
        vm.eval(
            r#"
            JSON.stringify({
              entries: performance.getEntriesByType("resource").map(entry => entry.name),
              events: __resourceBufferFullCount
            })
            "#,
        )
        .expect("post-task cleared resource buffer state should evaluate"),
        r#"{"entries":["https://resource-buffer-clear.test/resource-1.js","https://resource-buffer-clear.test/resource-2.js","https://resource-buffer-clear.test/resource-3.js"],"events":0}"#
    );
}

#[test]
fn resource_timing_added_during_full_event_stays_in_secondary_buffer() {
    let document_url = Url::parse("https://resource-buffer-reentrant.test/").expect("document URL");
    let mut vm = new_storage_test_vm(document_url.as_str());
    install_test_resource_recorder(&mut vm);

    vm.eval(
        r#"
        globalThis.__resourceBufferFullCount = 0;
        performance.clearResourceTimings();
        performance.setResourceTimingBufferSize(1);
        performance.addEventListener("resourcetimingbufferfull", () => {
          ++__resourceBufferFullCount;
          performance.setResourceTimingBufferSize(3);
          __recordResourceTiming(
            "https://resource-buffer-reentrant.test/resource-2.js"
          );
          if (performance.getEntriesByType("resource").length !== 1) {
            throw new Error("secondary entries became visible during the event");
          }
        });
        "#,
    )
    .expect("reentrant resource buffer setup should evaluate");

    record_failed_script_resource(&mut vm, &document_url, 0);
    record_failed_script_resource(&mut vm, &document_url, 1);
    vm.run_next_timeout_for_test()
        .expect("resource buffer-full task should run");

    assert_eq!(
        vm.eval(
            r#"
            JSON.stringify({
              entries: performance.getEntriesByType("resource").map(entry => entry.name),
              events: __resourceBufferFullCount
            })
            "#,
        )
        .expect("reentrant resource buffer result should evaluate"),
        r#"{"entries":["https://resource-buffer-reentrant.test/resource-0.js","https://resource-buffer-reentrant.test/resource-1.js","https://resource-buffer-reentrant.test/resource-2.js"],"events":1}"#
    );
}
