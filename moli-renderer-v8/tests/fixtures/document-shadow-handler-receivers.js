(() => {
  const assert = (condition, message) => { if (!condition) throw Error(message); };
  const frame = document.querySelector('iframe') || document.body.appendChild(document.createElement('iframe'));
  const other = frame.contentWindow;
  const owners = [['root', document], ['child', frame.contentDocument],
    ['windowless', document.implementation.createHTMLDocument('handlers')]];
  const rows = [];
  for (const [ownerName, owner] of owners) {
    if (globalThis.__handlerKind !== 'shadow') {
      for (const property of ['onclick', 'onreadystatechange']) {
        rows.push({name: ownerName + '/document/' + property, kind: 'document', target: owner, property});
      }
    }
    if (globalThis.__handlerKind !== 'document') {
      const host = owner.body.appendChild(owner.createElement('div'));
      rows.push({name: ownerName + '/shadow/onslotchange', kind: 'shadow',
        target: host.attachShadow({mode: 'open'}), property: 'onslotchange'});
    }
  }
  globalThis.__handlerTargetRows = rows;
  globalThis.runHandlerReceiverChecks = nativeReceivers => {
    const checks = [];
    const check = (name, run) => {
      try { run(); checks.push({name, passed: true}); }
      catch (error) { checks.push({name, passed: false, error: String(error)}); }
    };
    for (const [realmName, realm] of [['root', window], ['child', other]]) {
      for (let index = 0; index < rows.length; index++) {
        const {name, kind, target, property} = rows[index];
        const receiver = nativeReceivers ? nativeReceivers[index] : target;
        const prototype = kind === 'document' ? realm.Document.prototype : realm.ShadowRoot.prototype;
        const descriptor = Object.getOwnPropertyDescriptor(prototype, property);
        const prefix = realmName + '/' + name;
        const type = property.slice(2);
        const clear = () => { try { descriptor.set.call(receiver, null); } catch {} };
        check(prefix + '/identity and native dispatch', () => {
          clear();
          assert(descriptor.get.call(receiver) === null, 'initial null');
          let calls = 0, correctTarget = false;
          const callback = function(event) {
            calls++; correctTarget = this === target && event.currentTarget === target;
          };
          try {
            descriptor.set.call(receiver, callback);
            assert(descriptor.get.call(receiver) === callback && target[property] === callback, 'handler identity');
            target.dispatchEvent(new Event(type));
            assert(calls === 1 && correctTarget, 'callback target and receiver');
          } finally { clear(); }
        });
        check(prefix + '/listener order replacement and removal', () => {
          const trace = [], before = () => trace.push('before'), after = () => trace.push('after');
          clear(); target.addEventListener(type, before);
          try {
            descriptor.set.call(receiver, () => trace.push('handler'));
            target.addEventListener(type, after);
            target.dispatchEvent(new Event(type));
            assert(trace.join() === 'before,handler,after', 'initial listener position');
            descriptor.set.call(receiver, () => trace.push('replacement'));
            trace.length = 0; target.dispatchEvent(new Event(type));
            assert(trace.join() === 'before,replacement,after', 'replacement position');
            descriptor.set.call(receiver, null);
            trace.length = 0; target.dispatchEvent(new Event(type));
            assert(trace.join() === 'before,after' && descriptor.get.call(receiver) === null, 'remove handler');
          } finally {
            clear(); target.removeEventListener(type, before); target.removeEventListener(type, after);
          }
        });
        check(prefix + '/event handler conversion', () => {
          let conversions = 0;
          const object = {toString() { conversions++; throw 42; }, handleEvent() { conversions++; }};
          try {
            descriptor.set.call(receiver, object);
            assert(descriptor.get.call(receiver) === object, 'retain noncallable object');
            target.dispatchEvent(new Event(type));
            assert(conversions === 0, 'object is not an EventListener callback interface');
            for (const value of [undefined, null, false, 7, 'handler', Symbol('handler')]) {
              descriptor.set.call(receiver, value);
              assert(descriptor.get.call(receiver) === null, 'primitive becomes null');
            }
          } finally { clear(); }
        });
        check(prefix + '/return false cancellation', () => {
          let calls = 0;
          try {
            descriptor.set.call(receiver, () => { calls++; return false; });
            const event = new Event(type, {cancelable: true});
            assert(target.dispatchEvent(event) === false && event.defaultPrevented && calls === 1, 'cancel event');
          } finally { clear(); }
        });
        check(prefix + '/invalid receivers and callee error realm', () => {
          let traps = 0;
          const author = new Proxy(receiver, {
            get() { traps++; throw 42; }, getPrototypeOf() { traps++; throw 42; },
          });
          const revoked = Proxy.revocable(receiver, {}); revoked.revoke();
          const invalid = [author, revoked.proxy, Object.create(receiver), Object.create(prototype),
            document.createElement('div'), document.createTextNode('text'), {}, null, 3];
          const sentinel = () => {};
          try {
            descriptor.set.call(receiver, sentinel);
            for (const value of invalid) {
              for (const operation of [descriptor.get, descriptor.set]) {
                let error, result;
                try { result = operation.call(value, sentinel); } catch (caught) { error = caught; }
                if (property === 'onreadystatechange') {
                  assert(error === undefined && result === undefined, 'LegacyLenientThis');
                } else {
                  assert(error && Object.getPrototypeOf(error) === realm.TypeError.prototype, 'callee TypeError');
                }
                assert(traps === 0, 'brand validation must not execute author traps');
                assert(descriptor.get.call(receiver) === sentinel, 'invalid receiver must not mutate target');
              }
            }
          } finally { clear(); }
        });
      }
    }
    globalThis.__uiEventResults = {complete: true, total: checks.length,
      passed: checks.filter(row => row.passed).length, checks};
    return checks.every(row => row.passed);
  };
  return globalThis.__deferHandlerChecks ? true : runHandlerReceiverChecks();
})()
