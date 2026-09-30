(async () => {
  const rows = [], rejectedModeStates = [];
  const assert = (condition, message) => { if (!condition) throw Error(message); };
  const check = async (name, callback) => {
    try { await callback(); rows.push({name, pass: true}); }
    catch (error) { rows.push({name, pass: false, message: String(error)}); }
  };
  const thrown = callback => {
    try { callback(); } catch (error) { return error; }
    throw Error('expected exception');
  };
  const popup = open();
  const realms = [['main', window], ['child', document.getElementById('child').contentWindow], ['popup', popup]];
  try {
    for (const [label, w] of realms) {
      const context = new w.AudioContext();
      const types = [
        ['GainNode', 'createGain', 'gain', 1, -Math.fround(3.4028234663852886e38), Math.fround(3.4028234663852886e38), 'max'],
        ['DelayNode', 'createDelay', 'delayTime', 0, 0, 1, 'max'],
        ['StereoPannerNode', 'createStereoPanner', 'pan', 0, -1, 1, 'clamped-max']
      ];
      try {
        for (const [name, factory, key, defaultValue, min, max, mode] of types) {
          await check(label + '/' + name + '/interface', () => {
            const C = w[name], d = Object.getOwnPropertyDescriptor(w, name);
            assert(typeof C === 'function' && C.name === name && C.length === 1, 'constructor name and length');
            assert(d.writable && d.configurable && !d.enumerable, 'global descriptor');
            assert(Object.getPrototypeOf(C) === w.AudioNode && Object.getPrototypeOf(C.prototype) === w.AudioNode.prototype, 'native inheritance');
            assert(C.prototype.constructor === C && thrown(() => C(context)) instanceof w.TypeError, 'requires new');
            const getter = Object.getOwnPropertyDescriptor(C.prototype, key);
            assert(getter.enumerable && getter.configurable && getter.set === undefined && getter.get.name === 'get ' + key && getter.get.length === 0, 'readonly shared accessor');
          });
          await check(label + '/' + name + '/defaults and factory', () => {
            const C = w[name], nodes = [new C(context), new C(context, null), context[factory]()];
            const method = Object.getOwnPropertyDescriptor(w.BaseAudioContext.prototype, factory);
            assert(method.value.name === factory && method.value.length === 0 && method.enumerable && method.configurable && method.writable, 'native factory descriptor');
            for (const node of nodes) {
              assert(Object.getPrototypeOf(node) === C.prototype && node instanceof w.AudioNode && node instanceof w.EventTarget, 'node identity');
              assert(node.context === context && node.numberOfInputs === 1 && node.numberOfOutputs === 1 && node.channelCount === 2 && node.channelCountMode === mode && node.channelInterpretation === 'speakers', 'node defaults');
              const param = node[key];
              assert(!Object.hasOwn(node, key) && param === node[key] && param instanceof w.AudioParam, 'stable private AudioParam');
              assert(param.value === defaultValue && param.defaultValue === defaultValue && param.minValue === min && param.maxValue === max && param.automationRate === 'a-rate', 'AudioParam defaults and bounds');
              assert(Reflect.set(node, key, {}) === false && node[key] === param, 'readonly payload');
              param.automationRate = 'k-rate'; assert(param.automationRate === 'k-rate', 'variable automation rate');
              node.addEventListener('probe', event => assert(event.target === node, 'EventTarget identity'));
              assert(node.dispatchEvent(new w.Event('probe')), 'native event dispatch');
              assert(node.connect(context.destination) === context.destination && node.disconnect() === undefined, 'common graph methods');
            }
          });
          await check(label + '/' + name + '/context and dictionary conversion', () => {
            const C = w[name]; let reads = 0, traps = 0;
            const options = new Proxy({}, {get() { reads++; throw Error('options read'); }});
            const revoked = Proxy.revocable(context, {}); revoked.revoke();
            for (const bad of [undefined, null, {}, Object.create(context), new Proxy(context, {get() { traps++; }}), revoked.proxy]) {
              assert(thrown(() => new C(bad, options)) instanceof w.TypeError, 'reject forged context');
            }
            assert(reads === 0 && traps === 0, 'brand precedes author hooks');
            for (const bad of [1, 'options', Symbol(), true]) assert(thrown(() => new C(context, bad)) instanceof w.TypeError, 'dictionary object required');
            class Sub extends C {};
            const sub = new Sub(context); assert(sub instanceof Sub && sub instanceof C && sub.context === context, 'subclass prototype');
          });
          await check(label + '/' + name + '/options order and values', () => {
            const order = [], options = Object.create(null);
            const entries = [['channelCount', 1], ['channelCountMode', 'explicit'], ['channelInterpretation', 'discrete'], [key, 0.3]];
            if (name === 'DelayNode') entries.push(['maxDelayTime', 1.5]);
            for (const [key, value] of entries) Object.defineProperty(options, key, {get() { order.push(key); return value; }});
            const node = new w[name](context, options);
            assert(order.join() === entries.map(entry => entry[0]).join(), 'inherited then lexical members');
            assert(node.channelCount === 1 && node.channelCountMode === 'explicit' && node.channelInterpretation === 'discrete', 'inherited options applied');
            assert(node[key].value === Math.fround(0.3) && node[key].defaultValue === defaultValue, 'initial value keeps default metadata');
            const sentinel = {}; order.length = 0;
            Object.defineProperty(options, 'unused', {get() { throw Error('unused member'); }});
            const inherited = Object.create({[key]: 0.25});
            assert(new w[name](context, inherited)[key].value === 0.25, 'inherited dictionary members');
            assert(thrown(() => new w[name](context, new Proxy({}, {get() { throw sentinel; }}))) === sentinel, 'preserve getter exception');
            const late = {channelCount: 0, [key]: {valueOf() { order.push(key); throw sentinel; }}};
            assert(thrown(() => new w[name](context, late)) === sentinel && order.join() === key, 'all conversions before channel constraints');
          });
          await check(label + '/' + name + '/channel constraints', () => {
            const C = w[name], node = context[factory]();
            for (const value of [0, 33, -1, NaN]) {
              assert(thrown(() => new C(context, {channelCount: value})).name === 'NotSupportedError', 'constructor count range');
              assert(thrown(() => { node.channelCount = value; }).name === 'NotSupportedError' && node.channelCount === 2, 'setter count range');
            }
            assert(thrown(() => new C(context, {channelCountMode: 'wrong'})) instanceof w.TypeError, 'constructor enum must throw');
            assert(thrown(() => new C(context, {channelInterpretation: 'wrong'})) instanceof w.TypeError, 'interpretation enum must throw');
            node.channelCountMode = 'wrong'; node.channelInterpretation = 'wrong';
            assert(node.channelCountMode === mode && node.channelInterpretation === 'speakers', 'invalid attribute enums ignored');
            node.channelCount = 1; node.channelCountMode = 'explicit'; node.channelInterpretation = 'discrete';
            assert(node.channelCount === 1 && node.channelCountMode === 'explicit' && node.channelInterpretation === 'discrete', 'common setters');
            if (name === 'StereoPannerNode') {
              assert(thrown(() => new C(context, {channelCount: 3})).name === 'NotSupportedError', 'stereo constructor limit');
              assert(thrown(() => { node.channelCount = 3; }).name === 'NotSupportedError', 'stereo setter limit');
              assert(thrown(() => new C(context, {channelCountMode: 'max'})).name === 'NotSupportedError', 'stereo constructor mode');
              assert(thrown(() => { node.channelCountMode = 'max'; }).name === 'NotSupportedError' && node.channelCountMode !== 'max', 'stereo setter rejects max mode');
              rejectedModeStates.push({realm: label, after: node.channelCountMode});
            }
          });
          await check(label + '/' + name + '/parameter conversion', () => {
            const C = w[name];
            for (const value of [NaN, Infinity, -Infinity, Symbol(), 1n]) assert(thrown(() => new C(context, {[key]: value})) instanceof w.TypeError, 'restricted numeric option');
            if (name === 'DelayNode') {
              assert(new C(context, {delayTime: 1e100, maxDelayTime: 1.5}).delayTime.value === 1.5, 'restricted double may exceed float range before clamp');
              assert(new C(context, {delayTime: -2}).delayTime.value === 0, 'delay clamps to nominal range');
            } else {
              assert(thrown(() => new C(context, {[key]: 1e100})) instanceof w.TypeError, 'float overflow must reject');
              const positive = new C(context, {[key]: 2})[key], negative = new C(context, {[key]: -2})[key];
              assert(positive.value === (name === 'StereoPannerNode' ? 1 : 2) && negative.value === (name === 'StereoPannerNode' ? -1 : -2), 'initial parameter clamping');
            }
          });
          await check(label + '/' + name + '/getter and factory receivers', () => {
            const node = context[factory](), getter = Object.getOwnPropertyDescriptor(w[name].prototype, key).get;
            let traps = 0, conversions = 0;
            const trap = () => { traps++; throw Error('trap'); }, value = {valueOf() { conversions++; throw Error('conversion'); }};
            const revoked = Proxy.revocable(node, {}); revoked.revoke();
            for (const bad of [null, {}, w[name].prototype, Object.create(node), new Proxy(node, {get: trap, getPrototypeOf: trap}), revoked.proxy]) assert(thrown(() => getter.call(bad)) instanceof w.TypeError, 'getter native receiver');
            const method = w.BaseAudioContext.prototype[factory];
            for (const bad of [null, {}, w.BaseAudioContext.prototype, Object.create(context), new Proxy(context, {get: trap})]) assert(thrown(() => method.call(bad, value)) instanceof w.TypeError, 'factory native receiver');
            assert(traps === 0 && conversions === 0, 'receiver before author hooks');
            Object.setPrototypeOf(node, null); assert(getter.call(node) instanceof w.AudioParam, 'native identity survives prototype mutation');
          });
          await check(label + '/' + name + '/cross realm and constructor tampering', () => {
            const foreign = new window.AudioContext();
            try {
              const node = new w[name](foreign); assert(node.context === foreign && node[key] instanceof w.AudioParam, 'constructor realm parameter');
              const getter = Object.getOwnPropertyDescriptor(w[name].prototype, key).get, own = foreign[factory]();
              assert(getter.call(own) === own[key], 'borrowed getter accepts genuine native foreign node');
              const saved = w[name]; w[name] = function() { throw Error('author constructor'); };
              try { assert(context[factory]() instanceof saved, 'factory uses intrinsic constructor'); }
              finally { w[name] = saved; }
              assert(thrown(() => getter.call({})) instanceof w.TypeError, 'callee realm TypeError');
            } finally { foreign.close(); }
          });
        }
        await check(label + '/DelayNode/maximum delay conversion and range', () => {
          for (const value of [0, -1, 180, 181, null]) {
            assert(thrown(() => context.createDelay(value)).name === 'NotSupportedError', 'factory maximum range');
            assert(thrown(() => new w.DelayNode(context, {maxDelayTime: value})).name === 'NotSupportedError', 'constructor maximum range');
          }
          for (const value of [NaN, Infinity, -Infinity, Symbol(), 1n]) {
            assert(thrown(() => context.createDelay(value)) instanceof w.TypeError, 'factory restricted double');
            assert(thrown(() => new w.DelayNode(context, {maxDelayTime: value})) instanceof w.TypeError, 'constructor restricted double');
          }
          assert(context.createDelay(undefined).delayTime.maxValue === 1 && context.createDelay(0.3).delayTime.maxValue === Math.fround(0.3), 'factory default and float max');
        });
      } finally { await context.close(); }
    }
  } finally { popup.close(); }
  globalThis.__nodeReplacementResults = {rows, rejectedModeStates, passed: rows.filter(row => row.pass).length, total: rows.length, failures: rows.filter(row => !row.pass)};
  return rows.every(row => row.pass);
})()
