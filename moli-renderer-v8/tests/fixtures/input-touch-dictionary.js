(() => {
  const assert = (value, label) => { if (!value) throw Error(label); };
  const base = ['bubbles', 'cancelable', 'composed'];
  const ui = ['detail', 'sourceCapabilities', 'view'];
  const modifiers = ['altKey', 'ctrlKey', 'metaKey', 'modifierAltGraph', 'modifierCapsLock',
    'modifierFn', 'modifierNumLock', 'modifierScrollLock', 'modifierSymbol', 'shiftKey'];
  const cases = [
    [InputEvent, [...base, ...ui, 'data', 'dataTransfer', 'inputType', 'isComposing', 'targetRanges']],
    [TouchEvent, [...base, ...ui, ...modifiers, 'changedTouches', 'targetTouches', 'touches']]
  ];
  for (const [Constructor, members] of cases) {
    for (const value of [1, true, 'init', 1n, Symbol('init')]) {
      let caught;
      try { new Constructor('audit', value); } catch (e) { caught = e; }
      assert(caught instanceof TypeError, Constructor.name + ' primitive dictionary');
    }
    const reads = [];
    const options = new Proxy({}, {get(_, key) {
      if (members.includes(key)) reads.push(key);
      return base.includes(key) ? true : undefined;
    }, has() {throw Error('Get required');}});
    const event = new Constructor('audit', options);
    assert(reads.join() === members.join(), Constructor.name + ' inherited conversion order: ' + reads);
    assert(event.bubbles && event.cancelable && event.composed, Constructor.name + ' flags');
    for (const member of members) for (const sentinel of [{}, undefined, null, 7]) {
      reads.length = 0;
      const bad = new Proxy({}, {get(_, key) {
        if (members.includes(key)) reads.push(key);
        if (key === member) throw sentinel;
        return undefined;
      }});
      let caught = Symbol();
      try { new Constructor('audit', bad); } catch (e) { caught = e; }
      assert(caught === sentinel, Constructor.name + '/' + member + ' exception');
      assert(reads.join() === members.slice(0, members.indexOf(member) + 1).join(),
        Constructor.name + '/' + member + ' abrupt completion: ' + reads);
    }
  }
  const transfer = new DataTransfer();
  Object.setPrototypeOf(transfer, null);
  const utf16 = '\ud800\0\udc00';
  const input = new InputEvent('audit', {data: utf16, inputType: 'insertText', dataTransfer: transfer});
  assert(input.data === utf16 && input.inputType === 'insertText' && input.dataTransfer === transfer, 'InputEvent native and UTF16');
  assert(new InputEvent('audit').data === null && new InputEvent('audit', {data: null}).data === null, 'nullable data');
  assert(new InputEvent('audit', {data: {toString() {return 'text';}}}).data === 'text', 'data DOMString');
  for (const dataTransfer of [{}, new Proxy(transfer, {})]) {
    let caught;
    try { new InputEvent('audit', {dataTransfer}); } catch (e) { caught = e; }
    assert(caught instanceof TypeError, 'InputEvent DataTransfer brand');
  }
  const text = document.createTextNode('abcdef'); document.body.append(text);
  const original = new StaticRange({startContainer: text, startOffset: 1, endContainer: text, endOffset: 4});
  Object.setPrototypeOf(original, null);
  Object.defineProperty(original, 'startContainer', {get() {throw Error('public range boundary');}});
  const NativeRange = Range, NativeStaticRange = StaticRange;
  let ranged, a, b;
  try {
    globalThis.Range = function () {throw Error('public Range constructor');};
    globalThis.StaticRange = function () {throw Error('public StaticRange constructor');};
    ranged = new InputEvent('audit', {targetRanges: [original]});
    a = ranged.getTargetRanges(); b = ranged.getTargetRanges();
  } finally {
    globalThis.Range = NativeRange; globalThis.StaticRange = NativeStaticRange;
  }
  assert(a !== b && a[0] !== b[0] && a[0] !== original && a[0] instanceof StaticRange, 'fresh native snapshots');
  text.deleteData(0, 2);
  const after = ranged.getTargetRanges()[0];
  assert(after.startContainer === text && after.startOffset === 0 && after.endOffset === 2, 'internal live range');
  let caught;
  try { InputEvent.prototype.getTargetRanges.call({}); } catch (e) { caught = e; }
  assert(caught instanceof TypeError, 'range method native receiver');
  const touch = new Touch({identifier: 3, target: document.body, clientX: 2});
  Object.setPrototypeOf(touch, null);
  const touched = new TouchEvent('audit', {touches: new Set([touch]), altKey: true, ctrlKey: true});
  assert(touched.touches.length === 1 && touched.touches[0] === touch && touched.touches.item(0) === touch, 'native Touch sequence');
  assert(touched.altKey && touched.ctrlKey && !touched.metaKey && !touched.shiftKey, 'TouchEvent modifiers');
  try {
    Object.defineProperty(TouchList.prototype, '0', {configurable: true, set() {throw Error('author TouchList setter');}});
    assert(new TouchEvent('audit', {touches: [touch]}).touches[0] === touch, 'intrinsic TouchList item creation');
  } finally { delete TouchList.prototype[0]; }
  for (const [Constructor, member, native] of [[InputEvent, 'targetRanges', original],
    [TouchEvent, 'changedTouches', touch], [TouchEvent, 'targetTouches', touch], [TouchEvent, 'touches', touch]]) {
    for (const value of [null, {}, [new Proxy(native, {})]]) {
      let caught;
      try { new Constructor('audit', {[member]: value}); } catch (e) { caught = e; }
      assert(caught instanceof TypeError, Constructor.name + '/' + member + ' strict sequence/native brand');
    }
    let next = 0;
    const sequence = {[Symbol.iterator]() {return {next() {
      if (next++ === 0) return {done: false, value: {}};
      throw Error('must validate first item');
    }}}};
    let caught;
    try { new Constructor('audit', {[member]: sequence}); } catch (e) { caught = e; }
    assert(caught instanceof TypeError && next === 1, Constructor.name + '/' + member + ' item conversion timing');
  }
  text.remove();
  return true;
})()
