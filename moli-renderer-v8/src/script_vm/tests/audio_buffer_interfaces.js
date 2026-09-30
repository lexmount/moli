(async () => {
  const rows = [];
  const assert = (ok, message) => { if (!ok) throw Error(message); };
  const check = async (name, callback) => {
    try { await callback(); rows.push({name, pass: true}); }
    catch (error) { rows.push({name, pass: false, message: String(error)}); }
  };
  const thrown = callback => {
    try { callback(); } catch (error) { return error; }
    throw Error('expected exception');
  };
  const equal = (actual, expected) => actual.length === expected.length && expected.every((value, index) => Object.is(value, actual[index]));
  const popup = open();
  const realms = [['main', window], ['child', document.getElementById('child').contentWindow], ['popup', popup]];
  try {
    for (const [label, w] of realms) {
      const context = new w.AudioContext();
      const make = (channels = 2, length = 8) => new w.AudioBuffer({numberOfChannels: channels, length, sampleRate: 44100});
      try {
        await check(label + '/AudioBuffer/interface and constructor', () => {
          const C = w.AudioBuffer, descriptor = Object.getOwnPropertyDescriptor(w, 'AudioBuffer');
          assert(C.name === 'AudioBuffer' && C.length === 1 && descriptor.writable && descriptor.configurable && !descriptor.enumerable, 'constructor metadata');
          assert(Object.getPrototypeOf(C) === w.Function.prototype && Object.getPrototypeOf(C.prototype) === w.Object.prototype, 'ordinary interface inheritance');
          assert(thrown(() => C({length: 4, sampleRate: 8000})) instanceof w.TypeError, 'new required');
          for (const value of [undefined, null, {}, {length: 2}, {sampleRate: 8000}, 1, 'options', Symbol()]) assert(thrown(() => new C(value)) instanceof w.TypeError, 'required dictionary members');
          const b = new C({length: 4, sampleRate: 8000});
          assert(b instanceof C && b.numberOfChannels === 1 && b.length === 4 && b.sampleRate === 8000 && b.duration === 4 / 8000, 'default channel count and native metadata');
          assert(Object.keys(b).length === 0 && !Object.hasOwn(b, 'getChannelData'), 'prototype members and private state');
          class Sub extends C {};
          assert(new Sub({length: 2, sampleRate: 8000}) instanceof Sub, 'new.target preserved');
        });
        await check(label + '/AudioBuffer/dictionary order and float conversion', () => {
          const order = [], options = Object.create(null);
          for (const [key, value] of [['length', 3.9], ['numberOfChannels', 2.9], ['sampleRate', 44100.1]]) Object.defineProperty(options, key, {get() { order.push(key); return value; }});
          const b = new w.AudioBuffer(options);
          assert(order.join() === 'length,numberOfChannels,sampleRate', 'lexical dictionary order');
          assert(b.length === 3 && b.numberOfChannels === 2 && b.sampleRate === Math.fround(44100.1), 'unsigned long and restricted float conversion');
          const sentinel = {};
          assert(thrown(() => new w.AudioBuffer({length: 0, numberOfChannels: 0, get sampleRate() { throw sentinel; }})) === sentinel, 'all conversion precedes range checks');
          for (const sampleRate of [NaN, Infinity, -Infinity, 1e100, Symbol(), 1n]) assert(thrown(() => new w.AudioBuffer({length: 2, sampleRate})) instanceof w.TypeError, 'restricted float');
          for (const options of [{length: 0, sampleRate: 8000}, {length: 2, sampleRate: 100}, {length: 2, sampleRate: 8000, numberOfChannels: 0}, {length: 2, sampleRate: 8000, numberOfChannels: 33}]) assert(thrown(() => new w.AudioBuffer(options)).name === 'NotSupportedError', 'native support constraints');
          assert(new w.AudioBuffer({length: 1, sampleRate: 96000, numberOfChannels: 32}).numberOfChannels === 32, 'required supported channel and sample-rate limits');
        });
        await check(label + '/BaseAudioContext/createBuffer and argument order', () => {
          const method = w.BaseAudioContext.prototype.createBuffer;
          assert(method.name === 'createBuffer' && method.length === 3, 'factory metadata');
          let conversions = 0;
          const poison = {valueOf() { conversions++; throw Error('conversion'); }};
          for (const values of [[], [poison], [poison, poison]]) assert(thrown(() => method.call(context, ...values)) instanceof w.TypeError, 'arity before conversion');
          assert(conversions === 0, 'missing arguments do not run author code');
          const order = [], number = value => ({valueOf() { order.push(value); return value; }});
          const b = method.call(context, number(3), number(4), number(16000));
          assert(order.join() === '3,4,16000' && b instanceof w.AudioBuffer && b.numberOfChannels === 3 && b.length === 4 && b.sampleRate === 16000, 'factory conversions and native buffer');
          const sentinel = {};
          assert(thrown(() => method.call(context, 0, 0, {valueOf() { throw sentinel; }})) === sentinel, 'conversion exception before state validation');
        });
        await check(label + '/AudioBuffer/channel storage and unsigned indices', () => {
          const a = make(), b = make(), first = a.getChannelData(0), second = a.getChannelData(1);
          assert(first instanceof w.Float32Array && first.length === 8 && first === a.getChannelData(0), 'stable channel view');
          assert(first !== second && first !== b.getChannelData(0) && first.every(value => Object.is(value, 0)), 'independent zero-filled channel data');
          first[0] = 7; assert(a.getChannelData(0)[0] === 7 && second[0] === 0 && b.getChannelData(0)[0] === 0, 'native channel isolation');
          assert(a.getChannelData(0.9) === first && a.getChannelData(Infinity) === first && a.getChannelData(2 ** 32) === first, 'unsigned long conversion');
          for (const index of [-1, 2]) assert(thrown(() => a.getChannelData(index)).name === 'IndexSizeError', 'channel range');
          for (const index of [Symbol(), 1n]) assert(thrown(() => a.getChannelData(index)) instanceof w.TypeError, 'invalid numeric conversion');
        });
        await check(label + '/AudioBuffer/channel copy truncation and untouched tails', () => {
          const b = make(2, 5), source = new w.Float32Array([1, -0, Infinity, NaN, 5, 6]);
          b.copyToChannel(source, 1);
          assert(equal(b.getChannelData(1), [1, -0, Infinity, NaN, 5]), 'raw float bit values and source truncation');
          const destination = new w.Float32Array(8).fill(9);
          assert(b.copyFromChannel(destination, 1, 2) === undefined && equal(destination, [Infinity, NaN, 5, 9, 9, 9, 9, 9]), 'partial copy leaves extra destination entries');
          b.copyToChannel(new w.Float32Array([7, 8, 9]), 1, 4);
          assert(equal(b.getChannelData(1), [1, -0, Infinity, NaN, 7]), 'offset write leaves other entries');
          b.copyFromChannel(destination, 1, -1); b.copyToChannel(source, 1, 2 ** 32 - 1);
          assert(equal(destination, [Infinity, NaN, 5, 9, 9, 9, 9, 9]), 'out-of-range offset is a no-op');
        });
        await check(label + '/AudioBuffer/overlapping copies and intrinsic view bounds', () => {
          const b = make(1, 5), data = b.getChannelData(0);
          data.set([1, 2, 3, 4, 5]); b.copyToChannel(data.subarray(0, 4), 0, 1);
          assert(equal(data, [1, 1, 2, 3, 4]), 'overlapping write snapshots source');
          data.set([1, 2, 3, 4, 5]); b.copyFromChannel(data.subarray(1), 0);
          assert(equal(data, [1, 1, 2, 3, 4]), 'overlapping read snapshots source');
          const source = new w.Float32Array([8, 9]); let reads = 0;
          for (const key of ['length', 'byteLength', 'byteOffset', 'buffer']) Object.defineProperty(source, key, {get() { reads++; throw Error('public typed-array metadata'); }});
          b.copyToChannel(source, 0, 1); assert(reads === 0 && data[1] === 8 && data[2] === 9, 'copy uses intrinsic storage, not public getters');
        });
        await check(label + '/AudioBuffer/copy conversion before native mutation', () => {
          const b = make(1, 4), values = new w.Float32Array([8, 9]), sentinel = {};
          assert(thrown(() => b.copyToChannel(values, 99, {valueOf() { throw sentinel; }})) === sentinel, 'offset conversion precedes invalid channel');
          assert(b.getChannelData(0).every(value => value === 0), 'failed conversion does not mutate data');
          let conversions = 0;
          assert(thrown(() => b.copyToChannel(new Proxy(values, {}), {valueOf() { conversions++; return 0; }})) instanceof w.TypeError && conversions === 0, 'typed-array brand before channel conversion');
          const shared = new w.Float32Array(new w.WebAssembly.Memory({shared: true, initial: 1, maximum: 1}).buffer);
          assert(thrown(() => b.copyFromChannel(shared, 0)) instanceof w.TypeError, 'shared storage rejected');
          const resizable = new w.Float32Array(new w.ArrayBuffer(8, {maxByteLength: 16}));
          assert(thrown(() => b.copyToChannel(resizable, 0)) instanceof w.TypeError, 'resizable storage rejected');
          const detached = new w.Float32Array(2); w.structuredClone(detached.buffer, {transfer: [detached.buffer]});
          assert(b.copyToChannel(detached, 0) === undefined, 'detached zero-length source is a no-op');
        });
        await check(label + '/AudioBuffer/acquired channel views', () => {
          const b = make(), first = b.getChannelData(0), second = b.getChannelData(1), source = context.createBufferSource();
          first[0] = 7; second[0] = 8; source.buffer = b;
          assert(first.length === 8 && second.length === 8, 'assignment before start retains views');
          source.start(100);
          assert(first.buffer.byteLength === 0 && second.buffer.byteLength === 0, 'acquiring content detaches every returned channel view');
          const next = b.getChannelData(0);
          assert(next !== first && next.length === 8 && next[0] === 7 && next === b.getChannelData(0), 'fresh stable view after acquisition preserves data');
          next[0] = 9; assert(b.getChannelData(0)[0] === 9, 'post-acquisition buffer remains writable');
        });
        await check(label + '/AudioBuffer/cross-realm factory and native metadata', () => {
          const foreign = new window.AudioContext();
          try {
            const b = w.BaseAudioContext.prototype.createBuffer.call(foreign, 2, 4, 8000);
            assert(b instanceof window.AudioBuffer && b.getChannelData(0) instanceof window.Float32Array, 'factory result belongs to context realm');
            const getter = Object.getOwnPropertyDescriptor(w.AudioBuffer.prototype, 'sampleRate').get;
            Object.defineProperty(b, 'sampleRate', {get() { throw Error('author sampleRate'); }});
            Object.setPrototypeOf(b, null);
            assert(getter.call(b) === 8000 && w.AudioBuffer.prototype.getChannelData.call(b, 0).length === 4, 'native identity and metadata survive prototype and public property changes');
            assert(thrown(() => w.BaseAudioContext.prototype.createBuffer.call(foreign, 2, 4, Symbol())) instanceof w.TypeError, 'callee realm conversion exception');
          } finally { foreign.close(); }
        });
        for (const [key, kinds] of [['length', ['get']], ['sampleRate', ['get']], ['duration', ['get']], ['numberOfChannels', ['get']], ['getChannelData', ['value']], ['copyFromChannel', ['value']], ['copyToChannel', ['value']]]) {
          await check(label + '/AudioBuffer/' + key + '/native receivers', () => {
            const descriptor = Object.getOwnPropertyDescriptor(w.AudioBuffer.prototype, key), real = make();
            assert(descriptor.enumerable && descriptor.configurable, 'member descriptor');
            let conversions = 0, traps = 0;
            const poison = {valueOf() { conversions++; throw Error('conversion'); }}, trap = () => { traps++; throw Error('proxy trap'); };
            const revoked = Proxy.revocable(real, {}); revoked.revoke();
            for (const kind of kinds) {
              const method = descriptor[kind];
              assert(method.name === (kind === 'get' ? 'get ' : '') + key && method.length === (kind === 'get' ? 0 : key === 'getChannelData' ? 1 : 2), 'function metadata');
              for (const receiver of [null, {}, w.AudioBuffer.prototype, Object.create(real), new Proxy(real, {get: trap, getPrototypeOf: trap}), revoked.proxy]) assert(thrown(() => method.call(receiver, poison, poison, poison)) instanceof w.TypeError, 'callee realm native receiver required');
            }
            assert(conversions === 0 && traps === 0, 'receiver precedes author conversion and proxy traps');
          });
        }
      } finally { await context.close(); }
    }
  } finally { popup.close(); }
  globalThis.__nodeReplacementResults = {rows, passed: rows.filter(row => row.pass).length, total: rows.length, failures: rows.filter(row => !row.pass)};
  return rows.every(row => row.pass);
})()
