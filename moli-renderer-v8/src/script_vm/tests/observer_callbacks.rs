use super::*;

async fn observer_callback_test_vm(url: &str) -> crate::runtime::PageVmTaskExecutorTestHarness {
    let mut vm = new_storage_page_task_executor_test_vm(url);
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
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    vm.advance_timers_until_deadline_for_test(&loader)
        .await
        .expect("materialize observer callback child realm");
    vm
}

#[tokio::test(flavor = "current_thread")]
async fn observer_callbacks_use_webidl_callback_function_realm_receiver_and_proxy_semantics() {
    let mut vm =
        observer_callback_test_vm("https://observer-callback-function-semantics.test/").await;

    let queued = vm
        .eval(
            r#"
(() => {
  const child = __observerCallbackFrame.contentWindow;
  const target = document.body.appendChild(document.createElement("div"));
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

    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    vm.advance_timers_until_deadline_for_test(&loader)
        .await
        .expect("deliver rendering observers");
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
async fn observer_callback_exceptions_are_reported_to_the_callback_relevant_window() {
    let mut vm = observer_callback_test_vm("https://observer-callback-exception-realm.test/").await;

    vm.eval(
        r#"
(() => {
  const child = __observerCallbackFrame.contentWindow;
  const target = document.body.appendChild(document.createElement("div"));
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

    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    vm.advance_timers_until_deadline_for_test(&loader)
        .await
        .expect("deliver rendering observers");
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

#[tokio::test(flavor = "current_thread")]
async fn observer_deliveries_retire_with_the_exact_observer_or_callback_realm() {
    let mut vm = observer_callback_test_vm("https://observer-callback-retirement.test/").await;

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

    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    vm.advance_timers_until_deadline_for_test(&loader)
        .await
        .expect("deliver rendering observers");
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
    const target = document.createElement('div');
    resizeObserver.observe(target);
    if (index % 2) resizeObserver.unobserve(target);
    else resizeObserver.disconnect();

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
        crate::observer_runtime::callback_binding_count_for_test(
            &mut vm._context_host.borrow_mut()
        ),
        256,
        "each unreachable observer must still start with one exact callback identity binding"
    );

    vm.eval("delete globalThis.__observerCallbackCycles")
        .expect("observer callback cycles should become unreachable");

    vm.renderer_document_isolate_ops()
        .collect_renderer_document_isolate_garbage()
        .expect("renderer isolate GC should complete");

    assert_eq!(
        crate::observer_runtime::callback_binding_count_for_test(
            &mut vm._context_host.borrow_mut()
        ),
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
