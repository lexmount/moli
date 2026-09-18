use super::*;

#[test]
fn observable_flat_map_preserves_serial_order_conversion_reentrancy_and_cancellation() {
    let mut vm = new_storage_test_vm("https://observable-flat-map.test/");
    vm.eval(&format!(
        "({}).then(value => {{ globalThis.flatMapResult = JSON.stringify(value); }});",
        include_str!("../../../tests/fixtures/observable-flat-map.js")
    ))
    .expect("Observable.flatMap fixture should evaluate");
    let result: serde_json::Value =
        serde_json::from_str(&vm.eval("flatMapResult").unwrap()).unwrap();
    assert_eq!(result["failures"], serde_json::json!([]), "{result}");
    assert!(result["checks"].as_u64().unwrap() >= 120, "{result}");
}

#[test]
fn observable_flat_map_preserves_result_conversion_callback_and_cancellation_realms() {
    let mut vm = new_storage_test_vm("https://observable-flat-map-realms.test/");
    vm.eval("document.appendChild(document.createElement('iframe'))")
        .unwrap();
    materialize_single_child_default_realm_for_test(&mut vm, "Observable.flatMap realm");
    vm.eval(&format!(
        "({}).then(value => {{ globalThis.flatMapRealms = JSON.stringify(value); }});",
        include_str!("../../../tests/fixtures/observable-flat-map-realms.js")
    ))
    .expect("Observable.flatMap realms fixture should evaluate");
    let result: serde_json::Value =
        serde_json::from_str(&vm.eval("flatMapRealms").unwrap()).unwrap();
    assert_eq!(result["failures"], serde_json::json!([]), "{result}");
    assert_eq!(result["checks"], 31, "{result}");
}

#[test]
fn observable_flat_map_traces_both_producers_and_releases_queues_and_closed_graphs() {
    let mut vm = new_storage_test_vm("https://observable-flat-map-gc.test/");
    vm.eval(r#"
function makeFlatMapSource() {
  let subscriber;
  return {source: new Observable(s => { subscriber = s; }), get subscriber() { return subscriber; }};
}
function makeFlatMapper(inner) {
  const token = {values: []};
  return {token, callback: value => value === 0 ? inner : token.values};
}
globalThis.flatMapChains = [];
for (const mode of ['abandoned', 'complete', 'outer-error', 'inner-error', 'abort']) (() => {
  const outer = makeFlatMapSource(), inner = makeFlatMapSource(), mapper = makeFlatMapper(inner.source);
  const result = outer.source.flatMap(mapper.callback), ac = new AbortController(), queued = [{}, {}];
  const promise = result.toArray(mode === 'abort' ? {signal: ac.signal} : undefined);
  promise.catch(() => {});
  outer.subscriber.next(0); inner.subscriber.next(1);
  for (const value of queued) outer.subscriber.next(value);
  const entry = {mode, templates: [outer.source, result].map(v => new WeakRef(v)),
    outer: new WeakRef(outer.subscriber), inner: new WeakRef(inner.subscriber),
    callbacks: [mapper.callback, mapper.token].map(v => new WeakRef(v)), queued: queued.map(v => new WeakRef(v))};
  if (mode !== 'abandoned') entry.promise = promise;
  if (mode === 'abort') entry.controller = ac;
  flatMapChains.push(entry);
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
    assert_eq!(vm.eval(r#"JSON.stringify([
flatMapChains.every(c => c.templates.every(ref => ref.deref() === undefined)),
flatMapChains.every(c => [c.outer, c.inner, ...c.callbacks, ...c.queued].every(ref => (ref.deref() !== undefined) === (c.mode !== 'abandoned')))
])"#).unwrap(), "[true,true]");
    vm.eval(r#"
for (const c of flatMapChains.filter(c => c.promise)) {
  c.promise.then(values => { c.correct = c.mode === 'complete' && JSON.stringify(values) === '[1,2]'; }, error => { c.correct = error === c.mode; });
  const outer = c.outer.deref(), inner = c.inner.deref(); inner.next(2);
  if (c.mode === 'complete') { outer.complete(); inner.complete(); }
  else if (c.mode === 'outer-error') outer.error(c.mode);
  else if (c.mode === 'inner-error') inner.error(c.mode);
  else c.controller.abort(c.mode);
  c.closed = [outer, inner];
}
"#).unwrap();
    assert_eq!(vm.eval("flatMapChains.filter(c => c.promise).every(c => c.correct && c.closed.every(s => !s.active))").unwrap(), "true");
    collect(&mut vm);
    assert_eq!(vm.eval("flatMapChains.every(c => [...c.callbacks, ...c.queued].every(ref => ref.deref() === undefined))").unwrap(), "true");
    vm.eval(r#"
globalThis.flatMapDrain = (() => {
  const outer = makeFlatMapSource(), inner = makeFlatMapSource(), mapper = makeFlatMapper(inner.source), value = {};
  const promise = outer.source.flatMap(mapper.callback).toArray();
  outer.subscriber.next(0); outer.subscriber.next(value); outer.subscriber.complete();
  return {promise, outer: new WeakRef(outer.subscriber), inner: new WeakRef(inner.subscriber),
    value: new WeakRef(value), callback: new WeakRef(mapper.callback)};
})();
"#).unwrap();
    collect(&mut vm);
    assert_eq!(vm.eval("flatMapDrain.outer.deref() === undefined && flatMapDrain.inner.deref() !== undefined && flatMapDrain.value.deref() !== undefined && flatMapDrain.callback.deref() !== undefined").unwrap(), "true");
    vm.eval("flatMapDrain.promise.then(v => { flatMapDrain.correct = v.length === 0; }); flatMapDrain.inner.deref().complete();").unwrap();
    collect(&mut vm);
    assert_eq!(vm.eval("flatMapDrain.correct && flatMapDrain.inner.deref() === undefined && flatMapDrain.value.deref() === undefined && flatMapDrain.callback.deref() === undefined").unwrap(), "true");
}

#[test]
fn observable_finally_preserves_teardown_order_sharing_and_reentrant_cancellation() {
    let mut vm = new_storage_test_vm("https://observable-finally.test/");
    vm.eval(&format!(
        "({}).then(value => {{ globalThis.finallyResult = JSON.stringify(value); }});",
        include_str!("../../../tests/fixtures/observable-finally.js")
    ))
    .expect("Observable.finally fixture should evaluate");
    let result: serde_json::Value =
        serde_json::from_str(&vm.eval("finallyResult").unwrap()).unwrap();
    assert_eq!(result["failures"], serde_json::json!([]), "{result}");
    assert!(result["checks"].as_u64().unwrap() >= 120, "{result}");
}

#[test]
fn observable_finally_preserves_result_conversion_callback_and_exception_realms() {
    let mut vm = new_storage_test_vm("https://observable-finally-realms.test/");
    vm.eval("document.appendChild(document.createElement('iframe'))")
        .unwrap();
    materialize_single_child_default_realm_for_test(&mut vm, "Observable.finally realm");
    vm.eval(&format!(
        "({}).then(value => {{ globalThis.finallyRealms = JSON.stringify(value); }});",
        include_str!("../../../tests/fixtures/observable-finally-realms.js")
    ))
    .expect("Observable.finally realms fixture should evaluate");
    let result: serde_json::Value =
        serde_json::from_str(&vm.eval("finallyRealms").unwrap()).unwrap();
    assert_eq!(result["failures"], serde_json::json!([]), "{result}");
    assert_eq!(result["checks"], 36, "{result}");
}

#[test]
fn observable_finally_traces_pending_teardowns_and_releases_closed_or_abandoned_graphs() {
    let mut vm = new_storage_test_vm("https://observable-finally-gc.test/");
    vm.eval(r#"
globalThis.gcFinallyCalls = 0;
function makeFinallyCallback() {
  const token = {};
  return {token, callback: () => { gcFinallyCalls++; return token; }};
}
function makeFinallySource() {
  let subscriber;
  return {source: new Observable(s => { subscriber = s; }), get subscriber() { return subscriber; }};
}
globalThis.finallyChains = [];
for (const mode of ['abandoned', 'complete', 'error', 'abort']) (() => {
  const input = makeFinallySource(), finalizer = makeFinallyCallback();
  const result = input.source.finally(finalizer.callback), ac = new AbortController();
  const promise = result.toArray(mode === 'abort' ? {signal: ac.signal} : undefined);
  promise.catch(() => {}); input.subscriber.next(1);
  const entry = {mode, templates: [input.source, result].map(v => new WeakRef(v)),
    subscriber: new WeakRef(input.subscriber), finalizer: [finalizer.callback, finalizer.token].map(v => new WeakRef(v))};
  if (mode !== 'abandoned') entry.promise = promise;
  if (mode === 'abort') entry.controller = ac;
  finallyChains.push(entry);
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
    assert_eq!(vm.eval(r#"JSON.stringify([
finallyChains.every(c => c.templates.every(ref => ref.deref() === undefined)),
finallyChains.every(c => (c.subscriber.deref() !== undefined) === (c.mode !== 'abandoned')),
finallyChains.every(c => c.finalizer.every(ref => (ref.deref() !== undefined) === (c.mode !== 'abandoned'))),
gcFinallyCalls === 0
])"#).unwrap(), "[true,true,true,true]");
    vm.eval(r#"
for (const c of finallyChains.filter(c => c.promise)) {
  c.promise.then(values => { c.correct = c.mode === 'complete' && JSON.stringify(values) === '[1,2]'; }, error => { c.correct = error === c.mode; });
  const source = c.subscriber.deref(); source.next(2);
  if (c.mode === 'complete') source.complete();
  else if (c.mode === 'error') source.error('error');
  else c.controller.abort('abort');
  c.closedSubscriber = source;
}
"#).unwrap();
    assert_eq!(vm.eval("gcFinallyCalls === 3 && finallyChains.filter(c => c.promise).every(c => c.correct && !c.closedSubscriber.active)").unwrap(), "true");
    collect(&mut vm);
    assert_eq!(
        vm.eval("finallyChains.every(c => c.finalizer.every(ref => ref.deref() === undefined))")
            .unwrap(),
        "true"
    );
    vm.eval(r#"
globalThis.preabortedFinallySubscribers = [];
globalThis.preabortedFinallyRefs = (() => {
  const finalizer = makeFinallyCallback();
  new Observable(s => preabortedFinallySubscribers.push(s)).finally(finalizer.callback).subscribe({}, {signal: AbortSignal.abort('pre-aborted')});
  return [new WeakRef(finalizer.callback), new WeakRef(finalizer.token)];
})();
"#).unwrap();
    collect(&mut vm);
    assert_eq!(vm.eval("gcFinallyCalls === 4 && preabortedFinallySubscribers.length === 1 && preabortedFinallySubscribers.every(s => !s.active && s.signal.reason === 'pre-aborted') && preabortedFinallyRefs.every(ref => ref.deref() === undefined)").unwrap(), "true");
}

#[test]
fn observable_inspect_preserves_conversion_callbacks_cancellation_and_error_order() {
    let mut vm = new_storage_test_vm("https://observable-inspect.test/");
    vm.eval(&format!(
        "({}).then(value => {{ globalThis.inspectResult = JSON.stringify(value); }});",
        include_str!("../../../tests/fixtures/observable-inspect.js")
    ))
    .expect("Observable.inspect fixture should evaluate");
    let result: serde_json::Value =
        serde_json::from_str(&vm.eval("inspectResult").unwrap()).unwrap();
    assert_eq!(result["failures"], serde_json::json!([]), "{result}");
    assert_eq!(result["checks"], 169, "{result}");
}

#[test]
fn observable_inspect_preserves_conversion_result_and_callback_error_realms() {
    let mut vm = new_storage_test_vm("https://observable-inspect-realms.test/");
    vm.eval("document.appendChild(document.createElement('iframe'))")
        .unwrap();
    materialize_single_child_default_realm_for_test(&mut vm, "Observable.inspect realm");
    vm.eval(r#"
(async () => {
  const child = document.querySelector('iframe').contentWindow, checks = [];
  const method = child.Observable.prototype.inspect, source = Observable.from([1]);
  child.inspectCalls = [];
  const inspector = {};
  for (const name of ['subscribe', 'next', 'complete']) inspector[name] = child.Function(`inspectCalls.push('${name}'); globalThis.inspectThis = this;`);
  const result = method.call(source, inspector);
  checks.push(result instanceof child.Observable, !(result instanceof Observable), Object.getPrototypeOf(result) === child.Observable.prototype);
  checks.push(child.inspectCalls.length === 0);
  const values = await result.toArray();
  checks.push(values instanceof child.Array, values[0] === 1, child.inspectThis === child, child.inspectCalls.join(',') === 'subscribe,next,complete');
  const local = Observable.prototype.inspect.call(child.Observable.from([2]));
  checks.push(local instanceof Observable, !(local instanceof child.Observable), Object.getPrototypeOf(local) === Observable.prototype);
  const localValues = await local.toArray();
  checks.push(localValues instanceof Array, localValues[0] === 2);
  let reads = 0;
  const input = {get next() { reads++; return () => {}; }};
  const revoked = Proxy.revocable(source, {}); revoked.revoke();
  for (const invalid of [{}, new Proxy(source, {}), revoked.proxy]) {
    try { method.call(invalid, input); } catch (e) { checks.push(e instanceof child.TypeError, !(e instanceof TypeError)); }
  }
  checks.push(reads === 0);
  for (const invalid of [1, {abort: null}, {next: 1}]) {
    try { method.call(source, invalid); } catch (e) { checks.push(e instanceof child.TypeError, !(e instanceof TypeError)); }
  }
  const marker = new child.Error('inspector'); child.inspectError = marker;
  try { method.call(source, {get complete() { throw marker; }}); } catch (e) { checks.push(e === marker, e instanceof child.Error); }
  const callback = child.Function('throw globalThis.inspectError');
  const error = await method.call(source, callback).toArray().catch(e => e);
  checks.push(error === marker, error instanceof child.Error);
  const mainReports = [], childReports = [], ac = new AbortController(); let s;
  const onmain = e => { mainReports.push(e.error); e.preventDefault(); };
  const onchild = e => { childReports.push(e.error); e.preventDefault(); };
  addEventListener('error', onmain); child.addEventListener('error', onchild);
  try {
    new Observable(subscriber => { s = subscriber; }).inspect({abort: callback}).subscribe({}, {signal: ac.signal});
    ac.abort('stop');
    checks.push(mainReports.length === 0, childReports.length === 1 && childReports[0] === marker, !s.active && s.signal.reason === 'stop');
  } finally { removeEventListener('error', onmain); child.removeEventListener('error', onchild); }
  Object.setPrototypeOf(source, null);
  const branded = method.call(source);
  checks.push(branded instanceof child.Observable, (await branded.toArray())[0] === 1);
  globalThis.inspectRealms = JSON.stringify(checks);
})();
"#).unwrap();
    let checks: Vec<bool> = serde_json::from_str(&vm.eval("inspectRealms").unwrap()).unwrap();
    assert_eq!(checks.len(), 35);
    assert!(checks.iter().all(|value| *value), "{checks:?}");
}

#[test]
fn observable_inspect_traces_pending_callbacks_and_releases_closed_or_abandoned_graphs() {
    let mut vm = new_storage_test_vm("https://observable-inspect-gc.test/");
    vm.eval(r#"
globalThis.inspectorNames = ['subscribe', 'next', 'error', 'complete', 'abort'];
function makeInspectorCallback() {
  const token = {};
  return {token, callback: () => token};
}
function makeInspectorSource() {
  let subscriber;
  return {source: new Observable(s => { subscriber = s; }), get subscriber() { return subscriber; }};
}
globalThis.inspectedChains = [];
for (const mode of ['abandoned', 'complete', 'error', 'abort']) (() => {
  const input = makeInspectorSource(), callbacks = inspectorNames.map(makeInspectorCallback);
  const inspector = Object.fromEntries(callbacks.map((c, i) => [inspectorNames[i], c.callback]));
  const result = input.source.inspect(inspector), ac = new AbortController();
  const promise = result.toArray(mode === 'abort' ? {signal: ac.signal} : undefined);
  promise.catch(() => {}); input.subscriber.next(1);
  const entry = {mode, templates: [input.source, result, inspector].map(v => new WeakRef(v)),
    subscriber: new WeakRef(input.subscriber), callbacks: callbacks.map(c => [new WeakRef(c.callback), new WeakRef(c.token)])};
  if (mode !== 'abandoned') entry.promise = promise;
  if (mode === 'abort') entry.controller = ac;
  inspectedChains.push(entry);
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
    assert_eq!(vm.eval(r#"JSON.stringify([
inspectedChains.every(c => c.templates.every(ref => ref.deref() === undefined)),
inspectedChains.every(c => (c.subscriber.deref() !== undefined) === (c.mode !== 'abandoned')),
inspectedChains.every(c => c.callbacks.every((refs, i) => refs.every(ref => (ref.deref() !== undefined) === (c.mode !== 'abandoned' && i !== 0))))
])"#).unwrap(), "[true,true,true]");
    vm.eval(r#"
for (const c of inspectedChains.filter(c => c.promise)) {
  c.promise.then(values => { c.correct = c.mode === 'complete' && JSON.stringify(values) === '[1,2]'; }, error => { c.correct = error === c.mode; });
  const source = c.subscriber.deref(); source.next(2);
  if (c.mode === 'complete') source.complete();
  else if (c.mode === 'error') source.error('error');
  else c.controller.abort('abort');
  c.closedSubscriber = source;
}
"#).unwrap();
    assert_eq!(vm.eval("inspectedChains.filter(c => c.promise).every(c => c.correct && !c.closedSubscriber.active)").unwrap(), "true");
    collect(&mut vm);
    assert_eq!(
        vm.eval(
            "inspectedChains.every(c => c.callbacks.flat().every(ref => ref.deref() === undefined))"
        )
        .unwrap(),
        "true"
    );
    vm.eval(r#"
globalThis.preabortedInspectSubscribers = [];
globalThis.preabortedInspectorRefs = (() => {
  const callbacks = inspectorNames.map(makeInspectorCallback);
  const inspector = Object.fromEntries(callbacks.map((c, i) => [inspectorNames[i], c.callback]));
  new Observable(s => preabortedInspectSubscribers.push(s)).inspect(inspector).subscribe({}, {signal: AbortSignal.abort('pre-aborted')});
  return callbacks.flatMap(c => [new WeakRef(c.callback), new WeakRef(c.token)]);
})();
"#).unwrap();
    collect(&mut vm);
    assert_eq!(vm.eval("preabortedInspectSubscribers.length === 1 && preabortedInspectSubscribers.every(s => !s.active && s.signal.reason === 'pre-aborted') && preabortedInspectorRefs.every(ref => ref.deref() === undefined)").unwrap(), "true");
}

#[test]
fn observable_take_until_preserves_conversion_notifier_order_sharing_and_cancellation() {
    let mut vm = new_storage_test_vm("https://observable-take-until.test/");
    vm.eval(&format!(
        "({}).then(value => {{ globalThis.takeUntilResult = JSON.stringify(value); }});",
        include_str!("../../../tests/fixtures/observable-take-until.js")
    ))
    .expect("Observable.takeUntil fixture should evaluate");
    let result: serde_json::Value =
        serde_json::from_str(&vm.eval("takeUntilResult").unwrap()).unwrap();
    assert_eq!(result["failures"], serde_json::json!([]), "{result}");
    assert!(result["checks"].as_u64().unwrap() >= 154, "{result}");
}

#[test]
fn observable_take_until_preserves_result_conversion_and_exception_realms() {
    let mut vm = new_storage_test_vm("https://observable-take-until-realms.test/");
    vm.eval("document.appendChild(document.createElement('iframe'))")
        .unwrap();
    materialize_single_child_default_realm_for_test(&mut vm, "Observable.takeUntil realm");
    vm.eval(r#"
(async () => {
  const child = document.querySelector('iframe').contentWindow, checks = [], log = [];
  const method = child.Observable.prototype.takeUntil;
  const source = new Observable(s => { log.push('source'); s.next(1); s.complete(); });
  const notifier = new child.Observable(s => { log.push('notifier'); s.complete(); });
  const result = method.call(source, notifier);
  checks.push(result instanceof child.Observable, !(result instanceof Observable), Object.getPrototypeOf(result) === child.Observable.prototype);
  checks.push(log.length === 0);
  const values = await result.toArray();
  checks.push(values instanceof child.Array, values[0] === 1);
  checks.push(JSON.stringify(log) === '["notifier","source"]');
  const local = Observable.prototype.takeUntil.call(child.Observable.from([2]), new Observable(s => s.complete()));
  checks.push(local instanceof Observable, !(local instanceof child.Observable), Object.getPrototypeOf(local) === Observable.prototype);
  const localValues = await local.toArray();
  checks.push(localValues instanceof Array, localValues[0] === 2);
  let reads = 0;
  const input = {get [Symbol.iterator]() { reads++; return [][Symbol.iterator]; }};
  const revoked = Proxy.revocable(source, {}); revoked.revoke();
  for (const invalid of [{}, new Proxy(source, {}), revoked.proxy]) {
    try { method.call(invalid, input); } catch (e) { checks.push(e instanceof child.TypeError, !(e instanceof TypeError)); }
  }
  checks.push(reads === 0);
  for (const invalid of [1, {}, new Proxy(Promise.resolve(1), {})]) {
    try { method.call(source, invalid); } catch (e) { checks.push(e instanceof child.TypeError, !(e instanceof TypeError)); }
  }
  const marker = new child.Error('notifier');
  try { method.call(source, {get [Symbol.iterator]() { throw marker; }}); } catch (e) { checks.push(e === marker, e instanceof child.Error); }
  method.call(new Observable(s => s.error(marker)), new child.Observable(() => {}))
    .subscribe({error: e => checks.push(e === marker, e instanceof child.Error)});
  let starts = 0, errors = 0, completions = 0;
  method.call(new Observable(() => { starts++; }), new child.Observable(s => s.error(marker)))
    .subscribe({error: () => errors++, complete: () => completions++});
  checks.push(errors === 0 && completions === 1, starts === 0);
  Object.setPrototypeOf(notifier, null);
  const branded = method.call(source, notifier);
  checks.push(branded instanceof child.Observable, (await branded.toArray())[0] === 1);
  globalThis.takeUntilRealms = JSON.stringify(checks);
})();
"#).unwrap();
    let checks: Vec<bool> = serde_json::from_str(&vm.eval("takeUntilRealms").unwrap()).unwrap();
    assert_eq!(checks.len(), 33);
    assert!(checks.iter().all(|value| *value), "{checks:?}");
}

#[test]
fn observable_take_until_traces_both_inputs_and_releases_an_exhausted_notifier() {
    let mut vm = new_storage_test_vm("https://observable-take-until-gc.test/");
    vm.eval(r#"
globalThis.untilChains = [];
// Give each producer its own closure environment so a live source callback
// cannot itself retain the notifier's subscriber and token.
function makeUntilGcInput() {
  let subscriber;
  const token = {}, callback = value => token && value;
  const source = new Observable(s => { subscriber = s; });
  return {source, mapped: source.map(callback), callback, token, get subscriber() { return subscriber; }};
}
for (const mode of ['abandoned', 'notifier', 'source', 'notifier complete']) (() => {
  const source = makeUntilGcInput(), notifier = makeUntilGcInput();
  const result = source.mapped.takeUntil(notifier.mapped), promise = result.toArray();
  source.subscriber.next(1);
  if (mode === 'notifier complete') notifier.subscriber.complete();
  const entry = {mode, templates: [source.source, notifier.source, source.mapped, notifier.mapped, result].map(v => new WeakRef(v)),
    source: [source.subscriber, source.callback, source.token].map(v => new WeakRef(v)),
    notifier: [notifier.subscriber, notifier.callback, notifier.token].map(v => new WeakRef(v))};
  if (mode !== 'abandoned') entry.promise = promise;
  untilChains.push(entry);
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
    assert_eq!(vm.eval(r#"JSON.stringify([
untilChains.every(c => c.templates.every(ref => ref.deref() === undefined)),
untilChains.every(c => c.source.every(ref => (ref.deref() !== undefined) === (c.mode !== 'abandoned'))),
untilChains.every(c => c.notifier.every(ref => (ref.deref() !== undefined) === (c.mode === 'notifier' || c.mode === 'source')))
])"#).unwrap(), "[true,true,true]");
    vm.eval(
        r#"
for (const c of untilChains.filter(c => c.promise)) {
  c.promise.then(values => { c.correct = JSON.stringify(values) === '[1,2]'; });
  const source = c.source[0].deref(), notifier = c.notifier[0].deref();
  source.next(2);
  if (c.mode === 'notifier') notifier.next('stop'); else source.complete();
  c.closed = !source.active && (!notifier || !notifier.active);
}
"#,
    )
    .unwrap();
    assert_eq!(
        vm.eval("untilChains.filter(c => c.promise).every(c => c.correct && c.closed)")
            .unwrap(),
        "true"
    );
    collect(&mut vm);
    assert_eq!(vm.eval("untilChains.every(c => [...c.source, ...c.notifier].every(ref => ref.deref() === undefined))").unwrap(), "true");
    vm.eval(r#"
globalThis.cancelledUntil = (() => {
  let s, n;
  const sourceToken = {}, notifierToken = {}, ac = new AbortController();
  const sourceCallback = value => sourceToken && value, notifierCallback = value => notifierToken && value;
  const source = new Observable(subscriber => { s = subscriber; }).map(sourceCallback);
  const notifier = new Observable(subscriber => { n = subscriber; }).map(notifierCallback);
  const promise = source.takeUntil(notifier).toArray({signal: ac.signal});
  promise.catch(() => {}); ac.abort('cancel');
  return {promise, signal: ac.signal, subscribers: [s, n], refs: [sourceToken, notifierToken, sourceCallback, notifierCallback].map(v => new WeakRef(v))};
})();
"#).unwrap();
    collect(&mut vm);
    assert_eq!(vm.eval("cancelledUntil.subscribers.every(s => !s.active && s.signal.reason === 'cancel') && cancelledUntil.refs.every(ref => ref.deref() === undefined)").unwrap(), "true");
}

#[test]
fn observable_count_operators_preserve_conversion_sharing_reentrancy_and_cancellation() {
    let mut vm = new_storage_test_vm("https://observable-count-operators.test/");
    vm.eval(&format!(
        "({}).then(value => {{ globalThis.countOperatorResult = JSON.stringify(value); }});",
        include_str!("../../../tests/fixtures/observable-count-operators.js")
    ))
    .expect("Observable count operator fixture should evaluate");
    let result: serde_json::Value =
        serde_json::from_str(&vm.eval("countOperatorResult").unwrap()).unwrap();
    assert_eq!(result["failures"], serde_json::json!([]), "{result}");
    assert!(result["checks"].as_u64().unwrap() >= 194, "{result}");
}

#[test]
fn observable_count_operators_preserve_conversion_result_and_exception_realms() {
    let mut vm = new_storage_test_vm("https://observable-count-realms.test/");
    vm.eval("document.appendChild(document.createElement('iframe'))")
        .unwrap();
    materialize_single_child_default_realm_for_test(&mut vm, "Observable count operator realm");
    vm.eval(r#"
(async () => {
  const child = document.querySelector('iframe').contentWindow, checks = [];
  for (const name of ['take', 'drop']) {
    const method = child.Observable.prototype[name];
    child.countReads = 0;
    const amount = new child.Object();
    amount[Symbol.toPrimitive] = child.Function('globalThis.countThis = this; globalThis.countReads++; return 1;');
    let starts = 0;
    const source = new Observable(s => { starts++; s.next(1); s.next(2); s.complete(); });
    const result = method.call(source, amount);
    checks.push(result instanceof child.Observable, !(result instanceof Observable), Object.getPrototypeOf(result) === child.Observable.prototype);
    checks.push(starts === 0, child.countReads === 1, child.countThis === amount);
    const values = await result.toArray();
    checks.push(values instanceof child.Array, values.length === 1 && values[0] === (name === 'take' ? 1 : 2));
    const local = Observable.prototype[name].call(child.Observable.from([1, 2]), 1);
    checks.push(local instanceof Observable, !(local instanceof child.Observable));
    const localValues = await local.toArray();
    checks.push(localValues instanceof Array, localValues.length === 1 && localValues[0] === (name === 'take' ? 1 : 2));
    let reads = 0;
    try { method.call({}, {valueOf() { reads++; return 1; }}); } catch (e) { checks.push(e instanceof child.TypeError, !(e instanceof TypeError)); }
    checks.push(reads === 0);
    try { method.call(source, 1n); } catch (e) { checks.push(e instanceof child.TypeError, !(e instanceof TypeError)); }
    const marker = new child.RangeError('amount');
    try { method.call(source, {valueOf() { throw marker; }}); } catch (e) { checks.push(e === marker, e instanceof child.RangeError); }
    method.call(new Observable(s => s.error(marker)), 2).subscribe({error: e => checks.push(e === marker)});
  }
  globalThis.countOperatorRealms = JSON.stringify(checks);
})();
"#).unwrap();
    let checks: Vec<bool> = serde_json::from_str(&vm.eval("countOperatorRealms").unwrap()).unwrap();
    assert_eq!(checks.len(), 40);
    assert!(checks.iter().all(|value| *value), "{checks:?}");
}

#[test]
fn observable_count_operator_chains_trace_pending_state_and_release_after_early_completion() {
    let mut vm = new_storage_test_vm("https://observable-count-gc.test/");
    vm.eval(r#"
globalThis.countChains = [];
for (const kept of [false, true]) (() => {
  let subscriber;
  const token = {}, callback = value => token && value;
  const source = new Observable(s => { subscriber = s; });
  const mapped = source.map(callback), dropped = mapped.drop(1), taken = dropped.take(2);
  const promise = taken.toArray();
  subscriber.next(1);
  const entry = {kept, templates: [source, mapped, dropped, taken].map(value => new WeakRef(value)),
    refs: {subscriber: new WeakRef(subscriber), callback: new WeakRef(callback), token: new WeakRef(token)}};
  if (kept) entry.promise = promise;
  countChains.push(entry);
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
countChains.every(c => c.templates.every(ref => ref.deref() === undefined)),
countChains.every(c => Object.values(c.refs).every(ref => (ref.deref() !== undefined) === c.kept))
])"#
        )
        .unwrap(),
        "[true,true]"
    );
    vm.eval(
        r#"
const keptCountChain = countChains.find(c => c.kept);
keptCountChain.promise.then(values => { keptCountChain.values = values; });
{
  const subscriber = keptCountChain.refs.subscriber.deref();
  subscriber.next(2); subscriber.next(3);
  globalThis.countSourceClosed = !subscriber.active;
}
"#,
    )
    .unwrap();
    assert_eq!(
        vm.eval("countSourceClosed && JSON.stringify(keptCountChain.values) === '[2,3]'")
            .unwrap(),
        "true"
    );
    collect(&mut vm);
    assert_eq!(
        vm.eval(
            "countChains.every(c => Object.values(c.refs).every(ref => ref.deref() === undefined))"
        )
        .unwrap(),
        "true"
    );
    vm.eval(r#"
globalThis.cancelledCountChain = (() => {
  const ac = new AbortController(), token = {}, callback = value => token && value;
  let subscriber;
  const source = new Observable(s => { subscriber = s; });
  const promise = source.map(callback).drop(1).take(2).toArray({signal: ac.signal});
  promise.catch(() => {}); ac.abort();
  return {promise, subscriber, signal: ac.signal, refs: [new WeakRef(callback), new WeakRef(token)]};
})();
"#).unwrap();
    collect(&mut vm);
    assert_eq!(vm.eval("!cancelledCountChain.subscriber.active && cancelledCountChain.refs.every(ref => ref.deref() === undefined)").unwrap(), "true");
}

#[test]
fn observable_transforms_preserve_lazy_sharing_cancellation_and_callback_semantics() {
    let mut vm = new_storage_test_vm("https://observable-transforms.test/");
    vm.eval(&format!(
        "({}).then(value => {{ globalThis.transformResult = JSON.stringify(value); }});",
        include_str!("../../../tests/fixtures/observable-transforms.js")
    ))
    .expect("Observable transform fixture should evaluate");
    let result: serde_json::Value =
        serde_json::from_str(&vm.eval("transformResult").unwrap()).unwrap();
    assert_eq!(result["failures"], serde_json::json!([]), "{result}");
    assert!(result["checks"].as_u64().unwrap() >= 201, "{result}");
}

#[test]
fn observable_transforms_preserve_result_callback_and_exception_realms() {
    let mut vm = new_storage_test_vm("https://observable-transform-realms.test/");
    vm.eval("document.appendChild(document.createElement('iframe'))")
        .unwrap();
    materialize_single_child_default_realm_for_test(&mut vm, "Observable transform realm");
    vm.eval(r#"
(async () => {
  const child = document.querySelector('iframe').contentWindow, checks = [];
  const callback = child.Function('value', 'index', 'globalThis.transformThis = this; return value;');
  for (const name of ['map', 'filter']) {
    const method = child.Observable.prototype[name];
    delete child.transformThis;
    let starts = 0;
    const source = new Observable(s => { starts++; s.next(1); s.complete(); });
    const result = method.call(source, callback);
    checks.push(result instanceof child.Observable, !(result instanceof Observable), Object.getPrototypeOf(result) === child.Observable.prototype);
    checks.push(child.transformThis === undefined, starts === 0);
    const values = await result.toArray();
    checks.push(child.transformThis === child, values instanceof child.Array, values[0] === 1);
    const local = Observable.prototype[name].call(child.Observable.from([2]), value => value);
    checks.push(local instanceof Observable, !(local instanceof child.Observable));
    const localValues = await local.toArray();
    checks.push(localValues instanceof Array, localValues[0] === 2);
    try { method.call({}, callback); } catch (e) { checks.push(e instanceof child.TypeError, !(e instanceof TypeError)); }
    try { method.call(source, {}); } catch (e) { checks.push(e instanceof child.TypeError, !(e instanceof TypeError)); }
    const marker = new child.Error('transform');
    method.call(source, () => { throw marker; }).subscribe({error: e => checks.push(e === marker, e instanceof child.Error)});
    const invalidInvocation = child.Function('return class Callback {};')();
    method.call(source, invalidInvocation).subscribe({error: e => checks.push(e instanceof child.TypeError, !(e instanceof TypeError))});
  }
  globalThis.transformRealms = JSON.stringify(checks);
})();
"#).unwrap();
    let result: Vec<bool> = serde_json::from_str(&vm.eval("transformRealms").unwrap()).unwrap();
    assert_eq!(result.len(), 40);
    assert!(result.iter().all(|value| *value), "{result:?}");
}

#[test]
fn observable_transform_graphs_trace_live_upstreams_without_rooting_abandoned_chains() {
    let mut vm = new_storage_test_vm("https://observable-transform-gc.test/");
    vm.eval(r#"
globalThis.keptTransforms = [];
globalThis.abandonedTransforms = [];
for (const name of ['map', 'filter']) for (const depth of [1, 3]) {
  for (const kept of [false, true]) (() => {
    let subscriber;
    const token = {}, callback = value => token && (name === 'map' ? value + 1 : true);
    const source = new Observable(s => { subscriber = s; });
    const templates = [new WeakRef(source)];
    let transformed = source;
    for (let i = 0; i < depth; i++) { transformed = transformed[name](callback); templates.push(new WeakRef(transformed)); }
    const promise = transformed.toArray();
    const refs = {subscriber: new WeakRef(subscriber), callback: new WeakRef(callback), token: new WeakRef(token)};
    const entry = {name, depth, templates, refs};
    if (kept) keptTransforms.push({...entry, promise}); else abandonedTransforms.push(entry);
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
abandonedTransforms.every(c => [...c.templates, ...Object.values(c.refs)].every(ref => ref.deref() === undefined)),
keptTransforms.every(c => c.templates.every(ref => ref.deref() === undefined)),
keptTransforms.every(c => Object.values(c.refs).every(ref => ref.deref() !== undefined))
])"#).unwrap(), "[true,true,true]");
    vm.eval(r#"
for (const c of keptTransforms) {
  c.promise.then(values => { c.correct = values.length === 1 && values[0] === (c.name === 'map' ? 3 + c.depth : 3); });
  c.refs.subscriber.deref().next(3); c.refs.subscriber.deref().complete(); delete c.promise;
}
"#).unwrap();
    assert_eq!(
        vm.eval("keptTransforms.every(c => c.correct)").unwrap(),
        "true"
    );
    collect(&mut vm);
    assert_eq!(vm.eval("keptTransforms.every(c => Object.values(c.refs).every(ref => ref.deref() === undefined))").unwrap(), "true");
    vm.eval(r#"
globalThis.cancelledTransforms = [];
for (const name of ['map', 'filter']) (() => {
  const ac = new AbortController(), token = {}, callback = value => token && value;
  let subscriber;
  const source = new Observable(s => { subscriber = s; });
  const promise = source[name](callback)[name](callback).toArray({signal: ac.signal});
  promise.catch(() => {}); ac.abort('cancelled');
  cancelledTransforms.push({promise, signal: ac.signal, subscriber, refs: [new WeakRef(token), new WeakRef(callback)]});
})();
"#).unwrap();
    collect(&mut vm);
    assert_eq!(vm.eval("cancelledTransforms.every(c => !c.subscriber.active && c.refs.every(ref => ref.deref() === undefined))").unwrap(), "true");
}

#[test]
fn observable_preaborted_subscribers_do_not_retain_observers_or_transform_callbacks() {
    let mut vm = new_storage_test_vm("https://observable-preabort-gc.test/");
    vm.eval(
        r#"
globalThis.preabortedSubscribers = [];
globalThis.preabortedCallbacks = [];
for (const name of ['subscribe', 'map', 'filter']) (() => {
  const token = {}, callback = () => token;
  const source = new Observable(s => preabortedSubscribers.push(s));
  const options = {signal: AbortSignal.abort('pre-aborted')};
  if (name === 'subscribe') source.subscribe(callback, options);
  else source[name](callback).subscribe({}, options);
  preabortedCallbacks.push(new WeakRef(token), new WeakRef(callback));
})();
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
    assert_eq!(vm.eval("preabortedSubscribers.length === 3 && preabortedSubscribers.every(s => !s.active && s.signal.reason === 'pre-aborted')").unwrap(), "true");
    assert_eq!(
        vm.eval("preabortedCallbacks.every(ref => ref.deref() === undefined)")
            .unwrap(),
        "true"
    );
}

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
