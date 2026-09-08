use super::*;

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
