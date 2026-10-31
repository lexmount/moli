use super::*;

#[test]
fn observable_predicate_consumers_short_circuit_conversion_reentrancy_and_cancellation() {
    let mut vm = new_storage_test_vm("https://observable-predicates.test/");
    vm.eval(&format!(
        "({}).then(value => {{ globalThis.predicateResult = JSON.stringify(value); }});",
        include_str!("../../../tests/fixtures/observable-predicate-consumers.js")
    ))
    .expect("Observable predicate consumers fixture should evaluate");
    let result = vm.eval("predicateResult").unwrap();
    let result: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(result["failures"], serde_json::json!([]), "{result}");
    assert!(result["checks"].as_u64().unwrap() >= 302, "{result}");
}

#[test]
fn observable_predicate_consumers_preserve_callback_promise_and_abort_reason_realms() {
    let mut vm = new_storage_test_vm("https://observable-predicate-realms.test/");
    vm.eval("document.appendChild(document.createElement('iframe'))")
        .unwrap();
    materialize_single_child_default_realm_for_test(&mut vm, "Observable predicate consumer realm");
    vm.eval(r#"
(async () => {
  const child = document.querySelector('iframe').contentWindow, checks = [];
  const predicate = child.Function('value', 'index', 'globalThis.predicateThis = this; return value === 5;');
  for (const name of ['some', 'every', 'find']) {
    const method = child.Observable.prototype[name];
    const promise = method.call(Observable.from([5]), predicate);
    checks.push(promise instanceof child.Promise, !(promise instanceof Promise));
    checks.push(await promise === (name === 'find' ? 5 : true), child.predicateThis === child);
    const local = Observable.prototype[name].call(child.Observable.from([7]), () => true);
    checks.push(local instanceof Promise, !(local instanceof child.Promise), await local === (name === 'find' ? 7 : true));
    const invalid = method.call({}, () => true);
    checks.push(invalid instanceof child.Promise);
    await invalid.catch(e => checks.push(e instanceof child.TypeError, !(e instanceof TypeError)));
    const marker = new child.Error('predicate');
    await method.call(Observable.from([1]), () => { throw marker; }).catch(e => checks.push(e === marker, e instanceof child.Error));
    const badCallback = method.call(Observable.from([1]), {});
    checks.push(badCallback instanceof child.Promise);
    await badCallback.catch(e => checks.push(e instanceof child.TypeError));
    let subscriber;
    await method.call(new Observable(s => { subscriber = s; s.next(1); }), () => name !== 'every');
    checks.push(subscriber.signal.reason instanceof child.DOMException, !(subscriber.signal.reason instanceof DOMException));
  }
  globalThis.predicateRealms = JSON.stringify(checks);
})();
"#).unwrap();
    let result: Vec<bool> = serde_json::from_str(&vm.eval("predicateRealms").unwrap()).unwrap();
    assert_eq!(result.len(), 48);
    assert!(result.iter().all(|value| *value), "{result:?}");
}

#[test]
fn observable_predicate_consumers_trace_pending_and_release_cancelled_or_decided_callbacks() {
    let mut vm = new_storage_test_vm("https://observable-predicate-gc.test/");
    vm.eval(r#"
globalThis.keptPredicates = [];
globalThis.abandonedPredicates = [];
for (const name of ['some', 'every', 'find']) {
  for (const kept of [false, true]) (() => {
    let subscriber;
    const source = new Observable(s => { subscriber = s; }), token = {}, predicate = () => token;
    const promise = source[name](predicate);
    const refs = {source: new WeakRef(source), subscriber: new WeakRef(subscriber), predicate: new WeakRef(predicate), token: new WeakRef(token)};
    if (kept) keptPredicates.push({name, promise, refs}); else abandonedPredicates.push(refs);
  })();
}
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
    assert_eq!(vm.eval(r#"JSON.stringify([
abandonedPredicates.every(refs => Object.values(refs).every(ref => ref.deref() === undefined)),
keptPredicates.every(c => c.refs.source.deref() === undefined && c.refs.subscriber.deref() !== undefined && c.refs.predicate.deref() !== undefined && c.refs.token.deref() !== undefined)
])"#).unwrap(), "[true,true]");
    vm.eval(
        r#"
for (const c of keptPredicates) {
  c.promise.then(value => { c.correct = value === (c.name === 'find' ? 7 : true); });
  c.refs.subscriber.deref().next(7); c.refs.subscriber.deref().complete(); delete c.promise;
}
"#,
    )
    .unwrap();
    assert_eq!(
        vm.eval("keptPredicates.every(c => c.correct)").unwrap(),
        "true"
    );
    collect(&mut vm);
    assert_eq!(vm.eval("keptPredicates.every(c => Object.values(c.refs).every(ref => ref.deref() === undefined))").unwrap(), "true");
    vm.eval(r#"
globalThis.cancelledPredicates = [];
globalThis.sharedPredicates = [];
for (const name of ['some', 'every', 'find']) {
  (() => {
    const ac = new AbortController(), token = {}, predicate = () => token;
    let subscriber;
    const source = new Observable(s => { subscriber = s; });
    const promise = source[name](predicate, {signal: ac.signal});
    promise.catch(() => {}); ac.abort('cancelled');
    cancelledPredicates.push({promise, signal: ac.signal, refs: [new WeakRef(token), new WeakRef(predicate), new WeakRef(subscriber)]});
  })();
  (() => {
    let subscriber;
    const token = {}, predicate = () => token && name !== 'every';
    const source = new Observable(s => { subscriber = s; });
    const all = source.toArray(), promise = source[name](predicate);
    subscriber.next(1);
    sharedPredicates.push({subscriber, all, promise, refs: [new WeakRef(token), new WeakRef(predicate)]});
  })();
}
"#).unwrap();
    collect(&mut vm);
    assert_eq!(
        vm.eval("cancelledPredicates.every(c => c.refs.every(ref => ref.deref() === undefined))")
            .unwrap(),
        "true"
    );
    assert_eq!(vm.eval("sharedPredicates.every(c => c.subscriber.active && c.refs.every(ref => ref.deref() === undefined))").unwrap(), "true");
    vm.eval("sharedPredicates.forEach(c => c.subscriber.complete()); sharedPredicates = [];")
        .unwrap();
}

#[test]
fn observable_callback_consumers_conversion_reentrancy_cancellation_and_exception_identity() {
    let mut vm = new_storage_test_vm("https://observable-consumers.test/");
    vm.eval(&format!(
        "({}).then(value => {{ globalThis.consumerResult = JSON.stringify(value); }});",
        include_str!("../../../tests/fixtures/observable-callback-consumers.js")
    ))
    .expect("Observable callback consumers fixture should evaluate");
    let result = vm
        .eval("consumerResult")
        .expect("Observable consumers fixture should settle");
    let result: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(result["failures"], serde_json::json!([]), "{result}");
    assert!(result["checks"].as_u64().unwrap() >= 132, "{result}");
}

#[test]
fn observable_callback_consumers_preserve_callback_realms_and_callee_promises() {
    let mut vm = new_storage_test_vm("https://observable-consumers-realms.test/");
    vm.eval("document.appendChild(document.createElement('iframe'))")
        .unwrap();
    materialize_single_child_default_realm_for_test(&mut vm, "Observable callback consumers realm");
    vm.eval(r#"
(async () => {
  const child = document.querySelector('iframe').contentWindow, checks = [];
  const callback = child.Function('a', 'b', 'globalThis.consumerThis = this; return a + b;');
  for (const name of ['forEach', 'reduce']) {
    const method = child.Observable.prototype[name];
    const call = (method, source, cb) => Reflect.apply(method, source, name === 'reduce' ? [cb, 10] : [cb]);
    const promise = call(method, Observable.from([5]), callback);
    checks.push(promise instanceof child.Promise, !(promise instanceof Promise));
    checks.push(await promise === (name === 'reduce' ? 15 : undefined), child.consumerThis === child);
    const local = call(Observable.prototype[name], child.Observable.from([7]), (a, b) => a + b);
    checks.push(local instanceof Promise, !(local instanceof child.Promise), await local === (name === 'reduce' ? 17 : undefined));
    const invalid = call(method, {}, () => {});
    checks.push(invalid instanceof child.Promise);
    await invalid.catch(e => checks.push(e instanceof child.TypeError, !(e instanceof TypeError)));
    const marker = new child.Error('callback error');
    await call(method, Observable.from([1]), () => { throw marker; }).catch(e => checks.push(e === marker, e instanceof child.Error));
  }
  await child.Observable.prototype.reduce.call(Observable.from([]), () => {}).catch(e => checks.push(e instanceof child.TypeError, !(e instanceof TypeError)));
  globalThis.consumerRealms = JSON.stringify(checks);
})();
"#).unwrap();
    let result: Vec<bool> = serde_json::from_str(&vm.eval("consumerRealms").unwrap()).unwrap();
    assert_eq!(result.len(), 26);
    assert!(result.iter().all(|value| *value), "{result:?}");
}

#[test]
fn observable_callback_consumers_trace_callbacks_and_release_abandoned_and_cancelled_state() {
    let mut vm = new_storage_test_vm("https://observable-consumers-gc.test/");
    vm.eval(r#"
globalThis.consumerCases = [];
globalThis.abandonedConsumers = [];
for (const name of ['forEach', 'reduce']) {
  for (const kept of [false, true]) (() => {
    let subscriber;
    const source = new Observable(s => { subscriber = s; });
    const token = {}, seed = {}, callback = () => token;
    const promise = name === 'reduce' ? source.reduce(callback, seed) : source.forEach(callback);
    const refs = {source: new WeakRef(source), subscriber: new WeakRef(subscriber), callback: new WeakRef(callback), token: new WeakRef(token), seed: new WeakRef(seed)};
    if (kept) consumerCases.push({name, promise, refs});
    else abandonedConsumers.push(refs);
  })();
}
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
    assert_eq!(vm.eval(r#"JSON.stringify([
abandonedConsumers.every(refs => Object.values(refs).every(ref => ref.deref() === undefined)),
consumerCases.every(c => c.refs.source.deref() === undefined && c.refs.subscriber.deref() !== undefined && c.refs.callback.deref() !== undefined && c.refs.token.deref() !== undefined),
consumerCases[0].refs.seed.deref() === undefined, consumerCases[1].refs.seed.deref() !== undefined
])"#).unwrap(), "[true,true,true,true]");
    vm.eval(r#"
for (const c of consumerCases) {
  c.promise.then(value => { c.correct = value === (c.name === 'reduce' ? c.refs.seed.deref() : undefined); });
  c.refs.subscriber.deref().complete(); delete c.promise;
}
"#).unwrap();
    assert_eq!(
        vm.eval("consumerCases.every(c => c.correct)").unwrap(),
        "true"
    );
    collect(&mut vm);
    assert_eq!(vm.eval("consumerCases.every(c => Object.values(c.refs).every(ref => ref.deref() === undefined))").unwrap(), "true");
    vm.eval(r#"
globalThis.cancelledConsumers = [];
for (const name of ['forEach', 'reduce']) (() => {
  const ac = new AbortController(), token = {}, seed = {}, callback = () => token;
  let subscriber;
  const source = new Observable(s => { subscriber = s; });
  const promise = name === 'reduce' ? source.reduce(callback, seed, {signal: ac.signal}) : source.forEach(callback, {signal: ac.signal});
  promise.catch(() => {}); ac.abort('cancelled');
  cancelledConsumers.push({promise, signal: ac.signal, refs: [new WeakRef(token), new WeakRef(seed), new WeakRef(callback), new WeakRef(subscriber)]});
})();
"#).unwrap();
    collect(&mut vm);
    assert_eq!(
        vm.eval("cancelledConsumers.every(c => c.refs.every(ref => ref.deref() === undefined))")
            .unwrap(),
        "true"
    );
}

#[test]
fn observable_collect_values_abort_order_and_native_promise_observers() {
    let mut vm = new_storage_test_vm("https://observable-collect.test/");
    vm.eval(&format!(
        "({}).then(value => {{ globalThis.collectResult = JSON.stringify(value); }});",
        include_str!("../../../tests/fixtures/observable-collect.js")
    ))
    .expect("Observable collection fixture should evaluate");
    let result = vm
        .eval("collectResult")
        .expect("Observable collection fixture should settle");
    let result: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(result["failures"], serde_json::json!([]), "{result}");
    assert!(result["checks"].as_u64().unwrap() >= 110, "{result}");
}

#[test]
fn observable_collect_uses_callee_promise_array_and_error_realms() {
    let mut vm = new_storage_test_vm("https://observable-collect-realms.test/");
    vm.eval("document.appendChild(document.createElement('iframe'))")
        .unwrap();
    materialize_single_child_default_realm_for_test(&mut vm, "Observable collection realm");
    vm.eval(r#"
(async () => {
  const child = document.querySelector('iframe').contentWindow, checks = [];
  for (const name of ['last', 'toArray']) {
    const method = child.Observable.prototype[name];
    const promise = method.call(Observable.from([4, 5]));
    checks.push(promise instanceof child.Promise, !(promise instanceof Promise));
    const value = await promise;
    checks.push(name === 'last' ? value === 5 : value instanceof child.Array && !(value instanceof Array) && value[1] === 5);
    const invalid = method.call({});
    checks.push(invalid instanceof child.Promise);
    await invalid.catch(e => checks.push(e instanceof child.TypeError, !(e instanceof TypeError)));
    const ac = new AbortController(), marker = {};
    const pending = method.call(new Observable(() => {}), {signal: ac.signal});
    ac.abort(marker);
    await pending.catch(e => checks.push(e === marker));
    const local = Observable.prototype[name].call(child.Observable.from([7]));
    checks.push(local instanceof Promise, !(local instanceof child.Promise));
    const result = await local;
    checks.push(name === 'last' ? result === 7 : result instanceof Array && !(result instanceof child.Array) && result[0] === 7);
  }
  await child.Observable.prototype.last.call(Observable.from([])).catch(e => checks.push(e instanceof child.RangeError, !(e instanceof RangeError)));
  globalThis.collectRealms = JSON.stringify(checks);
})();
"#).unwrap();
    let result = vm.eval("collectRealms").unwrap();
    let result: Vec<bool> = serde_json::from_str(&result).unwrap();
    assert_eq!(result.len(), 22);
    assert!(result.iter().all(|value| *value), "{result:?}");
}

#[test]
fn observable_collect_traces_pending_values_and_releases_them_after_completion_or_abort() {
    let mut vm = new_storage_test_vm("https://observable-collect-gc.test/");
    vm.eval(r#"
globalThis.collectCases = [];
globalThis.abandonedCollect = [];
for (const mode of ['last', 'toArray']) {
  (() => {
    let subscriber;
    const source = new Observable(s => { subscriber = s; });
    const promise = source[mode](), value = {};
    subscriber.next(value);
    abandonedCollect.push([new WeakRef(source), new WeakRef(subscriber), new WeakRef(promise), new WeakRef(value)]);
  })();
  (() => {
    let subscriber;
    const source = new Observable(s => { subscriber = s; });
    const promise = source[mode](), first = {}, second = {};
    subscriber.next(first); subscriber.next(second);
    collectCases.push({mode, promise, source: new WeakRef(source), subscriber: new WeakRef(subscriber), first: new WeakRef(first), second: new WeakRef(second)});
  })();
}
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
    assert_eq!(vm.eval(r#"JSON.stringify([
abandonedCollect.every(refs => refs.every(ref => ref.deref() === undefined)),
collectCases.every(c => c.source.deref() === undefined && c.subscriber.deref() !== undefined && c.second.deref() !== undefined),
collectCases[0].first.deref() === undefined, collectCases[1].first.deref() !== undefined
])"#).unwrap(), "[true,true,true,true]");
    vm.eval(r#"
for (const c of collectCases) {
  c.promise.then(value => { c.correct = c.mode === 'last' ? value === c.second.deref() : value[0] === c.first.deref() && value[1] === c.second.deref(); });
  c.subscriber.deref().complete();
  delete c.promise;
}
"#).unwrap();
    assert_eq!(
        vm.eval("collectCases.every(c => c.correct)").unwrap(),
        "true"
    );
    collect(&mut vm);
    assert_eq!(vm.eval("collectCases.every(c => c.subscriber.deref() === undefined && c.first.deref() === undefined && c.second.deref() === undefined)").unwrap(), "true");
    vm.eval(r#"
globalThis.cancelledCollect = [];
for (const mode of ['last', 'toArray']) {
  (() => {
    const ac = new AbortController();
    let subscriber;
    const source = new Observable(s => { subscriber = s; });
    const promise = source[mode]({signal: ac.signal}), value = {};
    subscriber.next(value);
    promise.catch(() => {});
    ac.abort('cancelled');
    cancelledCollect.push({promise, value: new WeakRef(value), subscriber: new WeakRef(subscriber), signal: ac.signal});
  })();
}
"#).unwrap();
    collect(&mut vm);
    assert_eq!(vm.eval("cancelledCollect.every(c => c.value.deref() === undefined && c.subscriber.deref() === undefined)").unwrap(), "true");
}

#[test]
fn observable_first_promises_cancellation_reentrancy_and_native_observers() {
    let mut vm = new_storage_test_vm("https://observable-first.test/");
    vm.eval(&format!(
        "({}).then(value => {{ globalThis.firstResult = JSON.stringify(value); }});",
        include_str!("../../../tests/fixtures/observable-first.js")
    ))
    .expect("Observable.first fixture should evaluate");
    let result = vm
        .eval("firstResult")
        .expect("Observable.first fixture should settle");
    let result: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(result["failures"], serde_json::json!([]), "{result}");
    assert!(result["checks"].as_u64().unwrap() >= 72, "{result}");
}

#[test]
fn observable_first_pending_promises_trace_observers_without_rooting_abandoned_cycles() {
    let mut vm = new_storage_test_vm("https://observable-first-gc.test/");
    vm.eval(
        r#"
(() => {
  const captured = {};
  const source = new Observable(s => {
    globalThis.weakFirstSubscriber = new WeakRef(s);
    s.addTeardown(() => captured);
  });
  globalThis.weakFirstCapture = new WeakRef(captured);
  globalThis.weakFirstSource = new WeakRef(source);
  globalThis.weakFirstPromise = new WeakRef(source.first());
})();
(() => {
  const iterator = {next: () => new Promise(() => {})};
  globalThis.weakFirstIterator = new WeakRef(iterator);
  Observable.from({[Symbol.asyncIterator]: () => iterator}).first();
})();
(() => {
  const source = new Observable(s => {
    globalThis.weakKeptSubscriber = new WeakRef(s);
    globalThis.deliverFirst = s.next.bind(s);
  });
  globalThis.weakKeptSource = new WeakRef(source);
  globalThis.keptFirstPromise = source.first();
})();
(() => {
  const ac = new AbortController();
  globalThis.cancelFirst = ac.abort.bind(ac);
  new Observable(s => { globalThis.weakAbortFirstSubscriber = new WeakRef(s); })
    .first({signal: ac.signal}).catch(reason => { globalThis.firstAbortReason = reason; });
})();
"#,
    )
    .unwrap();
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
weakFirstSubscriber.deref() === undefined, weakFirstCapture.deref() === undefined,
weakFirstSource.deref() === undefined, weakFirstPromise.deref() === undefined,
weakFirstIterator.deref() === undefined, weakKeptSource.deref() === undefined,
weakKeptSubscriber.deref() !== undefined, weakAbortFirstSubscriber.deref() !== undefined
])"#
        )
        .unwrap(),
        "[true,true,true,true,true,true,true,true]"
    );
    vm.eval("delete globalThis.deliverFirst;").unwrap();
    collect(&mut vm);
    assert_eq!(
        vm.eval("weakKeptSubscriber.deref() !== undefined").unwrap(),
        "true"
    );
    vm.eval(
        r#"
keptFirstPromise.then(value => { globalThis.firstDelivered = value; });
weakKeptSubscriber.deref().next(31);
cancelFirst('cancelled');
delete globalThis.keptFirstPromise;
delete globalThis.cancelFirst;
"#,
    )
    .unwrap();
    assert_eq!(
        vm.eval("JSON.stringify([firstDelivered, firstAbortReason])")
            .unwrap(),
        "[31,\"cancelled\"]"
    );
    collect(&mut vm);
    assert_eq!(vm.eval("JSON.stringify([weakKeptSubscriber.deref() === undefined, weakAbortFirstSubscriber.deref() === undefined])").unwrap(), "[true,true]");
}

#[test]
fn observable_first_uses_callee_promise_and_error_realms_with_foreign_sources() {
    let mut vm = new_storage_test_vm("https://observable-first-realms.test/");
    vm.eval("document.appendChild(document.createElement('iframe'))")
        .unwrap();
    materialize_single_child_default_realm_for_test(&mut vm, "Observable.first realm");
    vm.eval(r#"
(async () => {
  const child = document.querySelector('iframe').contentWindow, checks = [];
  const first = child.Observable.prototype.first;
  const source = new Observable(s => s.next(5));
  const promise = first.call(source);
  checks.push(promise instanceof child.Promise, !(promise instanceof Promise), await promise === 5);
  let conversions = 0;
  const invalid = first.call({}, {get signal() { conversions++; }});
  checks.push(invalid instanceof child.Promise);
  await invalid.catch(e => checks.push(e instanceof child.TypeError, !(e instanceof TypeError)));
  checks.push(conversions === 0);
  await first.call(new Observable(s => s.complete())).catch(e => checks.push(e instanceof child.RangeError, !(e instanceof RangeError)));
  const foreign = new child.Observable(s => s.next(6));
  const local = Observable.prototype.first.call(foreign);
  checks.push(local instanceof Promise, !(local instanceof child.Promise), await local === 6);
  const marker = {}, ac = new AbortController();
  const pending = first.call(new Observable(() => {}), {signal: ac.signal});
  ac.abort(marker);
  await pending.catch(e => checks.push(e === marker));
  globalThis.firstRealms = JSON.stringify(checks);
})();
"#).unwrap();
    assert_eq!(
        vm.eval("firstRealms").unwrap(),
        "[true,true,true,true,true,true,true,true,true,true,true,true,true]"
    );
}

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
