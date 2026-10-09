(async () => {
  'use strict';
  const checks = [];
  globalThis.__codecChecks = checks;
  const check = async (name, operation) => {
    globalThis.__codecCurrentCheck = name;
    let watchdog;
    try {
      await Promise.race([operation(), new Promise((_, reject) => {
        watchdog = setTimeout(() => reject(Error('check timed out: ' + name)), 3000);
      })]);
      checks.push({name, passed: true});
    } catch (error) {
      checks.push({name, passed: false, error: String(error), stack: error?.stack});
    } finally {clearTimeout(watchdog);}
  };
  const assert = (value, message) => { if (!value) throw Error(message); };
  const throws = (operation, constructor, name) => {
    let thrown;
    try { operation(); } catch (error) { thrown = error; }
    assert(thrown instanceof constructor && (!name || thrown.name === name), 'unexpected exception: ' + thrown);
    return thrown;
  };
  const rejects = async (promise, constructor, name) => {
    assert(promise instanceof Promise, 'callee Promise');
    let thrown;
    try { await promise; } catch (error) { thrown = error; }
    assert(thrown instanceof constructor && (!name || thrown.name === name), 'unexpected rejection: ' + thrown);
    return thrown;
  };
  const turn = () => new Promise(resolve => setTimeout(resolve, 20));
  for (const name of ['VideoDecoder', 'VideoEncoder']) {
    const C = globalThis[name], encoder = name === 'VideoEncoder';
    const size = encoder ? 'encodeQueueSize' : 'decodeQueueSize';
    const config = () => encoder ? {codec: 'moli.unsupported', width: 32, height: 24} : {codec: 'moli.unsupported'};
    const init = () => ({error() {}, output() {throw Error('shim emitted output');}});
    const label = suffix => name + ' ' + suffix;
    await check(label('constructor and EventTarget inheritance'), () => {
      assert(typeof C === 'function' && C.length === 1, 'constructor descriptor');
      assert(Object.getPrototypeOf(C.prototype) === EventTarget.prototype, 'inheritance');
      const codec = new C(init());
      assert(codec.state === 'unconfigured' && codec[size] === 0 && codec.ondequeue === null, 'initial slots');
      codec.close();
    });
    for (const [method, length] of [['configure', 1], [encoder ? 'encode' : 'decode', 1], ['flush', 0], ['reset', 0], ['close', 0]]) {
      await check(label(method + ' descriptor'), () => {
        const descriptor = Object.getOwnPropertyDescriptor(C.prototype, method);
        assert(typeof descriptor?.value === 'function' && descriptor.value.length === length && descriptor.enumerable && descriptor.writable && descriptor.configurable, 'method descriptor');
      });
    }
    await check(label('static support descriptor'), () => {
      const descriptor = Object.getOwnPropertyDescriptor(C, 'isConfigSupported');
      assert(descriptor.value.length === 1 && descriptor.enumerable && descriptor.writable && descriptor.configurable, 'support descriptor');
    });
    for (const bad of [undefined, null, {}, {error() {}}, {output() {}}, {error: {}, output() {}}, {error() {}, output: {}}]) {
      await check(label('invalid init ' + JSON.stringify(bad)), () => throws(() => new C(bad), TypeError));
    }
    await check(label('callback conversion order and newTarget'), () => {
      const log = [], options = {get error() {log.push('error'); return () => {}}, get output() {log.push('output'); return () => {}}};
      const target = new Proxy(function() {}, {get(object, key) { if (key === 'prototype') log.push('prototype'); return Reflect.get(object, key); }});
      const codec = Reflect.construct(C, [options], target);
      assert(log.join(',') === 'error,output,prototype', log.join(','));
      C.prototype.close.call(codec);
    });
    await check(label('getter exception preserved'), () => {
      const marker = {};
      let caught;
      try { new C({get error() {throw marker}, get output() {throw Error('read output')}}); } catch (error) {caught = error;}
      assert(caught === marker, 'original exception');
    });
    await check(label('subclass brand'), () => {
      class Derived extends C {}
      const codec = new Derived(init());
      assert(codec instanceof Derived && codec.state === 'unconfigured', 'subclass');
      codec.reset(); codec.close();
    });
    await check(label('readonly slots'), () => {
      const codec = new C(init());
      assert(!Reflect.set(codec, 'state', 'configured') && !Reflect.set(codec, size, 88), 'readonly');
      codec.close();
    });
    await check(label('event handler uses ordered listener list'), () => {
      const codec = new C(init()), log = [];
      codec.addEventListener('dequeue', () => log.push('first'));
      codec.ondequeue = function(event) {assert(this === codec && event.target === codec, 'handler context'); log.push('old')};
      codec.addEventListener('dequeue', () => log.push('last'));
      codec.ondequeue = () => log.push('replacement');
      codec.dispatchEvent(new Event('dequeue'));
      assert(log.join(',') === 'first,replacement,last', log.join(','));
      codec.ondequeue = 42; assert(codec.ondequeue === null, 'nonobject handler');
      codec.ondequeue = {}; assert(typeof codec.ondequeue === 'object', 'legacy noncallable object retained');
      codec.dispatchEvent(new Event('dequeue')); codec.close();
    });
    for (const receiverKind of ['plain', 'inherited', 'authorProxy', 'revoked']) {
      await check(label('receiver before conversion ' + receiverKind), async () => {
        const codec = new C(init());
        const revoked = Proxy.revocable(codec, {}); revoked.revoke();
        const receiver = {plain: {}, inherited: Object.create(codec), authorProxy: new Proxy(codec, {get() {throw Error('proxy trap')}}), revoked: revoked.proxy}[receiverKind];
        let reads = 0;
        throws(() => C.prototype.configure.call(receiver, {get codec() {reads++; return 'bogus'}}), TypeError);
        throws(() => Object.getOwnPropertyDescriptor(C.prototype, 'state').get.call(receiver), TypeError);
        await rejects(C.prototype.flush.call(receiver), TypeError);
        assert(reads === 0, 'conversion before receiver check'); codec.close();
      });
    }
    const invalid = encoder ? [{}, {codec: ''}, {...config(), width: 0}, {...config(), height: 0}, {...config(), displayWidth: 0}, {...config(), displayHeight: 0}, {...config(), width: -1}, {...config(), width: Infinity}, {...config(), framerate: Infinity}, {...config(), hardwareAcceleration: 'bad'}]
      : [{}, {codec: ''}, {codec: ' \t\n\r\f '}, {...config(), codedWidth: 10}, {...config(), codedWidth: 0, codedHeight: 10}, {...config(), displayAspectHeight: 4}, {...config(), displayAspectWidth: 0, displayAspectHeight: 4}, {...config(), rotation: Infinity}, {...config(), hardwareAcceleration: 'bad'}, {...config(), description: new Proxy(new ArrayBuffer(1), {})}];
    for (const [index, value] of invalid.entries()) {
      await check(label('invalid configure ' + index), () => {
        const codec = new C(init());
        throws(() => codec.configure(value), TypeError);
        assert(codec.state === 'unconfigured', 'invalid configure mutated state'); codec.close();
      });
      await check(label('invalid support ' + index), () => rejects(C.isConfigSupported(value), TypeError));
    }
    await check(label('support is a task and ignores unknown dictionary keys'), async () => {
      const input = {...config(), unrecognized: 3}, log = [];
      const promise = C.isConfigSupported(input).then(value => {log.push('support'); return value});
      await Promise.resolve(); assert(log.length === 0, 'support settled in a microtask');
      const support = await promise;
      assert(support.supported === false && support.config !== input && !Object.hasOwn(support.config, 'unrecognized'), 'support snapshot');
      assert(support.config.hardwareAcceleration === 'no-preference', 'default acceleration');
      assert(Object.getPrototypeOf(support) === Object.prototype && Object.getPrototypeOf(support.config) === Object.prototype, 'result realm');
    });
    await check(label('DOMString code units survive snapshot'), async () => {
      const text = 'unsupported-\ud800-\udfff';
      const support = await C.isConfigSupported({...config(), codec: text});
      assert(support.config.codec === text && !support.supported, 'lossy codec string');
    });
    await check(label('configure closes in a task and rejects all flushes'), async () => {
      const log = [], codec = new C({output() {throw Error('output')}, error(error) {
        assert(this === undefined && error instanceof DOMException && error.name === 'NotSupportedError', 'error callback arguments');
        assert(codec.state === 'closed', 'callback before close'); log.push('error');
      }});
      codec.configure(config());
      assert(codec.state === 'configured', 'synchronous state');
      const promises = [codec.flush(), codec.flush()];
      for (const promise of promises) promise.catch(() => log.push('flush'));
      await Promise.resolve(); assert(log.length === 0 && codec.state === 'configured', 'async error is not a task');
      const errors = await Promise.all(promises.map(promise => rejects(promise, DOMException, 'NotSupportedError')));
      assert(log[0] === 'error' && log.filter(value => value === 'error').length === 1 && codec[size] === 0, 'close/flush ordering');
      assert(errors[0] === errors[1], 'close reason shared');
      throws(() => codec.close(), DOMException, 'InvalidStateError');
      throws(() => codec.reset(), DOMException, 'InvalidStateError');
      throws(() => codec.configure(config()), DOMException, 'InvalidStateError');
      throws(() => codec.configure({}), TypeError);
      await rejects(codec.flush(), DOMException, 'InvalidStateError');
    });
    for (const action of ['reset', 'close']) {
      await check(label(action + ' cancels old control tasks and pending flushes'), async () => {
        let errors = 0;
        const codec = new C({output() {throw Error('output')}, error() {errors++}});
        codec.configure(config()); const flushed = codec.flush();
        codec[action]();
        await rejects(flushed, DOMException, 'AbortError');
        await turn(); assert(errors === 0 && codec.state === (action === 'reset' ? 'unconfigured' : 'closed'), 'stale callback');
        if (action === 'reset') codec.close();
      });
    }
    await check(label('reset then reconfigure isolates generations'), async () => {
      let errors = 0;
      const codec = new C({output() {}, error() {errors++}});
      codec.configure(config()); codec.reset(); codec.configure(config());
      await rejects(codec.flush(), DOMException, 'NotSupportedError');
      await turn(); assert(errors === 1 && codec.state === 'closed', 'generation cancellation');
    });
    await check(label('unconfigured operations validate arguments before state'), async () => {
      const codec = new C(init());
      throws(() => codec[encoder ? 'encode' : 'decode']({}), TypeError);
      await rejects(codec.flush(), DOMException, 'InvalidStateError');
      codec.reset(); codec.close();
    });
    if (!encoder) {
      await check(label('description copies only converted view bytes'), async () => {
        const bytes = new Uint8Array([9, 1, 2, 3, 8]);
        const support = await C.isConfigSupported({...config(), description: new DataView(bytes.buffer, 1, 3), colorSpace: {matrix: 'bt709', fullRange: null}});
        assert(support.config.description instanceof ArrayBuffer && Array.from(new Uint8Array(support.config.description)).join(',') === '1,2,3', 'view copy');
        assert(support.config.colorSpace.matrix === 'bt709' && support.config.colorSpace.fullRange === null && support.config.colorSpace.primaries === null, 'color dictionary');
        new Uint8Array(support.config.description)[0] = 99; assert(bytes[1] === 1, 'independent backing');
      });
      await check(label('detachment in a later dictionary getter is validated'), async () => {
        const buffer = new ArrayBuffer(1), options = {...config(), description: buffer, get hardwareAcceleration() {structuredClone(buffer, {transfer: [buffer]}); return 'no-preference'}};
        await rejects(C.isConfigSupported(options), TypeError);
      });
      await check(label('dequeue coalesces mutations inside its event handler'), async () => {
        const codec = new C(init()), key = new EncodedVideoChunk({type:'key', timestamp:0, data:new Uint8Array([1])});
        let events = 0;
        codec.ondequeue = () => {
          events++;
          if (events === 1) {codec.configure(config()); codec.decode(key); codec.reset();}
        };
        codec.configure(config()); codec.decode(key); codec.reset();
        await turn(); await turn();
        assert(events === 1 && codec.state === 'unconfigured', 'reentrant dequeue was not coalesced');
        codec.close();
      });
      await check(label('key requirement and queue reset dispatch a trusted event'), async () => {
        const codec = new C(init()), events = [];
        const delta = new EncodedVideoChunk({type: 'delta', timestamp: 0, data: new Uint8Array([1])});
        const key = new EncodedVideoChunk({type: 'key', timestamp: 1, data: new Uint8Array([1])});
        codec.ondequeue = event => events.push([event.isTrusted, event.target === codec, codec.decodeQueueSize]);
        codec.configure(config());
        throws(() => codec.decode(delta), DOMException, 'DataError');
        assert(codec.decodeQueueSize === 0, 'delta changed queue');
        Object.defineProperty(key, 'type', {get() {throw Error('author type getter')}});
        codec.decode(key); codec.decode(delta); assert(codec.decodeQueueSize === 2, 'enqueue');
        codec.reset(); assert(codec.decodeQueueSize === 0, 'reset queue');
        assert(events.length === 0, 'synchronous dequeue');
        await turn(); assert(events.length === 1 && events[0].join(',') === 'true,true,0', 'dequeue event');
        codec.close();
      });
    } else {
      await check(label('codec-specific dictionaries are cloned'), async () => {
        const input = {...config(), avc: {format: 'annexb', unknown: 1}, hevc: {format: 'hevc'}};
        const support = await C.isConfigSupported(input);
        assert(support.config.avc !== input.avc && support.config.avc.format === 'annexb' && !Object.hasOwn(support.config.avc, 'unknown'), 'avc clone');
        assert(support.config.hevc !== input.hevc && support.config.hevc.format === 'hevc', 'hevc clone');
      });
    }
  }
  return {complete: true, total: checks.length, passed: checks.filter(row => row.passed).length, checks};
})()
