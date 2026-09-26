(() => {
  const assert = (condition, message) => { if (!condition) throw new Error(message); };
  const entryAttributes = ['target', 'contentRect', 'contentBoxSize', 'borderBoxSize', 'devicePixelContentBoxSize'];
  const boxAttributes = entryAttributes.slice(2);
  const sizeAttributes = ['inlineSize', 'blockSize'];
  const withFrame = async run => {
    const frame = document.body.appendChild(document.createElement('iframe'));
    try { await run(frame.contentWindow); }
    finally { frame.remove(); }
  };
  const withEntry = async (creator, callbackRealm, targetRealm, run) => {
    const target = targetRealm.document.body.appendChild(targetRealm.document.createElement('div'));
    target.style.cssText = 'width:31px;height:17px;padding:2px;border:1px solid';
    target.getBoundingClientRect();
    let observer;
    try {
      const delivery = await new Promise(resolve => {
        const callback = callbackRealm.Function('deliver', `return function(entries, observer) {
          deliver({entry: entries[0], entries, observer, receiver: this});
        };`)(resolve);
        observer = new creator.ResizeObserver(callback);
        observer.observe(target);
      });
      observer.disconnect();
      await run(delivery, target, observer);
    } finally { observer?.disconnect(); target.remove(); }
  };
  const cases = [];
  for (const creatorName of ['main', 'child']) for (const callbackName of ['main', 'child']) {
    for (const targetName of ['main', 'child']) {
      cases.push({
        name: `entry from ${creatorName} observer, ${callbackName} callback, ${targetName} target`,
        run: check => withFrame(other => {
          const realms = {main: window, child: other};
          const owner = realms[callbackName];
          return withEntry(realms[creatorName], owner, realms[targetName], ({entry, entries, observer, receiver}, target, expectedObserver) => {
            const size = entry.contentBoxSize[0];
            check('callback arguments retain identities and use the callback Array', () => {
              assert(observer === expectedObserver && receiver === observer, 'observer argument and callback this');
              assert(Object.getPrototypeOf(entries) === owner.Array.prototype, 'callback sequence realm');
              assert(!Object.isFrozen(entries), 'sequence remains mutable');
              assert(entry.target === target, 'observed target identity');
            });
            for (const [name, value, attributes] of [
              ['ResizeObserverEntry', entry, entryAttributes],
              ['ResizeObserverSize', size, sizeAttributes],
            ]) {
              check(`${name} native wrapper shape`, () => {
                assert(Object.getPrototypeOf(value) === owner[name].prototype, 'callback realm prototype');
                assert(Object.prototype.toString.call(value) === `[object ${name}]`, 'interface toStringTag');
                assert(Object.getOwnPropertyNames(value).length === 0, 'attributes are not own properties');
                assert(Object.isExtensible(value), 'native wrapper remains extensible');
              });
              for (const attribute of attributes) {
                check(`${name}.${attribute} is readonly`, () => {
                  const saved = value[attribute];
                  const written = Reflect.set(value, attribute, null);
                  if (written) Reflect.set(value, attribute, saved);
                  assert(!written, 'assignment must fail');
                  assert(value[attribute] === saved, 'assignment must retain the snapshot');
                });
                for (const [calleeName, realm] of Object.entries(realms)) {
                  const label = `${calleeName} ${name}.${attribute}`;
                  const getter = () => Object.getOwnPropertyDescriptor(realm[name].prototype, attribute).get;
                  check(`${label} descriptor and genuine receiver`, () => {
                    const descriptor = Object.getOwnPropertyDescriptor(realm[name].prototype, attribute);
                    assert(descriptor.enumerable && descriptor.configurable && descriptor.set === undefined, 'readonly WebIDL descriptor');
                    assert(descriptor.get.length === 0 && descriptor.get.name === `get ${attribute}`, 'getter signature');
                    assert(Object.getPrototypeOf(descriptor.get) === realm.Function.prototype, 'getter realm');
                    assert(getter().call(value) === value[attribute], 'borrowed getter retains native value');
                  });
                  let reads = 0;
                  const poison = () => { reads++; throw new Error('author trap ran'); };
                  const revoked = Proxy.revocable(value, {}); revoked.revoke();
                  const fake = Object.create(Object.getPrototypeOf(value));
                  Object.defineProperty(fake, attribute, {get: poison});
                  const invalid = [null, undefined, {}, Object.getPrototypeOf(value), Object.create(value),
                    Object.create(Object.getPrototypeOf(value), Object.getOwnPropertyDescriptors(value)),
                    new Proxy(value, {get: poison, getPrototypeOf: poison, has: poison}), revoked.proxy,
                    name === 'ResizeObserverEntry' ? size : entry, fake];
                  invalid.forEach((receiver, index) => check(`${label} rejects receiver ${index}`, () => {
                    const get = getter();
                    reads = 0;
                    let error;
                    try { get.call(receiver); } catch (caught) { error = caught; }
                    assert(error instanceof realm.TypeError, 'binding realm TypeError');
                    assert(reads === 0, 'brand validation must not invoke author code');
                  }));
                  check(`${label} survives public shadowing and prototype mutation`, () => {
                    const get = getter(), original = get.call(value), prototype = Object.getPrototypeOf(value);
                    reads = 0;
                    try {
                      Object.defineProperty(value, attribute, {get: poison, configurable: true});
                      Object.setPrototypeOf(value, null);
                      assert(get.call(value) === original && reads === 0, 'private state is independent of public properties');
                    } finally {
                      Object.setPrototypeOf(value, prototype);
                      Reflect.deleteProperty(value, attribute);
                    }
                  });
                }
              }
              check(`${name} cannot be structured cloned`, () => {
                let error;
                try { structuredClone(value); } catch (caught) { error = caught; }
                assert(error instanceof DOMException && error.name === 'DataCloneError', 'native interface has no clone codec');
              });
            }
            check('contentRect is a readonly native snapshot in the callback realm', () => {
              const rect = entry.contentRect;
              assert(Object.getPrototypeOf(rect) === owner.DOMRectReadOnly.prototype, 'DOMRectReadOnly realm and type');
              assert(Object.getOwnPropertyNames(rect).length === 0, 'private geometry');
              for (const key of ['x', 'y', 'width', 'height']) {
                const original = rect[key];
                const written = Reflect.set(rect, key, 900);
                if (written) Reflect.set(rect, key, original);
                assert(!written && rect[key] === original, `${key} is readonly`);
              }
            });
            for (const attribute of boxAttributes) check(`${attribute} is a stable FrozenArray of native sizes`, () => {
              const array = entry[attribute], first = array[0];
              assert(Object.getPrototypeOf(array) === owner.Array.prototype, 'array realm');
              assert(array === entry[attribute] && array.length === 1, 'stable snapshot array');
              assert(Object.isFrozen(array), 'FrozenArray');
              assert(Object.getPrototypeOf(first) === owner.ResizeObserverSize.prototype, 'size realm');
              assert(!Reflect.set(array, '0', null) && !Reflect.set(array, 'length', 0), 'index and length are readonly');
              assert(!Reflect.deleteProperty(array, '0') && !Reflect.defineProperty(array, 'extra', {value: 1}), 'array cannot shrink or grow');
              assert(array[0] === first && array.length === 1, 'failed changes retain contents');
            });
            check('box snapshots are independent objects', () => {
              const arrays = boxAttributes.map(attribute => entry[attribute]);
              assert(new Set(arrays).size === 3 && new Set(arrays.map(array => array[0])).size === 3, 'boxes must not share mutable wrappers');
            });
          });
        })
      });
    }
  }
  for (const owner of ['main', 'child']) cases.push({
    name: `${owner} interface exposure and intrinsic factories`,
    run: check => withFrame(async other => {
      const realm = owner === 'main' ? window : other;
      for (const name of ['ResizeObserverEntry', 'ResizeObserverSize']) {
        check(`${name} interface object`, () => {
          const constructor = realm[name];
          assert(typeof constructor === 'function' && constructor.name === name && constructor.length === 0, 'interface function');
          assert(Object.getPrototypeOf(constructor.prototype) === realm.Object.prototype, 'interface prototype parent');
          assert(constructor.prototype.constructor === constructor, 'prototype constructor');
          const tag = Object.getOwnPropertyDescriptor(constructor.prototype, Symbol.toStringTag);
          assert(tag.value === name && !tag.writable && !tag.enumerable && tag.configurable, 'toStringTag descriptor');
          for (const call of [() => constructor(), () => new constructor()]) {
            let error;
            try { call(); } catch (caught) { error = caught; }
            assert(error instanceof realm.TypeError, 'no public construction algorithm');
          }
        });
      }
      const names = ['ResizeObserverEntry', 'ResizeObserverSize', 'DOMRectReadOnly', 'Array'];
      const saved = names.map(name => [name, realm[name], Object.getOwnPropertyDescriptor(realm, name)]);
      let reads = 0;
      try {
        for (const name of names) Object.defineProperty(realm, name, {
          configurable: true, get() { reads++; throw new Error('public constructor getter ran'); }
        });
        await withEntry(realm, realm, realm, ({entry, entries}) => check('factories use intrinsics after global shadowing', () => {
          const originals = Object.fromEntries(saved.map(([name, value]) => [name, value]));
          assert(Object.getPrototypeOf(entry) === originals.ResizeObserverEntry.prototype, 'entry intrinsic prototype');
          assert(Object.getPrototypeOf(entry.contentRect) === originals.DOMRectReadOnly.prototype, 'rect intrinsic prototype');
          assert(Object.getPrototypeOf(entry.contentBoxSize[0]) === originals.ResizeObserverSize.prototype, 'size intrinsic prototype');
          assert(Object.getPrototypeOf(entry.contentBoxSize) === originals.Array.prototype, 'array intrinsic prototype');
          assert(Object.getPrototypeOf(entries) === originals.Array.prototype && reads === 0, 'author constructors are not consulted');
        }));
      } finally {
        for (const [name, , descriptor] of saved) {
          if (descriptor) Object.defineProperty(realm, name, descriptor); else Reflect.deleteProperty(realm, name);
        }
      }
    })
  });
  cases.push({
    name: 'successive deliveries keep retained snapshots independent',
    async run(check) {
      const target = document.body.appendChild(document.createElement('div'));
      target.style.cssText = 'width:31px;height:17px';
      let deliver;
      const observer = new ResizeObserver(entries => deliver(entries[0]));
      const sample = async () => {
        // The native fixture publishes layout explicitly; browser runs can
        // use the next rendering opportunity.
        await (__resizeEntryChecks.publishLayout?.() ?? new Promise(requestAnimationFrame));
        return new Promise(resolve => {
          deliver = resolve;
          observer.observe(target);
        });
      };
      try {
        const first = await sample();
        assert(first, 'initial notification');
        observer.unobserve(target);
        target.style.width = '47px';
        const second = await sample();
        check('old dimensions and object identities survive the resize', () => {
          assert(first !== second, 'a new observation creates a fresh snapshot');
          assert(first.contentRect.width === 31 && first.contentBoxSize[0].inlineSize === 31, 'old dimensions');
          assert(second.contentRect.width === 47 && second.contentBoxSize[0].inlineSize === 47, 'new dimensions');
          for (const attribute of ['contentRect', ...boxAttributes]) assert(first[attribute] !== second[attribute], 'fresh nested snapshot');
        });
        observer.disconnect(); target.remove();
        check('snapshot remains readable after observer disconnect and target removal', () => {
          assert(first.target === target && second.target === target, 'target identity remains');
          assert(first.contentRect.width === 31 && second.contentRect.width === 47, 'retained dimensions');
        });
      } finally { observer.disconnect(); target.remove(); }
    }
  });
  cases.push({
    name: 'Window-only result interfaces remain absent from Dedicated Workers',
    async run(check) {
      const source = `postMessage(['ResizeObserver', 'ResizeObserverEntry', 'ResizeObserverSize'].map(name => typeof globalThis[name]).join(','));`;
      const url = URL.createObjectURL(new Blob([source], {type: 'text/javascript'}));
      const worker = new Worker(url);
      try {
        const result = await new Promise((resolve, reject) => {
          worker.onmessage = event => resolve(event.data);
          worker.onerror = event => reject(new Error(event.message));
        });
        check('exposure set', () => assert(result === 'undefined,undefined,undefined', result));
      } finally { worker.terminate(); URL.revokeObjectURL(url); }
    }
  });
  globalThis.__resizeEntryChecks = {
    result: null,
    start(index) {
      this.result = null;
      const rows = [];
      const check = (name, run) => {
        try { run(); rows.push({name, passed: true}); }
        catch (error) { rows.push({name, passed: false, error: String(error)}); }
      };
      const finish = () => { this.result = JSON.stringify({total: rows.length, failures: rows.filter(row => !row.passed)}); };
      this.pending = cases[index].run(check).then(finish, error => {
        check('scenario completes', () => { throw error; }); finish();
      });
      return cases[index].name;
    },
    async all() {
      const results = [];
      for (let index = 0; index < cases.length; index++) {
        const name = this.start(index);
        await this.pending;
        results.push({name, ...JSON.parse(this.result)});
      }
      return JSON.stringify(results);
    }
  };
  return cases.length;
})();
