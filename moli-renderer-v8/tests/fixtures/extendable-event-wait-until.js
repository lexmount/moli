function checkExtendableEventWaitUntil(event) {
  const waitUntil = ExtendableEvent.prototype.waitUntil;
  const assert = (condition, message) => {
    if (!condition) throw Error(message);
  };
  const throws = (callback, expected, message) => {
    let caught;
    try { callback(); } catch (error) { caught = error; }
    assert(caught?.name === expected, message + ': ' + caught);
  };
  let reads = 0, traps = 0;
  const value = {get then() { reads++; return undefined; }};
  const proxyHandler = new Proxy({}, {
    get() { traps++; throw Error('author Proxy trap'); }
  });
  const revoked = Proxy.revocable(event, {});
  revoked.revoke();
  for (const receiver of [
    {}, Object.create(ExtendableEvent.prototype), new Event('plain'),
    new Proxy(event, proxyHandler), revoked.proxy
  ]) {
    throws(() => waitUntil.call(receiver, value), 'TypeError', 'invalid receiver');
  }
  assert(reads === 0 && traps === 0, 'brand checks precede conversion and do not invoke traps');
  for (const synthetic of [
    new ExtendableEvent('synthetic'),
    new ExtendableMessageEvent('synthetic'),
    new FetchEvent('synthetic', {request: new Request('https://example.test/')})
  ]) {
    throws(() => waitUntil.call(synthetic), 'TypeError', 'required argument');
    const before = reads;
    throws(() => waitUntil.call(synthetic, value), 'InvalidStateError', 'untrusted event');
    assert(reads === before + 1, 'Promise conversion precedes inactive-event check');
  }
  for (const method of [waitUntil, event.waitUntil]) {
    throws(() => method.call(event), 'TypeError', 'native event required argument');
    const before = reads;
    assert(method.call(event, value) === undefined, 'native event returns undefined');
    assert(reads === before + 1, 'native event converts thenables once');
    for (const scalar of [undefined, null, 1, 'value', Symbol('value')]) {
      assert(method.call(event, scalar) === undefined, 'Promise accepts scalar values');
    }
  }
}
