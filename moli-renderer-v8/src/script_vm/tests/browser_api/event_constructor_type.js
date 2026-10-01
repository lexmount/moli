(() => {
  const rows = [];
  const check = (name, passed) => rows.push({name, passed: !!passed});
  const throws = (fn, expected) => {
    try { fn(); return false; } catch (error) { return expected(error); }
  };
  const raw = 'type\ud800\0\udc00';
  const types = ['', 'load', '\ud800', '\udc00', raw, '\ud800\ud800',
    '\udc00\ud800', '\ud83d\ude80', '\ufffd', '事件\0type'];
  const names = ['Event', 'CustomEvent', 'UIEvent', 'FocusEvent', 'CompositionEvent',
    'MouseEvent', 'DragEvent', 'KeyboardEvent', 'WheelEvent', 'PointerEvent', 'TouchEvent',
    'MessageEvent', 'ErrorEvent', 'CloseEvent', 'SubmitEvent', 'InputEvent', 'PopStateEvent',
    'PageTransitionEvent', 'StorageEvent', 'SecurityPolicyViolationEvent', 'ClipboardEvent',
    'HashChangeEvent', 'PromiseRejectionEvent', 'FormDataEvent'];
  const init = w => ({bubbles: true, cancelable: true, composed: true,
    promise: w.Promise.resolve(), formData: new w.FormData()});
  for (const name of names) {
    const Constructor = window[name];
    for (const type of types) {
      const event = new Constructor(type, init(window));
      check(name + ' exact type ' + JSON.stringify(type), event.type === type &&
        event.type.length === type.length && event instanceof Event);
    }
    const order = [];
    const event = new Constructor({[Symbol.toPrimitive](hint) {
      order.push(hint); return raw;
    }}, {...init(window), get bubbles() { order.push('bubbles'); return true; }});
    check(name + ' converts once before dictionary', event.type === raw &&
      order.join(',') === 'string,bubbles');
    check(name + ' retains flags and fresh state', event.bubbles && event.cancelable &&
      event.composed && !event.isTrusted && !event.defaultPrevented && event.target === null);
    let reads = 0;
    const dictionary = {...init(window), get bubbles() { reads++; return true; }};
    check(name + ' Symbol rejects before dictionary', throws(() => new Constructor(Symbol(), dictionary),
      error => error instanceof TypeError) && reads === 0);
    check(name + ' Symbol primitive rejects before dictionary', throws(() => new Constructor({
      [Symbol.toPrimitive]() { return Symbol(); }
    }, dictionary), error => error instanceof TypeError) && reads === 0);
    const sentinel = new RangeError('type conversion');
    check(name + ' preserves conversion exception', throws(() => new Constructor({
      toString() { throw sentinel; }
    }, dictionary), error => error === sentinel) && reads === 0);
    check(name + ' missing type rejects', throws(() => new Constructor(),
      error => error instanceof TypeError));
    class Derived extends Constructor {}
    const derived = new Derived(raw, init(window));
    check(name + ' derived constructor retains type', derived.type === raw &&
      Object.getPrototypeOf(derived) === Derived.prototype && derived instanceof Constructor);
  }
  for (const type of [undefined, null, true, false, 7, 1n, new String(raw)]) {
    check('Event ToString ' + String(type), new Event(type).type === String(type));
    check('CustomEvent ToString ' + String(type), new CustomEvent(type).type === String(type));
  }
  let conversions = 0;
  const fallback = {toString() { conversions++; return {}; }, valueOf() { conversions++; return raw; }};
  check('ordinary ToPrimitive fallback retains units', new Event(fallback).type === raw && conversions === 2);
  const frozen = Object.freeze(new CustomEvent(raw, {detail: fallback}));
  check('frozen wrapper retains type and payload', frozen.type === raw && frozen.detail === fallback);
  const child = document.querySelector('#child').contentWindow;
  for (const name of names) {
    const Constructor = child[name];
    const event = new Constructor({toString() { return raw; }}, init(child));
    check(name + ' cross realm exact type', event.type === raw && event instanceof child.Event &&
      Object.getPrototypeOf(event) === Constructor.prototype);
    let reads = 0;
    const dictionary = {...init(child), get bubbles() { reads++; return true; }};
    check(name + ' conversion error belongs to callee realm', throws(() => new Constructor(Symbol(), dictionary),
      error => error instanceof child.TypeError && !(error instanceof TypeError)) && reads === 0);
    const sentinel = new RangeError('cross realm type conversion');
    check(name + ' cross realm exception identity', throws(() => new Constructor({
      toString() { throw sentinel; }
    }, dictionary), error => error === sentinel) && reads === 0);
    const withLocalPrototype = Reflect.construct(Constructor, [raw, init(child)], window[name]);
    check(name + ' Reflect.construct retains type and newTarget', withLocalPrototype.type === raw &&
      Object.getPrototypeOf(withLocalPrototype) === window[name].prototype);
  }
  globalThis.__eventConstructorTypeResults = {rows, passed: rows.filter(row => row.passed).length,
    total: rows.length, constructors: names};
  return rows.every(row => row.passed) || JSON.stringify(rows.filter(row => !row.passed));
})()
