(async () => {
  const rows = [], platformRequests = [];
  const assert = (condition, message) => { if (!condition) throw Error(message); };
  const check = async (name, callback) => {
    try { await callback(); rows.push({name, pass: true}); }
    catch (error) { rows.push({name, pass: false, message: String(error)}); }
  };
  const thrown = callback => {
    try { callback(); } catch (error) { return error; }
    throw Error('expected synchronous exception');
  };
  const rejected = async (w, callback) => {
    const promise = callback();
    assert(promise instanceof w.Promise, 'Promise belongs to the callee realm');
    try { await promise; } catch (error) { return error; }
    throw Error('expected rejected Promise');
  };
  const realms = [['main', window], ['child', document.getElementById('child').contentWindow]];
  const mainGetter = isSecureContext && Object.getOwnPropertyDescriptor(Navigator.prototype, 'wakeLock')?.get;
  for (const [label, w] of realms) {
      if (!w.isSecureContext) {
        await check(label + '/navigator/insecure prototype before materialization', () => {
          assert(!Object.hasOwn(w.Navigator.prototype, 'wakeLock'), 'secure prototype member hidden before Navigator getter');
        });
        for (const name of ['WakeLock', 'WakeLockSentinel']) {
          await check(label + '/' + name + '/insecure', () => assert(!(name in w), 'secure interface hidden'));
        }
        await check(label + '/navigator/insecure', () => assert(!('wakeLock' in w.navigator), 'secure Navigator member hidden'));
        await check(label + '/navigator/author properties after finalization', () => {
          Object.defineProperty(w.Navigator.prototype, 'wakeLock', {configurable: true, value: 42});
          try { assert(w.navigator.wakeLock === 42, 'author property remains observable'); }
          finally { delete w.Navigator.prototype.wakeLock; }
        });
        continue;
      }
      for (const name of ['WakeLock', 'WakeLockSentinel']) {
        await check(label + '/' + name + '/interface', () => {
          const C = w[name], parent = name === 'WakeLock' ? w.Object : w.EventTarget;
          const d = Object.getOwnPropertyDescriptor(w, name);
          assert(typeof C === 'function' && C.name === name && C.length === 0, 'interface constructor metadata');
          assert(d.writable && d.configurable && !d.enumerable, 'global descriptor');
          assert(Object.getPrototypeOf(C.prototype) === parent.prototype, 'prototype inheritance');
          assert(Object.getPrototypeOf(C) === (name === 'WakeLock' ? w.Function.prototype : parent), 'constructor inheritance');
          assert(C.prototype.constructor === C, 'prototype constructor');
          const tag = Object.getOwnPropertyDescriptor(C.prototype, Symbol.toStringTag);
          assert(tag.value === name && !tag.writable && !tag.enumerable && tag.configurable, 'interface tag');
          assert(thrown(() => C()) instanceof w.TypeError && thrown(() => new C()) instanceof w.TypeError, 'illegal constructor realm');
        });
      }
      let lock;
      await check(label + '/navigator/SameObject intrinsic and relevant realm', () => {
        const C = w.WakeLock, original = Object.getOwnPropertyDescriptor(w, 'WakeLock');
        const inherited = Object.getOwnPropertyDescriptor(w.Object.prototype, 'wakeLock');
        let constructorReads = 0, inheritedReads = 0;
        Object.defineProperty(w, 'WakeLock', {configurable: true, get() { constructorReads++; throw Error('author constructor'); }});
        Object.defineProperty(w.Object.prototype, 'wakeLock', {configurable: true, get() { inheritedReads++; throw Error('inherited cache hook'); }});
        try { lock = mainGetter.call(w.navigator); }
        finally {
          if (original) Object.defineProperty(w, 'WakeLock', original); else delete w.WakeLock;
          if (inherited) Object.defineProperty(w.Object.prototype, 'wakeLock', inherited); else delete w.Object.prototype.wakeLock;
        }
        assert(constructorReads === 0 && inheritedReads === 0, 'native materialization ignores public hooks');
        assert(lock === w.navigator.wakeLock && Object.getPrototypeOf(lock) === C.prototype, 'SameObject in Navigator relevant realm');
        assert(lock instanceof C && Object.prototype.toString.call(lock) === '[object WakeLock]', 'native WakeLock identity');
        assert(!Object.hasOwn(w.navigator, 'wakeLock') && !Object.hasOwn(lock, 'request'), 'shared prototype surface');
        const d = Object.getOwnPropertyDescriptor(w.Navigator.prototype, 'wakeLock');
        assert(d.enumerable && d.configurable && d.set === undefined && d.get.length === 0 && d.get.name === 'get wakeLock', 'readonly Navigator accessor');
        assert(Reflect.set(w.navigator, 'wakeLock', {}) === false && w.navigator.wakeLock === lock, 'readonly SameObject');
      });
      await check(label + '/navigator/receiver brands', () => {
        const get = Object.getOwnPropertyDescriptor(w.Navigator.prototype, 'wakeLock').get;
        let traps = 0; const trap = () => { traps++; throw Error('receiver hook'); };
        const revoked = Proxy.revocable(w.navigator, {}); revoked.revoke();
        for (const value of [null, {}, w.Navigator.prototype, Object.create(w.navigator), new Proxy(w.navigator, {get: trap, getPrototypeOf: trap}), revoked.proxy]) {
          assert(thrown(() => get.call(value)) instanceof w.TypeError, 'callee realm rejects unbranded Navigator');
        }
        assert(traps === 0, 'receiver validation does not run author hooks');
      });
      await check(label + '/request/descriptor', () => {
        const d = Object.getOwnPropertyDescriptor(w.WakeLock.prototype, 'request');
        assert(d.value.length === 0 && d.value.name === 'request' && d.writable && d.enumerable && d.configurable, 'request operation metadata');
      });
      await check(label + '/request/brand before conversion', async () => {
        const request = w.WakeLock.prototype.request;
        let conversions = 0, traps = 0;
        const argument = {toString() { conversions++; throw Error('argument conversion'); }};
        const trap = () => { traps++; throw Error('receiver hook'); };
        const revoked = Proxy.revocable(lock, {}); revoked.revoke();
        for (const receiver of [null, {}, w.WakeLock.prototype, Object.create(lock), new Proxy(lock, {get: trap, getPrototypeOf: trap}), revoked.proxy]) {
          assert(await rejected(w, () => request.call(receiver, argument)) instanceof w.TypeError, 'reject forged and author proxy receivers');
        }
        assert(conversions === 0 && traps === 0, 'native receiver checks precede argument conversion');
      });
      await check(label + '/request/enum rejection', async () => {
        for (const value of [null, '', 'system', 'SCREEN', 0, true, Symbol(), {}]) {
          assert(await rejected(w, () => lock.request(value)) instanceof w.TypeError, 'invalid WakeLockType rejects Promise');
        }
      });
      await check(label + '/request/conversion exceptions and order', async () => {
        const sentinel = {}, order = [];
        const argument = {
          get [Symbol.toPrimitive]() { order.push('get primitive'); return () => { order.push('call primitive'); throw sentinel; }; },
          toString() { throw Error('unexpected string fallback'); }
        };
        assert(await rejected(w, () => lock.request(argument)) === sentinel, 'preserve author conversion exception identity');
        assert(order.join() === 'get primitive,call primitive', 'enum string conversion runs exactly once');
      });
      await check(label + '/request/valid default and platform outcome', async () => {
        const request = w.WakeLock.prototype.request;
        const promises = [request.call(lock), request.call(lock, undefined), request.call(lock, 'screen')];
        for (let i = 0; i < promises.length; i++) {
          const promise = promises[i];
          assert(promise instanceof w.Promise && promises.filter(value => value === promise).length === 1, 'fresh Promise per request');
          try {
            const sentinel = await promise;
            assert(sentinel instanceof w.WakeLockSentinel && sentinel instanceof w.EventTarget && sentinel.type === 'screen', 'native sentinel if platform permits request');
            platformRequests.push({realm: label, call: i, result: 'acquired'});
            await sentinel.release();
          } catch (error) {
            assert(error instanceof w.DOMException && error.name === 'NotAllowedError' && error.code === 0, 'platform denial uses NotAllowedError');
            platformRequests.push({realm: label, call: i, result: 'denied', name: error.name, code: error.code});
          }
        }
      });
      await check(label + '/request/native identity independent of public prototype', async () => {
        const request = w.WakeLock.prototype.request, prototype = Object.getPrototypeOf(lock);
        Object.setPrototypeOf(lock, null);
        try {
          assert(await rejected(w, () => request.call(lock, 'invalid')) instanceof w.TypeError, 'native brand retained after prototype removal');
        } finally { Object.setPrototypeOf(lock, prototype); }
      });
      const entries = [['released', false], ['type', false], ['onrelease', true]];
      for (const [name, writable] of entries) {
        await check(label + '/sentinel/' + name, () => {
          const d = Object.getOwnPropertyDescriptor(w.WakeLockSentinel.prototype, name);
          assert(d.enumerable && d.configurable && d.get.name === 'get ' + name && d.get.length === 0, 'shared sentinel getter metadata');
          assert(writable ? d.set.name === 'set ' + name && d.set.length === 1 : d.set === undefined, 'sentinel attribute writability');
          let conversions = 0, traps = 0;
          const value = {toString() { conversions++; throw Error('unexpected conversion'); }};
          const trap = () => { traps++; throw Error('receiver hook'); };
          const fake = Object.create(w.WakeLockSentinel.prototype), revoked = Proxy.revocable(fake, {}); revoked.revoke();
          for (const receiver of [null, {}, w.WakeLockSentinel.prototype, fake, new Proxy(fake, {get: trap, getPrototypeOf: trap}), revoked.proxy, lock]) {
            assert(thrown(() => d.get.call(receiver)) instanceof w.TypeError, 'sentinel getter rejects unbranded receiver');
            if (writable) assert(thrown(() => d.set.call(receiver, value)) instanceof w.TypeError, 'sentinel setter checks receiver first');
          }
          assert(conversions === 0 && traps === 0, 'sentinel validation ignores author hooks');
        });
      }
      await check(label + '/sentinel/release', async () => {
        const d = Object.getOwnPropertyDescriptor(w.WakeLockSentinel.prototype, 'release');
        assert(d.value.name === 'release' && d.value.length === 0 && d.enumerable && d.configurable && d.writable, 'release operation metadata');
        const fake = Object.create(w.WakeLockSentinel.prototype), revoked = Proxy.revocable(fake, {}); revoked.revoke();
        for (const receiver of [null, {}, fake, new Proxy(fake, {}), revoked.proxy, lock]) {
          assert(await rejected(w, () => d.value.call(receiver)) instanceof w.TypeError, 'release rejects Promise for illegal receiver');
        }
      });
      await check(label + '/request/cross-realm callee Promise and errors', async () => {
        const other = w === window ? document.getElementById('child').contentWindow : window;
        const error = await rejected(w, () => w.WakeLock.prototype.request.call(other.navigator.wakeLock, 'invalid'));
        assert(error instanceof w.TypeError && !(error instanceof other.TypeError), 'TypeError belongs to binding realm');
      });
      await check(label + '/request/immutable Promise intrinsic', async () => {
        const original = w.Promise;
        w.Promise = function() { throw Error('author Promise constructor'); };
        let promise;
        try { promise = w.WakeLock.prototype.request.call(lock, 'invalid'); }
        finally { w.Promise = original; }
        assert(promise instanceof original && await rejected(w, () => promise) instanceof w.TypeError, 'Promise wrapper ignores public constructor');
      });
  }
  globalThis.__nodeReplacementResults = {rows, platformRequests, failures: rows.filter(row => !row.pass), passed: rows.filter(row => row.pass).length, total: rows.length};
  return rows.every(row => row.pass);
})()
