(async () => {
  const rows = [];
  const assert = (value, message) => { if (!value) throw Error(message); };
  const check = async (name, run) => {
    try { await run(); rows.push({ name, pass: true }); }
    catch (e) { rows.push({ name, pass: false, message: String(e) }); }
  };
  const names = ['PushManager', 'PushSubscription', 'PushSubscriptionOptions'];
  const popup = open();
  const realms = [['main', window], ['child', document.getElementById('child').contentWindow], ['popup', popup]];
  const methods = { PushManager: { subscribe: 0, getSubscription: 0, permissionState: 0 }, PushSubscription: { getKey: 1, unsubscribe: 0, toJSON: 0 } };
  const attributes = { PushSubscription: ['endpoint', 'expirationTime', 'options'], PushSubscriptionOptions: ['userVisibleOnly', 'applicationServerKey'] };
  try {
    for (const [label, w] of realms) {
      if (!w.isSecureContext) {
        await check(label + '/insecure', () => assert(names.every(name => !(name in w)), 'secure globals hidden'));
        continue;
      }
      for (const name of names) {
        await check(label + '/' + name + '/constructor', () => {
          const C = w[name], d = Object.getOwnPropertyDescriptor(w, name);
          assert(typeof C === 'function' && C.length === 0 && C.name === name, 'constructor metadata');
          assert(d.writable && d.configurable && !d.enumerable, 'global descriptor');
          assert(Object.getPrototypeOf(C) === w.Function.prototype && Object.getPrototypeOf(C.prototype) === w.Object.prototype, 'intrinsic inheritance');
          assert(C.prototype.constructor === C, 'prototype constructor');
          const tag = Object.getOwnPropertyDescriptor(C.prototype, Symbol.toStringTag);
          assert(tag.value === name && tag.configurable && !tag.writable && !tag.enumerable, 'prototype tag');
          for (const call of [() => C(), () => new C()]) {
            let error; try { call(); } catch (e) { error = e; }
            assert(error instanceof w.TypeError, 'illegal constructor error realm');
          }
        });
        for (const [method, length] of Object.entries(methods[name] || {})) {
          await check(label + '/' + name + '/' + method, async () => {
            const C = w[name], d = Object.getOwnPropertyDescriptor(C.prototype, method);
            assert(d.value.length === length && d.enumerable && d.configurable && d.writable, 'method descriptor');
            let conversions = 0, traps = 0;
            const trap = () => { traps++; throw Error('author trap'); };
            const value = { toString() { conversions++; return 'auth'; }, get applicationServerKey() { conversions++; return null; }, get userVisibleOnly() { conversions++; return false; } };
            const revoked = Proxy.revocable({}, {}); revoked.revoke();
            const promiseMethod = name === 'PushManager' || method === 'unsubscribe';
            for (const receiver of [null, {}, C.prototype, Object.create(C.prototype), new Proxy({}, { get: trap, getPrototypeOf: trap }), revoked.proxy]) {
              let error, returned;
              try { returned = d.value.call(receiver, value); } catch (e) { error = e; }
              if (promiseMethod) {
                assert(!error && returned instanceof w.Promise, 'callee-realm rejection promise');
                try { await returned; } catch (e) { error = e; }
              }
              assert(error instanceof w.TypeError, 'native receiver check');
            }
            assert(conversions === 0 && traps === 0, 'brand precedes arguments and traps');
          });
        }
        for (const attribute of attributes[name] || []) {
          await check(label + '/' + name + '/' + attribute, () => {
            const d = Object.getOwnPropertyDescriptor(w[name].prototype, attribute);
            assert(d.get.length === 0 && d.set === undefined && d.enumerable && d.configurable, 'readonly prototype accessor');
            let error; try { d.get.call({}); } catch (e) { error = e; }
            assert(error instanceof w.TypeError, 'getter receiver realm');
          });
        }
      }
      await check(label + '/encodings', () => {
        const C = w.PushManager, d = Object.getOwnPropertyDescriptor(C, 'supportedContentEncodings');
        const encodings = C.supportedContentEncodings;
        assert(d.get.length === 0 && d.set === undefined && d.enumerable && d.configurable, 'static accessor descriptor');
        assert(encodings instanceof w.Array && Object.isFrozen(encodings), 'realm frozen array');
        assert(encodings === C.supportedContentEncodings && d.get.call(C) === encodings, 'static SameObject');
        if (w !== window) assert(encodings !== PushManager.supportedContentEncodings, 'realm-specific identity');
        assert(encodings.every(value => typeof value === 'string'), 'encoding values');
      });
    }
  } finally { popup.close(); }
  globalThis.__nodeReplacementResults = { rows, failures: rows.filter(row => !row.pass), passed: rows.filter(row => row.pass).length, total: rows.length };
  return rows.every(row => row.pass);
})()
