use super::*;

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
