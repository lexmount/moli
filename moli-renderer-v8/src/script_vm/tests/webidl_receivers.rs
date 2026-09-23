use super::*;

mod document;
mod dom;

#[test]
fn webidl_receiver_checks_reject_prototypes_plain_objects_and_forged_instances() {
    let mut vm = new_storage_test_vm("https://receiver-check.test/");
    let result = vm.eval(r#"
JSON.stringify((() => {
  const failures = [];
  const html = document.appendChild(document.createElement('html'));
  html.appendChild(document.createElement('body'));
  const groups = [
    [Document, ['referrer']],
    [FontFace, ['family','status']],
    [HTMLCanvasElement, ['width','height']],
    [HTMLElement, ['offsetWidth','offsetHeight']],
    [HTMLIFrameElement, ['contentDocument','contentWindow']],
    [IntersectionObserverEntry, ['boundingClientRect','intersectionRect','rootBounds']],
  ];
  for (const [C, keys] of groups) {
    for (const key of keys) {
      const get = Object.getOwnPropertyDescriptor(C.prototype, key).get;
      for (const receiver of [C.prototype, {}, Object.create(C.prototype), new Proxy({}, {}), null]) {
        try { get.call(receiver); failures.push(`${C.name}.${key}:accepted`); }
        catch (e) { if (!(e instanceof TypeError)) failures.push(`${C.name}.${key}:${e.name}`); }
      }
      try { C.prototype[key]; failures.push(`${C.name}.${key}:prototype`); }
      catch (e) { if (!(e instanceof TypeError)) failures.push(e.name); }
    }
  }
  // A genuine DOM reflector of the wrong interface is also not a receiver.
  for (const [C, key, receiver] of [
    [Document, 'referrer', document.body],
    [HTMLCanvasElement, 'width', document.createElement('div')],
    [HTMLIFrameElement, 'contentWindow', document.createElement('div')],
    [HTMLElement, 'offsetHeight', document.createElementNS('http://www.w3.org/2000/svg', 'svg')],
  ]) {
    try { Object.getOwnPropertyDescriptor(C.prototype, key).get.call(receiver); failures.push(`${C.name}:wrong interface`); }
    catch (e) { if (!(e instanceof TypeError)) failures.push(e.name); }
  }
  return failures;
})())
"#).unwrap();
    assert_eq!(result, "[]");
}

#[test]
fn webidl_receiver_checks_preserve_native_values_and_cross_realm_receivers() {
    let mut vm = new_storage_test_vm("https://receiver-check.test/");
    let result = vm.eval(r#"
JSON.stringify((() => {
  const html = document.appendChild(document.createElement('html'));
  html.appendChild(document.createElement('body'));
  const frame = document.createElement('iframe');
  document.body.appendChild(frame);
  const child = frame.contentWindow;
  const canvas = child.document.createElement('canvas');
  canvas.width = 123;
  const face = new child.FontFace('ReceiverTest', 'local("sans-serif")');
  const rect = new DOMRect(1,2,3,4);
  const entry = new IntersectionObserverEntry({
    time: 17, rootBounds: rect, boundingClientRect: rect, intersectionRect: rect,
    target: document.body, isIntersecting: true, intersectionRatio: 1,
  });
  const get = (C, key, receiver) => Object.getOwnPropertyDescriptor(C.prototype, key).get.call(receiver);
  const before = get(IntersectionObserverEntry, 'boundingClientRect', entry);
  Object.defineProperty(entry, 'boundingClientRect', {value: 'shadow'});
  return [
    get(HTMLCanvasElement, 'width', canvas) === 123,
    get(HTMLCanvasElement, 'height', canvas) === 150,
    get(FontFace, 'family', face) === 'ReceiverTest',
    typeof get(FontFace, 'status', face) === 'string',
    typeof get(Document, 'referrer', child.document) === 'string',
    typeof get(HTMLElement, 'offsetWidth', child.document.body) === 'number',
    typeof get(HTMLElement, 'offsetHeight', child.document.body) === 'number',
    get(HTMLIFrameElement, 'contentWindow', frame) === child,
    get(HTMLIFrameElement, 'contentDocument', frame) === child.document,
    before.width === 3 && get(IntersectionObserverEntry, 'boundingClientRect', entry) === before,
    !Object.hasOwn(entry, 'time') && get(IntersectionObserverEntry, 'time', entry) === 17,
    get(IntersectionObserverEntry, 'rootBounds', entry) === rect,
  ];
})())
"#).unwrap();
    assert_eq!(
        result,
        "[true,true,true,true,true,true,true,true,true,true,true,true]"
    );
}

#[test]
fn webidl_receiver_checks_precede_canvas_and_fontface_setter_conversion() {
    let mut vm = new_storage_test_vm("https://receiver-check.test/");
    let result = vm.eval(r#"
JSON.stringify((() => {
  let conversions = 0;
  const value = {valueOf() { conversions++; return 1; }, toString() { conversions++; return 'x'; }};
  const errors = [];
  for (const [C, key] of [[HTMLCanvasElement,'width'],[HTMLCanvasElement,'height'],[FontFace,'family']]) {
    try { Object.getOwnPropertyDescriptor(C.prototype,key).set.call({},value); errors.push('accepted'); }
    catch (e) { errors.push(e.name); }
  }
  return [conversions,...errors];
})())
"#).unwrap();
    assert_eq!(result, r#"[0,"TypeError","TypeError","TypeError"]"#);
}

#[test]
fn webidl_receiver_fontface_load_rejects_its_promise_for_an_invalid_receiver() {
    let mut vm = new_storage_test_vm("https://receiver-check.test/");
    vm.eval(
        r#"
globalThis.loadResult = 'pending';
FontFace.prototype.load.call({}).then(
  () => loadResult = 'resolved',
  error => loadResult = error instanceof TypeError ? 'rejected:TypeError' : error.name
);
"#,
    )
    .unwrap();
    assert_eq!(vm.eval("loadResult").unwrap(), "rejected:TypeError");
}

#[test]
fn webidl_receiver_fontface_loaded_getter_rejects_instead_of_throwing() {
    let mut vm = new_storage_test_vm("https://receiver-check.test/");
    vm.eval(r#"
globalThis.loadedFailures = [];
globalThis.loadedRejections = 0;
const getLoaded = Object.getOwnPropertyDescriptor(FontFace.prototype, 'loaded').get;
const face = new FontFace('ReceiverTest', 'local("sans-serif")');
if (face.loaded !== face.loaded || getLoaded.call(face) !== face.loaded) {
  loadedFailures.push('lost cached Promise identity');
}
for (const receiver of [FontFace.prototype, {}, Object.create(FontFace.prototype), new Proxy(face, {}), null]) {
  try {
    const promise = getLoaded.call(receiver);
    if (!(promise instanceof Promise)) loadedFailures.push('not a Promise');
    promise.then(
      () => loadedFailures.push('resolved'),
      error => {
        loadedRejections++;
        if (!(error instanceof TypeError)) loadedFailures.push(error.name);
      }
    );
  } catch (error) {
    loadedFailures.push('synchronous ' + error.name);
  }
}
"#).unwrap();
    assert_eq!(
        vm.eval("JSON.stringify([loadedRejections, loadedFailures])")
            .unwrap(),
        "[5,[]]"
    );
}

#[test]
fn webidl_receiver_fontface_promise_errors_use_the_callee_realm() {
    let mut vm = new_storage_test_vm("https://receiver-check.test/");
    vm.eval(r#"
globalThis.realmFailures = [];
globalThis.realmRejections = 0;
const html = document.appendChild(document.createElement('html'));
html.appendChild(document.createElement('body'));
const frame = document.body.appendChild(document.createElement('iframe'));
const child = frame.contentWindow;
const getLoaded = Object.getOwnPropertyDescriptor(child.FontFace.prototype, 'loaded').get;
const getFamily = Object.getOwnPropertyDescriptor(child.FontFace.prototype, 'family').get;
try { getFamily.call({}); realmFailures.push('accepted receiver'); }
catch (error) {
  if (!(error instanceof child.TypeError) || error instanceof TypeError) realmFailures.push('wrong exception realm');
}
for (const promise of [getLoaded.call({}), child.FontFace.prototype.load.call({})]) {
  if (!(promise instanceof child.Promise) || promise instanceof Promise) realmFailures.push('wrong Promise realm');
  promise.catch(error => {
    realmRejections++;
    if (!(error instanceof child.TypeError) || error instanceof TypeError) realmFailures.push('wrong rejection realm');
  });
}
const face = new child.FontFace('ReceiverTest', 'local("sans-serif")');
const parentGetLoaded = Object.getOwnPropertyDescriptor(FontFace.prototype, 'loaded').get;
if (parentGetLoaded.call(face) !== face.loaded) realmFailures.push('cross-realm identity');
"#).unwrap();
    assert_eq!(
        vm.eval("JSON.stringify([realmRejections, realmFailures])")
            .unwrap(),
        "[2,[]]"
    );
}

#[test]
fn webidl_receiver_service_worker_operations_reject_before_argument_conversion() {
    let mut vm = new_storage_test_vm("https://service-worker-receiver.test/");
    vm.eval(
        r#"
(() => {
  const sw = navigator.serviceWorker;
  const register = sw.register;
  const getRegistration = sw.getRegistration;
  const getRegistrations = sw.getRegistrations;
  globalThis.serviceWorkerReceiverFailures = [];
  globalThis.serviceWorkerReceiverRejections = 0;
  let conversions = 0;
  const script = {toString() { conversions++; return 'https://['; }};
  const options = {get scope() { conversions++; return './'; }};
  function check(promise) {
    if (!(promise instanceof Promise)) serviceWorkerReceiverFailures.push('not a Promise');
    promise.then(
      () => serviceWorkerReceiverFailures.push('resolved'),
      error => {
        serviceWorkerReceiverRejections++;
        if (!(error instanceof TypeError)) serviceWorkerReceiverFailures.push('wrong error');
      }
    );
  }
  for (const receiver of [{}, Object.create(sw), new Proxy(sw, {}), null]) {
    check(register.call(receiver, script, options));
    check(getRegistration.call(receiver, script));
    check(getRegistrations.call(receiver));
  }
  if (conversions !== 0) serviceWorkerReceiverFailures.push('converted invalid receiver arguments');
  const prototype = Object.getPrototypeOf(sw);
  Object.setPrototypeOf(sw, null);
  check(register.call(sw, script));
  Object.setPrototypeOf(sw, prototype);
  if (conversions !== 1) serviceWorkerReceiverFailures.push('lost native receiver identity');
  if (register.length !== 1 || getRegistration.length !== 0 || getRegistrations.length !== 0) {
    serviceWorkerReceiverFailures.push('operation length');
  }
})()
"#,
    )
    .expect("ServiceWorkerContainer receiver checks should return rejected Promises");
    assert_eq!(
        vm.eval("JSON.stringify([serviceWorkerReceiverRejections, serviceWorkerReceiverFailures])")
            .unwrap(),
        "[13,[]]"
    );
}

#[test]
fn webidl_receiver_service_worker_promises_use_the_callee_realm() {
    let mut vm = new_storage_test_vm("https://service-worker-receiver.test/");
    vm.eval(
        r#"
(() => {
  const html = document.appendChild(document.createElement('html'));
  html.appendChild(document.createElement('body'));
  const child = document.body.appendChild(document.createElement('iframe')).contentWindow;
  const parentSw = navigator.serviceWorker;
  const childSw = child.navigator.serviceWorker;
  const marker = {sentinel: true};
  const throwingScript = {toString() { throw marker; }};
  globalThis.serviceWorkerRealmFailures = [];
  globalThis.serviceWorkerRealmRejections = 0;
  for (const [callback, P, E, expected] of [
    [() => childSw.register.call(parentSw, Symbol('script')), child.Promise, child.TypeError, null],
    [() => parentSw.register.call(childSw, Symbol('script')), Promise, TypeError, null],
    [() => childSw.getRegistration.call(parentSw, Symbol('client')), child.Promise, child.TypeError, null],
    [() => childSw.register.call({}), child.Promise, child.TypeError, null],
    [() => childSw.register.call(parentSw, throwingScript), child.Promise, child.TypeError, marker],
    [() => parentSw.register.call(childSw, throwingScript), Promise, TypeError, marker]
  ]) {
    const promise = callback();
    const otherP = P === Promise ? child.Promise : Promise;
    const otherE = E === TypeError ? child.TypeError : TypeError;
    if (!(promise instanceof P) || promise instanceof otherP) serviceWorkerRealmFailures.push('Promise realm');
    promise.then(
      () => serviceWorkerRealmFailures.push('resolved'),
      error => {
        serviceWorkerRealmRejections++;
        if (expected === marker ? error !== marker : (!(error instanceof E) || error instanceof otherE)) {
          serviceWorkerRealmFailures.push('rejection identity or realm');
        }
      }
    );
  }
})()
"#,
    )
    .expect("ServiceWorkerContainer cross-realm operations should return Promises");
    assert_eq!(
        vm.eval("JSON.stringify([serviceWorkerRealmRejections, serviceWorkerRealmFailures])")
            .unwrap(),
        "[6,[]]"
    );
}
