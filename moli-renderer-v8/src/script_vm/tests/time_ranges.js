(() => {
  const checks = [];
  const equal = (actual, expected) => {
    if (!Object.is(actual, expected)) throw Error(`expected ${String(expected)}, got ${String(actual)}`);
  };
  const throws = (realm, name, run) => {
    let error;
    try { run(); } catch (caught) { error = caught; }
    const C = name === 'TypeError' ? realm.TypeError : realm.DOMException;
    if (!error || error.name !== name || !(error instanceof C)) throw Error(`expected ${name} in callee realm, got ${error}`);
    return error;
  };
  const realms = [globalThis, document.getElementById('child').contentWindow];
  for (const [index, realm] of realms.entries()) {
    const C = realm.TimeRanges, prototype = C.prototype;
    const getLength = Object.getOwnPropertyDescriptor(prototype, 'length')?.get;
    const test = (name, run) => {
      try { run(); checks.push({realm: index, name, passed: true}); }
      catch (error) { checks.push({realm: index, name, passed: false, error: String(error), stack: error.stack}); }
    };
    test('interface metadata and illegal constructor', () => {
      equal(C.name, 'TimeRanges'); equal(C.length, 0); equal(prototype.constructor, C);
      equal(Object.getPrototypeOf(C), realm.Function.prototype);
      equal(Object.getPrototypeOf(prototype), realm.Object.prototype);
      const tag = Object.getOwnPropertyDescriptor(prototype, Symbol.toStringTag);
      equal(tag.value, 'TimeRanges'); equal(tag.writable, false); equal(tag.enumerable, false); equal(tag.configurable, true);
      for (const run of [() => C(), () => new C(), () => Reflect.construct(C, [], function Other() {})]) throws(realm, 'TypeError', run);
    });
    test('length descriptor', () => {
      const d = Object.getOwnPropertyDescriptor(prototype, 'length');
      equal(d.enumerable, true); equal(d.configurable, true); equal(d.set, undefined);
      equal(d.get.name, 'get length'); equal(d.get.length, 0); equal(Object.getPrototypeOf(d.get), realm.Function.prototype);
    });
    for (const name of ['start', 'end']) test(`${name} descriptor`, () => {
      const d = Object.getOwnPropertyDescriptor(prototype, name);
      equal(d.enumerable, true); equal(d.configurable, true); equal(d.writable, true);
      equal(d.value.name, name); equal(d.value.length, 1); equal(Object.getPrototypeOf(d.value), realm.Function.prototype);
    });
    const factories = [
      ['video', () => realm.document.createElement('video')],
      ['audio', () => realm.document.createElement('audio')],
      ['Audio', () => new realm.Audio()],
      ['windowless video', () => realm.document.implementation.createHTMLDocument('').createElement('video')],
    ];
    for (const property of ['buffered', 'played', 'seekable']) {
      const get = Object.getOwnPropertyDescriptor(realm.HTMLMediaElement.prototype, property)?.get;
      test(`${property} descriptor`, () => {
        const d = Object.getOwnPropertyDescriptor(realm.HTMLMediaElement.prototype, property);
        equal(d.enumerable, true); equal(d.configurable, true); equal(d.set, undefined);
        equal(get.name, `get ${property}`); equal(get.length, 0); equal(Object.getPrototypeOf(get), realm.Function.prototype);
      });
      for (const [label, factory] of factories) test(`${property} fresh empty snapshots from ${label}`, () => {
        const media = factory(), a = get.call(media), b = get.call(media);
        equal(a === b, false); equal(a.length, 0); equal(b.length, 0);
        equal(Object.getPrototypeOf(a), prototype); equal(Object.prototype.toString.call(a), '[object TimeRanges]');
        equal(a.constructor, C); equal(Object.keys(a).length, 0); equal(Array.isArray(a), false);
        equal(0 in a, false); equal(a[Symbol.iterator], undefined); equal(Reflect.set(a, 'length', 9), false);
        a.extra = 'author'; equal(b.extra, undefined); equal(getLength.call(a), 0);
      });
      test(`${property} accepts foreign native media in callee realm`, () => {
        const foreign = realms[1 - index].document.createElement('video'), ranges = get.call(foreign);
        equal(Object.getPrototypeOf(ranges), prototype); equal(getLength.call(ranges), 0);
      });
      test(`${property} rejects forged and proxy media without author traps`, () => {
        let traps = 0;
        const media = realm.document.createElement('video');
        const revoked = Proxy.revocable(media, {}); revoked.revoke();
        const receivers = [{}, Object.create(realm.HTMLMediaElement.prototype), Object.create(media),
          realm.document.createElement('div'), new Proxy(media, {get() {traps++;}, getPrototypeOf() {traps++;}}), revoked.proxy];
        for (const receiver of receivers) throws(realm, 'TypeError', () => get.call(receiver));
        equal(traps, 0);
      });
    }
    for (const name of ['start', 'end']) {
      const method = prototype[name];
      const values = [['undefined', undefined], ['null', null], ['false', false], ['true', true], ['zero', 0],
        ['negative zero', -0], ['one', 1], ['fraction', 0.9], ['negative fraction', -0.9], ['negative', -1],
        ['nan', NaN], ['positive infinity', Infinity], ['negative infinity', -Infinity],
        ['uint32 wrap', 2 ** 32], ['wrapped negative', -(2 ** 32)], ['maximum', 2 ** 32 - 1], ['numeric string', '0']];
      for (const [label, value] of values) test(`${name} empty index ${label}`, () => {
        const ranges = realm.document.createElement('video').buffered;
        const error = throws(realm, 'IndexSizeError', () => method.call(ranges, value)); equal(error.code, 1);
      });
      test(`${name} requires an argument before bounds`, () => throws(realm, 'TypeError', () => method.call(realm.document.createElement('video').buffered)));
      for (const [label, value] of [['symbol', Symbol()], ['bigint', 0n]]) test(`${name} rejects ${label} indices`, () =>
        throws(realm, 'TypeError', () => method.call(realm.document.createElement('video').buffered, value)));
      test(`${name} converts exactly once and propagates author exceptions`, () => {
        let conversions = 0; const ranges = realm.document.createElement('video').buffered;
        throws(realm, 'IndexSizeError', () => method.call(ranges, {[Symbol.toPrimitive](hint) {equal(hint, 'number'); conversions++; return 0;}}));
        equal(conversions, 1);
        const marker = {}; let caught;
        try { method.call(ranges, {valueOf() {throw marker;}}); } catch (error) {caught = error;}
        equal(caught, marker);
      });
      test(`${name} receiver validation precedes conversion and proxy traps`, () => {
        let calls = 0; const ranges = realm.document.createElement('video').buffered;
        const input = {valueOf() {calls++; return 0;}};
        const revoked = Proxy.revocable({}, {}); revoked.revoke();
        for (const receiver of [{}, Object.create(prototype), Object.create(ranges), new Proxy(ranges, {get() {calls++;}}), revoked.proxy])
          throws(realm, 'TypeError', () => method.call(receiver, input));
        equal(calls, 0);
      });
      test(`${name} accepts foreign snapshots and throws callee DOMException`, () => {
        const ranges = realms[1 - index].document.createElement('audio').seekable;
        throws(realm, 'IndexSizeError', () => method.call(ranges, 0));
      });
    }
    test('length receiver checks reject author proxies and forged prototypes', () => {
      const ranges = realm.document.createElement('video').buffered; let traps = 0;
      const revoked = Proxy.revocable(ranges, {}); revoked.revoke();
      for (const receiver of [{}, Object.create(prototype), Object.create(ranges), new Proxy(ranges, {get() {traps++;}}), revoked.proxy])
        throws(realm, 'TypeError', () => getLength.call(receiver));
      equal(traps, 0);
    });
    test('public properties do not change native length or bounds', () => {
      const ranges = realm.document.createElement('video').played;
      Object.defineProperty(ranges, 'length', {get() {throw Error('author length');}});
      ranges.__moliTimeRanges = [1, 2]; ranges[0] = 10;
      equal(getLength.call(ranges), 0); throws(realm, 'IndexSizeError', () => prototype.start.call(ranges, 0));
      Object.setPrototypeOf(ranges, null); equal(getLength.call(ranges), 0);
      Object.freeze(ranges); throws(realm, 'IndexSizeError', () => prototype.end.call(ranges, 0));
    });
    test('cloning rejects nested TimeRanges without public getter evaluation', () => {
      const ranges = realm.document.createElement('video').buffered; let gets = 0;
      Object.defineProperty(ranges, 'extra', {enumerable: true, get() {gets++; throw Error('author getter');}});
      throws(realm, 'DataCloneError', () => realm.structuredClone({ranges})); equal(gets, 0);
    });
    test('fresh media snapshots survive global constructor overrides', () => {
      const descriptor = Object.getOwnPropertyDescriptor(realm, 'TimeRanges');
      try {
        Object.defineProperty(realm, 'TimeRanges', {value: function() {throw Error('author constructor');}, configurable: true});
        const ranges = realm.document.createElement('video').buffered;
        equal(Object.getPrototypeOf(ranges), prototype); equal(getLength.call(ranges), 0);
      } finally {Object.defineProperty(realm, 'TimeRanges', descriptor);}
    });
  }
  globalThis.__uiEventResults = {complete: true, checks};
  return true;
})()
