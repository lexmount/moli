(() => {
  const own = (object, key) => Object.prototype.hasOwnProperty.call(object, key);
  const typeError = (body, realm) => {
    try { body(); return false; } catch (error) { return error instanceof realm.TypeError; }
  };
  const raises = (body, expected) => {
    try { body(); return false; } catch (error) { return Object.is(error, expected); }
  };
  globalThis.abortControllerReceiverProbe = (realm = globalThis, peers = [['self', realm]]) => {
    const rows = [];
    const check = (name, body) => {
      try { rows.push({name, passed: body() === true}); }
      catch (error) { rows.push({name, passed: false, error: String(error)}); }
    };
    const C = realm.AbortController;
    const signalDescriptor = Object.getOwnPropertyDescriptor(C.prototype, 'signal');
    const getter = signalDescriptor.get;
    const abortDescriptor = Object.getOwnPropertyDescriptor(C.prototype, 'abort');
    const abort = abortDescriptor.value;
    check('signal prototype descriptor', () => signalDescriptor.enumerable &&
      signalDescriptor.configurable && signalDescriptor.set === undefined &&
      getter.name === 'get signal' && getter.length === 0 &&
      Object.getPrototypeOf(getter) === realm.Function.prototype);
    check('abort prototype descriptor', () => abortDescriptor.enumerable &&
      abortDescriptor.configurable && abortDescriptor.writable && abort.name === 'abort' &&
      abort.length === 0 && Object.getPrototypeOf(abort) === realm.Function.prototype);
    check('constructor metadata and call requirements', () => C.length === 0 &&
      C.name === 'AbortController' && typeError(() => C(), realm));
    check('instances inherit methods and signal', () => {
      const controller = new C();
      return !own(controller, 'signal') && !own(controller, 'abort') &&
        Object.getPrototypeOf(controller) === C.prototype && controller.abort === abort;
    });
    check('signal is SameObject across abort', () => {
      const controller = new C(), signal = getter.call(controller);
      const before = getter.call(controller) === signal && !signal.aborted && signal.reason === undefined;
      abort.call(controller, 'reason');
      return before && getter.call(controller) === signal && controller.signal === signal &&
        signal.aborted && signal.reason === 'reason';
    });
    check('signal is readonly without converting assigned values', () => {
      const controller = new C(), signal = controller.signal;
      let calls = 0;
      const value = {[Symbol.toPrimitive]() {calls++; throw new Error('coercion');}};
      const set = realm.Function('controller', 'value', '"use strict"; controller.signal = value;');
      return Reflect.set(controller, 'signal', value) === false &&
        typeError(() => set(controller, value), realm) && controller.signal === signal && calls === 0;
    });
    for (const [peerName, peer] of peers) {
      const controller = new peer.AbortController();
      let traps = 0;
      const author = new Proxy(controller, {
        get() {traps++;}, getPrototypeOf() {traps++;}, has() {traps++;},
        ownKeys() {traps++;}, getOwnPropertyDescriptor() {traps++;}
      });
      const revoked = Proxy.revocable(controller, {}); revoked.revoke();
      const invalid = [['plain', {}], ['interface prototype', peer.AbortController.prototype],
        ['forged prototype', Object.create(peer.AbortController.prototype)],
        ['inherited instance', Object.create(controller)], ['author Proxy', author],
        ['revoked Proxy', revoked.proxy], ['signal', controller.signal],
        ['global', peer], ['null', null], ['undefined', undefined], ['number', 1],
        ['boolean', true], ['string', 'controller'], ['symbol', Symbol()], ['bigint', 1n],
        ['fake public identity', {__lmAbortControllerId: 1, __lmAbortControllerSignal: controller.signal}]];
      for (const [label, receiver] of invalid) {
        check(`${peerName} getter rejects ${label} in callee realm`, () =>
          typeError(() => getter.call(receiver), realm));
        check(`${peerName} abort rejects ${label} in callee realm`, () =>
          typeError(() => abort.call(receiver, controller), realm));
      }
      check(`${peerName} invalid receivers do not run author traps`, () => traps === 0);
      check(`${peerName} borrowed getter preserves original signal and realm`, () => {
        const controller = new peer.AbortController();
        return getter.call(controller) === controller.signal && controller.signal instanceof peer.AbortSignal;
      });
      check(`${peerName} borrowed abort retains reason and synchronously dispatches`, () => {
        const controller = new peer.AbortController(), signal = controller.signal, reason = {};
        let count = 0;
        signal.addEventListener('abort', event => {
          if (event.target === signal && signal.aborted && signal.reason === reason) count++;
        });
        const result = abort.call(controller, reason);
        return result === undefined && signal.aborted && signal.reason === reason && count === 1;
      });
      check(`${peerName} borrowed abort creates default error in callee realm`, () => {
        const controller = new peer.AbortController();
        abort.call(controller);
        return controller.signal.aborted && controller.signal.reason instanceof realm.DOMException &&
          controller.signal.reason.name === 'AbortError';
      });
      check(`${peerName} prototype removal preserves native controller state`, () => {
        const controller = new peer.AbortController(), signal = getter.call(controller), reason = {};
        Object.setPrototypeOf(controller, null);
        abort.call(controller, reason);
        return getter.call(controller) === signal && signal.aborted && signal.reason === reason;
      });
    }
    for (const [label, reason] of [['null', null], ['false', false], ['zero', 0],
      ['negative zero', -0], ['NaN', NaN], ['string', '\ud800'], ['symbol', Symbol('reason')],
      ['bigint', 9n], ['object', {}], ['array', []], ['function', () => {}]]) {
      check(`raw reason identity ${label}`, () => {
        const controller = new C(), signal = controller.signal;
        const result = abort.call(controller, reason);
        abort.call(controller, 'replacement');
        return result === undefined && signal.aborted && Object.is(signal.reason, reason) &&
          raises(() => signal.throwIfAborted(), reason);
      });
    }
    check('Proxy and revoked Proxy are accepted as raw reasons', () => {
      let traps = 0;
      const reason = new Proxy({}, {get() {traps++; throw 1;}, getPrototypeOf() {traps++; throw 2;}});
      const revoked = Proxy.revocable({}, {}); revoked.revoke();
      const a = new C(), b = new C();
      abort.call(a, reason); abort.call(b, revoked.proxy);
      return a.signal.reason === reason && b.signal.reason === revoked.proxy && traps === 0;
    });
    check('undefined reason uses a callee-realm AbortError', () => {
      const controller = new C(); abort.call(controller, undefined);
      return controller.signal.reason instanceof realm.DOMException &&
        controller.signal.reason.name === 'AbortError';
    });
    check('frozen controller and signal retain private state', () => {
      const controller = new C(), signal = controller.signal, reason = {};
      Object.freeze(controller); Object.freeze(signal);
      abort.call(controller, reason);
      return getter.call(controller) === signal && signal.aborted && signal.reason === reason;
    });
    check('own signal and abort shadows do not affect native operations', () => {
      const controller = new C(), signal = controller.signal, reason = {};
      let calls = 0;
      for (const key of ['signal', 'abort']) Object.defineProperty(controller, key, {get() {calls++; throw 1;}});
      abort.call(controller, reason);
      return getter.call(controller) === signal && signal.aborted && signal.reason === reason && calls === 0;
    });
    check('subclasses retain native state', () => {
      class Derived extends C {}
      const controller = new Derived(), signal = getter.call(controller), reason = {};
      abort.call(controller, reason);
      return controller instanceof Derived && signal.aborted && signal.reason === reason;
    });
    check('abort dispatch is once-only even for reentrant abort', () => {
      const controller = new C(), signal = controller.signal, first = {}, second = {};
      let count = 0;
      signal.addEventListener('abort', () => {count++; abort.call(controller, second);});
      abort.call(controller, first); abort.call(controller, second);
      return count === 1 && signal.reason === first;
    });
    check('author constructor replacement does not affect signal factory', () => {
      const original = realm.AbortSignal;
      let calls = 0;
      try {
        realm.AbortSignal = function() {calls++; throw 1;};
        const controller = new C();
        return controller.signal instanceof original && calls === 0;
      } finally { realm.AbortSignal = original; }
    });
    check('prototype replacement does not affect private initialization or abort', () => {
      let calls = 0;
      try {
        Object.defineProperty(C.prototype, 'signal', {configurable: true, get() {calls++; throw 1;}});
        Object.defineProperty(C.prototype, 'abort', {configurable: true, value() {calls++; throw 1;}});
        const controller = new C(), reason = {};
        abort.call(controller, reason);
        const signal = getter.call(controller);
        return signal.aborted && signal.reason === reason && !own(controller, 'signal') && calls === 0;
      } finally {
        Object.defineProperty(C.prototype, 'signal', signalDescriptor);
        Object.defineProperty(C.prototype, 'abort', abortDescriptor);
      }
    });
    return {complete: true, total: rows.length, passed: rows.filter(row => row.passed).length, rows};
  };
  const realms = typeof document === 'undefined' ? [['worker', globalThis]] :
    [['top', globalThis], ['child', document.querySelector('#child').contentWindow]];
  const rows = realms.flatMap(([name, realm]) => abortControllerReceiverProbe(realm, realms).rows
    .map(row => ({...row, name: `${name}: ${row.name}`})));
  globalThis.__uiEventResults = {complete: true, total: rows.length,
    passed: rows.filter(row => row.passed).length, rows};
  return rows.every(row => row.passed);
})()
