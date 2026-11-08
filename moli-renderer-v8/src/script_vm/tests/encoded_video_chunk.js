(() => {
  const checks = [];
  const check = (name, run) => {
    try {
      if (run() === false) throw Error('assertion returned false');
      checks.push({name, passed: true});
    } catch (error) {
      checks.push({name, passed: false, error: String(error), stack: error.stack});
    }
  };
  const equal = (actual, expected) => {
    if (!Object.is(actual, expected)) throw Error(`expected ${expected}, got ${actual}`);
  };
  const bytesEqual = (actual, expected) => equal(Array.from(actual).join(','), expected.join(','));
  const throws = (realm, name, run) => {
    let error;
    try { run(); } catch (caught) { error = caught; }
    if (!error || error.name !== name || !(error instanceof realm[name === 'TypeError' ? 'TypeError' : 'DOMException']))
      throw Error(`expected ${name} in the callee realm, got ${error}`);
    if (realm !== globalThis && name === 'TypeError' && error instanceof TypeError)
      throw Error('wrong TypeError realm');
    return error;
  };
  const init = overrides => ({type: 'key', timestamp: -17, data: new Uint8Array([1, 2, 3]), ...overrides});
  const copy = chunk => {const result = new Uint8Array(chunk.byteLength); chunk.copyTo(result); return result;};
  const realms = [globalThis];
  if (typeof document !== 'undefined') realms.push(document.getElementById('child').contentWindow);
  for (const [realmIndex, realm] of realms.entries()) {
    const C = realm.EncodedVideoChunk;
    const test = (name, run) => check(`${realmIndex}: ${name}`, run);
    test('interface descriptor and native prototype', () => {
      equal(typeof C, 'function'); equal(C.name, 'EncodedVideoChunk'); equal(C.length, 1);
      const d = Object.getOwnPropertyDescriptor(realm, 'EncodedVideoChunk');
      equal(d.enumerable, false); equal(d.writable, true); equal(d.configurable, true);
      equal(Object.getPrototypeOf(C), realm.Function.prototype);
      equal(Object.getPrototypeOf(C.prototype), realm.Object.prototype);
      equal(C.prototype.constructor, C);
      const tag = Object.getOwnPropertyDescriptor(C.prototype, Symbol.toStringTag);
      equal(tag.value, 'EncodedVideoChunk'); equal(tag.writable, false);
      equal(tag.enumerable, false); equal(tag.configurable, true);
    });
    for (const args of [[], [undefined], [null], [{}], [0], ['x']])
      test(`invalid required dictionary ${String(args)}`, () => throws(realm, 'TypeError', () => Reflect.construct(C, args)));
    test('requires new before conversion', () => {
      let converted = false;
      throws(realm, 'TypeError', () => C(new Proxy({}, {get() {converted = true;}})));
      equal(converted, false);
    });
    test('nullish dictionary ignores Object.prototype while objects inherit members', () => {
      const proto = realm.Object.prototype, saved = new Map(); let reads = 0;
      try {
        for (const [name, value] of [['data', new Uint8Array([1])], ['timestamp', -2], ['type', 'key']]) {
          saved.set(name, Object.getOwnPropertyDescriptor(proto, name));
          Object.defineProperty(proto, name, {configurable: true, get() {reads++; return value;}});
        }
        throws(realm, 'TypeError', () => new C(null));
        throws(realm, 'TypeError', () => new C(undefined)); equal(reads, 0);
        const inherited = new C(new realm.Object());
        equal(inherited.timestamp, -2); bytesEqual(copy(inherited), [1]); equal(reads, 3);
      } finally {
        for (const [name, descriptor] of saved) {
          if (descriptor) Object.defineProperty(proto, name, descriptor); else Reflect.deleteProperty(proto, name);
        }
      }
    });
    for (const type of ['key', 'delta']) test(`immutable ${type} bytes`, () => {
      const data = new Uint8Array([9, 1, 2, 3, 8]);
      const chunk = new C(init({type, duration: 27, data: data.subarray(1, 4)}));
      data.fill(99);
      equal(chunk.type, type); equal(chunk.timestamp, -17); equal(chunk.duration, 27); equal(chunk.byteLength, 3);
      bytesEqual(copy(chunk), [1, 2, 3]); equal(Object.keys(chunk).length, 0);
      equal(Object.prototype.toString.call(chunk), '[object EncodedVideoChunk]');
    });
    for (const type of ['', 'KEY', ' key', null, undefined, 1, Symbol('type')])
      test(`invalid enum ${String(type)}`, () => throws(realm, 'TypeError', () => new C(init({type}))));
    for (const [value, expected] of [[-1.9, -1], [1.9, 1], [-0.9, 0], [-0, 0], [null, 0], [true, 1], ['23', 23], [Number.MAX_SAFE_INTEGER, Number.MAX_SAFE_INTEGER], [-Number.MAX_SAFE_INTEGER, -Number.MAX_SAFE_INTEGER]])
      test(`timestamp conversion ${String(value)}`, () => equal(new C(init({timestamp: value})).timestamp, expected));
    for (const value of [undefined, NaN, Infinity, -Infinity, 2 ** 53, -(2 ** 53), 2 ** 63, Symbol('timestamp'), 1n])
      test(`invalid timestamp ${String(value)}`, () => throws(realm, 'TypeError', () => new C(init({timestamp: value}))));
    for (const [value, expected] of [[undefined, null], [null, 0], [-0.9, 0], [-0, 0], [0, 0], [1.9, 1], ['12', 12], [Number.MAX_SAFE_INTEGER, Number.MAX_SAFE_INTEGER]])
      test(`duration conversion ${String(value)}`, () => equal(new C(init({duration: value})).duration, expected));
    for (const value of [NaN, Infinity, -Infinity, -1, 2 ** 53, 2 ** 64, Symbol('duration'), 1n])
      test(`invalid duration ${String(value)}`, () => throws(realm, 'TypeError', () => new C(init({duration: value}))));
    test('missing duration remains null', () => equal(new C(init()).duration, null));
    const dataBuffer = new Uint8Array([9, 1, 2, 3, 8]).buffer;
    for (const [name, data, expected] of [
      ['ArrayBuffer', dataBuffer, [9, 1, 2, 3, 8]],
      ['Uint8Array offset', new Uint8Array(dataBuffer, 1, 3), [1, 2, 3]],
      ['DataView offset', new DataView(dataBuffer, 1, 3), [1, 2, 3]],
      ['Int16Array', new Int16Array(new Uint8Array([1, 2, 3, 4]).buffer), [1, 2, 3, 4]],
      ['empty buffer', new ArrayBuffer(0), []],
      ['empty view', new Uint8Array(dataBuffer, 5, 0), []],
    ]) test(`construct from ${name}`, () => bytesEqual(copy(new C(init({data}))), expected));
    const revokedBuffer = Proxy.revocable(dataBuffer, {}); revokedBuffer.revoke();
    for (const [index, data] of [undefined, null, 0, [], {}, 'data', Symbol('data'), 1n, new Proxy(dataBuffer, {}), revokedBuffer.proxy].entries())
      test(`invalid data ${index}`, () => throws(realm, 'TypeError', () => new C(init({data}))));
    test('detached source is an empty byte sequence', () => {
      const data = new Uint8Array([1, 2]); data.buffer.transfer();
      equal(new C(init({data})).byteLength, 0); equal(new C(init({data: data.buffer})).byteLength, 0);
    });
    test('copy after all dictionary getter effects', () => {
      const data = new Uint8Array([1, 2, 3]);
      const options = init({data}); Object.defineProperty(options, 'type', {get() {data[1] = 7; return 'delta';}});
      bytesEqual(copy(new C(options)), [1, 7, 3]);
    });
    test('dictionary getters may detach data before copy', () => {
      const data = new Uint8Array([1, 2, 3]);
      const options = init({data}); Object.defineProperty(options, 'duration', {get() {data.buffer.transfer(); return 1;}});
      equal(new C(options).byteLength, 0);
    });
    test('dictionary conversion order', () => {
      const order = [], options = init({duration: 5});
      const proxy = new Proxy(options, {get(target, key) {order.push(key); return target[key];}});
      new C(proxy); bytesEqual(order, ['data', 'duration', 'timestamp', 'transfer', 'type']);
    });
    for (const property of ['data', 'duration', 'timestamp', 'transfer', 'type']) test(`propagates ${property} getter exception`, () => {
      const marker = {}, options = init(); Object.defineProperty(options, property, {get() {throw marker;}});
      let caught; try {new C(options);} catch (error) {caught = error;} equal(caught, marker);
    });
    for (const property of ['duration', 'timestamp', 'type']) test(`propagates ${property} conversion exception`, () => {
      const marker = {}, options = init({[property]: {[Symbol.toPrimitive]() {throw marker;}}});
      let caught; try {new C(options);} catch (error) {caught = error;} equal(caught, marker);
    });
    for (const property of ['type', 'timestamp', 'data']) test(`missing required ${property}`, () => {
      const options = init(); delete options[property]; throws(realm, 'TypeError', () => new C(options));
    });
    test('subclass retains native slots', () => {
      class Sub extends C {}
      const chunk = new Sub(init()); equal(Object.getPrototypeOf(chunk), Sub.prototype); bytesEqual(copy(chunk), [1, 2, 3]);
    });
    const chunk = (() => {try {return new C(init());} catch {return null;}})();
    for (const name of ['type', 'timestamp', 'duration', 'byteLength']) test(`readonly ${name} descriptor`, () => {
      const d = Object.getOwnPropertyDescriptor(C.prototype, name);
      equal(typeof d.get, 'function'); equal(d.get.length, 0); equal(d.set, undefined);
      equal(d.enumerable, true); equal(d.configurable, true);
      equal(Reflect.set(chunk, name, 'replacement'), false);
    });
    test('copyTo descriptor', () => {
      const d = Object.getOwnPropertyDescriptor(C.prototype, 'copyTo');
      equal(d.value.name, 'copyTo'); equal(d.value.length, 1);
      equal(d.writable, true); equal(d.configurable, true); equal(d.enumerable, true);
    });
    for (const name of ['type', 'timestamp', 'duration', 'byteLength', 'copyTo']) {
      let trapCount = 0;
      const revoked = Proxy.revocable(chunk || {}, {}); revoked.revoke();
      const receivers = [null, undefined, 1, {}, C.prototype, Object.create(C.prototype), Object.create(chunk || {}),
        new Proxy(chunk || {}, {get() {trapCount++;}, getPrototypeOf() {trapCount++;}}), revoked.proxy];
      for (const [index, receiver] of receivers.entries()) test(`${name} receiver brand ${index}`, () => {
        const method = name === 'copyTo' ? C.prototype.copyTo : Object.getOwnPropertyDescriptor(C.prototype, name).get;
        throws(realm, 'TypeError', () => Reflect.apply(method, receiver, [new Uint8Array(3)])); equal(trapCount, 0);
      });
    }
    for (const kind of ['ArrayBuffer', 'Uint8Array', 'DataView', 'Int16Array', 'Float32Array']) test(`copyTo ${kind} respects offsets and tail`, () => {
      const backing = new ArrayBuffer(12), bytes = new Uint8Array(backing); bytes.fill(8);
      const destination = kind === 'ArrayBuffer' ? backing : kind === 'DataView' ? new DataView(backing, 4, 4) : new globalThis[kind](backing, 4, 4 / globalThis[kind].BYTES_PER_ELEMENT);
      equal(chunk.copyTo(destination), undefined);
      bytesEqual(bytes, kind === 'ArrayBuffer' ? [1, 2, 3, 8, 8, 8, 8, 8, 8, 8, 8, 8] : [8, 8, 8, 8, 1, 2, 3, 8, 8, 8, 8, 8]);
    });
    test('copyTo insufficient destination is unchanged', () => {
      const dest = new Uint8Array([9, 9]); throws(realm, 'TypeError', () => chunk.copyTo(dest)); bytesEqual(dest, [9, 9]);
    });
    test('copyTo detached destination', () => {
      const dest = new Uint8Array(3); dest.buffer.transfer(); throws(realm, 'TypeError', () => chunk.copyTo(dest));
    });
    test('copyTo required argument', () => throws(realm, 'TypeError', () => chunk.copyTo()));
    for (const [index, dest] of [null, {}, [], 1, 'x', new Proxy(new Uint8Array(3), {})].entries())
      test(`copyTo invalid destination ${index}`, () => throws(realm, 'TypeError', () => chunk.copyTo(dest)));
    test('zero chunk can copy to zero destination', () => {
      const empty = new C(init({data: new ArrayBuffer(0)})); equal(empty.copyTo(new ArrayBuffer(0)), undefined);
    });
    test('transfer includes unused buffers and preserves view data', () => {
      const data = new Uint8Array([8, 1, 2, 3, 9]), extra = new ArrayBuffer(2);
      const result = new C(init({data: new DataView(data.buffer, 1, 3), transfer: new Set([extra, data.buffer])}));
      equal(data.byteLength, 0); equal(extra.byteLength, 0); bytesEqual(copy(result), [1, 2, 3]);
    });
    test('transfer iterator finishes before byte snapshot', () => {
      const data = new Uint8Array([1, 2, 3]);
      const transfer = {*[Symbol.iterator]() {data[0] = 4; yield data.buffer; data[2] = 6;}};
      bytesEqual(copy(new C(init({data, transfer}))), [4, 2, 6]); equal(data.byteLength, 0);
    });
    test('duplicate transfer rejects before detaching anything', () => {
      const a = new ArrayBuffer(1), b = new ArrayBuffer(2);
      throws(realm, 'DataCloneError', () => new C(init({transfer: [a, b, a]}))); equal(a.byteLength, 1); equal(b.byteLength, 2);
    });
    test('detached transfer rejects before detaching other buffers', () => {
      const a = new ArrayBuffer(1), b = new ArrayBuffer(2); b.transfer();
      throws(realm, 'DataCloneError', () => new C(init({transfer: [a, b]}))); equal(a.byteLength, 1);
    });
    for (const bad of [null, {}, 1, 'x', [new Uint8Array(1)], [new Proxy(new ArrayBuffer(1), {})]])
      test(`invalid transfer ${String(bad)}`, () => throws(realm, 'TypeError', () => new C(init({transfer: bad}))));
    test('failed transfer conversion preserves iterator and attached buffers', () => {
      const buffer = new ArrayBuffer(1); let closed = 0;
      const transfer = {*[Symbol.iterator]() {try {yield buffer; yield {};} finally {closed++;}}};
      // WebIDL sequence conversion propagates without IteratorClose.
      throws(realm, 'TypeError', () => new C(init({transfer}))); equal(closed, 0); equal(buffer.byteLength, 1);
    });
    test('failed type conversion leaves valid transfers attached', () => {
      const buffer = new ArrayBuffer(1); throws(realm, 'TypeError', () => new C(init({type: 'bad', transfer: [buffer]}))); equal(buffer.byteLength, 1);
    });
    if (typeof ArrayBuffer.prototype.resize === 'function') for (const view of [false, true]) {
      test(`resizable data rejected ${view}`, () => {
        const data = new ArrayBuffer(3, {maxByteLength: 8});
        throws(realm, 'TypeError', () => new C(init({data: view ? new Uint8Array(data) : data})));
      });
      test(`resizable destination rejected ${view}`, () => {
        const dest = new ArrayBuffer(3, {maxByteLength: 8});
        throws(realm, 'TypeError', () => chunk.copyTo(view ? new DataView(dest) : dest));
      });
    }
    if (typeof ArrayBuffer.prototype.resize === 'function') test('resizable transfer rejected without detaching valid entry', () => {
      const fixed = new ArrayBuffer(1), resizable = new ArrayBuffer(1, {maxByteLength: 2});
      throws(realm, 'TypeError', () => new C(init({transfer: [fixed, resizable]}))); equal(fixed.byteLength, 1);
    });
    if (typeof SharedArrayBuffer === 'function') {
      for (const view of [false, true]) test(`shared BufferSource snapshot and destination ${view}`, () => {
        const data = new SharedArrayBuffer(5), source = new Uint8Array(data); source.set([8, 1, 2, 3, 9]);
        const result = new C(init({data: view ? new DataView(data, 1, 3) : data})); source.fill(7);
        const destination = new SharedArrayBuffer(9), bytes = new Uint8Array(destination); bytes.fill(8);
        result.copyTo(new Uint8Array(destination, 2, 5));
        bytesEqual(bytes, view ? [8, 8, 1, 2, 3, 8, 8, 8, 8] : [8, 8, 8, 1, 2, 3, 9, 8, 8]);
      });
      test('shared buffer is not an ArrayBuffer transfer', () => throws(realm, 'TypeError', () => new C(init({transfer: [new SharedArrayBuffer(3)]}))));
      if (typeof SharedArrayBuffer.prototype.grow === 'function') for (const view of [false, true]) test(`growable shared source rejected ${view}`, () => {
        const data = new SharedArrayBuffer(3, {maxByteLength: 8});
        throws(realm, 'TypeError', () => new C(init({data: view ? new Uint8Array(data) : data})));
      });
    }
    test('structured clone preserves slots and graph aliases in destination realm', () => {
      const source = new C(init({duration: Number.MAX_SAFE_INTEGER}));
      const graph = realm.structuredClone([source, source]);
      equal(graph[0], graph[1]); equal(graph[0] === source, false);
      equal(Object.getPrototypeOf(graph[0]), C.prototype); equal(graph[0].timestamp, -17);
      equal(graph[0].duration, Number.MAX_SAFE_INTEGER); bytesEqual(copy(graph[0]), [1, 2, 3]);
    });
    test('structured clone ignores overridden public getters', () => {
      const source = new C(init());
      for (const key of ['type', 'timestamp', 'duration', 'byteLength', 'copyTo', 'extra'])
        Object.defineProperty(source, key, {enumerable: true, get() {throw Error('public getter read');}});
      const cloned = realm.structuredClone(source);
      equal(cloned.type, 'key'); equal(cloned.timestamp, -17); equal(cloned.duration, null); bytesEqual(copy(cloned), [1, 2, 3]);
    });
    test('structured clone uses intrinsic constructor after global replacement', () => {
      const source = new C(init());
      try {
        realm.EncodedVideoChunk = function () {throw Error('author constructor called');};
        const cloned = realm.structuredClone(source);
        equal(Object.getPrototypeOf(cloned), C.prototype); bytesEqual(copy(cloned), [1, 2, 3]);
      } finally {realm.EncodedVideoChunk = C;}
    });
    test('chunk is serializable but not transferable', () => {
      const buffer = new ArrayBuffer(2);
      throws(realm, 'DataCloneError', () => realm.structuredClone(chunk, {transfer: [buffer, chunk]})); equal(buffer.byteLength, 2);
    });
    if (typeof document !== 'undefined') {
      test('history serialization rejects nested chunk without mutation', () => {
        const before = realm.history.length;
        throws(realm, 'DataCloneError', () => realm.history.pushState({chunk}, '')); equal(realm.history.length, before);
      });
      test('postMessage serialization preserves source slots', () => {
        const channel = new realm.MessageChannel();
        channel.port1.postMessage(chunk); equal(chunk.timestamp, -17); bytesEqual(copy(chunk), [1, 2, 3]);
        channel.port1.close(); channel.port2.close();
      });
    }
  }
  if (typeof document !== 'undefined') check('cross-realm genuine receiver shares native slots', () => {
    const main = new EncodedVideoChunk(init()), other = realms[1];
    equal(Object.getOwnPropertyDescriptor(other.EncodedVideoChunk.prototype, 'timestamp').get.call(main), -17);
    const dest = new Uint8Array(3); other.EncodedVideoChunk.prototype.copyTo.call(main, dest); bytesEqual(dest, [1, 2, 3]);
    const clone = other.structuredClone(main); equal(Object.getPrototypeOf(clone), other.EncodedVideoChunk.prototype); bytesEqual(copy(clone), [1, 2, 3]);
  });
  globalThis.__uiEventResults = {complete: true, passed: checks.filter(row => row.passed).length, total: checks.length, checks};
  return true;
})()
