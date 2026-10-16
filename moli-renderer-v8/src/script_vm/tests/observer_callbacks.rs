use super::*;

fn observer_callback_test_vm(url: &str) -> StandaloneScriptVmHarness {
    let mut vm = new_storage_test_vm(url);
    vm.eval(
        r#"
(() => {
  const root =
    document.documentElement ||
    document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  body.appendChild(frame);
  globalThis.__observerCallbackFrame = frame;
})()
"#,
    )
    .expect("observer callback child-realm setup should evaluate");
    materialize_single_child_default_realm_for_test(&mut vm, "observer callback child-realm setup");
    vm
}

async fn observer_callback_rendering_test_vm(
    url: &str,
    loader: &crate::network::ResourceRequestClient,
) -> crate::runtime::PageVmTaskExecutorTestHarness {
    let mut vm = new_storage_page_task_executor_test_vm(url);
    vm.eval(
        r#"
(() => {
  const root =
    document.documentElement ||
    document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  const target = document.createElement("div");
  target.style.cssText = "display:block;width:10px;height:20px";
  body.style.minHeight = "2000px";
  body.appendChild(frame);
  body.appendChild(target);
  globalThis.__observerCallbackFrame = frame;
  globalThis.__observerCallbackTarget = target;
  globalThis.__observerCallbackInitialLayout = false;
  requestAnimationFrame(() => {
    scrollTo(0, 1);
    globalThis.__observerCallbackInitialLayout = true;
  });
})()
"#,
    )
    .expect("observer callback child-realm setup should evaluate");
    assert_eq!(
        vm.eval("String(document.querySelector('iframe').contentWindow !== null)")
            .expect("observer callback child Window exposure should evaluate"),
        "true",
        "observer callback child Window exposure should prebootstrap its default realm"
    );
    assert!(
        vm.run_one_child_frame_task_executor_turn(
            ChildFrameSemanticTurnKind::RealmMaterialization,
            loader,
        )
        .await
        .expect("observer callback child realm materialization should run"),
        "observer callback child Window exposure should schedule one realm-materialization turn"
    );
    vm.advance_timers_until_deadline_for_test(loader)
        .await
        .expect("observer callback fixture animation frame should run");
    assert_eq!(
        vm.eval("String(__observerCallbackInitialLayout)")
            .expect("observer callback fixture animation-frame marker should be readable"),
        "true",
        "fixture animation frame must run before its rendering update"
    );
    assert!(
        vm.run_one_rendering_update_executor_turn(loader)
            .await
            .expect("observer callback fixture initial rendering update should run"),
        "animation-frame scroll should schedule the fixture's initial rendering update"
    );
    assert_eq!(
        vm.eval(
            "JSON.stringify([__observerCallbackTarget.getBoundingClientRect().width, \
             __observerCallbackTarget.getBoundingClientRect().height])"
        )
        .expect("observer callback fixture target geometry should be readable"),
        "[10,20]",
        "fixture target must have published geometry before observers are registered"
    );
    vm
}

#[test]
fn borrowed_performance_methods_use_receiver_timeline_and_entry_realms() {
    let mut vm = observer_callback_test_vm("https://performance-borrowed-method-realms.test/");
    let source = format!(
        "({})(window, __observerCallbackFrame.contentWindow).then(result => globalThis.__borrowedTimeline = result, error => globalThis.__borrowedTimeline = {{ error: String(error) }});",
        include_str!("performance_timeline_borrowed_methods.js"),
    );
    vm.eval(&source)
        .expect("queue borrowed mark and measure deliveries");
    assert_eq!(
        vm.eval("JSON.stringify(__borrowedTimeline)").unwrap(),
        r#"{"received":{"parent":["parent-mark","parent-measure"],"child":["child-mark","child-measure"]},"failures":[]}"#,
    );
}

#[test]
fn borrowed_performance_observe_reads_and_delivers_the_observers_own_buffer() {
    let mut vm = observer_callback_test_vm("https://performance-borrowed-buffered-observe.test/");
    vm.eval(
        r#"
(() => {
  const child = __observerCallbackFrame.contentWindow;
  globalThis.__borrowedBuffered = { parent: [], child: [] };
  performance.mark('parent-buffered');
  child.performance.mark('child-buffered');
  const parentObserver = new PerformanceObserver(list => {
    __borrowedBuffered.parent.push(...list.getEntries().map(entry => entry.name));
  });
  const childObserver = new child.PerformanceObserver(list => {
    __borrowedBuffered.child.push(...list.getEntries().map(entry => entry.name));
  });
  child.PerformanceObserver.prototype.observe.call(parentObserver, { type: 'mark', buffered: true });
  PerformanceObserver.prototype.observe.call(childObserver, { type: 'mark', buffered: true });
})()
"#,
    )
    .expect("subscribe to buffers using the other Realm's observe method");
    assert_eq!(
        vm.eval("JSON.stringify(__borrowedBuffered)").unwrap(),
        r#"{"parent":["parent-buffered"],"child":["child-buffered"]}"#,
    );
}

#[test]
fn borrowed_performance_delivery_survives_retirement_of_the_method_realm() {
    let mut vm = observer_callback_test_vm("https://performance-method-realm-retirement.test/");
    vm.eval(
        r#"
(() => {
  const child = __observerCallbackFrame.contentWindow;
  globalThis.__survivingTimeline = [];
  new PerformanceObserver(list => {
    __survivingTimeline.push(...list.getEntries().map(entry => entry.name));
  }).observe({ type: 'mark' });
  const mark = child.Performance.prototype.mark;
  mark.call(performance, 'before-method-realm-retirement');
  __observerCallbackFrame.remove();
  mark.call(performance, 'after-method-realm-retirement');
})()
"#,
    )
    .expect("retire the method Realm after queuing to the surviving timeline");
    assert_eq!(
        vm.eval("JSON.stringify(__survivingTimeline)").unwrap(),
        r#"["before-method-realm-retirement","after-method-realm-retirement"]"#,
    );
}

#[test]
fn common_callback_registry_keeps_active_observers_until_owner_teardown() {
    let mut vm = new_storage_test_vm("https://performance-observer-active-root.test/");
    let common = std::rc::Rc::clone(&vm._context_host.borrow().common);
    vm.eval(
        r#"
globalThis.__activeObserverEntries = [];
new PerformanceObserver(list => {
  __activeObserverEntries.push(...list.getEntries().map(entry => entry.name));
}).observe({ type: 'mark' });
"#,
    )
    .expect("register an observer with no independent JS root");
    vm.renderer_document_isolate_ops()
        .collect_renderer_document_isolate_garbage()
        .expect("collect unreachable observer wrappers");
    assert_eq!(common.callbacks.len(), 1);
    vm.eval("performance.mark('after-gc')")
        .expect("deliver to the active observer after GC");
    assert_eq!(
        vm.eval("JSON.stringify(__activeObserverEntries)").unwrap(),
        r#"["after-gc"]"#,
    );
    drop(vm);
    assert_eq!(common.callbacks.len(), 0);
    assert_eq!(common.finalizer_count(), 0);
}

#[test]
fn performance_observer_registration_uses_its_own_global_with_a_foreign_callback_realm() {
    let mut vm = observer_callback_test_vm("https://performance-observer-global-isolation.test/");
    vm.eval(
        r#"
(() => {
  const child = __observerCallbackFrame.contentWindow;
  globalThis.__timelineEntries = { parent: [], child: [], callbackRealms: [] };
  globalThis.__parentTimelineObserver = new PerformanceObserver(child.Function(
    'list',
    `parent.__timelineEntries.parent.push(...list.getEntries().map(entry => entry.name));
     parent.__timelineEntries.callbackRealms.push(globalThis === parent.__observerCallbackFrame.contentWindow);`
  ));
  globalThis.__childTimelineObserver = new child.PerformanceObserver(list => {
    __timelineEntries.child.push(...list.getEntries().map(entry => entry.name));
    __timelineEntries.callbackRealms.push(globalThis === window);
  });
  __parentTimelineObserver.observe({ type: 'mark' });
  __childTimelineObserver.observe({ type: 'mark' });
  performance.mark('parent-only');
  child.performance.mark('child-only');
})()
"#,
    )
    .expect("queue separate global timelines with crossed callback realms");
    assert_eq!(
        vm.eval("JSON.stringify(__timelineEntries)").unwrap(),
        r#"{"parent":["parent-only"],"child":["child-only"],"callbackRealms":[true,true]}"#,
    );
}

#[test]
fn document_preinsert_validation_precedes_mutation_and_adoption() {
    let mut vm = observer_callback_test_vm("https://document-preinsert.test/");
    assert_eq!(
        vm.eval(include_str!("document_preinsert_validation.js"))
            .expect("Document pre-insert validation should precede mutation and adoption"),
        "true"
    );
}

#[test]
fn windowless_fragment_insertions_preserve_native_mutation_records() {
    let mut vm = observer_callback_test_vm("https://windowless-fragments.test/");
    assert_eq!(
        vm.eval(include_str!("windowless_fragment_mutations.js"))
            .expect("fragment insertion should preserve native mutation records"),
        "true"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn observer_callbacks_use_webidl_callback_function_realm_receiver_and_proxy_semantics() {
    let loader = crate::network::ResourceRequestClient::new(&moli_fetch::FetchConfig::default())
        .expect("loader");
    let mut vm = observer_callback_rendering_test_vm(
        "https://observer-callback-function-semantics.test/",
        &loader,
    )
    .await;

    let queued = vm
        .eval(
            r#"
(() => {
  const child = __observerCallbackFrame.contentWindow;
  const target = __observerCallbackTarget;
  globalThis.__observerCallbackFacts = [];
  globalThis.__observerProxyCalls = [];
  globalThis.__observerCallbackChild = child;

  const makeCallback = (label) => child.Function(
    "label",
    `return new Proxy(
      function(records, observer, options) {
        const recordCount =
          label === "performance" ? records.getEntries().length : records.length;
        parent.__observerCallbackFacts.push({
          label,
          callbackRealm: globalThis === parent.__observerCallbackChild,
          receiver: this === observer,
          observerArgument: observer === parent["__" + label + "Observer"],
          recordCount,
          argumentCount: arguments.length,
          optionsObject:
            label !== "performance" ||
            (options !== null && typeof options === "object")
        });
      },
      {
        apply(target, receiver, args) {
          parent.__observerProxyCalls.push(label);
          return Reflect.apply(target, receiver, args);
        }
      }
    );`
  )(label);

  const mutationObserver =
    new MutationObserver(makeCallback("mutation"));
  globalThis.__mutationObserver = mutationObserver;
  mutationObserver.observe(target, { attributes: true });
  target.setAttribute("data-observer-callback", "queued");

  const intersectionObserver =
    new IntersectionObserver(makeCallback("intersection"));
  globalThis.__intersectionObserver = intersectionObserver;
  intersectionObserver.observe(target);

  const resizeObserver =
    new ResizeObserver(makeCallback("resize"));
  globalThis.__resizeObserver = resizeObserver;
  resizeObserver.observe(target);

  const performanceObserver =
    new PerformanceObserver(makeCallback("performance"));
  globalThis.__performanceObserver = performanceObserver;
  performanceObserver.observe({ type: "mark" });
  performance.mark("observer-callback-semantics");

  return "queued";
})()
"#,
        )
        .expect("observer callback-function semantics should queue");
    assert_eq!(queued, "queued");
    assert!(
        vm.run_one_rendering_update_executor_turn(&loader)
            .await
            .expect("ResizeObserver rendering update should run")
    );

    let result = vm
        .eval(
            r#"
JSON.stringify({
  facts: globalThis.__observerCallbackFacts.sort((a, b) =>
    a.label.localeCompare(b.label)
  ),
  proxyCalls: globalThis.__observerProxyCalls.sort()
})
"#,
        )
        .expect("observer callback-function semantics should flush");
    assert_eq!(
        result,
        r#"{"facts":[{"label":"intersection","callbackRealm":true,"receiver":true,"observerArgument":true,"recordCount":1,"argumentCount":2,"optionsObject":true},{"label":"mutation","callbackRealm":true,"receiver":true,"observerArgument":true,"recordCount":1,"argumentCount":2,"optionsObject":true},{"label":"performance","callbackRealm":true,"receiver":true,"observerArgument":true,"recordCount":1,"argumentCount":3,"optionsObject":true},{"label":"resize","callbackRealm":true,"receiver":true,"observerArgument":true,"recordCount":1,"argumentCount":2,"optionsObject":true}],"proxyCalls":["intersection","mutation","performance","resize"]}"#
    );
}

#[tokio::test(flavor = "current_thread")]
async fn intersection_entries_use_the_borrowed_callback_realm_in_both_layout_modes() {
    for policy in [
        moli_page_types::LayoutPolicy::Mock,
        moli_page_types::LayoutPolicy::OnDemand,
    ] {
        let mut vm = observer_callback_test_vm("https://intersection-entry-realm.test/");
        vm.set_layout_policy(policy);
        vm.eval(
            r#"
            globalThis.rows = [];
            globalThis.child = __observerCallbackFrame.contentWindow;
            const target = document.body.appendChild(document.createElement('div'));
            target.id = 'intersection-target';
            target.style.cssText = 'width:100px;height:20px;padding:0;border:0';
        "#,
        )
        .unwrap();
        let before_ready = vm.layout_pass_observability_for_test().1;
        if policy == moli_page_types::LayoutPolicy::OnDemand {
            // Legacy observers sample an already-published tree. Establish the
            // initial sample after fixture setup, before observer registration.
            assert_eq!(
                vm.eval(
                    "document.getElementById('intersection-target').getBoundingClientRect().width"
                )
                .unwrap(),
                "100"
            );
        }
        let builds = vm.layout_pass_observability_for_test().1;
        assert_eq!(
            builds,
            before_ready + u64::from(policy == moli_page_types::LayoutPolicy::OnDemand),
            "readiness publishes one initial real layout and none for Mock: {policy:?}"
        );
        vm.eval(
            r#"
            globalThis.rootIO = new IntersectionObserver(child.Function('entries', 'observer', `
                const entry = entries[0];
                parent.rows.push({
                    callbackRealm: window === parent.child,
                    observerRealm: Object.getPrototypeOf(observer) === parent.IntersectionObserver.prototype,
                    receiver: this === observer && observer === parent.rootIO,
                    arrayRealm: Object.getPrototypeOf(entries) === Array.prototype,
                    entryRealm: Object.getPrototypeOf(entry) === IntersectionObserverEntry.prototype,
                    rectRealm: [entry.boundingClientRect, entry.intersectionRect, entry.rootBounds].every(
                        rect => rect !== null && DOMRectReadOnly.prototype.isPrototypeOf(rect)),
                    target: entry.target === parent.document.getElementById('intersection-target'),
                    count: entries.length,
                    argc: arguments.length
                });
                parent.entryWidth = entry.boundingClientRect.width;
            `));
            rootIO.observe(target);
        "#,
        )
        .unwrap();
        assert_eq!(
            vm.eval("JSON.stringify(rows)").unwrap(),
            r#"[{"callbackRealm":true,"observerRealm":true,"receiver":true,"arrayRealm":true,"entryRealm":true,"rectRealm":true,"target":true,"count":1,"argc":2}]"#,
            "{policy:?}"
        );
        if policy == moli_page_types::LayoutPolicy::OnDemand {
            assert_eq!(vm.eval("entryWidth").unwrap(), "100");
        }
        assert_eq!(
            vm.layout_pass_observability_for_test().1,
            builds,
            "observer registration, delivery and record reads must reuse the initial sample: {policy:?}"
        );
    }
}

#[tokio::test(flavor = "current_thread")]
async fn intersection_entries_project_targets_into_the_isolated_callback_world() {
    for policy in [
        moli_page_types::LayoutPolicy::Mock,
        moli_page_types::LayoutPolicy::OnDemand,
    ] {
        let mut vm = new_storage_test_vm("https://intersection-isolated-world.test/");
        vm.set_layout_policy(policy);
        vm.eval(
            r#"
            const root = document.documentElement || document.appendChild(document.createElement('html'));
            const body = document.body || root.appendChild(document.createElement('body'));
            const target = body.appendChild(document.createElement('div'));
            target.id = 'intersection-target';
            target.style.cssText = 'width:100px;height:20px;padding:0;border:0';
        "#,
        )
        .unwrap();
        let isolated = vm
            .create_isolated_world("intersection-entry-realm", false)
            .unwrap();
        vm.eval_in_isolated_context(
            isolated,
            r#"
            globalThis.rows = [];
            globalThis.observer = new IntersectionObserver(function(entries, observer) {
                const entry = entries[0];
                rows.push([
                    this === observer && observer === globalThis.observer,
                    Object.getPrototypeOf(entries) === Array.prototype,
                    Object.getPrototypeOf(entry) === IntersectionObserverEntry.prototype,
                    [entry.boundingClientRect, entry.intersectionRect, entry.rootBounds].every(
                        rect => rect !== null && DOMRectReadOnly.prototype.isPrototypeOf(rect)),
                    entry.target === document.getElementById('intersection-target'),
                    HTMLDivElement.prototype.isPrototypeOf(entry.target),
                    entries.length === 1 && arguments.length === 2
                ]);
            });
            observer.observe(document.getElementById('intersection-target'));
        "#,
        )
        .unwrap();
        assert_eq!(
            vm.eval_in_isolated_context(isolated, "JSON.stringify(rows)")
                .unwrap(),
            "[[true,true,true,true,true,true,true]]",
            "{policy:?}"
        );
        vm.destroy_isolated_world_context(isolated);
        assert_eq!(
            vm.eval("'default-realm-still-live'").unwrap(),
            "default-realm-still-live"
        );
    }
}

#[tokio::test(flavor = "current_thread")]
async fn observer_callback_exceptions_are_reported_to_the_callback_relevant_window() {
    let loader = crate::network::ResourceRequestClient::new(&moli_fetch::FetchConfig::default())
        .expect("loader");
    let mut vm = observer_callback_rendering_test_vm(
        "https://observer-callback-exception-realm.test/",
        &loader,
    )
    .await;

    vm.eval(
        r#"
(() => {
  const child = __observerCallbackFrame.contentWindow;
  const target = __observerCallbackTarget;
  globalThis.__observerCallbackErrors = [];
  child.addEventListener("error", child.Function(
    "event",
    `parent.__observerCallbackErrors.push({
       message: event.error && event.error.message,
       errorRealm: event.error instanceof Error,
       targetRealm: event.currentTarget === globalThis
     });
     event.preventDefault();`
  ));

  const mutationObserver = new MutationObserver(
    child.Function(`throw new Error("mutation-observer-error");`)
  );
  mutationObserver.observe(target, { attributes: true });
  target.setAttribute("data-observer-error", "queued");

  const intersectionObserver = new IntersectionObserver(
    child.Function(`throw new Error("intersection-observer-error");`)
  );
  intersectionObserver.observe(target);

  const resizeObserver = new ResizeObserver(
    child.Function(`throw new Error("resize-observer-error");`)
  );
  resizeObserver.observe(target);

  const performanceObserver = new PerformanceObserver(
    child.Function(`throw new Error("performance-observer-error");`)
  );
  performanceObserver.observe({ type: "mark" });
  performance.mark("observer-callback-error");
  return "queued";
})()
"#,
    )
    .expect("observer exception projection should queue");
    assert!(
        vm.run_one_rendering_update_executor_turn(&loader)
            .await
            .expect("ResizeObserver rendering update should run")
    );

    let result = vm
        .eval(
            "JSON.stringify(globalThis.__observerCallbackErrors.sort((a, b) => a.message.localeCompare(b.message)))",
        )
        .expect("observer exception projection should flush");
    assert_eq!(
        result,
        r#"[{"message":"intersection-observer-error","errorRealm":true,"targetRealm":true},{"message":"mutation-observer-error","errorRealm":true,"targetRealm":true},{"message":"performance-observer-error","errorRealm":true,"targetRealm":true},{"message":"resize-observer-error","errorRealm":true,"targetRealm":true}]"#
    );
}

#[test]
fn observer_deliveries_retire_with_the_exact_observer_or_callback_realm() {
    let mut vm = observer_callback_test_vm("https://observer-callback-retirement.test/");

    let queued = vm
        .eval(
            r#"
(() => {
  const frame = __observerCallbackFrame;
  const child = frame.contentWindow;
  const target = document.body.appendChild(document.createElement("div"));
  globalThis.__retiredObserverCallbacks = [];
  const detachedCallback = child.Function(
    `parent.__retiredObserverCallbacks.push("already-retired-callback-realm");`
  );

  // The observer belongs to the parent Window while the callback's relevant
  // Realm belongs to the child Window.
  const mutationObserver = new MutationObserver(
    child.Function(
      `parent.__retiredObserverCallbacks.push("retired-callback-realm");`
    )
  );
  mutationObserver.observe(target, { attributes: true });
  target.setAttribute("data-retired-observer", "queued");

  // This observer object belongs to the child Window while its callback
  // belongs to the still-live parent Window.
  const intersectionObserver = new child.IntersectionObserver(() => {
    globalThis.__retiredObserverCallbacks.push("retired-observer-realm");
  });
  intersectionObserver.observe(target);

  const resizeObserver = new ResizeObserver(
    child.Function(
      `parent.__retiredObserverCallbacks.push("retired-resize-callback-realm");`
    )
  );
  resizeObserver.observe(target);

  const performanceObserver = new child.PerformanceObserver(() => {
    globalThis.__retiredObserverCallbacks.push("retired-performance-observer-realm");
  });
  performanceObserver.observe({ type: "mark" });
  child.performance.mark("retired-performance-observer");

  const performanceCallbackRealmObserver = new PerformanceObserver(
    child.Function(
      `parent.__retiredObserverCallbacks.push("retired-performance-callback-realm");`
    )
  );
  performanceCallbackRealmObserver.observe({ type: "mark" });
  performance.mark("retired-performance-callback");

  frame.remove();

  // Registration after retirement must not turn an unresolved Window
  // identity into an immortal callback residence.
  const observerWithDetachedCallback = new MutationObserver(detachedCallback);
  observerWithDetachedCallback.observe(target, { attributes: true });
  target.setAttribute("data-retired-observer", "detached-callback");

  const resizeObserverWithDetachedCallback = new ResizeObserver(detachedCallback);
  resizeObserverWithDetachedCallback.observe(target);

  return String(frame.contentWindow === null);
})()
"#,
        )
        .expect("observer retirement should queue");
    assert_eq!(queued, "true");

    let result = vm
        .eval("JSON.stringify(globalThis.__retiredObserverCallbacks)")
        .expect("retired observer callbacks should remain suppressed");
    assert_eq!(result, "[]");
}

#[test]
fn js_owned_observer_callback_cycles_remain_v8_collectable() {
    let mut vm = new_storage_test_vm("https://resize-observer-callback-gc.test/");

    let created = vm
        .eval(
            r#"
(() => {
  // Keep every observer reachable until Rust has observed the complete
  // callback-binding population. V8 may otherwise collect early loop
  // iterations before this evaluation returns, which conflates registration
  // with the unreachable-cycle collection phase exercised below.
  globalThis.__observerCallbackCycles = [];
  for (let index = 0; index < 128; index++) {
    let resizeObserver;
    resizeObserver = new ResizeObserver(() => resizeObserver.disconnect());

    let performanceObserver;
    performanceObserver = new PerformanceObserver(
      () => performanceObserver.disconnect()
    );
    performanceObserver.observe({ type: "mark" });
    performanceObserver.disconnect();

    globalThis.__observerCallbackCycles.push(
      resizeObserver,
      performanceObserver
    );
  }
  return 256;
})()
"#,
        )
        .expect("JS-owned observer callback cycles should be created");
    assert_eq!(created, "256");
    assert_eq!(
        vm._context_host.borrow().common.callbacks.len(),
        256,
        "each unreachable observer must still start with one exact callback identity binding"
    );

    vm.eval("delete globalThis.__observerCallbackCycles")
        .expect("observer callback cycles should become unreachable");

    vm.renderer_document_isolate_ops()
        .collect_renderer_document_isolate_garbage()
        .expect("renderer isolate GC should complete");

    assert_eq!(
        vm._context_host.borrow().common.callbacks.len(),
        0,
        "a Rust callback registry must not root an unreachable callback-observer cycle"
    );
}

#[test]
fn performance_observer_disconnect_retires_and_reobserve_restores_delivery() {
    let mut vm = new_storage_test_vm("https://performance-observer-reobserve.test/");

    vm.eval(
        r#"
(() => {
  globalThis.__performanceReobserveEntries = [];
  const observer = new PerformanceObserver(list => {
    __performanceReobserveEntries.push(
      ...list.getEntries().map(entry => entry.name)
    );
  });
  observer.observe({ type: "mark" });
  performance.mark("cleared-by-disconnect");
  observer.disconnect();
  performance.mark("ignored-while-disconnected");
  observer.observe({ type: "mark" });
  performance.mark("delivered-after-reobserve");
})()
"#,
    )
    .expect("PerformanceObserver re-observe workflow should queue");

    let delivered = vm
        .eval("JSON.stringify(globalThis.__performanceReobserveEntries)")
        .expect("PerformanceObserver re-observe workflow should flush");
    assert_eq!(delivered, r#"["delivered-after-reobserve"]"#);
}

#[tokio::test(flavor = "current_thread")]
async fn mutation_record_node_lists_are_native_readonly_snapshots() {
    let mut vm = observer_callback_test_vm("https://mutation-record-node-lists.test/");
    vm.eval(include_str!("mutation_record_node_lists.js"))
        .expect("MutationRecord NodeList fixture should evaluate");
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    vm.advance_timers_until_deadline_for_test(&loader)
        .await
        .expect("deliver mutation records");
    assert_eq!(
        vm.eval("String(globalThis.__mutationRecordNodeListResult)")
            .unwrap(),
        "true"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn dom_implementation_derives_receiver_and_argument_conversion_in_each_document_realm() {
    let loader = static_http_loader(std::iter::empty::<String>());
    let mut vm = new_page_task_executor_test_vm_with_loader(
        "https://dom-implementation-arguments.test/",
        &loader,
    );
    // Actual popup Pages run the same fixture over CDP.
    vm.exec("globalThis.__domImplementationIncludePopup = false;", None)
        .unwrap();
    vm.exec(include_str!("dom_implementation_arguments.js"), None)
        .unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(globalThis.__uiEventResults?.complete === true)",
        "true",
        "DOMImplementation receiver and argument conversion",
    )
    .await;
    assert_eq!(
        vm.eval("JSON.stringify(__uiEventResults.checks.filter(row => !row.passed))")
            .unwrap(),
        "[]",
    );
    assert_eq!(vm.eval("__uiEventResults.includePopup").unwrap(), "false");
    assert_eq!(vm.eval("__uiEventResults.total").unwrap(), "4344");
}

#[tokio::test(flavor = "current_thread")]
async fn element_creation_options_convert_before_dom_validation_in_each_document_realm() {
    let loader = static_http_loader(std::iter::empty::<String>());
    let mut vm = new_page_task_executor_test_vm_with_loader(
        "https://element-creation-options.test/",
        &loader,
    );
    // Real popup Pages run the same fixture over CDP.
    vm.exec("globalThis.__elementOptionsIncludePopup = false;", None)
        .unwrap();
    vm.exec(include_str!("element_creation_options.js"), None)
        .unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(globalThis.__uiEventResults?.complete === true)",
        "true",
        "element creation options conversion",
    )
    .await;
    assert_eq!(
        vm.eval("JSON.stringify(__uiEventResults.checks.filter(row => !row.passed))")
            .unwrap(),
        "[]",
    );
    assert_eq!(vm.eval("__uiEventResults.includePopup").unwrap(), "false");
    assert_eq!(vm.eval("__uiEventResults.total").unwrap(), "3106");
}
