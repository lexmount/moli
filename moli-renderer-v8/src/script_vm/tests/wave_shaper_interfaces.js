(async () => {
  const rows = [];
  const assert = (ok, message) => { if (!ok) throw Error(message); };
  const check = async (name, callback) => {
    try { await callback(); rows.push({name, pass: true}); }
    catch (error) { rows.push({name, pass: false, message: String(error)}); }
  };
  const thrown = callback => { try { callback(); } catch (error) { return error; } throw Error('expected exception'); };
  const equal = (actual, expected) => actual.length === expected.length && expected.every((value, index) => Object.is(value, actual[index]));
  const popup = open();
  const realms = [['main', window], ['child', document.getElementById('child').contentWindow], ['popup', popup]];
  try {
    for (const [label, w] of realms) {
      const context = new w.AudioContext();
      try {
        await check(label + '/WaveShaperNode/interface and defaults', () => {
          const C = w.WaveShaperNode, descriptor = Object.getOwnPropertyDescriptor(w, 'WaveShaperNode'), node = new C(context);
          assert(C.name === 'WaveShaperNode' && C.length === 1 && descriptor.writable && descriptor.configurable && !descriptor.enumerable, 'constructor metadata');
          assert(Object.getPrototypeOf(C) === w.AudioNode && Object.getPrototypeOf(C.prototype) === w.AudioNode.prototype, 'AudioNode inheritance');
          assert(node instanceof C && node instanceof w.AudioNode && node instanceof w.EventTarget && node.context === context, 'native inheritance and context');
          assert(node.numberOfInputs === 1 && node.numberOfOutputs === 1 && node.channelCount === 2 && node.channelCountMode === 'max' && node.channelInterpretation === 'speakers' && node.curve === null && node.oversample === 'none', 'native defaults');
          assert(thrown(() => C(context)) instanceof w.TypeError, 'new required');
          let reads = 0; const options = {get curve() { reads++; throw Error('options'); }};
          for (const bad of [undefined, null, {}, Object.create(context), new Proxy(context, {})]) assert(thrown(() => new C(bad, options)) instanceof w.TypeError, 'context native brand');
          assert(reads === 0, 'context check before options');
          class Sub extends C {}; assert(new Sub(context) instanceof Sub, 'new.target');
        });
        await check(label + '/WaveShaperNode/dictionary inheritance and sequence order', () => {
          const order = [], options = Object.create(null), expected = ['channelCount', 'channelCountMode', 'channelInterpretation', 'curve', 'iterator', 'first', 'second', 'oversample'];
          const curve = {[Symbol.iterator]() { order.push('iterator'); return [ {valueOf() { order.push('first'); return -0; }}, {valueOf() { order.push('second'); return 0.1; }} ][Symbol.iterator](); }};
          for (const [key, value] of [['channelCount', 3], ['channelCountMode', 'explicit'], ['channelInterpretation', 'discrete'], ['curve', curve], ['oversample', '2x']]) Object.defineProperty(options, key, {get() { order.push(key); return value; }});
          const node = new w.WaveShaperNode(context, options);
          assert(order.join() === expected.join(), 'base dictionary before derived and per-item float conversion');
          assert(node.channelCount === 3 && node.channelCountMode === 'explicit' && node.channelInterpretation === 'discrete' && node.oversample === '2x' && equal(node.curve, [-0, Math.fround(0.1)]), 'converted controls');
          const sentinel = {};
          assert(thrown(() => new w.WaveShaperNode(context, {channelCount: 0, curve: [1, 2], get oversample() { throw sentinel; }})) === sentinel, 'conversion before channel validation');
          assert(thrown(() => new w.WaveShaperNode(context, {curve: [1], get oversample() { throw sentinel; }})) === sentinel, 'later conversion before curve validation');
        });
        await check(label + '/WaveShaperNode/restricted sequence float and exception propagation', () => {
          for (const value of [NaN, Infinity, -Infinity, 1e100, Symbol(), 1n]) {
            let later = 0, closed = 0;
            const curve = {[Symbol.iterator]() { let count = 0; return {next() { return count++ === 0 ? {value, done: false} : {done: true}; }, return() { closed++; return {}; }}; }};
            assert(thrown(() => new w.WaveShaperNode(context, {curve, get oversample() { later++; return 'none'; }})) instanceof w.TypeError && later === 0 && closed === 0, 'float failure stops later conversion without iterator close');
          }
          const sentinel = {};
          assert(thrown(() => new w.WaveShaperNode(context, {curve: {[Symbol.iterator]() { throw sentinel; }}})) === sentinel, 'iterator exception');
          for (const curve of [null, 1, {}, '12']) assert(thrown(() => new w.WaveShaperNode(context, {curve})) instanceof w.TypeError, 'sequence requires an iterable object');
          assert(equal(new w.WaveShaperNode(context, {curve: [Number.MIN_VALUE, -Number.MIN_VALUE]}).curve, [0, -0]), 'float underflow preserves signed zero');
        });
        await check(label + '/WaveShaperNode/internal curve copy and owner realm', () => {
          const input = new w.Float32Array([-1, 0, 1]), node = new w.WaveShaperNode(context, {curve: input}), curve = node.curve;
          assert(curve instanceof w.Float32Array && curve !== input && curve !== node.curve && equal(curve, [-1, 0, 1]), 'fresh owner-realm copy');
          input[1] = 9; curve[1] = 8; w.structuredClone(curve.buffer, {transfer: [curve.buffer]});
          assert(equal(node.curve, [-1, 0, 1]), 'author mutation and transfer do not modify native curve');
          const nativeGetter = Object.getOwnPropertyDescriptor(window.WaveShaperNode.prototype, 'curve').get;
          assert(nativeGetter.call(node) instanceof w.Float32Array, 'borrowed getter returns receiver realm array');
        });
        await check(label + '/WaveShaperNode/typed setter intrinsic storage and nonfinite samples', () => {
          const node = context.createWaveShaper(), input = new w.Float32Array([9, NaN, Infinity, -0, 9]).subarray(1, 4); let reads = 0;
          for (const key of ['length', 'byteLength', 'byteOffset', 'buffer', Symbol.iterator]) Object.defineProperty(input, key, {get() { reads++; throw Error('public metadata'); }});
          node.curve = input;
          assert(reads === 0 && equal(node.curve, [NaN, Infinity, -0]), 'setter copies intrinsic view bounds, accepts unrestricted element data');
          input[1] = 8; assert(equal(node.curve, [NaN, Infinity, -0]), 'setter internal copy');
          for (const invalid of [[], new w.Float64Array(2), new Proxy(new w.Float32Array(2), {})]) assert(thrown(() => { context.createWaveShaper().curve = invalid; }) instanceof w.TypeError, 'Float32Array brand');
          const shared = new w.Float32Array(new w.WebAssembly.Memory({shared: true, initial: 1, maximum: 1}).buffer), resizable = new w.Float32Array(new w.ArrayBuffer(8, {maxByteLength: 16}));
          for (const invalid of [shared, resizable]) assert(thrown(() => { context.createWaveShaper().curve = invalid; }) instanceof w.TypeError, 'shared and resizable views rejected');
        });
        await check(label + '/WaveShaperNode/short and detached curve errors preserve state', () => {
          const node = context.createWaveShaper();
          for (const curve of [new w.Float32Array(0), new w.Float32Array(1)]) assert(thrown(() => { node.curve = curve; }).name === 'InvalidStateError' && node.curve === null, 'short curve rejection before mutation');
          const detached = new w.Float32Array(2); w.structuredClone(detached.buffer, {transfer: [detached.buffer]});
          assert(thrown(() => { node.curve = detached; }).name === 'InvalidStateError' && node.curve === null, 'detached curve has insufficient entries');
          node.curve = new w.Float32Array([-1, 1]); assert(equal(node.curve, [-1, 1]), 'failed writes do not consume once-only state');
          for (const curve of [[], [1]]) assert(thrown(() => new w.WaveShaperNode(context, {curve})).name === 'InvalidStateError', 'constructor applies curve minimum');
        });
        await check(label + '/WaveShaperNode/once-only curve survives clear', () => {
          const node = context.createWaveShaper(); node.curve = null; node.curve = undefined; node.curve = new w.Float32Array([-1, 1]);
          assert(thrown(() => { node.curve = new w.Float32Array([2, 3]); }).name === 'InvalidStateError' && equal(node.curve, [-1, 1]), 'once-only non-null setter');
          node.curve = null;
          assert(node.curve === null && thrown(() => { node.curve = new w.Float32Array([2, 3]); }).name === 'InvalidStateError', 'clear keeps curve-set flag');
          const constructed = new w.WaveShaperNode(context, {curve: [-1, 1]});
          assert(thrown(() => { constructed.curve = new w.Float32Array([2, 3]); }).name === 'InvalidStateError', 'constructor curve also consumes flag');
        });
        await check(label + '/WaveShaperNode/enum conversion and mutation', () => {
          const node = context.createWaveShaper();
          for (const oversample of ['none', '2x', '4x']) { node.oversample = oversample; assert(node.oversample === oversample, 'supported enum'); }
          for (const invalid of ['invalid', null, undefined, 1]) { node.oversample = invalid; assert(node.oversample === '4x', 'invalid enum attribute is ignored'); }
          let conversions = 0; node.oversample = {toString() { conversions++; return '2x'; }};
          assert(conversions === 1 && node.oversample === '2x', 'ToString once');
          const sentinel = {}; assert(thrown(() => { node.oversample = {toString() { throw sentinel; }}; }) === sentinel && node.oversample === '2x', 'conversion exception preserves state');
          assert(thrown(() => { node.oversample = Symbol(); }) instanceof w.TypeError, 'symbol enum value');
          for (const oversample of ['invalid', null, 1]) assert(thrown(() => new w.WaveShaperNode(context, {oversample})) instanceof w.TypeError, 'constructor enum rejects invalid token');
        });
        await check(label + '/WaveShaperNode/cross-realm factory and graph', () => {
          const foreign = new window.AudioContext();
          try {
            const method = w.BaseAudioContext.prototype.createWaveShaper;
            assert(method.name === 'createWaveShaper' && method.length === 0, 'factory metadata');
            const node = method.call(foreign, {toString() { throw Error('ignored argument'); }});
            assert(node instanceof window.WaveShaperNode && node.context === foreign, 'factory owner realm and ignored extra argument');
            assert(node.connect(foreign.destination) === foreign.destination, 'native graph connection'); node.disconnect();
            assert(thrown(() => node.connect(context.destination)).name === 'InvalidAccessError', 'cross-context graph rejection');
            let reads = 0; const poison = {toString() { reads++; return '4x'; }};
            assert(thrown(() => method.call(new Proxy(foreign, {}), poison)) instanceof w.TypeError && reads === 0, 'factory native receiver');
          } finally { foreign.close(); }
        });
        for (const key of ['curve', 'oversample']) await check(label + '/WaveShaperNode/' + key + '/native receivers', () => {
          const descriptor = Object.getOwnPropertyDescriptor(w.WaveShaperNode.prototype, key), real = context.createWaveShaper();
          assert(descriptor.enumerable && descriptor.configurable, 'member descriptor');
          let conversions = 0, traps = 0; const poison = {toString() { conversions++; throw Error('conversion'); }}, trap = () => { traps++; throw Error('proxy trap'); };
          const revoked = Proxy.revocable(real, {}); revoked.revoke();
          for (const kind of ['get', 'set']) {
            assert(descriptor[kind].name === kind + ' ' + key && descriptor[kind].length === (kind === 'get' ? 0 : 1), 'accessor metadata');
            for (const receiver of [null, {}, w.WaveShaperNode.prototype, Object.create(real), new Proxy(real, {get: trap, getPrototypeOf: trap}), revoked.proxy]) assert(thrown(() => descriptor[kind].call(receiver, poison)) instanceof w.TypeError, 'native receiver in callee realm');
          }
          assert(conversions === 0 && traps === 0, 'receiver check precedes author code');
        });
      } finally { await context.close(); }
    }
  } finally { popup.close(); }
  globalThis.__nodeReplacementResults = {rows, passed: rows.filter(row => row.pass).length, total: rows.length, failures: rows.filter(row => !row.pass)};
  return rows.every(row => row.pass);
})()
