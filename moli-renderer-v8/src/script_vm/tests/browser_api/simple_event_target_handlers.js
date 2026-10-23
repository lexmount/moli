function checkHandlerDispatch(target, type, realm = globalThis) {
  const property = 'on' + type;
  const trace = [];
  const check = (actual, expected, step) => {
    if (JSON.stringify(actual) !== JSON.stringify(expected)) {
      throw new Error(`${target.constructor.name}.${property} ${step}: ${JSON.stringify(actual)}`);
    }
  };
  const fire = () => target.dispatchEvent(new realm.Event(type, {cancelable: true}));
  let during = () => {};
  const first = event => { trace.push('first'); during(event); };
  const last = () => trace.push('last');
  const capture = () => trace.push('capture');
  const handler = () => { trace.push('handler'); return false; };
  target.addEventListener(type, first);
  target[property] = handler;
  target.addEventListener(type, last);
  target.addEventListener(type, capture, true);
  check(fire(), false, 'handler cancellation');
  check(trace.splice(0), ['first', 'handler', 'last', 'capture'], 'registration order');

  during = () => { target[property] = () => trace.push('replacement'); };
  check(fire(), true, 'replacement cancellation');
  check(trace.splice(0), ['first', 'replacement', 'last', 'capture'], 'replacement during dispatch');

  during = () => { target[property] = null; target[property] = handler; };
  fire();
  check(trace.splice(0), ['first', 'last', 'capture'], 'removed entry stays inactive in snapshot');
  during = () => {};
  fire();
  check(trace.splice(0), ['first', 'last', 'capture', 'handler'], 'reactivation appends');

  during = event => event.stopImmediatePropagation();
  fire();
  check(trace.splice(0), ['first'], 'earlier listener stops handler');
  during = () => {};
  target[property] = null;
  target.removeEventListener(type, last);
  target[property] = event => { trace.push('handler'); event.stopImmediatePropagation(); };
  target.addEventListener(type, last);
  fire();
  check(trace.splice(0), ['first', 'capture', 'handler'], 'handler stops later listener');

  target[property] = handler;
  during = event => event.stopPropagation();
  fire();
  check(trace.splice(0), ['first', 'capture', 'handler', 'last'], 'stopPropagation keeps same target');
  during = () => {};

  // Dispatch uses the registered handler even when script shadows its IDL
  // accessor, and never reads arbitrary on-prefixed properties.
  let reads = 0;
  const ownDescriptor = Object.getOwnPropertyDescriptor(target, property);
  Object.defineProperty(target, property, {configurable: true, get() { reads++; throw new Error('handler getter read'); }});
  fire();
  check(trace.splice(0), ['first', 'capture', 'handler', 'last'], 'shadowed accessor retains listener');
  check(reads, 0, 'no property lookup during dispatch');
  delete target[property];
  if (ownDescriptor) Object.defineProperty(target, property, ownDescriptor);
  target[property] = null;
  for (const callback of [first, last]) target.removeEventListener(type, callback);
  target.removeEventListener(type, capture, true);
}

function checkUnregisteredHandler(target, realm = globalThis) {
  let calls = 0;
  target.onunregistered = () => calls++;
  target.dispatchEvent(new realm.Event('unregistered'));
  if (calls !== 0) throw new Error('ordinary on-prefixed property was invoked');
  Object.defineProperty(target, 'onunregistered', {configurable: true, get() { throw new Error('ordinary on-prefixed getter was read'); }});
  target.dispatchEvent(new realm.Event('unregistered'));
  delete target.onunregistered;
}

function checkListenerIsolation(targets, realm = globalThis) {
  const type = 'registry-isolation';
  const trace = [];
  function listener() { trace.push(targets.indexOf(this)); }
  for (const target of targets) {
    // Ordinary properties cannot select or replace private listener storage.
    target.__moliEventTargetSlot = 'page-owned-slot';
    target.__moliEventTargetListeners = {};
    target.addEventListener(type, listener);
  }
  for (let index = 0; index < targets.length; index++) {
    targets[index].dispatchEvent(new realm.Event(type));
    if (JSON.stringify(trace.splice(0)) !== JSON.stringify([index])) {
      throw new Error('listener registry leaked across targets at ' + index);
    }
  }
  targets[0].removeEventListener(type, listener);
  for (let index = 0; index < targets.length; index++) {
    targets[index].dispatchEvent(new realm.Event(type));
    const expected = index === 0 ? [] : [index];
    if (JSON.stringify(trace.splice(0)) !== JSON.stringify(expected)) {
      throw new Error('removing a listener changed another target at ' + index);
    }
    targets[index].removeEventListener(type, listener);
    delete targets[index].__moliEventTargetSlot;
    delete targets[index].__moliEventTargetListeners;
  }
}
