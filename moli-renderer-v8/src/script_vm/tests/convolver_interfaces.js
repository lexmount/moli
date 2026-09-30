(async () => {
  const rows = [];
  const assert = (ok, message) => { if (!ok) throw Error(message); };
  const check = async (name, callback) => {
    try { await callback(); rows.push({name, pass: true}); }
    catch (error) { rows.push({name, pass: false, message: String(error)}); }
  };
  const thrown = callback => { try { callback(); } catch (error) { return error; } throw Error('expected exception'); };
  const popup = open();
  const realms = [['main', window], ['child', document.getElementById('child').contentWindow], ['popup', popup]];
  try {
    for (const [label, w] of realms) {
      const context = new w.AudioContext();
      const make = (channels = 1, rate = context.sampleRate) => new w.AudioBuffer({numberOfChannels: channels, length: 8, sampleRate: rate});
      try {
        await check(label + '/ConvolverNode/interface and defaults', () => {
          const C = w.ConvolverNode, descriptor = Object.getOwnPropertyDescriptor(w, 'ConvolverNode'), node = new C(context);
          assert(C.name === 'ConvolverNode' && C.length === 1 && descriptor.writable && descriptor.configurable && !descriptor.enumerable, 'constructor metadata');
          assert(Object.getPrototypeOf(C) === w.AudioNode && Object.getPrototypeOf(C.prototype) === w.AudioNode.prototype, 'AudioNode inheritance');
          assert(node instanceof C && node instanceof w.AudioNode && node instanceof w.EventTarget && node.context === context, 'native inheritance and context');
          assert(node.numberOfInputs === 1 && node.numberOfOutputs === 1 && node.channelCount === 2 && node.channelCountMode === 'clamped-max' && node.channelInterpretation === 'speakers' && node.buffer === null && node.normalize === true, 'native defaults');
          assert(thrown(() => C(context)) instanceof w.TypeError, 'new required');
          let reads = 0; const options = {get buffer() { reads++; throw Error('options'); }};
          for (const bad of [undefined, null, {}, Object.create(context), new Proxy(context, {})]) assert(thrown(() => new C(bad, options)) instanceof w.TypeError, 'context native brand');
          assert(reads === 0, 'context check before options');
          class Sub extends C {}; assert(new Sub(context) instanceof Sub, 'new.target');
        });
        await check(label + '/ConvolverNode/dictionary order before state validation', () => {
          const order = [], options = Object.create(null), buffer = make();
          for (const [key, value] of [['channelCount', 1], ['channelCountMode', 'explicit'], ['channelInterpretation', 'discrete'], ['buffer', buffer], ['disableNormalization', true]]) Object.defineProperty(options, key, {get() { order.push(key); return value; }});
          const node = new w.ConvolverNode(context, options);
          assert(order.join() === 'channelCount,channelCountMode,channelInterpretation,buffer,disableNormalization', 'inherited dictionary first then derived lexical order');
          assert(node.channelCount === 1 && node.channelCountMode === 'explicit' && node.channelInterpretation === 'discrete' && node.buffer === buffer && node.normalize === false, 'converted options');
          const sentinel = {};
          for (const invalid of [{channelCount: 3}, {buffer: make(3)}]) assert(thrown(() => new w.ConvolverNode(context, {...invalid, get disableNormalization() { throw sentinel; }})) === sentinel, 'all conversion before range and buffer checks');
          let later = 0;
          assert(thrown(() => new w.ConvolverNode(context, {buffer: {}, get disableNormalization() { later++; return true; }})) instanceof w.TypeError && later === 0, 'buffer brand before later dictionary members');
        });
        await check(label + '/ConvolverNode/channel constraints and unchanged state on error', () => {
          const node = context.createConvolver();
          for (const count of [1, 2]) { node.channelCount = count; assert(node.channelCount === count, 'supported channel count'); }
          for (const count of [0, 3, 33, -1]) assert(thrown(() => { node.channelCount = count; }).name === 'NotSupportedError' && node.channelCount === 2, 'channel constraint before mutation');
          for (const mode of ['explicit', 'clamped-max']) { node.channelCountMode = mode; assert(node.channelCountMode === mode, 'supported channel mode'); }
          assert(thrown(() => { node.channelCountMode = 'max'; }).name === 'NotSupportedError' && node.channelCountMode === 'clamped-max', 'max mode prohibited');
          node.channelCountMode = 'invalid'; assert(node.channelCountMode === 'clamped-max', 'unknown attribute token ignored');
          for (const options of [{channelCount: 0}, {channelCount: 3}, {channelCountMode: 'max'}]) assert(thrown(() => new w.ConvolverNode(context, options)).name === 'NotSupportedError', 'constructor uses same channel constraints');
        });
        await check(label + '/ConvolverNode/nullable and replaceable impulse response', () => {
          const node = context.createConvolver();
          for (const count of [1, 2, 4]) {
            const buffer = make(count); node.buffer = buffer; assert(node.buffer === buffer, 'supported response channels');
            node.buffer = buffer; assert(node.buffer === buffer, 'same buffer can be assigned again');
            node.buffer = null; assert(node.buffer === null, 'clear response');
          }
          node.buffer = undefined; assert(node.buffer === null, 'undefined converts to null');
          const real = make(), revoked = Proxy.revocable(real, {}); revoked.revoke();
          for (const value of [{}, Object.create(real), new Proxy(real, {}), revoked.proxy]) assert(thrown(() => { node.buffer = value; }) instanceof w.TypeError && node.buffer === null, 'native AudioBuffer argument required');
        });
        await check(label + '/ConvolverNode/native metadata prevents forged constraints', () => {
          const node = context.createConvolver(), real = make(2); let reads = 0;
          for (const key of ['sampleRate', 'numberOfChannels', 'length', 'getChannelData']) Object.defineProperty(real, key, {get() { reads++; throw Error('public audio metadata'); }});
          Object.setPrototypeOf(real, null); node.buffer = real;
          assert(node.buffer === real && reads === 0, 'configuration uses native metadata and channel content');
          for (const invalid of [make(3), make(1, context.sampleRate / 2)]) {
            Object.defineProperty(invalid, 'sampleRate', {value: context.sampleRate}); Object.defineProperty(invalid, 'numberOfChannels', {value: 1});
            const view = invalid.getChannelData(0);
            assert(thrown(() => { node.buffer = invalid; }).name === 'NotSupportedError' && node.buffer === real && view.length === 8, 'public data cannot bypass constraints and failed set cannot acquire content');
          }
        });
        await check(label + '/ConvolverNode/synchronous acquisition preserves writable buffer', () => {
          const node = context.createConvolver(), buffer = make(2), first = buffer.getChannelData(0), second = buffer.getChannelData(1);
          first[0] = 7; second[0] = 8; node.buffer = buffer;
          assert(first.buffer.byteLength === 0 && second.buffer.byteLength === 0, 'buffer setter synchronously acquires every channel');
          const next = buffer.getChannelData(0);
          assert(next !== first && next.length === 8 && next[0] === 7 && next === buffer.getChannelData(0), 'new stable channel view preserves content');
          next[0] = 9; node.normalize = false; node.buffer = buffer;
          assert(next.buffer.byteLength === 0 && buffer.getChannelData(0)[0] === 9 && node.normalize === false, 'reassignment acquires latest content with current normalization');
        });
        await check(label + '/ConvolverNode/boolean normalize does not run author conversion', () => {
          const node = context.createConvolver(); let reads = 0;
          const object = {valueOf() { reads++; throw Error('valueOf'); }, toString() { reads++; throw Error('toString'); }};
          for (const value of [false, 0, -0, '', null, undefined, NaN]) { node.normalize = value; assert(node.normalize === false, 'falsy normalization'); }
          for (const value of [true, 1, 'false', Symbol(), 1n, object]) { node.normalize = value; assert(node.normalize === true, 'truthy normalization'); }
          assert(reads === 0 && new w.ConvolverNode(context, {disableNormalization: object}).normalize === false, 'ToBoolean has no author hooks');
        });
        await check(label + '/ConvolverNode/fractional offline context sample-rate', async () => {
          for (const sampleRate of [NaN, Infinity, -Infinity, 1e100, Symbol(), 1n]) assert(thrown(() => new w.OfflineAudioContext(1, 16, sampleRate)) instanceof w.TypeError, 'context sample rate uses restricted float');
          const offline = new w.OfflineAudioContext(1, 16, 44100.1), rate = Math.fround(44100.1);
          assert(offline.sampleRate === rate, 'context preserves converted float sample rate');
          const buffer = offline.createBuffer(1, 4, rate), node = new w.ConvolverNode(offline, {buffer});
          assert(buffer.sampleRate === rate && node.buffer === buffer, 'matching fractional native buffer accepted');
          Object.defineProperty(offline, 'sampleRate', {get() { throw Error('public context metadata'); }});
          const factory = offline.createConvolver(); factory.buffer = buffer;
          assert(factory.buffer === buffer, 'sample-rate validation uses native context metadata');
          const rendered = await offline.startRendering();
          assert(rendered instanceof w.AudioBuffer && rendered.sampleRate === rate && rendered.getChannelData(0).every(value => value === 0), 'silent rendered buffer preserves native float sample rate');
        });
        await check(label + '/ConvolverNode/cross-realm factory and graph', () => {
          const foreign = new window.AudioContext();
          try {
            const method = w.BaseAudioContext.prototype.createConvolver;
            assert(method.name === 'createConvolver' && method.length === 0, 'factory metadata');
            const node = method.call(foreign, {toString() { throw Error('ignored argument'); }});
            assert(node instanceof window.ConvolverNode && node.context === foreign, 'factory result belongs to context realm');
            const buffer = new w.AudioBuffer({length: 8, sampleRate: foreign.sampleRate}); node.buffer = buffer;
            assert(node.buffer === buffer, 'native cross-realm buffer');
            assert(node.connect(foreign.destination) === foreign.destination, 'native graph connection'); node.disconnect();
            assert(thrown(() => node.connect(context.destination)).name === 'InvalidAccessError', 'cross-context graph rejection');
            const setter = Object.getOwnPropertyDescriptor(w.ConvolverNode.prototype, 'buffer').set;
            assert(thrown(() => setter.call(node, {})) instanceof w.TypeError, 'callee realm TypeError');
            assert(thrown(() => setter.call(node, make(3))) instanceof w.DOMException, 'callee realm DOMException');
            assert(thrown(() => method.call(new Proxy(foreign, {}))) instanceof w.TypeError, 'factory native receiver');
          } finally { foreign.close(); }
        });
        for (const key of ['buffer', 'normalize']) await check(label + '/ConvolverNode/' + key + '/native receivers', () => {
          const descriptor = Object.getOwnPropertyDescriptor(w.ConvolverNode.prototype, key), real = context.createConvolver();
          assert(descriptor.enumerable && descriptor.configurable, 'member descriptor');
          let conversions = 0, traps = 0; const poison = {valueOf() { conversions++; throw Error('conversion'); }}, trap = () => { traps++; throw Error('proxy trap'); };
          const revoked = Proxy.revocable(real, {}); revoked.revoke();
          for (const kind of ['get', 'set']) {
            assert(descriptor[kind].name === kind + ' ' + key && descriptor[kind].length === (kind === 'get' ? 0 : 1), 'accessor metadata');
            for (const receiver of [null, {}, w.ConvolverNode.prototype, Object.create(real), new Proxy(real, {get: trap, getPrototypeOf: trap}), revoked.proxy]) assert(thrown(() => descriptor[kind].call(receiver, poison)) instanceof w.TypeError, 'native receiver in callee realm');
          }
          assert(conversions === 0 && traps === 0, 'receiver check before author code');
        });
      } finally { await context.close(); }
    }
  } finally { popup.close(); }
  globalThis.__nodeReplacementResults = {rows, passed: rows.filter(row => row.pass).length, total: rows.length, failures: rows.filter(row => !row.pass)};
  return rows.every(row => row.pass);
})()
