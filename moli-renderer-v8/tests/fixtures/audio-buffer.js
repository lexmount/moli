(() => {
  const results = {};
  const check = (condition, message) => { if (!condition) throw new Error(message); };
  const equal = (actual, expected, message) => check(JSON.stringify(actual) === JSON.stringify(expected), message);
  const throws = (name, fn) => {
    try { fn(); } catch (error) { check(error.name === name, `expected ${name}, got ${error.name}`); return; }
    throw new Error(`expected ${name}`);
  };
  const run = (name, fn) => {
    try { fn(); results[name] = 'ok'; }
    catch (error) { results[name] = {name: error.name, message: error.message}; }
  };

  run('construction', () => {
    const buffer = new AudioBuffer({length: 8, sampleRate: 8000});
    check(buffer instanceof AudioBuffer, 'brand');
    check(Object.prototype.toString.call(buffer) === '[object AudioBuffer]', 'tag');
    equal([buffer.length, buffer.sampleRate, buffer.duration, buffer.numberOfChannels], [8, 8000, .001, 1], 'metadata');
    check(Object.getOwnPropertyNames(buffer).length === 0, 'no public backing fields');
    check(AudioBuffer.length === 1, 'constructor arity');
    class Derived extends AudioBuffer {}
    check(Object.getPrototypeOf(new Derived({length: 1, sampleRate: 44100})) === Derived.prototype, 'newTarget prototype');
    throws('TypeError', () => AudioBuffer({length: 8, sampleRate: 8000}));
    for (const options of [undefined, null, 1, {}, {length: 8}, {sampleRate: 8000}]) {
      throws('TypeError', () => new AudioBuffer(options));
    }
  });

  run('metadata', () => {
    const buffer = new AudioBuffer({length: 8, sampleRate: 8000, numberOfChannels: 2});
    for (const name of ['length', 'sampleRate', 'duration', 'numberOfChannels']) {
      const descriptor = Object.getOwnPropertyDescriptor(AudioBuffer.prototype, name);
      check(descriptor.enumerable && descriptor.configurable, `${name} flags`);
      check(typeof descriptor.get === 'function' && descriptor.set === undefined, `${name} readonly getter`);
      check(descriptor.get.name === `get ${name}` && descriptor.get.length === 0, `${name} function metadata`);
      check(Reflect.set(buffer, name, 100) === false, `${name} write`);
      for (const receiver of [{}, AudioBuffer.prototype, Object.create(buffer), new Proxy(buffer, {})]) {
        throws('TypeError', () => descriptor.get.call(receiver));
      }
    }
    for (const [name, length] of [['getChannelData', 1], ['copyFromChannel', 2], ['copyToChannel', 2]]) {
      const descriptor = Object.getOwnPropertyDescriptor(AudioBuffer.prototype, name);
      check(descriptor.enumerable && descriptor.configurable && descriptor.writable, `${name} flags`);
      check(descriptor.value.name === name && descriptor.value.length === length, `${name} function metadata`);
      check(!Object.hasOwn(buffer, name), `${name} not per-instance`);
    }
  });

  run('channels', () => {
    const buffer = new AudioBuffer({length: 8, sampleRate: 8000, numberOfChannels: 3});
    const channels = [0, 1, 2].map(i => buffer.getChannelData(i));
    for (let i = 0; i < channels.length; i++) {
      check(channels[i] instanceof Float32Array && channels[i].length === 8, 'float32 channel');
      check(channels[i] === buffer.getChannelData(i), 'stable view identity');
      check(channels[i].every(x => x === 0), 'zero initialized');
    }
    channels[0][0] = 1;
    channels[1][1] = 2;
    equal(channels.map(x => [x[0], x[1]]), [[1, 0], [0, 2], [0, 0]], 'independent channels');
    buffer.__moliOfflineAudioBuffer = new Float32Array([99]);
    buffer.__moliAudioBufferChannels = [new Float32Array([99])];
    Object.defineProperty(buffer, 'length', {value: 99});
    check(buffer.getChannelData(0) === channels[0] && channels[0].length === 8, 'private storage');
  });

  run('copy', () => {
    const buffer = new AudioBuffer({length: 6, sampleRate: 8000, numberOfChannels: 2});
    const source = new Float32Array([90, 1, 2, 3, 4, 91]);
    check(buffer.copyToChannel(source.subarray(1, 5), 1, 2) === undefined, 'copyTo return');
    equal([...buffer.getChannelData(1)], [0, 0, 1, 2, 3, 4], 'source view and offset');
    check(buffer.getChannelData(0).every(x => x === 0), 'other channel unchanged');
    const target = new Float32Array(8).fill(99);
    check(buffer.copyFromChannel(target.subarray(1, 7), 1, 2) === undefined, 'copyFrom return');
    equal([...target], [99, 1, 2, 3, 4, 99, 99, 99], 'destination view and untouched suffix');
    buffer.copyToChannel(new Float32Array([5, 6]), 1, 5);
    equal([...buffer.getChannelData(1)], [0, 0, 1, 2, 3, 5], 'clamped write');
    buffer.copyFromChannel(target, 1, 6);
    buffer.copyToChannel(source, 1, 6);
    buffer.copyToChannel(source, 1, -1);
    equal([...target], [99, 1, 2, 3, 4, 99, 99, 99], 'offset at/past end is no-op');
    const channel = buffer.getChannelData(0);
    channel.set([1, 2, 3, 4, 5, 6]);
    buffer.copyToChannel(channel.subarray(0, 4), 0, 1);
    equal([...channel], [1, 1, 2, 3, 4, 6], 'overlapping copyTo');
    channel.set([1, 2, 3, 4, 5, 6]);
    buffer.copyFromChannel(channel.subarray(1), 0);
    equal([...channel], [1, 1, 2, 3, 4, 5], 'overlapping copyFrom');
  });

  run('numeric_conversion', () => {
    const buffer = new AudioBuffer({length: '4.9', numberOfChannels: 2 ** 32 + 2, sampleRate: 8000.123456});
    equal([buffer.length, buffer.numberOfChannels, buffer.sampleRate], [4, 2, Math.fround(8000.123456)], 'WebIDL conversion');
    const zero = buffer.getChannelData(0);
    for (const channel of [null, undefined, NaN, Infinity, 2 ** 32, '0.9']) {
      check(buffer.getChannelData(channel) === zero, 'unsigned long channel');
    }
    for (const channel of [-1, 2]) throws('IndexSizeError', () => buffer.getChannelData(channel));
    for (const channel of [Symbol(), 1n]) throws('TypeError', () => buffer.getChannelData(channel));
    throws('TypeError', () => buffer.getChannelData());
    for (const length of [0, 2 ** 32]) throws('NotSupportedError', () => new AudioBuffer({length, sampleRate: 8000}));
    for (const numberOfChannels of [0, 33, -1]) throws('NotSupportedError', () => new AudioBuffer({length: 1, sampleRate: 8000, numberOfChannels}));
    for (const sampleRate of [0, 2999, 768001]) throws('NotSupportedError', () => new AudioBuffer({length: 1, sampleRate}));
    for (const sampleRate of [NaN, Infinity, -Infinity, 1e40, Symbol(), 1n]) throws('TypeError', () => new AudioBuffer({length: 1, sampleRate}));
    check(new AudioBuffer({length: 1, sampleRate: 2999.99999}).sampleRate === 3000, 'float conversion precedes range check');
    check(new AudioBuffer({length: 1, sampleRate: 768000}).sampleRate === 768000, 'sample rate upper bound');
  });

  run('bitwise_storage', () => {
    const buffer = new AudioBuffer({length: 4, sampleRate: 8000});
    const source = new Float32Array(4);
    const bits = [0x80000000, 0x7fc00001, 0xff800000, 0x00000001];
    new Uint32Array(source.buffer).set(bits);
    buffer.copyToChannel(source, 0);
    const channel = buffer.getChannelData(0);
    equal([...new Uint32Array(channel.buffer)], bits, 'preserve sample bits');
    const copy = new Float32Array(4);
    Object.defineProperty(copy, 'length', {value: 0});
    Object.defineProperty(copy, 'byteOffset', {value: 100000});
    buffer.copyFromChannel(copy, 0);
    equal([...new Uint32Array(copy.buffer)], bits, 'intrinsic destination bounds');
    const previous = Object.getOwnPropertyDescriptor(Array.prototype, '0');
    try {
      Object.defineProperty(Array.prototype, '0', {
        configurable: true,
        get() { throw new Error('internal channel read invoked page getter'); },
        set() { throw new Error('internal channel write invoked page setter'); }
      });
      const clean = new AudioBuffer({length: 4, sampleRate: 8000});
      check(clean.getChannelData(0)[0] === 0, 'own channel storage');
    } finally {
      if (previous) Object.defineProperty(Array.prototype, '0', previous);
      else delete Array.prototype[0];
    }
  });

  run('receivers_and_argument_order', () => {
    const buffer = new AudioBuffer({length: 4, sampleRate: 8000});
    const array = new Float32Array(4);
    let conversions = 0;
    const channel = {valueOf() { conversions++; return 0; }};
    for (const name of ['getChannelData', 'copyFromChannel', 'copyToChannel']) {
      const args = name === 'getChannelData' ? [channel] : [array, channel];
      for (const receiver of [{}, AudioBuffer.prototype, Object.create(buffer), new Proxy(buffer, {})]) {
        throws('TypeError', () => buffer[name].apply(receiver, args));
      }
    }
    check(conversions === 0, 'brand check before conversion');
    for (const name of ['copyFromChannel', 'copyToChannel']) {
      for (const bad of [undefined, null, [], new Float64Array(4), new Proxy(array, {})]) {
        throws('TypeError', () => buffer[name](bad, channel));
      }
      check(conversions === 0, 'typed array check before channel conversion');
      throws('TypeError', () => buffer[name](array));
      throws('IndexSizeError', () => buffer[name](array, 1));
      check(buffer[name](new Float32Array(0), 99) === undefined, 'empty view returns before range check');
      const rab = new ArrayBuffer(16, {maxByteLength: 32});
      throws('TypeError', () => buffer[name](new Float32Array(rab), 0));
      if (typeof SharedArrayBuffer === 'function') {
        throws('TypeError', () => buffer[name](new Float32Array(new SharedArrayBuffer(16)), 0));
      }
    }
    const order = [];
    new AudioBuffer({
      get sampleRate() { order.push('sampleRate'); return 8000; },
      get numberOfChannels() { order.push('numberOfChannels'); return 1; },
      get length() { order.push('length'); return 4; }
    });
    equal(order, ['length', 'numberOfChannels', 'sampleRate'], 'dictionary member order');
    const sentinel = new Error('sentinel');
    try { new AudioBuffer({get length() { throw sentinel; }}); throw new Error('missing exception'); }
    catch (error) { check(error === sentinel, 'preserve getter exception'); }
    try { buffer.copyFromChannel(array, {valueOf() { throw sentinel; }}); throw new Error('missing exception'); }
    catch (error) { check(error === sentinel, 'preserve argument exception'); }
  });

  run('detached_views', () => {
    const buffer = new AudioBuffer({length: 4, sampleRate: 8000});
    const channel = buffer.getChannelData(0);
    channel.set([1, 2, 3, 4]);
    structuredClone(channel.buffer, {transfer: [channel.buffer]});
    check(buffer.getChannelData(0) === channel && channel.length === 0, 'retain detached channel view');
    equal([buffer.length, buffer.duration, buffer.numberOfChannels], [4, .0005, 1], 'detaching does not change metadata');
    const target = new Float32Array([9, 9, 9, 9]);
    buffer.copyFromChannel(target, 0);
    equal([...target], [9, 9, 9, 9], 'detached source is empty');
    buffer.copyToChannel(target, 0);
    const second = new AudioBuffer({length: 4, sampleRate: 8000});
    second.copyFromChannel(target, {valueOf() { structuredClone(target.buffer, {transfer: [target.buffer]}); return 0; }});
    check(target.length === 0, 'argument conversion may detach destination');
  });

  run('context_factory', () => {
    const contexts = [new AudioContext(), new OfflineAudioContext(1, 4, 8000)];
    const method = BaseAudioContext.prototype.createBuffer;
    check(method.length === 3, 'factory arity');
    for (const context of contexts) {
      check(context.createBuffer === method, 'shared factory');
      const buffer = context.createBuffer(2, 16, 16000);
      equal([buffer.numberOfChannels, buffer.length, buffer.sampleRate, buffer.duration], [2, 16, 16000, .001], 'factory metadata');
      check(buffer instanceof AudioBuffer && buffer.getChannelData(1).every(x => x === 0), 'factory channel storage');
      throws('TypeError', () => method.call({}, 1, 4, 8000));
      throws('TypeError', () => method.call(context, 1, 4));
      throws('NotSupportedError', () => method.call(context, 0, 4, 8000));
    }
    contexts[0].close();
  });

  return results;
})()
