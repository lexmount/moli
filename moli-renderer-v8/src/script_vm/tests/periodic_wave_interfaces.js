(async () => {
  const rows = [];
  const assert = (ok, message) => { if (!ok) throw Error(message); };
  const thrown = callback => { try { callback(); } catch (error) { return error; } throw Error('expected exception'); };
  const check = async (name, run) => {
    try { await run(); rows.push({name, pass: true}); }
    catch (error) { rows.push({name, pass: false, message: String(error)}); }
  };
  const popup = open();
  const realms = [['main', window], ['child', document.getElementById('child').contentWindow], ['popup', popup]];
  try {
    for (const [label, w] of realms) {
      const context = new w.OfflineAudioContext(1, 16, 48000);
      const wave = () => new w.PeriodicWave(context);
      const oscillator = () => new w.OscillatorNode(context);
      await check(label + '/PeriodicWave/interface and native allocation', () => {
        const C = w.PeriodicWave, descriptor = Object.getOwnPropertyDescriptor(w, 'PeriodicWave'), value = wave();
        assert(C.name === 'PeriodicWave' && C.length === 1, 'constructor name and length');
        assert(descriptor.writable && descriptor.configurable && !descriptor.enumerable, 'global descriptor');
        assert(Object.getPrototypeOf(C.prototype) === w.Object.prototype && Object.getPrototypeOf(C) === w.Function.prototype, 'plain interface inheritance');
        assert(value instanceof C && Object.prototype.toString.call(value) === '[object PeriodicWave]', 'native interface and tag');
        assert(Object.keys(value).length === 0, 'coefficients and context remain internal');
        class Sub extends C {}; assert(new Sub(context) instanceof Sub, 'new.target');
      });
      await check(label + '/PeriodicWave/context and optional dictionary', () => {
        const C = w.PeriodicWave;
        for (const run of [() => C(context), () => new C(), ...[null, undefined, {}, Object.create(context), new Proxy(context, {})].map(value => () => new C(value))]) assert(thrown(run) instanceof w.TypeError, 'new and genuine context required');
        let reads = 0; const poison = {get disableNormalization() { reads++; throw Error('read'); }};
        for (const bad of [null, {}, new Proxy(context, {})]) assert(thrown(() => new C(bad, poison)) instanceof w.TypeError, 'context conversion before dictionary');
        assert(reads === 0, 'no options side effects');
        for (const options of [undefined, null, {}, {real: undefined, imag: undefined}]) assert(new C(context, options) instanceof C, 'optional defaults');
        for (const options of [1, false, 'x', Symbol()]) assert(thrown(() => new C(context, options)) instanceof w.TypeError, 'dictionary type');
      });
      await check(label + '/PeriodicWave/dictionary inheritance and order', () => {
        const order = [], options = Object.create(null);
        for (const [key, value] of [['disableNormalization', false], ['imag', [0, 1]], ['real', [0, 2]]]) Object.defineProperty(options, key, {get() { order.push(key); return value; }});
        assert(new w.PeriodicWave(context, options) instanceof w.PeriodicWave, 'valid options');
        assert(order.join() === 'disableNormalization,imag,real', 'inherited then derived lexical order');
        assert(new w.PeriodicWave(context, Object.create({imag: [0, 1]})) instanceof w.PeriodicWave, 'inherited members');
        const sentinel = {};
        assert(thrown(() => new w.PeriodicWave(context, {imag: [], get real() { throw sentinel; }})) === sentinel, 'complete conversion before length validation');
        assert(thrown(() => new w.PeriodicWave(context, {get disableNormalization() { throw sentinel; }})) === sentinel, 'original getter exception');
        const boolean = {valueOf() { throw sentinel; }, [Symbol.toPrimitive]() { throw sentinel; }};
        assert(new w.PeriodicWave(context, {disableNormalization: boolean}) instanceof w.PeriodicWave, 'ToBoolean does not invoke conversion hooks');
      });
      await check(label + '/PeriodicWave/partial and boundary coefficients', () => {
        const C = w.PeriodicWave;
        for (const options of [{real: [3, 1]}, {imag: [4, 1]}, {real: [0, 0], imag: [0, 0]}]) assert(new C(context, options) instanceof C, 'one sequence zero-fills the other and all-zero is legal');
        for (const options of [{real: []}, {imag: [0]}, {real: [0, 1], imag: []}, {real: [0], imag: [0, 1]}, {real: [0, 1], imag: [0, 1, 2]}]) assert(thrown(() => new C(context, options)).name === 'IndexSizeError', 'minimum and equal-length rules');
        for (const length of [2, 8192]) assert(new C(context, {real: new w.Float32Array(length), imag: new w.Float32Array(length)}) instanceof C, 'required supported size');
      });
      await check(label + '/PeriodicWave/restricted float and DC conversion', () => {
        const C = w.PeriodicWave;
        for (const value of [NaN, Infinity, -Infinity, 1e100, undefined, 'x', Symbol(), 1n]) {
          for (const options of [{real: [value, 1]}, {imag: [0, value]}]) assert(thrown(() => new C(context, options)) instanceof w.TypeError, 'finite float required including DC entry');
          assert(thrown(() => context.createPeriodicWave([0, value], [0, 1])) instanceof w.TypeError, 'factory restricted float');
        }
        assert(new C(context, {real: [null, true, '2.5'], imag: [false, '1', 0]}) instanceof C, 'ToNumber before float rounding');
        for (const value of [null, 1, {}, {length: 2, 0: 0, 1: 1}]) assert(thrown(() => new C(context, {real: value})) instanceof w.TypeError, 'iterable sequence required');
      });
      await check(label + '/PeriodicWave/author iterators and original exceptions', () => {
        const order = [], sentinel = {};
        const sequence = name => ({*[Symbol.iterator]() { order.push(name); yield 0; yield {valueOf() { order.push(name + '/number'); return 1; }}; }});
        new w.PeriodicWave(context, {imag: sequence('imag'), real: sequence('real')});
        assert(order.join() === 'imag,imag/number,real,real/number', 'dictionary conversion order');
        assert(thrown(() => new w.PeriodicWave(context, {imag: {[Symbol.iterator]() { throw sentinel; }}})) === sentinel, 'iterator exception');
        assert(thrown(() => new w.PeriodicWave(context, {imag: [0, {valueOf() { throw sentinel; }}]})) === sentinel, 'number conversion exception');
        const source = new w.Float32Array([0, 1]);
        source[Symbol.iterator] = function* () { order.push('override'); yield 0; yield 2; };
        assert(new w.PeriodicWave(context, {real: source}) instanceof w.PeriodicWave && order.includes('override'), 'sequence conversion observes overridden typed-array iterator');
      });
      await check(label + '/PeriodicWave/sequences backed by shared or resizable buffers', () => {
        if (typeof w.SharedArrayBuffer === 'function') {
          const values = new w.Float32Array(new w.SharedArrayBuffer(8)); values[1] = 1;
          assert(context.createPeriodicWave(values, values) instanceof w.PeriodicWave, 'shared backing is allowed for iterable sequences');
        }
        if (typeof w.ArrayBuffer.prototype.resize === 'function') {
          const buffer = new w.ArrayBuffer(8, {maxByteLength: 16}), values = new w.Float32Array(buffer); values[1] = 1;
          assert(new w.PeriodicWave(context, {real: values}) instanceof w.PeriodicWave, 'resizable backing is allowed for iterable sequences');
        }
      });
      await check(label + '/PeriodicWave/factory arity and conversion order', () => {
        const order = [], sentinel = {};
        const sequence = name => ({*[Symbol.iterator]() { order.push(name); yield 0; yield 1; }});
        assert(context.createPeriodicWave(sequence('real'), sequence('imag'), {get disableNormalization() { order.push('constraints'); return false; }}) instanceof w.PeriodicWave, 'factory result');
        assert(order.join() === 'real,imag,constraints', 'positional conversions');
        let reads = 0; const poison = {get [Symbol.iterator]() { reads++; throw sentinel; }};
        assert(thrown(() => context.createPeriodicWave(poison)) instanceof w.TypeError && reads === 0, 'arity before sequence conversion');
        assert(thrown(() => context.createPeriodicWave([], [0, 1], {get disableNormalization() { throw sentinel; }})) === sentinel, 'constraints conversion before mismatched-length rule');
        for (const run of [() => context.createPeriodicWave([], []), () => context.createPeriodicWave([0], [0]), () => context.createPeriodicWave([0, 1], [0, 1, 2])]) assert(thrown(run).name === 'IndexSizeError', 'factory lengths');
        assert(thrown(() => context.createPeriodicWave([0, 1], [0, 1], 1)) instanceof w.TypeError, 'constraints dictionary type');
      });
      await check(label + '/PeriodicWave/factory receiver and error realm', () => {
        const create = w.BaseAudioContext.prototype.createPeriodicWave, sentinel = {}; let reads = 0, traps = 0;
        const input = {get [Symbol.iterator]() { reads++; throw sentinel; }};
        const proxy = new Proxy(context, {get() { traps++; throw sentinel; }, getPrototypeOf() { traps++; throw sentinel; }});
        const revoked = Proxy.revocable(context, {}); revoked.revoke();
        for (const bad of [{}, Object.create(context), proxy, revoked.proxy]) assert(thrown(() => create.call(bad, input, input)) instanceof w.TypeError, 'brand before arguments');
        assert(reads === 0 && traps === 0, 'receiver check does not invoke author code');
        assert(create.length === 2, 'factory length');
        if (w !== window) assert(!(thrown(() => create.call({}, [0, 1], [0, 1])) instanceof TypeError), 'callee realm error');
      });
      await check(label + '/PeriodicWave/relevant realm and public constructor override', () => {
        const saved = w.PeriodicWave;
        try {
          w.PeriodicWave = function () { throw Error('public constructor'); };
          const value = BaseAudioContext.prototype.createPeriodicWave.call(context, [0, 1], [0, 1]);
          assert(Object.getPrototypeOf(value) === saved.prototype, 'factory relevant context realm and intrinsic constructor');
        } finally { w.PeriodicWave = saved; }
        const value = new w.PeriodicWave(new OfflineAudioContext(1, 16, 48000));
        assert(Object.getPrototypeOf(value) === w.PeriodicWave.prototype, 'constructor realm despite foreign context');
      });
      await check(label + '/Oscillator/interface defaults and inheritance', () => {
        const C = w.OscillatorNode, node = oscillator();
        assert(C.name === 'OscillatorNode' && C.length === 1, 'constructor metadata');
        assert(Object.getPrototypeOf(C) === w.AudioScheduledSourceNode && Object.getPrototypeOf(C.prototype) === w.AudioScheduledSourceNode.prototype, 'scheduled-source inheritance');
        assert(node instanceof w.AudioNode && node instanceof w.EventTarget && node.context === context && node.type === 'sine', 'brand and defaults');
        assert(node.numberOfInputs === 0 && node.numberOfOutputs === 1 && node.channelCount === 2 && node.channelCountMode === 'max', 'node metadata');
        assert(Object.prototype.toString.call(node) === '[object OscillatorNode]' && Object.keys(node).length === 0, 'tag and no public state');
        class Sub extends C {}; assert(new Sub(context) instanceof Sub, 'new.target');
      });
      await check(label + '/Oscillator/AudioParam identity and initial options', () => {
        const node = new w.OscillatorNode(context, {detune: 7.1, frequency: 918.1, type: 'sawtooth'});
        assert(node.frequency === node.frequency && node.detune === node.detune && node.frequency !== node.detune, 'distinct SameObject parameters');
        assert(node.frequency instanceof w.AudioParam && node.detune instanceof w.AudioParam, 'parameter realm');
        assert(node.frequency.value === Math.fround(918.1) && node.detune.value === Math.fround(7.1), 'restricted float rounding');
        assert(node.frequency.defaultValue === 440 && node.detune.defaultValue === 0 && node.frequency.automationRate === 'a-rate', 'metadata independent of initial values');
        assert(node.frequency.minValue === -24000 && node.frequency.maxValue === 24000, 'native context sample rate');
        assert(!Reflect.set(node, 'frequency', {}) && !Reflect.set(node, 'detune', {}), 'readonly attributes');
        assert(new w.OscillatorNode(context, {frequency: 1e8}).frequency.value === 24000, 'nominal initial clamp');
      });
      await check(label + '/Oscillator/constructor conversion and validation order', () => {
        const order = [], options = Object.create(null);
        for (const [key, value] of [['channelCount', 1], ['channelCountMode', 'explicit'], ['channelInterpretation', 'discrete'], ['detune', 1], ['frequency', 2], ['periodicWave', wave()], ['type', 'sine']]) Object.defineProperty(options, key, {get() { order.push(key); return value; }});
        const node = new w.OscillatorNode(context, options);
        assert(order.join() === 'channelCount,channelCountMode,channelInterpretation,detune,frequency,periodicWave,type', 'inherited then derived lexical order');
        assert(node.channelCount === 1 && node.channelCountMode === 'explicit' && node.channelInterpretation === 'discrete' && node.type === 'custom', 'options and wave override');
        const sentinel = {}; let reads = 0;
        const poison = {get type() { reads++; throw sentinel; }};
        assert(thrown(() => new w.OscillatorNode({}, poison)) instanceof w.TypeError && reads === 0, 'native context before options');
        assert(thrown(() => new w.OscillatorNode(context, {channelCount: 0, get type() { throw sentinel; }})) === sentinel, 'all conversions before channel validation');
        for (const options of [{type: 'invalid'}, {frequency: Infinity}, {detune: 1e100}, {periodicWave: null}]) assert(thrown(() => new w.OscillatorNode(context, options)) instanceof w.TypeError, 'enum, float, and interface conversion');
      });
      await check(label + '/Oscillator/custom construction and wave precedence', () => {
        assert(thrown(() => new w.OscillatorNode(context, {type: 'custom'})).name === 'InvalidStateError', 'custom needs a wave');
        for (const type of ['sine', 'square', 'sawtooth', 'triangle', 'custom']) assert(new w.OscillatorNode(context, {type, periodicWave: wave()}).type === 'custom', 'wave selects custom type');
        for (const type of ['sine', 'square', 'sawtooth', 'triangle']) assert(new w.OscillatorNode(context, {type}).type === type, 'built-in construction');
      });
      await check(label + '/Oscillator/type setter and preserved failures', () => {
        const node = oscillator();
        for (const type of ['triangle', 'square', 'sawtooth', 'sine']) { node.type = type; assert(node.type === type, 'built-in type'); }
        node.type = 'invalid'; assert(node.type === 'sine', 'invalid enum setter ignored');
        assert(thrown(() => { node.type = 'custom'; }).name === 'InvalidStateError' && node.type === 'sine', 'custom setter rejected without mutation');
        const sentinel = {}; assert(thrown(() => { node.type = {toString() { throw sentinel; }}; }) === sentinel && node.type === 'sine', 'original conversion exception');
        assert(thrown(() => { node.type = Symbol(); }) instanceof w.TypeError, 'DOMString symbol');
        node.type = {toString() { return 'square'; }}; assert(node.type === 'square', 'DOMString conversion');
      });
      await check(label + '/Oscillator/setPeriodicWave identity and native receiver', () => {
        const node = oscillator(), value = wave(), set = w.OscillatorNode.prototype.setPeriodicWave;
        assert(set.length === 1 && set.call(node, value) === undefined && node.type === 'custom', 'method and type transition');
        const revoked = Proxy.revocable(value, {}); revoked.revoke();
        for (const bad of [undefined, null, {}, Object.create(value), new Proxy(value, {}), revoked.proxy]) assert(thrown(() => set.call(node, bad)) instanceof w.TypeError && node.type === 'custom', 'wave brand and no failed mutation');
        assert(thrown(() => set.call(node)) instanceof w.TypeError, 'required argument');
        let traps = 0; const sentinel = {};
        const proxy = new Proxy(node, {get() { traps++; throw sentinel; }, getPrototypeOf() { traps++; throw sentinel; }});
        for (const bad of [{}, Object.create(node), proxy]) assert(thrown(() => set.call(bad, value)) instanceof w.TypeError, 'receiver brand');
        assert(traps === 0, 'author receiver traps not invoked');
      });
      await check(label + '/Oscillator/native slots ignore public expandos and setters', () => {
        const node = oscillator(), value = wave(), type = Object.getOwnPropertyDescriptor(w.OscillatorNode.prototype, 'type');
        Object.defineProperty(node, 'type', {get() { throw Error('public type'); }, configurable: true});
        node.__moliOscillatorType = 'square'; node.__moliOscillatorPeriodicWave = null;
        node.setPeriodicWave(value);
        assert(type.get.call(node) === 'custom', 'method writes private state without public accessor');
        type.set.call(node, 'triangle'); assert(type.get.call(node) === 'triangle', 'prototype setter bypasses expando');
        const frequency = Object.getOwnPropertyDescriptor(w.OscillatorNode.prototype, 'frequency');
        node.__moliOscillatorFrequency = {}; assert(frequency.get.call(node) instanceof w.AudioParam, 'native parameter identity');
      });
      await check(label + '/Oscillator/cross realm factory and methods', () => {
        const saved = w.OscillatorNode;
        try {
          w.OscillatorNode = function () { throw Error('public constructor'); };
          const node = BaseAudioContext.prototype.createOscillator.call(context);
          assert(Object.getPrototypeOf(node) === saved.prototype && Object.getPrototypeOf(node.frequency) === w.AudioParam.prototype, 'factory relevant realm and intrinsic');
          saved.prototype.setPeriodicWave.call(node, new PeriodicWave(context));
          assert(node.type === 'custom', 'foreign binding and genuine wave');
        } finally { w.OscillatorNode = saved; }
        const error = thrown(() => Object.getOwnPropertyDescriptor(w.OscillatorNode.prototype, 'frequency').get.call({}));
        assert(error instanceof w.TypeError, 'getter callee realm');
        if (w !== window) assert(!(error instanceof TypeError), 'distinct error realm');
      });
      await check(label + '/Oscillator/shared source scheduling and EventTarget', () => {
        const node = oscillator(); let calls = 0; node.onended = () => calls++;
        node.addEventListener('ended', () => calls++); assert(node.dispatchEvent(new w.Event('ended')) && calls === 2, 'shared ordered EventTarget');
        node.start(); assert(thrown(() => node.start()).name === 'InvalidStateError', 'once-only scheduled source');
        node.stop(); node.type = 'triangle'; node.setPeriodicWave(wave()); assert(node.type === 'custom', 'wave changes allowed after start/stop');
      });
    }
  } finally { if (popup) popup.close(); }
  const result = {total: rows.length, passed: rows.filter(row => row.pass).length, failures: rows.filter(row => !row.pass), rows};
  globalThis.__nodeReplacementResults = result;
  return result.failures.length === 0;
})()
