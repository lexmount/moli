function abortSignalStaticsProbe(realm = globalThis) {
  const Signal = realm.AbortSignal;
  const Controller = realm.AbortController;
  const failures = [];
  const scenarios = [];
  const check = (ok, label) => { if (!ok) failures.push(label); };
  const scenario = (name, run) => {
    scenarios.push(name);
    try { run(); } catch (error) { failures.push(name + ': ' + error.name + ': ' + error.message); }
  };
  const typeError = (run, label) => {
    try { run(); failures.push(label + ': did not throw'); }
    catch (error) { check(error instanceof realm.TypeError, label + ': exception realm'); }
  };
  const marker = {};
  const exactError = (run, label) => {
    try { run(); failures.push(label + ': did not throw'); }
    catch (error) { check(error === marker, label + ': exception identity'); }
  };
  const branded = (signal, label) => check(Object.getPrototypeOf(signal) === Signal.prototype, label + ': prototype');
  scenario('static receiver independence', () => {
    let reads = 0;
    const poison = new Proxy({}, {get() { ++reads; throw marker; }});
    const revoked = Proxy.revocable({}, {});
    revoked.revoke();
    class Derived extends Signal {}
    const receivers = [undefined, null, 1, Symbol(), {}, poison, revoked.proxy, Derived];
    const results = new Set();
    for (const receiver of receivers) for (const name of ['abort', 'any', 'timeout']) {
      const signal = Reflect.apply(Signal[name], receiver, name === 'abort' ? [marker] : name === 'any' ? [[]] : [1000000]);
      branded(signal, name);
      check(signal.aborted === (name === 'abort'), name + ': state');
      check(signal.reason === (name === 'abort' ? marker : undefined), name + ': reason');
      check(!results.has(signal), name + ': new object');
      results.add(signal);
    }
    check(reads === 0, 'static receiver properties read');
  });
  scenario('intrinsic signal construction', () => {
    const descriptor = Object.getOwnPropertyDescriptor(realm, 'AbortSignal');
    let reads = 0;
    Object.defineProperty(realm, 'AbortSignal', {configurable: true, get() { ++reads; throw marker; }});
    try {
      const controller = new Controller();
      branded(controller.signal, 'controller');
      for (const name of ['abort', 'any', 'timeout']) {
        const signal = Reflect.apply(Signal[name], null, name === 'any' ? [[controller.signal]] : name === 'timeout' ? [1000000] : []);
        branded(signal, name);
        if (name === 'abort') check(signal.reason instanceof realm.DOMException && signal.reason.name === 'AbortError', 'default reason realm');
        if (name === 'any') { controller.abort(marker); check(signal.reason === marker, 'intrinsic dependent signal'); }
      }
      check(reads === 0, 'global AbortSignal getter read');
    } finally { Object.defineProperty(realm, 'AbortSignal', descriptor); }
  });
  scenario('timeout WebIDL bounds', () => {
    typeError(() => Reflect.apply(Signal.timeout, {}, []), 'missing timeout');
    for (const value of [undefined, NaN, Infinity, -Infinity, -1, -1.9, 2 ** 53, 2 ** 64, Symbol(), 1n])
      typeError(() => Signal.timeout(value), 'invalid timeout ' + String(value));
    for (const value of [-0.9, -0, 0, null, false, true, '4.9', 1.9, 2 ** 32, Number.MAX_SAFE_INTEGER]) {
      const signal = Signal.timeout(value);
      branded(signal, 'converted timeout');
      check(!signal.aborted && signal.reason === undefined, 'timeout fires asynchronously');
    }
    const order = [];
    const value = {[Symbol.toPrimitive](hint) { order.push(hint); new Controller().abort(); return 1; }};
    Signal.timeout(value);
    check(order.join() === 'number', 'timeout ToPrimitive order');
    exactError(() => Signal.timeout({valueOf() { throw marker; }}), 'timeout valueOf');
    exactError(() => Signal.timeout({get [Symbol.toPrimitive]() { throw marker; }}), 'timeout ToPrimitive getter');
  });
  scenario('any sequence conversion', () => {
    typeError(() => Reflect.apply(Signal.any, {}, []), 'missing any');
    for (const value of [undefined, null, 1, '', {}, { [Symbol.iterator]: 1 }])
      typeError(() => Signal.any(value), 'invalid sequence');
    const real = new Controller().signal;
    const revoked = Proxy.revocable(real, {});
    revoked.revoke();
    for (const value of [{}, Object.create(Signal.prototype), Object.create(real), new Proxy(real, {}), revoked.proxy])
      typeError(() => Signal.any([value]), 'invalid sequence member');
    for (const step of ['iterator getter', 'iterator call', 'next getter', 'next call', 'done getter', 'value getter']) {
      let advances = 0;
      const iterable = {get [Symbol.iterator]() {
        if (step === 'iterator getter') throw marker;
        return function() {
          if (step === 'iterator call') throw marker;
          return {get next() {
            if (step === 'next getter') throw marker;
            return function() {
              if (++advances > 1) return {done: true};
              if (step === 'next call') throw marker;
              return {get done() { if (step === 'done getter') throw marker; return false; },
                get value() { if (step === 'value getter') throw marker; return real; }};
            };
          }};
        };
      }};
      exactError(() => Signal.any(iterable), step);
    }
    let closed = false;
    function* invalidAfterAborted() {
      try { yield Signal.abort(marker); yield {}; }
      finally { closed = true; }
    }
    const iterator = invalidAfterAborted();
    typeError(() => Signal.any(iterator), 'validate entire sequence before abort');
    check(!closed, 'sequence conversion must not IteratorClose');
    iterator.return();
    const controller = new Controller();
    const dependent = Signal.any(new Set([controller.signal]));
    controller.abort(marker);
    check(dependent.aborted && dependent.reason === marker, 'converted sources propagate abort');
  });
  return {scenarios, failures};
}

async function abortSignalTimeoutProbe(realm = globalThis) {
  const failures = [];
  const signal = realm.AbortSignal.timeout.call(null, 1);
  const long = [2 ** 32, 2 ** 32 + 1, Number.MAX_SAFE_INTEGER].map(value => realm.AbortSignal.timeout(value));
  let calls = 0;
  await new Promise(resolve => {
    signal.onabort = event => {
      ++calls;
      if (!event.isTrusted || event.target !== signal || !(event instanceof realm.Event)) failures.push('timeout event realm or state');
      if (!(signal.reason instanceof realm.DOMException) || signal.reason.name !== 'TimeoutError') failures.push('timeout reason');
      resolve();
    };
  });
  if (calls !== 1 || !signal.aborted || long.some(value => value.aborted)) failures.push('timeout delay or count');
  return {calls, failures};
}
