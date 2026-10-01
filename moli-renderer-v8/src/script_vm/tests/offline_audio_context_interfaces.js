(async () => {
  const rows = [];
  const assert = (ok, message) => { if (!ok) throw Error(message); };
  const thrown = run => { try { run(); } catch (error) { return error; } throw Error('expected exception'); };
  const check = async (name, run) => {
    try { await run(); rows.push({name, pass: true}); }
    catch (error) { rows.push({name, pass: false, message: String(error)}); }
  };
  const popup = open();
  try {
    for (const [label, w] of [['main', window], ['child', document.getElementById('child').contentWindow], ['popup', popup]]) {
      const C = w.OfflineAudioContext;
      const options = {length: 16, sampleRate: 48000};
      await check(label + '/interface and new.target', () => {
        assert(C.name === 'OfflineAudioContext' && C.length === 1, 'shortest constructor overload');
        const value = new C(options);
        assert(value instanceof C && value instanceof w.BaseAudioContext && value instanceof w.EventTarget, 'native inheritance');
        assert(Object.prototype.toString.call(value) === '[object OfflineAudioContext]', 'native tag');
        class Sub extends C {};
        assert(new Sub(options) instanceof Sub, 'subclass receiver');
        assert(thrown(() => C(options)) instanceof w.TypeError, 'new required');
      });
      await check(label + '/overload arity before conversion', () => {
        let reads = 0;
        const poison = new Proxy({}, {get() { reads++; throw Error('read'); }});
        assert(thrown(() => new C()) instanceof w.TypeError, 'zero arguments');
        assert(thrown(() => new C(poison, poison)) instanceof w.TypeError, 'two arguments');
        assert(thrown(() => C(poison)) instanceof w.TypeError, 'call without new');
        assert(reads === 0, 'no property access for invalid arity or call');
        const first = {valueOf() { reads++; return 2; }, get length() { throw Error('dictionary branch'); }};
        const extra = {valueOf() { throw Error('extra conversion'); }};
        const value = new C(first, 17, 48000, extra);
        assert(value.length === 17 && value.destination.channelCount === 2 && reads === 1, 'three or more arguments select positional overload');
      });
      await check(label + '/dictionary type and required members', () => {
        for (const value of [undefined, null, {}, 1, true, 'x', Symbol(), 1n, {length: 16}, {sampleRate: 48000}])
          assert(thrown(() => new C(value)) instanceof w.TypeError, 'dictionary and required fields');
        const value = new C(Object.create(options));
        assert(value.length === 16 && value.sampleRate === 48000 && value.destination.channelCount === 1, 'inherited options and default channels');
        const callable = () => {};
        Object.defineProperty(callable, 'length', {value: 16});
        callable.sampleRate = 48000;
        assert(new C(callable).length === 16, 'callable dictionary');
      });
      await check(label + '/dictionary conversion order', () => {
        const order = [], value = Object.create(null);
        for (const [key, number] of [['length', 16], ['numberOfChannels', 2], ['renderSizeHint', 129], ['sampleRate', 48000]]) {
          Object.defineProperty(value, key, {get() {
            order.push(key);
            return key === 'renderSizeHint' ? number : {valueOf() { order.push(key + '/number'); return number; }};
          }});
        }
        const context = new C(value);
        assert(context.length === 16 && context.destination.channelCount === 2 && context.renderQuantumSize === 129, 'converted metadata');
        assert(order.join() === 'length,length/number,numberOfChannels,numberOfChannels/number,renderSizeHint,sampleRate,sampleRate/number', 'lexical order with immediate conversion');
        const sentinel = {};
        assert(thrown(() => new C({length: 0, get sampleRate() { throw sentinel; }})) === sentinel, 'conversion before format validation');
        assert(thrown(() => new C({get length() { throw sentinel; }})) === sentinel, 'original getter exception');
        const revoked = Proxy.revocable(options, {}); revoked.revoke();
        assert(thrown(() => new C(revoked.proxy)) instanceof w.TypeError, 'revoked dictionary proxy');
      });
      await check(label + '/dictionary proxy Get semantics', () => {
        const reads = [];
        const value = new Proxy(options, {
          get(target, key) { reads.push(key); return Reflect.get(target, key); },
          ownKeys() { throw Error('enumeration'); }, has() { throw Error('has'); }
        });
        assert(new C(value).length === 16, 'proxy dictionary');
        assert(reads.join() === 'length,numberOfChannels,renderSizeHint,sampleRate', 'Get only, once per field');
      });
      await check(label + '/positional conversion order and exceptions', () => {
        const order = [], number = (key, value) => ({valueOf() { order.push(key); return value; }});
        const context = new C(number('channels', 2), number('length', 18), number('sampleRate', 48000));
        assert(context.length === 18 && context.destination.channelCount === 2 && order.join() === 'channels,length,sampleRate', 'left to right');
        const sentinel = {};
        assert(thrown(() => new C(0, 0, {valueOf() { throw sentinel; }})) === sentinel, 'all conversion before format validation');
        let reads = 0;
        assert(thrown(() => new C(Symbol(), {valueOf() { reads++; return 16; }}, 48000)) instanceof w.TypeError && reads === 0, 'stop at failed conversion');
      });
      await check(label + '/unsigned long wrapping and truncation', () => {
        for (const [channels, length, expectedChannels, expectedLength] of [[1.9, 16.9, 1, 16], [4294967297, 4294967313, 1, 17], [-4294967295, -4294967278, 1, 18], ['2', '19', 2, 19], [true, true, 1, 1]]) {
          for (const context of [new C(channels, length, 48000), new C({numberOfChannels: channels, length, sampleRate: 48000})])
            assert(context.destination.channelCount === expectedChannels && context.length === expectedLength, 'IDL unsigned long conversion');
        }
      });
      await check(label + '/format errors after integer conversion', () => {
        for (const channels of [0, -1, 33, 4294967296, NaN, Infinity, undefined, null]) {
          assert(thrown(() => new C(channels, 16, 48000)).name === 'NotSupportedError', 'invalid positional channels');
          if (channels !== undefined) assert(thrown(() => new C({...options, numberOfChannels: channels})).name === 'NotSupportedError', 'invalid dictionary channels');
        }
        for (const length of [0, .9, NaN, Infinity, null, 4294967296]) {
          assert(thrown(() => new C(1, length, 48000)).name === 'NotSupportedError', 'invalid positional length');
          assert(thrown(() => new C({...options, length})).name === 'NotSupportedError', 'invalid dictionary length');
        }
        assert(new C({...options, numberOfChannels: undefined}).destination.channelCount === 1, 'undefined applies default');
        for (const sampleRate of [0, -1, 2999, 768001])
          assert(thrown(() => new C(1, 16, sampleRate)).name === 'NotSupportedError', 'unsupported finite sample rate');
      });
      await check(label + '/restricted float and rounding', () => {
        for (const sampleRate of [NaN, Infinity, -Infinity, 1e100, undefined, 'x', Symbol(), 1n]) {
          assert(thrown(() => new C(1, 16, sampleRate)) instanceof w.TypeError, 'positional finite float');
          assert(thrown(() => new C({...options, sampleRate})) instanceof w.TypeError, 'dictionary finite float');
        }
        assert(new C(1, 16, 44100.1).sampleRate === Math.fround(44100.1), 'float rounding before storage');
      });
      await check(label + '/sample rate boundaries shared with AudioBuffer', () => {
        for (const rate of [3000, 8000, 192000, 768000]) {
          const context = new C({length: 1, numberOfChannels: 32, sampleRate: rate});
          const buffer = context.createBuffer(1, 1, rate);
          const direct = new w.AudioBuffer({length: 1, sampleRate: rate});
          assert(context.sampleRate === rate && context.destination.channelCount === 32 && buffer.sampleRate === rate && direct.sampleRate === rate, 'common supported format');
        }
      });
      await check(label + '/render size default and enum conversion', () => {
        for (const hint of [undefined, 'default', 'hardware'])
          assert(new C({...options, renderSizeHint: hint}).renderQuantumSize === 128, 'default quantum');
        const conversions = [];
        const hint = {[Symbol.toPrimitive](kind) { conversions.push(kind); return 'hardware'; }};
        assert(new C({...options, renderSizeHint: hint}).renderQuantumSize === 128 && conversions.join() === 'string', 'object uses enum string conversion');
        const sentinel = {};
        assert(thrown(() => new C({...options, renderSizeHint: {toString() { throw sentinel; }}})) === sentinel, 'original enum conversion exception');
        for (const hint of [null, true, '128', 'bogus', new w.Number(128), Symbol(), 128n])
          assert(thrown(() => new C({...options, renderSizeHint: hint})) instanceof w.TypeError, 'non-number branch must be enum');
      });
      await check(label + '/render size numeric range and integer conversion', () => {
        for (const [sampleRate, hint, expected] of [[48000, 1, 1], [48000, 13, 13], [48000, 127, 127], [48000, 129.9, 129], [48000, 288000, 288000], [3000, 18000, 18000], [44100.1, 264600, 264600], [48000, 4294967425, 129]])
          assert(new C({...options, sampleRate, renderSizeHint: hint}).renderQuantumSize === expected, 'supported quantum is honored');
        for (const [sampleRate, hint] of [[48000, 0], [48000, -1], [48000, NaN], [48000, Infinity], [48000, 288001], [3000, 18001]])
          assert(thrown(() => new C({...options, sampleRate, renderSizeHint: hint})).name === 'NotSupportedError', 'converted quantum bounds');
      });
      await check(label + '/render quantum native getter and receiver realm', () => {
        const get = Object.getOwnPropertyDescriptor(w.BaseAudioContext.prototype, 'renderQuantumSize').get;
        const context = new C({...options, renderSizeHint: 129});
        assert(get.length === 0 && get.call(context) === 129, 'prototype accessor');
        let traps = 0;
        for (const receiver of [null, {}, Object.create(context), new Proxy(context, {get() { traps++; throw Error('trap'); }})])
          assert(thrown(() => get.call(receiver)) instanceof w.TypeError, 'native brand and callee error realm');
        assert(traps === 0, 'no author proxy traps');
        const foreign = new OfflineAudioContext(options);
        assert(get.call(foreign) === 128, 'genuine foreign context');
        context.__moliAudioRenderQuantumSize = 7;
        context.renderQuantumSize = 9;
        assert(context.renderQuantumSize === 129, 'private state and readonly attribute');
      });
      await check(label + '/rendered buffer uses converted private format', async () => {
        const context = new C({length: 4294967313, numberOfChannels: 4294967298, sampleRate: 44100.1, renderSizeHint: 13});
        context.__moliOfflineAudioLength = 99;
        context.__moliOfflineAudioSampleRate = 123;
        context.__moliOfflineAudioChannelCount = 9;
        const buffer = await context.startRendering();
        assert(buffer.length === 17 && buffer.numberOfChannels === 2 && buffer.sampleRate === Math.fround(44100.1), 'native format snapshot');
        assert(buffer instanceof w.AudioBuffer && buffer.getChannelData(0).every(value => value === 0), 'empty graph silence in context realm');
      });
    }
  } finally { popup.close(); }
  globalThis.__nodeReplacementResults = {total: rows.length, passed: rows.filter(row => row.pass).length, failures: rows.filter(row => !row.pass), rows};
  return rows.every(row => row.pass);
})()
