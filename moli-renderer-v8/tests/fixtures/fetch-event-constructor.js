async function runFetchEventConstructorChecks() {
  const assert = (condition, message) => { if (!condition) throw Error(message); };
  const thrown = callback => {
    try { callback(); } catch (error) { return error; }
    throw Error('expected exception');
  };
  const C = FetchEvent, P = Promise, R = Request;
  const request = new R('https://example.test/input', {method: 'POST', body: 'body'});
  const event = new C('\ud800\0\udfff', {request});
  assert(C.name === 'FetchEvent' && C.length === 2, 'constructor metadata');
  assert(Object.getPrototypeOf(C) === ExtendableEvent, 'constructor parent');
  assert(Object.getPrototypeOf(C.prototype) === ExtendableEvent.prototype, 'prototype parent');
  assert(event instanceof C && event instanceof ExtendableEvent && event instanceof Event, 'inheritance');
  assert(Object.prototype.toString.call(event) === '[object FetchEvent]', 'toStringTag');
  assert(event.type === '\ud800\0\udfff' && !event.bubbles && !event.cancelable && !event.composed, 'default Event state');
  assert(event.isTrusted === false && event.request === request && !request.bodyUsed, 'request identity without consumption');
  assert(event.clientId === '' && event.resultingClientId === '' && event.replacesClientId === '', 'default IDs');
  assert(event.preloadResponse === event.preloadResponse && await event.preloadResponse === undefined, 'default preload');
  assert(event.handled === event.handled && event.handled instanceof P, 'handled identity');
  let handledSettled = false;
  event.handled.then(() => { handledSettled = true; }, () => { handledSettled = true; });
  await 0; await 0;
  assert(!handledSettled, 'default handled stays pending');
  for (const name of ['request', 'preloadResponse', 'clientId', 'resultingClientId', 'replacesClientId', 'handled']) {
    const descriptor = Object.getOwnPropertyDescriptor(C.prototype, name);
    assert(descriptor && typeof descriptor.get === 'function' && descriptor.set === undefined && descriptor.enumerable && descriptor.configurable, 'readonly descriptor ' + name);
    assert(!Object.hasOwn(event, name), 'state is not an own public data property ' + name);
    assert(thrown(() => descriptor.get.call({})) instanceof TypeError, 'receiver brand ' + name);
    assert(thrown(() => descriptor.get.call(Object.create(C.prototype))) instanceof TypeError, 'forged receiver ' + name);
    assert(Reflect.set(event, name, null) === false, 'readonly assignment ' + name);
  }
  assert(C.prototype.respondWith.length === 1, 'respondWith arity');
  for (const callback of [() => C('fetch', {request}), () => new C(), () => new C('fetch'), () => new C('fetch', {}), () => new C('fetch', {request: null}), () => new C('fetch', {request: {}}), () => new C('fetch', {request: Object.create(R.prototype)}), () => new C('fetch', 1)]) {
    assert(thrown(callback) instanceof TypeError, 'constructor validation');
  }
  assert(thrown(() => event.respondWith()) instanceof TypeError, 'required method argument');
  for (const callback of [() => event.respondWith(new Response('synthetic')), () => event.waitUntil(P.resolve())]) {
    const error = thrown(callback);
    assert(error instanceof DOMException && error.name === 'InvalidStateError', 'synthetic lifetime guard');
  }
  const marker = {}, log = [];
  const thenable = name => ({get then() {
    log.push(name + '.then');
    return resolve => { log.push(name + '.resolve'); resolve(marker); };
  }});
  const init = {};
  const values = {bubbles: true, cancelable: true, composed: true,
    clientId: '\udfff\0client', handled: thenable('handled'), preloadResponse: thenable('preload'),
    replacesClientId: null, request, resultingClientId: 123};
  for (const name of Object.keys(values)) Object.defineProperty(init, name, {get() { log.push(name); return values[name]; }});
  const custom = new C({toString() { log.push('type'); return 'custom'; }}, Object.create(init));
  assert(log.join() === 'type,bubbles,cancelable,composed,clientId,handled,handled.then,preloadResponse,preload.then,replacesClientId,request,resultingClientId', 'dictionary and promise conversion order: ' + log);
  assert(custom.bubbles && custom.cancelable && custom.composed && !custom.defaultPrevented, 'explicit Event flags');
  custom.preventDefault();
  assert(custom.defaultPrevented, 'Event.preventDefault uses native Event state');
  assert(custom.clientId === '\udfff\0client' && custom.resultingClientId === '123' && custom.replacesClientId === 'null', 'lossless DOMString conversion');
  assert(await custom.handled === marker && await custom.preloadResponse === marker, 'promise assimilation preserves values');
  const sentinel = {}, reads = [];
  const error = thrown(() => new C('bad', {
    get clientId() { reads.push('clientId'); throw sentinel; },
    get request() { reads.push('request'); return request; }
  }));
  assert(error === sentinel && reads.join() === 'clientId', 'getter exceptions keep identity and stop conversion');
  reads.length = 0;
  assert(thrown(() => new C('bad', {request: null, get resultingClientId() { reads.push('late'); return ''; }})) instanceof TypeError && !reads.length, 'invalid required member stops later getters');
  assert(thrown(() => new C('bad', {request, clientId: Symbol()})) instanceof TypeError, 'DOMString rejects symbols');
  const rejected = new C('rejected', {request, preloadResponse: {get then() {throw sentinel;}}});
  let rejection;
  try { await rejected.preloadResponse; } catch (error) { rejection = error; }
  assert(rejection === sentinel, 'then getter exception becomes promise rejection');
  class Subclass extends C {}
  const subclass = new Subclass('subclass', {request});
  assert(Object.getPrototypeOf(subclass) === Subclass.prototype && subclass instanceof C && subclass.request === request, 'newTarget prototype');
  let received = null;
  const dispatched = new C('constructor-dispatch', {request, cancelable: true});
  self.addEventListener('constructor-dispatch', event => { received = event; event.preventDefault(); }, {once: true});
  assert(self.dispatchEvent(dispatched) === false && received === dispatched && dispatched.defaultPrevented && !dispatched.isTrusted, 'synthetic EventTarget dispatch');
  const requestDescriptor = Object.getOwnPropertyDescriptor(self, 'Request');
  const promiseDescriptor = Object.getOwnPropertyDescriptor(self, 'Promise');
  try {
    Object.defineProperty(self, 'Request', {configurable: true, get() {throw Error('public Request getter');}});
    Object.defineProperty(self, 'Promise', {configurable: true, get() {throw Error('public Promise getter');}});
    const poisoned = new C('intrinsics', {request, handled: 42, preloadResponse: 'preload'});
    assert(poisoned.request === request && poisoned.handled instanceof P && poisoned.preloadResponse instanceof P, 'intrinsic promise and Request brand');
    assert(await poisoned.handled === 42 && await poisoned.preloadResponse === 'preload', 'non-promise values');
  } finally {
    Object.defineProperty(self, 'Request', requestDescriptor);
    Object.defineProperty(self, 'Promise', promiseDescriptor);
  }
  return {passed: true, legacyIsReload: event.isReload === undefined ? 'absent' : event.isReload};
}
