(async () => {
  const rows = [];
  const assert = (ok, message) => { if (!ok) throw Error(message); };
  const check = async (name, callback) => {
    try { await callback(); rows.push({name, pass: true}); }
    catch (error) { rows.push({name, pass: false, message: String(error)}); }
  };
  const thrown = callback => { try { callback(); } catch (error) { return error; } throw Error('expected exception'); };
  const closeTo = (actual, expected, label) => assert(Math.abs(actual - expected) <= 5e-6 * Math.max(1, Math.abs(expected)), label + ': ' + actual + ' != ' + expected);
  const popup = open();
  const realms = [['main', window], ['child', document.getElementById('child').contentWindow], ['popup', popup]];
  try {
    for (const [label, w] of realms) {
      const context = new w.OfflineAudioContext(1, 16, 48000);
      const options = {feedforward: [1], feedback: [1, -.9]};
      const make = (b = options.feedforward, a = options.feedback) => new w.IIRFilterNode(context, {feedforward: b, feedback: a});
      const curve = (node, frequencies) => {
        const f = new w.Float32Array(frequencies), m = new w.Float32Array(f.length), p = new w.Float32Array(f.length);
        assert(node.getFrequencyResponse(f, m, p) === undefined, 'undefined return');
        return [m, p];
      };
      await check(label + '/IIR/interface and defaults', () => {
        const C = w.IIRFilterNode, descriptor = Object.getOwnPropertyDescriptor(w, 'IIRFilterNode'), node = make();
        assert(C.name === 'IIRFilterNode' && C.length === 2 && descriptor.writable && descriptor.configurable && !descriptor.enumerable, 'constructor metadata');
        assert(Object.getPrototypeOf(C) === w.AudioNode && Object.getPrototypeOf(C.prototype) === w.AudioNode.prototype, 'AudioNode inheritance');
        assert(node instanceof C && node instanceof w.AudioNode && node instanceof w.EventTarget && node.context === context, 'native identity and context');
        assert(Object.prototype.toString.call(node) === '[object IIRFilterNode]' && Object.keys(node).length === 0, 'tag and no coefficient expandos');
        assert(node.numberOfInputs === 1 && node.numberOfOutputs === 1 && node.channelCount === 2 && node.channelCountMode === 'max' && node.channelInterpretation === 'speakers', 'native defaults');
        class Sub extends C {}; assert(new Sub(context, options) instanceof Sub, 'new.target');
      });
      await check(label + '/IIR/required constructor and context', () => {
        const C = w.IIRFilterNode;
        for (const run of [() => C(context, options), () => new C(), () => new C(context), ...[undefined, null, {}, 1, true].map(value => () => new C(context, value))]) assert(thrown(run) instanceof w.TypeError, 'required arguments and coefficients');
        let reads = 0; const poisoned = {get feedback() { reads++; throw Error('options read'); }};
        for (const bad of [null, undefined, {}, Object.create(context), new Proxy(context, {})]) assert(thrown(() => new C(bad, poisoned)) instanceof w.TypeError, 'native context required');
        assert(reads === 0, 'context validation before options');
        for (const incomplete of [{feedback: [1]}, {feedforward: [1]}, {feedback: undefined, feedforward: [1]}, {feedback: [1], feedforward: null}]) assert(thrown(() => new C(context, incomplete)) instanceof w.TypeError, 'required members');
        const inherited = Object.create(options); assert(new C(context, inherited).context === context, 'inherited dictionary members');
      });
      await check(label + '/IIR/dictionary order and pending exceptions', () => {
        const order = [], dictionary = Object.create(null);
        for (const [key, value] of [['channelCount', 1], ['channelCountMode', 'explicit'], ['channelInterpretation', 'discrete'], ['feedback', [1, -.5]], ['feedforward', [2]]]) Object.defineProperty(dictionary, key, {get() { order.push(key); return value; }});
        const node = new w.IIRFilterNode(context, dictionary);
        assert(order.join() === 'channelCount,channelCountMode,channelInterpretation,feedback,feedforward', 'inherited members then derived lexical order');
        assert(node.channelCount === 1 && node.channelCountMode === 'explicit' && node.channelInterpretation === 'discrete', 'inherited values applied');
        const sentinel = {}; let reads = 0;
        assert(thrown(() => new w.IIRFilterNode(context, {feedback: [], get feedforward() { throw sentinel; }})) === sentinel, 'dictionary conversion before coefficient validation');
        assert(thrown(() => new w.IIRFilterNode(context, {channelCount: 0, feedback: [1], get feedforward() { throw sentinel; }})) === sentinel, 'dictionary conversion before channel validation');
        assert(thrown(() => new w.IIRFilterNode(context, {get feedback() { throw sentinel; }, get feedforward() { reads++; return [1]; }})) === sentinel && reads === 0, 'pending exception stops later member reads');
        assert(thrown(() => new w.IIRFilterNode(context, {feedback: [NaN], get feedforward() { reads++; return [1]; }})) instanceof w.TypeError && reads === 0, 'element conversion stops later member reads');
      });
      await check(label + '/IIR/channel options and graph identity', () => {
        for (const mode of ['max', 'clamped-max', 'explicit']) {
          const node = new w.IIRFilterNode(context, {...options, channelCount: 32, channelCountMode: mode, channelInterpretation: 'discrete'});
          assert(node.channelCount === 32 && node.channelCountMode === mode && node.channelInterpretation === 'discrete', 'common AudioNode options');
          assert(node.connect(context.destination) === context.destination, 'connection identity'); node.disconnect();
        }
        for (const value of [0, 33, -1]) assert(thrown(() => new w.IIRFilterNode(context, {...options, channelCount: value})).name === 'NotSupportedError', 'channel range');
        for (const extra of [{channelCountMode: 'invalid'}, {channelInterpretation: 'invalid'}]) assert(thrown(() => new w.IIRFilterNode(context, {...options, ...extra})) instanceof w.TypeError, 'enum conversion');
      });
      await check(label + '/IIR/coefficient lengths and zero constraints', () => {
        const create = (b, a) => context.createIIRFilter(b, a);
        for (const [b, a] of [[[], [1]], [[1], []], [[], []], [new Array(21).fill(1), [1]], [[1], new Array(21).fill(1)]]) {
          for (const run of [() => create(b, a), () => make(b, a)]) assert(thrown(run).name === 'NotSupportedError', 'coefficient length range');
        }
        for (const [b, a] of [[[0], [1]], [[-0, 0], [1]], [[1], [0]], [[1], [-0, .1]]]) {
          for (const run of [() => create(b, a), () => make(b, a)]) assert(thrown(run).name === 'InvalidStateError', 'nonzero constraints');
        }
        for (const length of [1, 2, 20]) {
          const b = new Array(length).fill(0), a = new Array(length).fill(0); b[length - 1] = 1; a[0] = 1;
          assert(create(b, a) instanceof w.IIRFilterNode && make(b, a) instanceof w.IIRFilterNode, 'supported boundary lengths');
        }
      });
      await check(label + '/IIR/sequence double conversion and precision', () => {
        for (const value of [NaN, Infinity, -Infinity, undefined, 'x', Symbol(), 1n]) {
          for (const run of [() => context.createIIRFilter([value], [1]), () => context.createIIRFilter([1], [1, value]), () => make([value], [1]), () => make([1], [1, value])]) assert(thrown(run) instanceof w.TypeError, 'restricted double');
        }
        const converted = context.createIIRFilter(['2', null, false], [true]); closeTo(curve(converted, [0])[0][0], 2, 'ToNumber conversions');
        closeTo(curve(make([1e-100], [1e-100]), [0])[0][0], 1, 'double coefficients do not underflow to float');
        const nearPole = make([1], [1, -(1 - 2 ** -40)]);
        closeTo(curve(nearPole, [0])[0][0], 2 ** 40, 'coefficients preserve double precision near a pole');
        assert(make([1], [1, -2]) instanceof w.IIRFilterNode, 'unstable filters are allowed');
      });
      await check(label + '/IIR/factory sequence order and arity', () => {
        const order = [];
        const sequence = key => ({*[Symbol.iterator]() { order.push(key); yield {valueOf() { order.push(key + '/number'); return 1; }}; }});
        context.createIIRFilter(sequence('feedforward'), sequence('feedback'));
        assert(order.join() === 'feedforward,feedforward/number,feedback,feedback/number', 'positional sequence conversion order');
        let reads = 0; const poison = {get [Symbol.iterator]() { reads++; throw Error('iterator'); }};
        assert(thrown(() => context.createIIRFilter(poison)) instanceof w.TypeError && reads === 0, 'required arity preflight');
        const sentinel = {}; const throwing = {[Symbol.iterator]() { throw sentinel; }};
        assert(thrown(() => context.createIIRFilter(throwing, poison)) === sentinel && reads === 0, 'preserve original iterator exception');
        for (const invalid of [undefined, null, 1, {}, {length: 1, 0: 1}]) assert(thrown(() => context.createIIRFilter(invalid, [1])) instanceof w.TypeError, 'iterable required');
        assert(thrown(() => context.createIIRFilter([], throwing)) === sentinel, 'later conversion precedes earlier range checks');
      });
      await check(label + '/IIR/copied immutable native coefficients', () => {
        const b = new w.Float64Array([2]), a = new w.Float64Array([1, -.5]);
        const node = context.createIIRFilter(b, a); b[0] = 7; a[1] = .5;
        structuredClone(b.buffer, {transfer: [b.buffer]}); structuredClone(a.buffer, {transfer: [a.buffer]});
        node.feedforward = [100]; node.feedback = [1]; node.__moliIirFeedforward = [100]; node.__moliIirFeedback = [1];
        closeTo(curve(node, [0])[0][0], 4, 'private copy survives author mutation and transfer');
        if (typeof w.SharedArrayBuffer === 'function') {
          const shared = new w.Float64Array(new w.SharedArrayBuffer(8)); shared[0] = 2;
          closeTo(curve(context.createIIRFilter(shared, [1]), [0])[0][0], 2, 'sequence is not a BufferSource shared-memory restriction');
        }
      });
      await check(label + '/IIR/one-pole analytic response', () => {
        const f = Array.from({length: 1000}, (_, index) => index * 24), [m, p] = curve(make(), f);
        for (let index = 0; index < f.length; index++) {
          const omega = 2 * Math.PI * f[index] / 48000;
          closeTo(m[index], 1 / Math.sqrt(1.81 - 1.8 * Math.cos(omega)), 'one-pole magnitude');
          closeTo(p[index], Math.atan(-.9 * Math.sin(omega) / (1 - .9 * Math.cos(omega))), 'one-pole phase');
        }
      });
      await check(label + '/IIR/FIR and general transfer functions', () => {
        for (const [b, a] of [[[2], [.5]], [[0, 1], [1]], [[1, .25, -.125], [2, -.2]], [Array.from({length: 20}, (_, i) => 1 / 2 ** i), [1, -.15, .01]]]) {
          const f = Array.from({length: 97}, (_, i) => i * 240), [m, p] = curve(make(b, a), f);
          for (let i = 0; i < f.length; i++) {
            const omega = 2 * Math.PI * f[i] / 48000;
            const polynomial = coefficients => coefficients.reduce(([r, j], coefficient, k) => [r + coefficient * Math.cos(k * omega), j - coefficient * Math.sin(k * omega)], [0, 0]);
            const [br, bi] = polynomial(b), [ar, ai] = polynomial(a), divisor = ar * ar + ai * ai;
            const real = (br * ar + bi * ai) / divisor, imag = (bi * ar - br * ai) / divisor;
            closeTo(m[i], Math.hypot(real, imag), 'general magnitude'); closeTo(p[i], Math.atan2(imag, real), 'general phase');
          }
        }
      });
      await check(label + '/IIR/valid range includes endpoints', () => {
        const node = make([2], [1]), f = [-1, 24001, Infinity, -Infinity, NaN, -0, 0, 24000], [m, p] = curve(node, f);
        for (let i = 0; i < 5; i++) assert(Number.isNaN(m[i]) && Number.isNaN(p[i]), 'invalid frequency produces both NaNs');
        for (let i = 5; i < f.length; i++) { closeTo(m[i], 2, 'inclusive endpoint gain'); closeTo(p[i], 0, 'inclusive endpoint phase'); }
        const pole = curve(make([1], [1, -1]), [0]); assert(pole[0][0] === Infinity && Number.isNaN(pole[1][0]), 'pole is not rejected as unstable');
      });
      await check(label + '/IIR/native sample rate and view boundaries', () => {
        const offline = new w.OfflineAudioContext(1, 16, 44100.1), rate = Math.fround(44100.1), node = new w.IIRFilterNode(offline, {feedforward: [1], feedback: [1, -.5]});
        Object.defineProperty(offline, 'sampleRate', {get() { throw Error('public sample rate'); }});
        let reads = 0; const f = new w.Float32Array([99, 0, 8000, 22051, 99]), m = new w.Float32Array(5).fill(57), p = new w.Float32Array(5).fill(57);
        const fv = f.subarray(1, 4), mv = m.subarray(1, 4), pv = p.subarray(1, 4);
        for (const view of [fv, mv, pv]) for (const key of ['length', 'byteLength', 'byteOffset', 'buffer']) Object.defineProperty(view, key, {get() { reads++; throw Error('public view metadata'); }});
        node.getFrequencyResponse(fv, mv, pv);
        assert(reads === 0 && m[0] === 57 && m[4] === 57 && p[0] === 57 && p[4] === 57 && f[0] === 99 && f[4] === 99, 'native bounds and no out-of-view writes');
        closeTo(m[1], 2, 'DC gain'); const omega = 2 * Math.PI * 8000 / rate;
        closeTo(m[2], 1 / Math.sqrt(1.25 - Math.cos(omega)), 'native fractional rate'); assert(Number.isNaN(m[3]) && Number.isNaN(p[3]), 'native Nyquist limit');
      });
      await check(label + '/IIR/typed array argument conversion', () => {
        const node = make(), output = new w.Float32Array(2).fill(57);
        for (const invalid of [undefined, null, [], {}, new w.Float64Array(2), new w.Int32Array(2), new w.DataView(new w.ArrayBuffer(8)), new Proxy(output, {}), ...(typeof w.SharedArrayBuffer === 'function' ? [new w.Float32Array(new w.SharedArrayBuffer(8))] : []), new w.Float32Array(new w.ArrayBuffer(8, {maxByteLength: 16}))]) {
          for (let index = 0; index < 3; index++) {
            const args = [output, output, output]; args[index] = invalid;
            assert(thrown(() => node.getFrequencyResponse(...args)) instanceof w.TypeError && output.every(value => value === 57), 'strict fixed non-shared Float32Array before writes');
          }
        }
        for (const args of [[], [output], [output, output]]) assert(thrown(() => node.getFrequencyResponse(...args)) instanceof w.TypeError, 'three required arguments');
      });
      await check(label + '/IIR/equal lengths and detached arrays', () => {
        const node = make(), output = new w.Float32Array(3).fill(57);
        for (const args of [[new w.Float32Array(2), output, new w.Float32Array(2)], [new w.Float32Array(2), new w.Float32Array(2), output]]) assert(thrown(() => node.getFrequencyResponse(...args)).name === 'InvalidAccessError' && output.every(value => value === 57), 'length mismatch without mutation');
        const detached = new w.Float32Array(1); structuredClone(detached.buffer, {transfer: [detached.buffer]});
        assert(node.getFrequencyResponse(detached, new w.Float32Array(0), new w.Float32Array(0)) === undefined, 'detached arrays have native zero length');
        assert(thrown(() => node.getFrequencyResponse(detached, output, output)).name === 'InvalidAccessError', 'detached length mismatch');
        assert(thrown(() => node.getFrequencyResponse(new w.Float32Array(2), output, {})) instanceof w.TypeError, 'all conversion before length validation');
      });
      await check(label + '/IIR/overlapping input and output views', () => {
        const node = make([2], [1]), buffer = new w.Float32Array([0, 1000, 24000, 57, 57]), f = buffer.subarray(0, 3), m = buffer.subarray(1, 4), p = new w.Float32Array(3);
        node.getFrequencyResponse(f, m, p); assert(m.every(value => value === 2) && p.every(value => value === 0), 'frequency snapshot precedes writes');
        const output = new w.Float32Array(4).fill(57);
        node.getFrequencyResponse(new w.Float32Array(3), output.subarray(0, 3), output.subarray(1, 4));
        assert(output.join() === '2,2,2,0', 'per-frequency magnitude then phase writes');
      });
      await check(label + '/IIR/native receiver before arguments', () => {
        const method = w.IIRFilterNode.prototype.getFrequencyResponse, real = make(), descriptor = Object.getOwnPropertyDescriptor(w.IIRFilterNode.prototype, 'getFrequencyResponse');
        assert(method.name === 'getFrequencyResponse' && method.length === 3 && descriptor.enumerable && descriptor.configurable && descriptor.writable, 'method metadata');
        let traps = 0; const trap = () => { traps++; throw Error('proxy trap'); }; const revoked = Proxy.revocable(real, {}); revoked.revoke();
        for (const receiver of [null, {}, w.IIRFilterNode.prototype, Object.create(real), new Proxy(real, {get: trap, getPrototypeOf: trap}), revoked.proxy]) assert(thrown(() => method.call(receiver, {}, {}, {})) instanceof w.TypeError, 'strict native receiver and callee realm');
        assert(traps === 0, 'brand check never invokes author Proxy traps');
        Object.setPrototypeOf(real, null); const [m] = (() => { const m = new w.Float32Array(1), p = new w.Float32Array(1); method.call(real, new w.Float32Array(1), m, p); return [m, p]; })();
        closeTo(m[0], 10, 'genuine native identity survives prototype mutation');
      });
      await check(label + '/IIR/cross-realm factory and exception realm', () => {
        const foreign = new window.OfflineAudioContext(1, 16, 44100), method = w.BaseAudioContext.prototype.createIIRFilter;
        assert(method.name === 'createIIRFilter' && method.length === 2, 'factory metadata');
        const node = method.call(foreign, [1], [1]); assert(Object.getPrototypeOf(node) === window.IIRFilterNode.prototype && node.context === foreign, 'factory uses context owner realm');
        assert(thrown(() => method.call(foreign, [], [1])) instanceof w.DOMException, 'factory errors use callee realm');
        assert(thrown(() => method.call(new Proxy(foreign, {}), [1], [1])) instanceof w.TypeError, 'native factory receiver');
        const response = w.IIRFilterNode.prototype.getFrequencyResponse;
        const m = new window.Float32Array(1), p = new window.Float32Array(1); response.call(node, new window.Float32Array(1), m, p); assert(m[0] === 1 && p[0] === 0, 'cross-realm native node and views');
        assert(thrown(() => response.call(node, m, new window.Float32Array(2), p)) instanceof w.DOMException, 'response errors use callee realm');
        assert(thrown(() => response.call(node, {}, m, p)) instanceof w.TypeError, 'conversion errors use callee realm');
        const intrinsic = window.IIRFilterNode; try { window.IIRFilterNode = function() { throw Error('author constructor'); }; assert(Object.getPrototypeOf(method.call(foreign, [1], [1])) === intrinsic.prototype, 'factory uses intrinsic constructor'); } finally { window.IIRFilterNode = intrinsic; }
      });
    }
  } finally { popup.close(); }
  globalThis.__nodeReplacementResults = {rows, passed:rows.filter(row => row.pass).length, total:rows.length, failures:rows.filter(row => !row.pass)};
  return rows.every(row => row.pass);
})()
