(async () => {
  const rows = [];
  const assert = (condition, message) => { if (!condition) throw Error(message); };
  const check = async (name, callback) => {
    try { await callback(); rows.push({name, pass: true}); }
    catch (error) { rows.push({name, pass: false, message: String(error)}); }
  };
  const thrown = callback => {
    try { callback(); } catch (error) { return error; }
    throw Error('expected exception');
  };
  const exception = (callback, name, w) => {
    const error = thrown(callback);
    assert(error instanceof w.DOMException && error.name === name, 'expected ' + name + ', got ' + error);
  };
  const popup = open();
  const realms = [['main', window], ['child', document.getElementById('child').contentWindow], ['popup', popup]];
  try {
    for (const [label, w] of realms) {
      const context = new w.AudioContext();
      try {
        for (const [name, factory, member, merger] of [
          ['ChannelMergerNode', 'createChannelMerger', 'numberOfInputs', true],
          ['ChannelSplitterNode', 'createChannelSplitter', 'numberOfOutputs', false]
        ]) {
          const C = w[name];
          await check(label + '/' + name + '/interface', () => {
            const descriptor = Object.getOwnPropertyDescriptor(w, name);
            assert(typeof C === 'function' && C.name === name && C.length === 1, 'constructor name and length');
            assert(descriptor.writable && descriptor.configurable && !descriptor.enumerable, 'global descriptor');
            assert(Object.getPrototypeOf(C) === w.AudioNode && Object.getPrototypeOf(C.prototype) === w.AudioNode.prototype, 'native inheritance');
            assert(C.prototype.constructor === C && thrown(() => C(context)) instanceof w.TypeError, 'requires new');
            const method = Object.getOwnPropertyDescriptor(w.BaseAudioContext.prototype, factory);
            assert(method.value.name === factory && method.value.length === 0 && method.enumerable && method.configurable && method.writable, 'shared factory descriptor');
          });
          await check(label + '/' + name + '/defaults and factories', () => {
            for (const node of [new C(context), new C(context, null), context[factory](), context[factory](undefined)]) {
              assert(Object.getPrototypeOf(node) === C.prototype && node instanceof w.AudioNode && node instanceof w.EventTarget, 'native identity');
              assert(node.context === context && node.numberOfInputs === (merger ? 6 : 1) && node.numberOfOutputs === (merger ? 1 : 6), 'port defaults');
              assert(node.channelCount === (merger ? 1 : 6) && node.channelCountMode === 'explicit' && node.channelInterpretation === (merger ? 'speakers' : 'discrete'), 'channel defaults');
              assert(!Object.hasOwn(node, member) && Reflect.set(node, member, 8) === false, 'readonly inherited port count');
              let calls = 0;
              node.addEventListener('probe', event => { assert(event.target === node, 'event target identity'); calls++; });
              node.dispatchEvent(new w.Event('probe')); assert(calls === 1, 'EventTarget listener storage');
            }
            class Sub extends C {}
            const sub = new Sub(context, {[member]: 3});
            assert(sub instanceof Sub && sub instanceof C && sub[member] === 3, 'subclass prototype and state');
          });
          await check(label + '/' + name + '/port conversions and bounds', () => {
            for (const value of [1, 3, 32, 3.75, '3', 4294967299]) {
              const expected = value === 32 ? 32 : value === 1 ? 1 : 3;
              assert(new C(context, {[member]: value})[member] === expected && context[factory](value)[member] === expected, 'unsigned long conversion');
            }
            for (const value of [0, 33, -1, NaN, Infinity, null]) {
              exception(() => new C(context, {[member]: value}), 'IndexSizeError', w);
              exception(() => context[factory](value), 'IndexSizeError', w);
            }
            for (const value of [Symbol(), 1n]) {
              assert(thrown(() => new C(context, {[member]: value})) instanceof w.TypeError, 'port option type error');
              assert(thrown(() => context[factory](value)) instanceof w.TypeError, 'factory conversion type error');
            }
            assert(new C(context, {[member]: undefined})[member] === 6, 'undefined dictionary member uses default');
          });
          await check(label + '/' + name + '/dictionary and context brands', () => {
            let reads = 0, traps = 0;
            const options = new Proxy({}, {get() { reads++; throw Error('options read'); }});
            const revoked = Proxy.revocable(context, {}); revoked.revoke();
            for (const bad of [undefined, null, {}, Object.create(context), new Proxy(context, {get() { traps++; }}), revoked.proxy]) {
              assert(thrown(() => new C(bad, options)) instanceof w.TypeError, 'native BaseAudioContext required');
            }
            assert(reads === 0 && traps === 0, 'context brand precedes author hooks');
            for (const bad of [1, 'options', true, Symbol()]) assert(thrown(() => new C(context, bad)) instanceof w.TypeError, 'dictionary requires object');
            assert(new C(context, Object.create({[member]: 3}))[member] === 3, 'inherited dictionary member');
          });
          await check(label + '/' + name + '/conversion order', () => {
            const order = [], options = Object.create(null);
            const entries = [['channelCount', merger ? 1 : 3], ['channelCountMode', 'explicit'], ['channelInterpretation', 'discrete'], [member, 3]];
            for (const [key, value] of entries) Object.defineProperty(options, key, {get() { order.push(key); return value; }});
            const node = new C(context, options);
            assert(order.join() === entries.map(entry => entry[0]).join() && node[member] === 3, 'inherited members then derived member');
            const sentinel = {};
            assert(thrown(() => new C(context, {channelCount: 0, [member]: {valueOf() { throw sentinel; }}})) === sentinel, 'late conversion before channel constraints');
            assert(thrown(() => new C(context, new Proxy({}, {get() { throw sentinel; }}))) === sentinel, 'preserve getter exception');
            exception(() => new C(context, {channelCount: 0, [member]: 0}), 'IndexSizeError', w);
            let lateReads = 0;
            assert(thrown(() => new C(context, {channelCountMode: 'invalid', get [member]() { lateReads++; return 3; }})) instanceof w.TypeError && lateReads === 0, 'bad inherited enum aborts derived conversion');
          });
          await check(label + '/' + name + '/fixed channel count and mode', () => {
            const node = new C(context, {[member]: 3}), count = merger ? 1 : 3;
            node.channelCount = count; node.channelCountMode = 'explicit';
            for (const value of [0, 2, 33, -1, NaN]) {
              exception(() => { node.channelCount = value; }, 'InvalidStateError', w);
              exception(() => new C(context, {[member]: 3, channelCount: value}), 'InvalidStateError', w);
              assert(node.channelCount === count, 'failed count setter leaves state unchanged');
            }
            for (const value of ['max', 'clamped-max']) {
              exception(() => { node.channelCountMode = value; }, 'InvalidStateError', w);
              exception(() => new C(context, {channelCountMode: value}), 'InvalidStateError', w);
              assert(node.channelCountMode === 'explicit', 'failed mode setter leaves state unchanged');
            }
            node.channelCount = count + 4294967296;
            assert(node.channelCount === count, 'setter unsigned long wrap');
          });
          await check(label + '/' + name + '/interpretation and enum rules', () => {
            const node = context[factory](3);
            node.channelCountMode = 'invalid'; node.channelInterpretation = 'invalid';
            assert(node.channelCountMode === 'explicit' && node.channelInterpretation === (merger ? 'speakers' : 'discrete'), 'unknown attribute tokens ignored');
            assert(thrown(() => new C(context, {channelInterpretation: 'invalid'})) instanceof w.TypeError, 'constructor enum rejects unknown token');
            node.channelInterpretation = 'discrete';
            if (merger) { node.channelInterpretation = 'speakers'; assert(node.channelInterpretation === 'speakers', 'merger interpretation remains mutable'); }
            else {
              exception(() => { node.channelInterpretation = 'speakers'; }, 'InvalidStateError', w);
              exception(() => new C(context, {channelInterpretation: 'speakers'}), 'InvalidStateError', w);
              assert(node.channelInterpretation === 'discrete', 'splitter interpretation is fixed');
            }
            const sentinel = {};
            assert(thrown(() => { node.channelInterpretation = {toString() { throw sentinel; }}; }) === sentinel, 'preserve setter string conversion exception');
          });
          await check(label + '/' + name + '/receiver checks and intrinsic factory', () => {
            const node = context[factory](), method = w.BaseAudioContext.prototype[factory];
            let traps = 0, conversions = 0;
            const trap = () => { traps++; throw Error('trap'); }, value = {valueOf() { conversions++; throw Error('conversion'); }};
            for (const bad of [null, {}, Object.create(context), new Proxy(context, {get: trap})]) assert(thrown(() => method.call(bad, value)) instanceof w.TypeError, 'factory receiver');
            const setter = Object.getOwnPropertyDescriptor(w.AudioNode.prototype, 'channelCount').set;
            const revoked = Proxy.revocable(node, {}); revoked.revoke();
            for (const bad of [null, {}, Object.create(node), new Proxy(node, {get: trap}), revoked.proxy]) assert(thrown(() => setter.call(bad, value)) instanceof w.TypeError, 'node receiver');
            assert(traps === 0 && conversions === 0, 'receiver before conversion');
            const prototype = C.prototype;
            try { w[name] = function() { throw Error('author constructor'); }; assert(Object.getPrototypeOf(method.call(context, 3)) === prototype, 'factory uses intrinsic prototype'); }
            finally { w[name] = C; }
            Object.setPrototypeOf(node, null);
            assert(Object.getOwnPropertyDescriptor(w.AudioNode.prototype, 'context').get.call(node) === context, 'native identity independent of prototype');
          });
          await check(label + '/' + name + '/cross-realm construction and errors', () => {
            const foreign = new window.AudioContext();
            try {
              const node = new C(foreign, {[member]: 3});
              assert(Object.getPrototypeOf(node) === C.prototype && node.context === foreign, 'foreign genuine context');
              const made = w.BaseAudioContext.prototype[factory].call(foreign, 3);
              assert(Object.getPrototypeOf(made) === window[name].prototype && made.context === foreign, 'factory allocation in context relevant realm');
              assert(thrown(() => w.BaseAudioContext.prototype[factory].call(foreign, Symbol())) instanceof w.TypeError, 'conversion error in callee realm');
              exception(() => w.BaseAudioContext.prototype[factory].call(foreign, 0), 'IndexSizeError', w);
            } finally { foreign.close(); }
          });
        }
        await check(label + '/graph/distinct ports and duplicate edges', () => {
          const source = context.createChannelSplitter(3), target = context.createChannelMerger(3);
          for (const [output, input] of [[0, 0], [0, 1], [1, 0], [2, 2], [0, 0]]) assert(source.connect(target, output, input) === target, 'connect returns target');
          source.disconnect(target, 0, 0);
          exception(() => source.disconnect(target, 0, 0), 'InvalidAccessError', w);
          source.disconnect(target, 0); source.disconnect(target, 1, 0); source.disconnect(target, 2, 2);
          exception(() => source.disconnect(target), 'InvalidAccessError', w);
        });
        await check(label + '/graph/output and destination selectors', () => {
          const source = context.createChannelSplitter(3), a = context.createChannelMerger(3), b = context.createChannelMerger(3);
          source.connect(a, 1, 0); source.connect(a, 2, 1); source.connect(b, 1, 2); source.connect(b, 2, 0);
          source.disconnect({valueOf() { return 1; }});
          exception(() => source.disconnect(a, 1), 'InvalidAccessError', w);
          exception(() => source.disconnect(b, 1), 'InvalidAccessError', w);
          source.disconnect(a); source.disconnect(b, 2, 0);
          source.disconnect(1); source.disconnect();
          assert(thrown(() => source.disconnect({valueOf() { return 0; }}, 0)) instanceof w.TypeError, 'two-argument overload requires interface');
        });
        await check(label + '/graph/port validation and conversion order', () => {
          const source = context.createChannelSplitter(3), target = context.createChannelMerger(3), sentinel = {}, order = [];
          const output = {valueOf() { order.push('output'); return 99; }}, input = {valueOf() { order.push('input'); throw sentinel; }};
          assert(thrown(() => source.connect(target, output, input)) === sentinel && order.join() === 'output,input', 'connect converts all ports before range checks');
          order.length = 0;
          assert(thrown(() => source.disconnect(target, output, input)) === sentinel && order.join() === 'output,input', 'disconnect converts all ports before range and edge checks');
          for (const fn of [() => source.connect(target, 3, 0), () => source.connect(target, 0, 3), () => source.disconnect(3), () => source.disconnect(target, 3), () => source.disconnect(target, 0, 3)]) exception(fn, 'IndexSizeError', w);
          assert(thrown(() => source.connect(target, Symbol())) instanceof w.TypeError && thrown(() => source.disconnect(1n)) instanceof w.TypeError, 'port type errors');
          source.connect(target, 4294967298, 4294967298); source.disconnect(target, 2, 2);
          Object.defineProperty(source, 'numberOfOutputs', {value: 99}); Object.defineProperty(target, 'numberOfInputs', {value: 99});
          exception(() => source.connect(target, 3, 0), 'IndexSizeError', w);
          exception(() => source.connect(target, 0, 3), 'IndexSizeError', w);
        });
        await check(label + '/graph/cross-context and zero ports', () => {
          const other = new w.AudioContext(), node = context.createChannelSplitter(3), source = context.createOscillator();
          try {
            exception(() => node.connect(other.destination), 'InvalidAccessError', w);
            exception(() => node.connect(other.destination, 3), 'IndexSizeError', w);
            exception(() => node.connect(source), 'IndexSizeError', w);
            exception(() => context.destination.connect(node), 'IndexSizeError', w);
            exception(() => context.destination.disconnect(0), 'IndexSizeError', w);
          } finally { other.close(); }
        });
      } finally { await context.close(); }
    }
  } finally { popup.close(); }
  globalThis.__nodeReplacementResults = {rows, passed: rows.filter(row => row.pass).length, total: rows.length, failures: rows.filter(row => !row.pass)};
  return rows.every(row => row.pass);
})()
