use super::*;

#[test]
fn dom_receiver_templates_reject_invalid_interfaces_before_conversion() {
    let mut vm = new_storage_test_vm("https://dom-receiver-templates.test/");
    let result = vm.eval(r#"
(() => {
  const failures = [];
  let conversions = 0;
  const value = {toString() { conversions++; return 'changed'; }, valueOf() { conversions++; return 0; }};
  const element = document.createElement('div');
  const text = document.createTextNode('original');
  const svg = document.createElementNS('http://www.w3.org/2000/svg', 'svg');
  const track = document.createElement('track');
  const style = document.createElement('style');
  const cases = [];
  for (const key of ['title', 'head', 'body', 'doctype', 'documentElement']) {
    cases.push([Document, key, 'get', document, element, []]);
  }
  cases.push([Document, 'title', 'set', document, element, [value]]);
  for (const key of ['data', 'length']) {
    cases.push([CharacterData, key, 'get', text, element, []]);
  }
  cases.push([CharacterData, 'data', 'set', text, element, [value]]);
  for (const key of ['appendData', 'insertData', 'deleteData', 'replaceData', 'substringData']) {
    cases.push([CharacterData, key, 'method', text, element, [value, value, value]]);
  }
  cases.push([Text, 'wholeText', 'get', text, document.createComment('comment'), []]);
  cases.push([Text, 'splitText', 'method', text, document.createComment('comment'), [value]]);
  for (const key of ['click', 'focus', 'blur']) {
    cases.push([HTMLElement, key, 'method', element, svg, []]);
  }
  for (const [C, key, real] of [[HTMLTrackElement, 'srclang', track], [HTMLStyleElement, 'media', style]]) {
    cases.push([C, key, 'get', real, element, []]);
    cases.push([C, key, 'set', real, element, [value]]);
  }
  let traps = 0;
  for (const [C, key, kind, real, wrong, args] of cases) {
    const descriptor = Object.getOwnPropertyDescriptor(C.prototype, key);
    const fn = kind === 'method' ? descriptor.value : descriptor[kind];
    const revoked = Proxy.revocable(real, {});
    revoked.revoke();
    const authorProxy = new Proxy(real, {get() { traps++; throw new Error('proxy trap'); }});
    for (const receiver of [wrong, {}, C.prototype, Object.create(C.prototype), Object.create(real), authorProxy, revoked.proxy, null]) {
      try {
        Reflect.apply(fn, receiver, args);
        failures.push(`${C.name}.${key} ${kind}: accepted`);
      } catch (error) {
        if (!(error instanceof TypeError)) failures.push(`${C.name}.${key} ${kind}: ${error.name}`);
      }
    }
  }
  if (text.data !== 'original') failures.push('mutated real target');
  return JSON.stringify([conversions, traps, failures]);
})()
"#).unwrap();
    assert_eq!(result, "[0,0,[]]");
}

#[test]
fn dom_receiver_templates_preserve_cross_realm_native_identity() {
    let mut vm = new_storage_test_vm("https://dom-receiver-realms.test/");
    vm.eval(
        r#"
const root = document.appendChild(document.createElement('html'));
const body = root.appendChild(document.createElement('body'));
body.appendChild(document.createElement('iframe')).id = 'receiver-frame';
"#,
    )
    .unwrap();
    materialize_single_child_default_realm_for_test(&mut vm, "DOM receiver template child Realm");
    let result = vm.eval(r#"
(() => {
  const child = document.getElementById('receiver-frame').contentWindow;
  const failures = [];
  for (const [callee, owner] of [[window, child], [child, window]]) {
    const doc = owner.document.implementation.createHTMLDocument('original');
    const title = Object.getOwnPropertyDescriptor(callee.Document.prototype, 'title');
    const data = Object.getOwnPropertyDescriptor(callee.CharacterData.prototype, 'data');
    const length = Object.getOwnPropertyDescriptor(callee.CharacterData.prototype, 'length').get;
    const text = doc.createTextNode('a');
    const button = doc.createElement('button');
    const click = callee.HTMLElement.prototype.click;
    const hidden = Object.getOwnPropertyDescriptor(callee.HTMLElement.prototype, 'hidden');
    Object.setPrototypeOf(text, null);
    Object.setPrototypeOf(button, null);
    Object.setPrototypeOf(doc, null);
    title.set.call(doc, 'changed');
    if (title.get.call(doc) !== 'changed') failures.push('Document native identity');
    data.set.call(text, 'a\uD800');
    callee.CharacterData.prototype.appendData.call(text, 'b');
    if (data.get.call(text) !== 'a\uD800b' || length.call(text) !== 3) failures.push('CharacterData native identity');
    hidden.set.call(button, true);
    if (hidden.get.call(button) !== true) failures.push('HTMLElement native identity');
    click.call(button);
    for (const invoke of [
      () => title.set.call({}, 'bad'),
      () => length.call({}),
      () => callee.CharacterData.prototype.appendData.call({}, 'bad'),
      () => click.call({})
    ]) {
      try { invoke(); failures.push('accepted invalid cross-realm receiver'); }
      catch (error) {
        if (!(error instanceof callee.TypeError) || error instanceof owner.TypeError) failures.push('wrong error realm');
      }
    }
  }
  return JSON.stringify(failures);
})()
"#).unwrap();
    assert_eq!(result, "[]");
}

#[test]
fn dom_receiver_templates_allow_registered_native_proxy_event_targets() {
    let mut vm = new_storage_test_vm("https://native-proxy-event-target.test/");
    vm.eval(
        r#"
globalThis.nativeReceiver = document.implementation.createHTMLDocument('').createElement('select');
"#,
    )
    .unwrap();
    vm.with_default_context_scope_and_checkpoint_for_test(|scope, _host_ptr| {
        let global = scope.get_current_context().global(scope);
        let key = v8::String::new(scope, "nativeReceiver").unwrap();
        let value = global.get(scope, key.into()).unwrap();
        assert!(
            value.is_proxy(),
            "fixture must exercise a registered native Proxy"
        );
        let object = v8::Local::<v8::Object>::try_from(value).unwrap();
        assert!(crate::web_api_interfaces::EventTarget::is_instance(
            scope, object
        ));
        Ok(())
    })
    .unwrap();
    let result = vm.eval(r#"
(() => {
  const receiver = nativeReceiver;
  const failures = [];
  const {addEventListener: add, removeEventListener: remove, dispatchEvent: dispatch} = EventTarget.prototype;
  let calls = 0;
  const listener = () => calls++;
  add.call(receiver, 'test', listener);
  if (dispatch.call(receiver, new Event('test')) !== true) failures.push('first dispatch');
  remove.call(receiver, 'test', listener);
  if (dispatch.call(receiver, new Event('test')) !== true) failures.push('second dispatch');
  if (calls !== 1) failures.push(`listener calls: ${calls}`);
  let conversions = 0;
  const type = {toString() { conversions++; return 'test'; }};
  for (const proxy of [new Proxy(receiver, {}), Object.create(receiver)]) {
    for (const invoke of [() => add.call(proxy, type, listener), () => remove.call(proxy, type, listener), () => dispatch.call(proxy, new Event('test'))]) {
      try { invoke(); failures.push('accepted author receiver'); }
      catch (error) { if (!(error instanceof TypeError)) failures.push(error.name); }
    }
  }
  if (conversions !== 0) failures.push('converted author receiver arguments');
  return JSON.stringify(failures);
})()
"#).unwrap();
    assert_eq!(result, "[]");
}

#[test]
fn dom_receiver_templates_whole_text_preserves_cdata_and_utf16() {
    let mut vm = new_parsed_test_vm(
        "https://dom-receiver-whole-text.test/",
        "<html><body></body></html>",
    );
    let result = vm
        .eval(
            r#"
(() => {
  const failures = [];
  const xml = document.implementation.createDocument(null, 'root');
  const get = Object.getOwnPropertyDescriptor(Text.prototype, 'wholeText').get;
  const data = Object.getOwnPropertyDescriptor(CharacterData.prototype, 'data');
  for (const doc of [document, document.implementation.createHTMLDocument(''), xml]) {
    const parent = doc.createElement('section');
    (doc.body || doc.documentElement).appendChild(parent);
    const left = doc.createTextNode('a');
    const middle = doc.adoptNode(xml.createCDATASection('b'));
    const right = doc.createTextNode('c');
    parent.append(left, middle, right);
    for (const node of [left, middle, right]) {
      if (get.call(node) !== 'abc') failures.push('contiguous Text and CDATA');
    }
    data.set.call(left, 'a\uD800');
    data.set.call(middle, '\uDC00b');
    data.set.call(right, 'c\uD800');
    for (const node of [left, middle, right]) {
      if (get.call(node) !== 'a\uD800\uDC00bc\uD800') failures.push('UTF-16 across nodes');
    }
    const barrier = parent.insertBefore(doc.createComment('barrier'), middle);
    if (get.call(left) !== 'a\uD800') failures.push('left boundary');
    for (const node of [middle, right]) {
      if (get.call(node) !== '\uDC00bc\uD800') failures.push('right boundary');
    }
    parent.removeChild(barrier);
    if (get.call(middle) !== 'a\uD800\uDC00bc\uD800') failures.push('removed boundary');
  }
  return JSON.stringify(failures);
})()
"#,
        )
        .unwrap();
    assert_eq!(result, "[]");
}

#[test]
fn dom_receiver_templates_character_data_edits_invalidate_detached_xpath_iterators() {
    let mut vm = new_storage_test_vm("https://dom-receiver-text-mutations.test/");
    let result = vm.eval(r#"
(() => {
  const failures = [];
  const iterate = doc => doc.evaluate('//div', doc, null, XPathResult.ORDERED_NODE_ITERATOR_TYPE);
  const otherDoc = document.implementation.createHTMLDocument('');
  const other = iterate(otherDoc);
  for (const [name, edit, expected] of [
    ['data', text => { text.data = 'after'; }, 'after'],
    ['appendData', text => text.appendData('!'), 'before!'],
    ['insertData', text => text.insertData(1, '!'), 'b!efore'],
    ['deleteData', text => text.deleteData(1, 2), 'bore'],
    ['replaceData', text => text.replaceData(1, 2, '!'), 'b!ore'],
    ['splitText', text => text.splitText(3), 'before']
  ]) {
    const doc = document.implementation.createHTMLDocument('');
    const parent = doc.body.appendChild(doc.createElement('div'));
    const text = parent.appendChild(doc.createTextNode('before'));
    const iterator = iterate(doc);
    const snapshot = doc.evaluate('//div', doc, null, XPathResult.ORDERED_NODE_SNAPSHOT_TYPE);
    edit(text);
    if (!iterator.invalidIteratorState) failures.push(`${name}: iterator remained valid`);
    try { iterator.iterateNext(); failures.push(`${name}: iterateNext succeeded`); }
    catch (error) { if (error.name !== 'InvalidStateError') failures.push(`${name}: ${error.name}`); }
    if (snapshot.invalidIteratorState || snapshot.snapshotItem(0) !== parent) failures.push(`${name}: snapshot changed`);
    const value = doc.evaluate('string(//div)', doc, null, XPathResult.STRING_TYPE).stringValue;
    if (value !== expected) failures.push(`${name}: stale data ${value}`);
  }
  if (other.invalidIteratorState) failures.push('invalidated unrelated document');
  return JSON.stringify(failures);
})()
"#).unwrap();
    assert_eq!(result, "[]");
}
