(async () => {
  const rows = [];
  const assert = (value, message) => { if (!value) throw Error(message); };
  const check = async (name, run) => {
    try { await run(); rows.push({ name, pass: true }); }
    catch (e) { rows.push({ name, pass: false, error: e.name, message: e.message }); }
  };
  const realms = [['main', window], ['child', document.getElementById('child').contentWindow]];
  const names = ['ServiceWorker', 'ServiceWorkerContainer', 'ServiceWorkerRegistration', 'NavigationPreloadManager'];
  const promiseMethods = {
    ServiceWorkerContainer: { register: 1, getRegistration: 0, getRegistrations: 0 },
    ServiceWorkerRegistration: { unregister: 0, update: 0, showNotification: 1, getNotifications: 0 },
    NavigationPreloadManager: { enable: 0, disable: 0, getState: 0, setHeaderValue: 1 }
  };
  const attributes = {
    ServiceWorker: ['scriptURL', 'state', 'onstatechange', 'onerror'],
    ServiceWorkerContainer: ['controller', 'ready', 'onmessage', 'onmessageerror', 'oncontrollerchange'],
    ServiceWorkerRegistration: ['scope', 'updateViaCache', 'installing', 'waiting', 'active', 'navigationPreload', 'onupdatefound']
  };
    for (const [label, w] of realms) {
      if (!w.isSecureContext) {
        await check(label + '/insecure', () => assert(names.every(name => !(name in w)), 'secure globals hidden'));
        continue;
      }
      for (const name of names) {
        await check(label + '/' + name + '/constructor', () => {
          const C = w[name];
          const parent = name === 'NavigationPreloadManager' ? w.Object : w.EventTarget;
          const d = Object.getOwnPropertyDescriptor(w, name);
          assert(typeof C === 'function' && C.name === name && C.length === 0, 'constructor metadata');
          assert(d.writable && d.configurable && !d.enumerable, 'global descriptor');
          assert(Object.getPrototypeOf(C.prototype) === parent.prototype, 'prototype inheritance');
          assert(Object.getPrototypeOf(C) === (parent === w.Object ? w.Function.prototype : parent), 'constructor inheritance');
          assert(C.prototype.constructor === C, 'prototype constructor');
          const tag = Object.getOwnPropertyDescriptor(C.prototype, Symbol.toStringTag);
          assert(tag.value === name && tag.configurable && !tag.writable && !tag.enumerable, 'prototype tag');
          for (const call of [() => C(), () => new C()]) {
            let error; try { call(); } catch (e) { error = e; }
            assert(error instanceof w.TypeError, 'illegal constructor in callee realm');
          }
        });
        for (const [method, length] of Object.entries(promiseMethods[name] || {})) {
          await check(label + '/' + name + '/' + method, async () => {
            const C = w[name];
            const d = Object.getOwnPropertyDescriptor(C.prototype, method);
            assert(d.enumerable && d.writable && d.configurable && d.value.length === length, 'method descriptor');
            let conversions = 0, traps = 0;
            const value = { toString() { conversions++; return 'x'; } };
            const trap = () => { traps++; throw Error('author trap'); };
            const revoked = Proxy.revocable({}, {}); revoked.revoke();
            for (const receiver of [{}, C.prototype, Object.create(C.prototype), new Proxy({}, { get: trap, getPrototypeOf: trap }), revoked.proxy]) {
              const promise = d.value.call(receiver, value, value);
              assert(promise instanceof w.Promise, 'callee-realm rejection promise');
              let error; try { await promise; } catch (e) { error = e; }
              assert(error instanceof w.TypeError, 'invalid receiver rejects');
            }
            assert(conversions === 0 && traps === 0, 'brand check precedes conversion and Proxy traps');
          });
        }
        for (const attribute of attributes[name] || []) {
          await check(label + '/' + name + '/' + attribute, async () => {
            const C = w[name], d = Object.getOwnPropertyDescriptor(C.prototype, attribute);
            assert(d.enumerable && d.configurable && d.get.length === 0, 'accessor descriptor');
            assert(attribute.startsWith('on') ? d.set.length === 1 : d.set === undefined, 'setter presence');
            let error;
            try { await d.get.call({}); } catch (e) { error = e; }
            assert(error instanceof w.TypeError, 'accessor receiver check uses callee realm');
            if (d.set) {
              error = undefined; try { d.set.call({}, null); } catch (e) { error = e; }
              assert(error instanceof w.TypeError, 'setter receiver check');
            }
          });
        }
      }
      await check(label + '/container/native-factory', () => {
        const C = w.ServiceWorkerContainer, d = Object.getOwnPropertyDescriptor(w, 'ServiceWorkerContainer');
        let calls = 0;
        Object.defineProperty(w, 'ServiceWorkerContainer', { configurable: true, get() { calls++; throw Error('author constructor'); } });
        try {
          const container = w.navigator.serviceWorker;
          assert(Object.getPrototypeOf(container) === C.prototype && container instanceof w.EventTarget, 'native prototype and EventTarget');
          assert(container === w.navigator.serviceWorker && calls === 0, 'SameObject and intrinsic factory');
          assert(container.ready === container.ready && container.ready instanceof w.Promise, 'stable ready promise');
          assert(!Object.hasOwn(container, 'ready') && !Object.hasOwn(container, 'register'), 'shared interface members');
        } finally { Object.defineProperty(w, 'ServiceWorkerContainer', d); }
      });
      await check(label + '/container/event-target', () => {
        const container = w.navigator.serviceWorker, order = [];
        const before = () => order.push('before'), after = () => order.push('after');
        assert(container.addEventListener === w.EventTarget.prototype.addEventListener, 'inherited EventTarget methods');
        container.addEventListener('message', before);
        container.onmessage = function(e) { assert(this === container && e.target === container, 'handler receiver'); order.push('handler'); };
        container.addEventListener('message', after);
        container.onmessage = () => order.push('replacement');
        container.dispatchEvent(new w.Event('message'));
        assert(order.join() === 'before,replacement,after', 'shared ordered handler dispatch');
        container.removeEventListener('message', before); container.removeEventListener('message', after); container.onmessage = null;
        assert(container.startMessages() === undefined, 'startMessages return value');
      });
    }
  globalThis.__nodeReplacementResults = { rows, failures: rows.filter(row => !row.pass), passed: rows.filter(row => row.pass).length, total: rows.length };
  return rows.every(row => row.pass);
})()
