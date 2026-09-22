function abortSignalEventTargetProbe(realm = globalThis, methods = realm.EventTarget.prototype) {
  const failures = [];
  const scenarios = [];
  const check = (condition, label) => { if (!condition) failures.push(label); };
  const scenario = (name, run) => {
    scenarios.push(name);
    try { run(); } catch (error) { failures.push(name + ': ' + error.name + ': ' + error.message); }
  };
  scenario('inherited methods', () => {
    const signal = new realm.AbortController().signal;
    check(Object.getPrototypeOf(realm.AbortSignal.prototype) === realm.EventTarget.prototype, 'parent prototype');
    for (const name of ['addEventListener', 'removeEventListener', 'dispatchEvent']) {
      check(!Object.hasOwn(realm.AbortSignal.prototype, name), 'own method ' + name);
      check(signal[name] === realm.EventTarget.prototype[name], 'method identity ' + name);
    }
    check(realm.AbortSignal.abort.length === 0, 'optional abort reason length');
  });
  scenario('borrowed native listeners', () => {
    const controller = new realm.AbortController();
    const signal = controller.signal;
    const calls = [];
    function kept(event) {
      calls.push('kept');
      check(this === signal && event.target === signal && event.currentTarget === signal, 'native callback receiver');
      check(event.eventPhase === 2 && event.isTrusted && signal.aborted && signal.reason === 'reason', 'native abort event state');
    }
    const removed = () => calls.push('removed');
    methods.addEventListener.call(signal, 'abort', kept);
    signal.addEventListener('abort', kept);
    signal.addEventListener('abort', removed);
    methods.removeEventListener.call(signal, 'abort', removed);
    controller.abort('reason');
    controller.abort('again');
    check(calls.join() === 'kept', 'native listener sharing: ' + calls);
  });
  scenario('ordered handler and capture', () => {
    const signal = new realm.AbortController().signal;
    const calls = [];
    signal.onabort = () => calls.push('old');
    methods.addEventListener.call(signal, 'abort', () => calls.push('bubble'));
    signal.onabort = () => { calls.push('handler'); return false; };
    signal.addEventListener('abort', () => calls.push('capture'), true);
    const event = new realm.Event('abort', {cancelable: true});
    check(methods.dispatchEvent.call(signal, event) === false && event.defaultPrevented, 'handler cancels event');
    check(calls.join() === 'capture,handler,bubble', 'listener order: ' + calls);
    check(!signal.aborted && signal.reason === undefined, 'synthetic abort preserves signal state');
    check(event.target === signal && event.currentTarget === null && event.eventPhase === 0 &&
      event.composedPath().length === 0, 'dispatch cleanup');
    signal.onabort = null;
    signal.onabort = () => calls.push('last');
    calls.length = 0;
    signal.dispatchEvent(new realm.Event('abort'));
    check(calls.join() === 'capture,bubble,last', 'handler reactivation: ' + calls);
    const handler = {handleEvent() { failures.push('EventHandler invoked handleEvent'); }};
    signal.onabort = handler;
    check(signal.onabort === handler, 'noncallable EventHandler identity');
    calls.length = 0;
    signal.dispatchEvent(new realm.Event('abort'));
    check(calls.join() === 'capture,bubble', 'object replaces callable handler: ' + calls);
    signal.addEventListener('abort', () => calls.push('tail'));
    signal.onabort = () => calls.push('restored');
    calls.length = 0;
    signal.dispatchEvent(new realm.Event('abort'));
    check(calls.join() === 'capture,bubble,restored,tail', 'object preserves handler position: ' + calls);
    signal.onabort = 4;
    check(signal.onabort === null, 'primitive EventHandler conversion');
  });
  scenario('signal option removal', () => {
    const target = new realm.AbortController().signal;
    const controller = new realm.AbortController();
    let count = 0;
    const callback = () => ++count;
    target.addEventListener('x', callback, {signal: controller.signal});
    methods.dispatchEvent.call(target, new realm.Event('x'));
    controller.abort();
    target.dispatchEvent(new realm.Event('x'));
    methods.addEventListener.call(target, 'x', callback, {signal: controller.signal});
    target.dispatchEvent(new realm.Event('x'));
    check(count === 1, 'aborted option listener count: ' + count);
    const owner = new realm.AbortController();
    owner.signal.addEventListener('abort', () => failures.push('self-signal listener'), {signal: owner.signal});
    owner.abort();
  });
  scenario('dispatch phase mutation', () => {
    const signal = new realm.AbortController().signal;
    const calls = [];
    const removed = () => calls.push('removed');
    signal.addEventListener('x', removed);
    signal.addEventListener('x', () => {
      calls.push('capture');
      signal.removeEventListener('x', removed);
      signal.addEventListener('x', () => calls.push('late'));
    }, {capture: true, once: true});
    signal.addEventListener('x', () => calls.push('bubble'));
    methods.dispatchEvent.call(signal, new realm.Event('x'));
    check(calls.join() === 'capture,bubble,late', 'phase snapshot: ' + calls);
    calls.length = 0;
    signal.dispatchEvent(new realm.Event('x'));
    check(calls.join() === 'bubble,late', 'once listener removed: ' + calls);
  });
  scenario('reentrant once and passive', () => {
    const signal = new realm.AbortController().signal;
    let once = 0;
    signal.addEventListener('x', () => {
      ++once;
      signal.dispatchEvent(new realm.Event('x'));
    }, {once: true});
    signal.dispatchEvent(new realm.Event('x'));
    check(once === 1, 'once listener recursion');
    signal.addEventListener('x', event => event.preventDefault(), {passive: true});
    const event = new realm.Event('x', {cancelable: true});
    check(signal.dispatchEvent(event) === true && !event.defaultPrevented, 'passive listener cancellation');
  });
  return {scenarios, failures};
}

function abortSignalReceiverProbe(realm = globalThis) {
  const failures = [];
  let checks = 0;
  const signal = new realm.AbortController().signal;
  const revoked = Proxy.revocable(signal, {});
  revoked.revoke();
  const invalid = [{}, Object.create(realm.AbortSignal.prototype), Object.create(signal),
    new Proxy(signal, {}), revoked.proxy];
  const prototype = realm.AbortSignal.prototype;
  let reads = 0;
  const type = {toString() { ++reads; return 'x'; }};
  const options = {get capture() { ++reads; return false; }};
  const operations = [
    value => prototype.addEventListener.call(value, type, () => {}, options),
    value => prototype.removeEventListener.call(value, type, () => {}, options),
    value => prototype.dispatchEvent.call(value, new realm.Event('x')),
    value => prototype.throwIfAborted.call(value),
    ...['aborted', 'reason', 'onabort'].map(name =>
      value => Object.getOwnPropertyDescriptor(prototype, name).get.call(value)),
    value => Object.getOwnPropertyDescriptor(prototype, 'onabort').set.call(value, () => {}),
  ];
  for (const [index, operation] of operations.entries()) for (const value of invalid) {
    ++checks;
    try { operation(value); failures.push('accepted receiver ' + index); }
    catch (error) { if (!(error instanceof realm.TypeError)) failures.push('wrong exception realm ' + index); }
  }
  if (reads) failures.push('converted arguments before receiver check');
  let dictionaryReads = [];
  signal.addEventListener('x', null, new Proxy({}, {get(_, key) {
    dictionaryReads.push(key); return undefined;
  }}));
  if (dictionaryReads.join() !== 'capture,once,passive,signal') failures.push('dictionary order: ' + dictionaryReads);
  const sentinel = new realm.Error('capture conversion');
  let invoked = false;
  try {
    signal.addEventListener('failed-conversion', () => { invoked = true; }, {
      get capture() { throw sentinel; },
      get once() { failures.push('conversion continued after capture threw'); return false; },
    });
    failures.push('accepted throwing options');
  } catch (error) {
    if (error !== sentinel) failures.push('replaced options exception');
  }
  signal.dispatchEvent(new realm.Event('failed-conversion'));
  if (invoked) failures.push('registered listener after options exception');
  const removedReads = [];
  signal.removeEventListener('x', null, new Proxy({}, {get(_, key) {
    removedReads.push(key); return undefined;
  }}));
  if (removedReads.join() !== 'capture') failures.push('remove dictionary order: ' + removedReads);
  return {checks, failures};
}

function abortSignalLifetimeProbe() {
  const failures = [];
  const frame = document.body.appendChild(document.createElement('iframe'));
  const realm = frame.contentWindow;
  const controller = new realm.AbortController();
  const signal = controller.signal;
  const methods = [EventTarget.prototype.dispatchEvent, realm.EventTarget.prototype.dispatchEvent];
  let calls = 0;
  signal.addEventListener('x', () => ++calls);
  frame.remove();
  for (const method of methods) {
    const event = new Event('x');
    try {
      if (method.call(signal, event) !== false || event.target !== null) failures.push('retired dispatch result');
    } catch (error) { failures.push('retired dispatch: ' + error.name); }
    for (const [value, expected] of [[null, 'TypeError'], [{}, 'TypeError'], [document.createEvent('Event'), 'InvalidStateError']]) {
      try { method.call(signal, value); failures.push('accepted invalid event'); }
      catch (error) { if (error.name !== expected) failures.push('wrong retired error: ' + error.name); }
    }
  }
  if (calls) failures.push('retired callback invoked');
  return {calls, failures};
}
