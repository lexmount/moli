use super::*;

#[test]
fn observable_from_iterables_promises_cancellation_and_exception_timing() {
    let mut vm = new_storage_test_vm("https://observable-from.test/");
    vm.eval(&format!(
        "({}).then(value => {{ globalThis.fromResult = JSON.stringify(value); }});",
        include_str!("../../../tests/fixtures/observable-from.js")
    ))
    .expect("Observable.from fixture should evaluate");
    let result = vm
        .eval("fromResult")
        .expect("Observable.from fixture should settle");
    let result: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(result["failures"], serde_json::json!([]), "{result}");
    assert!(result["checks"].as_u64().unwrap() >= 80, "{result}");
}

#[test]
fn observable_from_inputs_and_abandoned_iterators_are_collectible() {
    let mut vm = new_storage_test_vm("https://observable-from-gc.test/");
    vm.eval(r#"
(() => {
  const input = [7];
  globalThis.weakFromInput = new WeakRef(input);
  globalThis.keptFrom = Observable.from(input);
})();
(() => {
  const captured = {}, iterator = {next: () => new Promise(() => {})};
  globalThis.weakFromObserver = new WeakRef(captured);
  globalThis.weakFromIterator = new WeakRef(iterator);
  Observable.from({[Symbol.asyncIterator]: () => iterator}).subscribe(() => captured);
})();
(() => {
  const captured = {}, iterator = {next: () => new Promise(resolve => { globalThis.resolveFrom = resolve; })};
  globalThis.weakLiveFromObserver = new WeakRef(captured);
  globalThis.weakLiveFromIterator = new WeakRef(iterator);
  Observable.from({[Symbol.asyncIterator]: () => iterator}).subscribe(() => captured);
})();
"#).unwrap();
    let collect = |vm: &mut StandaloneScriptVmHarness| {
        vm.renderer_document_isolate
            .clone()
            .with_entered_renderer_document_isolate(|isolate| {
                isolate.clear_kept_objects();
                isolate.low_memory_notification();
                Ok(())
            })
            .unwrap();
    };
    collect(&mut vm);
    assert_eq!(
        vm.eval(
            r#"JSON.stringify([
weakFromInput.deref() !== undefined,
weakFromObserver.deref() === undefined, weakFromIterator.deref() === undefined,
weakLiveFromObserver.deref() !== undefined, weakLiveFromIterator.deref() !== undefined
])"#
        )
        .unwrap(),
        "[true,true,true,true,true]"
    );
    vm.eval("delete globalThis.keptFrom; resolveFrom({done:true}); delete globalThis.resolveFrom;")
        .unwrap();
    collect(&mut vm);
    assert_eq!(
        vm.eval(
            r#"JSON.stringify([
weakFromInput.deref() === undefined,
weakLiveFromObserver.deref() === undefined, weakLiveFromIterator.deref() === undefined
])"#
        )
        .unwrap(),
        "[true,true,true]"
    );
}

#[test]
fn observable_from_uses_callee_realm_and_preserves_foreign_observables() {
    let mut vm = new_storage_test_vm("https://observable-from-realms.test/");
    vm.eval("document.appendChild(document.createElement('iframe'))")
        .unwrap();
    materialize_single_child_default_realm_for_test(&mut vm, "Observable.from realm");
    assert_eq!(vm.eval(r#"
JSON.stringify((() => {
  const child = document.querySelector('iframe').contentWindow, checks = [];
  const native = new child.Observable(() => {});
  Object.defineProperty(native, Symbol.asyncIterator, {get() { throw 1; }});
  checks.push(Observable.from(native) === native, child.Observable.from(native) === native);
  const source = child.Observable.from([1]);
  checks.push(source instanceof child.Observable, !(source instanceof Observable));
  try { child.Observable.from(1); } catch (e) { checks.push(e instanceof child.TypeError, !(e instanceof TypeError)); }
  const input = {[Symbol.iterator]() { return null; }};
  child.Observable.from(input).subscribe({error(e) { checks.push(e instanceof child.TypeError, !(e instanceof TypeError)); }});
  return checks;
})())
"#).unwrap(), "[true,true,true,true,true,true,true,true]");
}

#[test]
fn observable_event_target_listener_lifecycle_and_dom_propagation() {
    let mut vm = new_storage_test_vm("https://observable-events.test/");
    vm.eval("document.appendChild(document.createElement('html')); document.documentElement.appendChild(document.createElement('body'));").unwrap();
    let result = vm
        .eval(&format!(
            "JSON.stringify({})",
            include_str!("../../../tests/fixtures/observable-event-target.js")
        ))
        .expect("EventTarget.when fixture should evaluate");
    let result: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(result["failures"], serde_json::json!([]), "{result}");
    assert!(result["checks"].as_u64().unwrap() >= 70, "{result}");
}

#[test]
fn observable_event_target_borrowed_methods_keep_target_and_callback_realms() {
    let mut vm = new_storage_test_vm("https://observable-event-realms.test/");
    vm.eval("document.appendChild(document.createElement('iframe'))")
        .unwrap();
    materialize_single_child_default_realm_for_test(&mut vm, "EventTarget.when child realm");
    let result = vm.eval(r#"
JSON.stringify((() => {
  const frame = document.querySelector('iframe'), child = frame.contentWindow;
  const checks = [], events = [], ac = new AbortController();
  let conversions = 0;
  try {
    child.EventTarget.prototype.when.call({}, {toString() { conversions++; return 'test'; }});
    checks.push(false);
  } catch (error) {
    checks.push(error instanceof child.TypeError && !(error instanceof TypeError));
  }
  checks.push(conversions === 0);
  const source = EventTarget.prototype.when.call(child, 'test');
  checks.push(source instanceof Observable && !(source instanceof child.Observable));
  source.subscribe(event => events.push(event.type), {signal: ac.signal});
  dispatchEvent(new Event('test'));
  child.dispatchEvent(new child.Event('test'));
  ac.abort();
  child.dispatchEvent(new child.Event('test'));
  const parentTarget = new EventTarget(), childSource = child.EventTarget.prototype.when.call(parentTarget, 'child');
  checks.push(childSource instanceof child.Observable);
  childSource.subscribe(event => events.push(event.type));
  parentTarget.dispatchEvent(new Event('child'));
  const pending = EventTarget.prototype.when.call(child, 'retired');
  frame.remove();
  pending.subscribe(() => events.push('retired'));
  parentTarget.dispatchEvent(new Event('child'));
  return {checks, events};
})())
"#).unwrap();
    assert_eq!(
        result,
        r#"{"checks":[true,true,true,true],"events":["test","child"]}"#
    );
}

#[test]
fn observable_event_target_does_not_keep_its_target_alive() {
    let mut vm = new_storage_test_vm("https://observable-events-gc.test/");
    vm.eval(
        r#"
(() => {
  const target = new EventTarget();
  globalThis.__events = target.when('test');
  globalThis.__weakTarget = new WeakRef(target);
})()
"#,
    )
    .unwrap();
    vm.renderer_document_isolate
        .clone()
        .with_entered_renderer_document_isolate(|isolate| {
            isolate.clear_kept_objects();
            isolate.low_memory_notification();
            Ok(())
        })
        .unwrap();
    assert_eq!(
        vm.eval(
            r#"
JSON.stringify([__weakTarget.deref() === undefined,
  __events.subscribe(() => { throw new Error('collected target'); }) === undefined])
"#
        )
        .unwrap(),
        "[true,true]"
    );
}

#[tokio::test]
async fn observable_event_target_window_sources_do_not_follow_navigation() {
    let mut vm = new_parsed_test_vm(
        "https://observable-navigation.test/",
        "<!doctype html><html><body></body></html>",
    );
    vm.eval(
        r#"
globalThis.frame = document.createElement('iframe');
frame.srcdoc = '<!doctype html><title>first</title>';
document.body.appendChild(frame);
"#,
    )
    .unwrap();
    run_child_navigation_commit_and_host_load_for_test(&mut vm, "Observable first document").await;
    vm.eval(
        r#"
globalThis.events = [];
globalThis.oldWindow = frame.contentWindow;
globalThis.oldEvent = oldWindow.Event;
globalThis.active = EventTarget.prototype.when.call(oldWindow, 'test');
globalThis.pending = EventTarget.prototype.when.call(oldWindow, 'test');
active.subscribe(event => events.push('active:' + event.type));
oldWindow.dispatchEvent(new oldWindow.Event('test'));
frame.srcdoc = '<!doctype html><title>replacement</title>';
"#,
    )
    .unwrap();
    run_child_navigation_commit_and_host_load_for_test(&mut vm, "Observable replacement document")
        .await;
    assert_eq!(vm.eval(r#"
JSON.stringify((() => {
  const child = frame.contentWindow;
  const checks = [child === oldWindow, child.Event !== oldEvent, child.document.title === 'replacement'];
  pending.subscribe(() => events.push('retired'));
  child.dispatchEvent(new child.Event('test'));
  const controller = new AbortController();
  EventTarget.prototype.when.call(child, 'test').subscribe(() => events.push('new'), {signal: controller.signal});
  child.dispatchEvent(new child.Event('test'));
  controller.abort();
  child.dispatchEvent(new child.Event('test'));
  return {checks, events};
})())
"#).unwrap(), r#"{"checks":[true,true,true],"events":["active:test","new"]}"#);
}

#[test]
fn observable_core_lifecycle_conversion_brands_and_exceptions() {
    let mut vm = new_storage_test_vm("https://observable.test/");
    let result = vm
        .eval(&format!(
            "JSON.stringify({})",
            include_str!("../../../tests/fixtures/observable-core.js")
        ))
        .expect("Observable fixture should evaluate");
    let result: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(result["failures"], serde_json::json!([]), "{result}");
    assert!(result["checks"].as_u64().unwrap() >= 65, "{result}");
}

#[test]
fn observable_realm_guards_use_the_receiver_and_teardown_owner() {
    let mut vm = new_storage_test_vm("https://observable-realms.test/");
    vm.eval(
        r#"
(() => {
  const frame = document.createElement('iframe');
  frame.id = 'observable-child';
  (document.body || document.documentElement || document).appendChild(frame);
})()
"#,
    )
    .unwrap();
    materialize_single_child_default_realm_for_test(&mut vm, "Observable child realm");
    let result = vm
        .eval(
            r#"
JSON.stringify((() => {
  const frame = document.getElementById('observable-child');
  const child = frame.contentWindow;
  const events = [];
  let subscriber, initializers = 0, conversions = 0;
  const source = new child.Observable(s => { subscriber = s; initializers++; });
  child.Observable.prototype.subscribe.call(source, value => events.push(value));
  const checks = [subscriber instanceof child.Subscriber,
    subscriber.signal instanceof child.AbortSignal];
  try {
    child.Observable.prototype.subscribe.call({}, {get next() { conversions++; }});
    checks.push(false);
  } catch (error) {
    checks.push(error instanceof child.TypeError && !(error instanceof TypeError));
  }
  checks.push(conversions === 0);
  Subscriber.prototype.next.call(subscriber, 'live');
  const pending = new child.Observable(() => events.push('retired initializer'));
  subscriber.addTeardown(() => events.push('retired teardown'));
  subscriber.addTeardown(() => { events.push('remove'); frame.remove(); });
  Subscriber.prototype.complete.call(subscriber);
  Observable.prototype.subscribe.call(pending);
  Subscriber.prototype.next.call(subscriber, 'retired next');
  Subscriber.prototype.addTeardown.call(subscriber, () => events.push('retired added teardown'));
  checks.push(initializers === 1, !subscriber.active, subscriber.signal.aborted);
  return {checks, events};
})())
"#,
        )
        .unwrap();
    assert_eq!(
        result,
        r#"{"checks":[true,true,true,true,true,true,true],"events":["live","remove"]}"#
    );
}

#[test]
fn observable_weak_subscription_and_callback_cycles_are_collectible() {
    let mut vm = new_storage_test_vm("https://observable-gc.test/");
    vm.eval(
        r#"
(() => {
  globalThis.__initializers = 0;
  globalThis.__source = new Observable(s => {
    __initializers++;
    globalThis.__weakSubscriber = new WeakRef(s);
  });
  __source.subscribe();
  // Isolate this lexical environment from the live source initializer above.
  (() => {
    let cycle;
    cycle = new Observable(() => cycle);
    globalThis.__weakCycle = new WeakRef(cycle);
  })();
  (() => {
    const captured = {};
    globalThis.__weakObserver = new WeakRef(captured);
    globalThis.__controller = new AbortController();
    new Observable(s => s.complete()).subscribe(() => captured, {signal: __controller.signal});
  })();
})()
"#,
    )
    .unwrap();
    vm.renderer_document_isolate
        .clone()
        .with_entered_renderer_document_isolate(|isolate| {
            isolate.clear_kept_objects();
            isolate.low_memory_notification();
            Ok(())
        })
        .unwrap();
    assert_eq!(
        vm.eval(
            r#"
JSON.stringify([__weakSubscriber.deref() === undefined, __weakCycle.deref() === undefined,
  __weakObserver.deref() === undefined, (__source.subscribe(), __initializers)])
"#
        )
        .unwrap(),
        "[true,true,true,2]"
    );
}
