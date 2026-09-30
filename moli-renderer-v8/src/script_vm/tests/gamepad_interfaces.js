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
  const popup = open();
  const realms = [['main', window], ['child', document.getElementById('child').contentWindow], ['popup', popup]];
  try {
    for (const [label, w] of realms) {
      for (const [name, fields] of [
        ['Gamepad', ['id', 'index', 'connected', 'timestamp', 'mapping', 'axes', 'buttons', 'vibrationActuator']],
        ['GamepadButton', ['pressed', 'touched', 'value']],
        ['GamepadHapticActuator', ['effects']]
      ]) {
        await check(label + '/' + name + '/interface', () => {
          const C = w[name], descriptor = Object.getOwnPropertyDescriptor(w, name);
          assert(typeof C === 'function' && C.name === name && C.length === 0, 'interface object');
          assert(descriptor.writable && descriptor.configurable && !descriptor.enumerable, 'global descriptor');
          assert(Object.getPrototypeOf(C) === w.Function.prototype && Object.getPrototypeOf(C.prototype) === w.Object.prototype, 'ordinary interface inheritance');
          assert(C.prototype.constructor === C && Object.prototype.toString.call(C.prototype) === '[object ' + name + ']', 'prototype identity');
          assert(thrown(() => C()) instanceof w.TypeError && thrown(() => new C()) instanceof w.TypeError, 'not author constructible');
        });
        await check(label + '/' + name + '/readonly getters and native receivers', () => {
          let traps = 0;
          const trap = () => { traps++; throw Error('author trap'); }, revoked = Proxy.revocable({}, {}); revoked.revoke();
          for (const field of fields) {
            const descriptor = Object.getOwnPropertyDescriptor(w[name].prototype, field), getter = descriptor.get;
            assert(descriptor.enumerable && descriptor.configurable && !descriptor.set && getter.name === 'get ' + field && getter.length === 0, 'readonly shared ' + field);
            for (const receiver of [null, undefined, {}, w[name].prototype, Object.create(w[name].prototype), new Proxy({}, {get: trap, getPrototypeOf: trap}), revoked.proxy]) {
              assert(thrown(() => getter.call(receiver)) instanceof w.TypeError, 'native receiver required for ' + field);
            }
          }
          assert(traps === 0, 'author traps are not consulted for branding');
        });
      }
      for (const name of ['playEffect', 'reset']) {
        await check(label + '/GamepadHapticActuator/' + name + '/rejected Promise receivers', async () => {
          const descriptor = Object.getOwnPropertyDescriptor(w.GamepadHapticActuator.prototype, name), method = descriptor.value;
          assert(descriptor.enumerable && descriptor.configurable && descriptor.writable && method.name === name, 'method descriptor');
          if (name === 'reset') assert(method.length === 0, 'reset arity');
          let conversions = 0, traps = 0;
          const trap = () => { traps++; throw Error('trap'); }, value = {toString() { conversions++; throw Error('conversion'); }};
          const params = new Proxy({}, {get() { conversions++; throw Error('dictionary conversion'); }});
          const revoked = Proxy.revocable({}, {}); revoked.revoke();
          for (const receiver of [null, {}, w.GamepadHapticActuator.prototype, new Proxy({}, {get: trap}), revoked.proxy]) {
            const promise = method.call(receiver, value, params);
            assert(promise instanceof w.Promise, 'brand failure returns callee Promise');
            let error; try { await promise; } catch (failure) { error = failure; }
            assert(error instanceof w.TypeError, 'callee realm TypeError rejection');
          }
          assert(conversions === 0 && traps === 0, 'receiver before author conversion');
        });
      }
      await check(label + '/GamepadEvent/interface and defaults', () => {
        const C = w.GamepadEvent, descriptor = Object.getOwnPropertyDescriptor(w, 'GamepadEvent');
        assert(typeof C === 'function' && C.name === 'GamepadEvent' && C.length === 1 && descriptor.writable && descriptor.configurable && !descriptor.enumerable, 'event constructor');
        assert(Object.getPrototypeOf(C) === w.Event && Object.getPrototypeOf(C.prototype) === w.Event.prototype, 'Event inheritance');
        assert(thrown(() => C('test')) instanceof w.TypeError && thrown(() => new C()) instanceof w.TypeError, 'requires new and type');
        for (const init of [undefined, null, {}, {gamepad: null}, {gamepad: undefined}]) {
          const event = new C('test', init);
          assert(event instanceof C && event instanceof w.Event && event.gamepad === null && event.gamepad === event.gamepad, 'nullable default and identity');
          assert(event.type === 'test' && !event.bubbles && !event.cancelable && !event.composed && !event.isTrusted, 'base event defaults');
          assert(!Object.hasOwn(event, 'gamepad') && Reflect.set(event, 'gamepad', {}) === false && event.gamepad === null, 'readonly private payload');
        }
      });
      await check(label + '/GamepadEvent/dictionary and payload validation', () => {
        const C = w.GamepadEvent;
        for (const init of [1, true, 'options', Symbol()]) assert(thrown(() => new C('test', init)) instanceof w.TypeError, 'dictionary must be an object');
        for (const gamepad of [{}, Object.create(w.Gamepad.prototype), new Proxy({}, {}), 1, 'device', false, Symbol()]) {
          assert(thrown(() => new C('test', {gamepad})) instanceof w.TypeError, 'nullable native Gamepad required');
        }
        let traps = 0;
        const proxy = new Proxy({}, {get() { traps++; throw Error('trap'); }, getPrototypeOf() { traps++; throw Error('trap'); }});
        assert(thrown(() => new C('test', {gamepad: proxy})) instanceof w.TypeError && traps === 0, 'payload branding avoids author traps');
        const revoked = Proxy.revocable({}, {}); revoked.revoke();
        assert(thrown(() => new C('test', {gamepad: revoked.proxy})) instanceof w.TypeError, 'revoked proxy rejected');
        const inherited = Object.create({bubbles: true, gamepad: null});
        const event = new C('test', inherited); assert(event.bubbles && event.gamepad === null, 'inherited dictionary members');
      });
      await check(label + '/GamepadEvent/conversion order and exception identity', () => {
        const order = [], init = Object.create(null), sentinel = {};
        const type = {toString() { order.push('type'); return 'test'; }};
        for (const [key, value] of [['bubbles', true], ['cancelable', true], ['composed', true], ['gamepad', null]]) Object.defineProperty(init, key, {get() { order.push(key); return value; }});
        const event = new w.GamepadEvent(type, init);
        assert(order.join() === 'type,bubbles,cancelable,composed,gamepad' && event.bubbles && event.cancelable && event.composed, 'type before inherited and derived members');
        assert(thrown(() => new w.GamepadEvent('test', {get gamepad() { throw sentinel; }})) === sentinel, 'preserve payload getter exception');
        let reads = 0;
        assert(thrown(() => new w.GamepadEvent(Symbol(), new Proxy({}, {get() { reads++; throw sentinel; }}))) instanceof w.TypeError && reads === 0, 'type failure precedes dictionary read');
        assert(thrown(() => new w.GamepadEvent({toString() { throw sentinel; }}, init)) === sentinel, 'preserve type conversion exception');
        const lone = new w.GamepadEvent('\ud800'); assert(lone.type.charCodeAt(0) === 0xd800, 'DOMString retains lone surrogate');
      });
      await check(label + '/GamepadEvent/getter brands and cross-realm calls', () => {
        const getter = Object.getOwnPropertyDescriptor(w.GamepadEvent.prototype, 'gamepad').get;
        assert(getter.name === 'get gamepad' && getter.length === 0, 'getter name and arity');
        const event = new w.GamepadEvent('test'), foreign = new window.GamepadEvent('foreign');
        assert(getter.call(foreign) === null, 'foreign genuine event accepted');
        let traps = 0; const trap = () => { traps++; throw Error('trap'); }, revoked = Proxy.revocable(event, {}); revoked.revoke();
        for (const receiver of [null, {}, new w.Event('test'), w.GamepadEvent.prototype, Object.create(event), new Proxy(event, {get: trap, getPrototypeOf: trap}), revoked.proxy]) assert(thrown(() => getter.call(receiver)) instanceof w.TypeError, 'callee realm receiver error');
        assert(traps === 0, 'brand check does not unwrap author Proxy');
        Object.setPrototypeOf(event, null); assert(getter.call(event) === null, 'native identity survives prototype mutation');
        class Sub extends w.GamepadEvent {};
        const sub = new Sub('test'); assert(sub instanceof Sub && sub instanceof w.GamepadEvent && sub.gamepad === null, 'new.target prototype');
      });
      await check(label + '/GamepadEvent/dispatch and Window handler', () => {
        const event = new w.GamepadEvent('gamepadconnected', {cancelable: true}), target = new w.EventTarget();
        let calls = 0;
        const listener = value => { assert(value === event && value.target === target && value.gamepad === null, 'dispatch retains event payload'); calls++; value.preventDefault(); };
        target.addEventListener('gamepadconnected', listener);
        assert(target.dispatchEvent(event) === false && calls === 1, 'native Event cancellation');
        target.removeEventListener('gamepadconnected', listener);
        const windowEvent = new w.GamepadEvent('gamepaddisconnected');
        w.ongamepaddisconnected = function(value) { assert(this === w && value === windowEvent && value.gamepad === null, 'Window handler receiver and event'); calls++; };
        try { assert(w.dispatchEvent(windowEvent), 'Window dispatch'); } finally { w.ongamepaddisconnected = null; }
        assert(calls === 2, 'one Window handler invocation');
        event.initEvent('reused', false, false); assert(event.type === 'reused' && event.gamepad === null, 'base reinitialization retains subclass payload');
      });
      await check(label + '/Navigator/polling snapshots and receivers', () => {
        const method = w.Navigator.prototype.getGamepads, first = method.call(w.navigator), second = method.call(w.navigator);
        assert(method.name === 'getGamepads' && method.length === 0 && Array.isArray(first) && first !== second, 'fresh Array snapshots');
        assert(first.every(value => value === null), 'no hardware in the control environment');
        first.push('author mutation'); assert(!method.call(w.navigator).includes('author mutation'), 'snapshot mutation does not alter polling state');
        let traps = 0; const trap = () => { traps++; throw Error('trap'); }, revoked = Proxy.revocable(w.navigator, {}); revoked.revoke();
        for (const receiver of [null, {}, w.Navigator.prototype, Object.create(w.navigator), new Proxy(w.navigator, {get: trap, getPrototypeOf: trap}), revoked.proxy]) assert(thrown(() => method.call(receiver)) instanceof w.TypeError, 'native Navigator receiver');
        assert(traps === 0, 'polling brand does not execute author traps');
      });
    }
  } finally { popup.close(); }
  globalThis.__nodeReplacementResults = {rows, passed: rows.filter(row => row.pass).length, total: rows.length, failures: rows.filter(row => !row.pass)};
  return rows.every(row => row.pass);
})()
