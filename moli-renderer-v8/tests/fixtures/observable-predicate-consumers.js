(async () => {
  'use strict';
  const failures = [];
  let checks = 0;
  const check = (value, label) => { checks++; if (!value) failures.push(label); };
  const same = (a, b, label) => check(JSON.stringify(a) === JSON.stringify(b), label);
  const rejected = p => p.then(() => { throw new Error('expected rejection'); }, e => e);
  const thrown = fn => { try { fn(); } catch (e) { return e; } };
  const test = async (label, fn) => { try { await fn(); } catch (e) { check(false, label + ': ' + e); } };
  const names = ['some', 'every', 'find'];
  for (const name of names) check(typeof Observable.prototype[name] === 'function', name + ' exposed');
  if (failures.length) return {checks, failures};

  for (const name of names) {
    const method = Observable.prototype[name];
    const call = (source, callback, options) => Reflect.apply(method, source,
      [callback, options]);
    const continuing = name === 'every', deciding = !continuing;
    const emptyResult = name === 'find' ? undefined : continuing;
    const decidedResult = value => name === 'find' ? value : !continuing;
    await test(name + ' conversion', async () => {
      const desc = Object.getOwnPropertyDescriptor(Observable.prototype, name);
      check(method.name === name && method.length === 1, name + ' name and length');
      check(desc.enumerable && desc.writable && desc.configurable, name + ' descriptor');
      check(thrown(() => new method(() => {})) instanceof TypeError, name + ' non-constructor');
      const source = Observable.from([1]);
      let reads = 0, traps = 0;
      const options = {get signal() { reads++; }};
      const revoked = Proxy.revocable(source, {}); revoked.revoke();
      for (const value of [undefined, null, false, 1, Symbol(), {}, Object.create(source),
        Object.create(Observable.prototype), new Proxy(source, {get() { traps++; }}), revoked.proxy]) {
        const promise = call(value, () => {}, options);
        check(promise instanceof Promise && await rejected(promise) instanceof TypeError, name + ' receiver rejects Promise');
      }
      for (const callback of [undefined, null, {}, {handleEvent() {}}, 1, Object.create(Function.prototype)]) {
        check(await rejected(call(source, callback, options)) instanceof TypeError, name + ' invalid callback');
      }
      check(await rejected(method.call(source)) instanceof TypeError, name + ' required callback');
      check(reads === 0 && traps === 0, name + ' receiver and callback validation precede options');
      for (const options of [1, true, {signal: null}, {signal: {}}, {signal: new Proxy(new AbortController().signal, {})}]) {
        check(await rejected(call(source, () => {}, options)) instanceof TypeError, name + ' invalid options');
      }
      const marker = {};
      check(await rejected(call(source, () => {}, {get signal() { throw marker; }})) === marker, name + ' getter error identity');
      Object.setPrototypeOf(source, null);
      await call(source, () => {}, options);
      check(reads === 1, name + ' genuine brand with changed prototype and one signal read');
      const proxy = Proxy.revocable(() => {}, {});
      const promise = call(Observable.from([1]), proxy.proxy, {get signal() { proxy.revoke(); }});
      check(await rejected(promise) instanceof TypeError, name + ' callback revoked after conversion rejects');
      check(await rejected(call(Observable.from([1]), class Visitor {})) instanceof TypeError, name + ' class callback fails at invocation');
    });

    await test(name + ' invocation and booleans', async () => {
      let subscriber;
      const source = new Observable(s => { subscriber = s; }), calls = [];
      const predicate = new Proxy(function (...args) {
        check(this === undefined, name + ' strict callback this');
        calls.push(args); return continuing;
      }, {});
      const promise = call(source, predicate);
      subscriber.next('a'); subscriber.next('b');
      same(calls, [['a', 0], ['b', 1]], name + ' synchronous callback arguments');
      check(subscriber.active, name + ' undecided predicate keeps subscription');
      subscriber.complete();
      check(await promise === emptyResult, name + ' completion without a decision');
      check(await call(Observable.from([]), () => { throw 1; }) === emptyResult, name + ' empty result');
      let reads = 0;
      const poison = {get then() { reads++; throw 1; }, [Symbol.toPrimitive]() { reads++; throw 2; }};
      const revoked = Proxy.revocable({}, {}); revoked.revoke();
      const ignoredRejection = Promise.reject('predicate return'); ignoredRejection.catch(() => {});
      const token = {};
      for (const value of [undefined, null, false, 0, -0, 0n, NaN, '', true, 1, -1, 1n,
        'false', Symbol(), {}, [], new Boolean(false), poison, revoked.proxy, Promise.resolve(false), ignoredRejection]) {
        const result = await call(Observable.from([token]), () => value);
        check(result === (name === 'find' ? (Boolean(value) ? token : undefined) : Boolean(value)), name + ' predicate boolean conversion');
      }
      check(reads === 0, name + ' predicate results do not run conversions or then getters');
      const marker = {};
      check(await rejected(call(new Observable(s => s.error(marker)), () => continuing)) === marker, name + ' source error identity');
      check(await rejected(call(new Observable(() => { throw marker; }), () => continuing)) === marker, name + ' initializer error identity');
    });

    await test(name + ' early decision and order', async () => {
      let subscriber, teardowns = 0, visits = 0;
      const ac = new AbortController();
      const source = new Observable(s => {
        subscriber = s; s.addTeardown(() => teardowns++);
        s.next(1); s.next(2); s.next(3); s.complete();
      });
      const result = await call(source, value => { visits++; return value === 2 ? deciding : continuing; }, {signal: ac.signal});
      check(result === decidedResult(2) && visits === 2 && teardowns === 1 && !subscriber.active, name + ' decision closes own subscription synchronously');
      check(subscriber.signal.reason.name === 'AbortError' && !ac.signal.aborted, name + ' private controller default reason');
      const log = [], deferred = new Observable(s => { subscriber = s; });
      const promise = call(deferred, () => deciding).then(value => { log.push('resolve'); return value; });
      subscriber.signal.addEventListener('abort', () => { log.push('abort'); queueMicrotask(() => log.push('abort job')); });
      subscriber.addTeardown(() => { log.push('teardown'); queueMicrotask(() => log.push('teardown job')); });
      subscriber.next(7); log.push('after next');
      same(log, ['abort', 'teardown', 'after next'], name + ' synchronous decision order');
      check(await promise === decidedResult(7), name + ' early resolved value');
      same(log, ['abort', 'teardown', 'after next', 'resolve', 'abort job', 'teardown job'], name + ' resolution precedes abort microtasks');
    });

    await test(name + ' exceptions and cancellation', async () => {
      for (const marker of [{}, null, undefined]) {
        let subscriber, visits = 0, teardowns = 0;
        const promise = call(new Observable(s => {
          subscriber = s; s.addTeardown(() => teardowns++); s.next(1); s.next(2); s.complete();
        }), () => { visits++; throw marker; });
        check(await rejected(promise) === marker && visits === 1 && teardowns === 1 && !subscriber.active, name + ' predicate throw rejects and closes once');
        check(marker === undefined ? subscriber.signal.reason.name === 'AbortError' : subscriber.signal.reason === marker, name + ' thrown abort reason');
      }
      const reason = {}, preaborted = AbortSignal.abort(reason);
      let starts = 0, calls = 0, subscriber;
      const source = new Observable(s => { starts++; subscriber = s; });
      check(await rejected(call(source, () => calls++, {signal: preaborted})) === reason && starts === 0 && calls === 0, name + ' pre-abort skips source');
      const ac = new AbortController(), log = [];
      ac.signal.addEventListener('abort', () => { log.push('outer'); queueMicrotask(() => log.push('outer job')); });
      const promise = call(source, () => continuing, {signal: ac.signal}).catch(e => { log.push('reject'); return e; });
      subscriber.signal.addEventListener('abort', () => { log.push('inner'); queueMicrotask(() => log.push('inner job')); });
      subscriber.addTeardown(() => log.push('teardown'));
      ac.abort(reason);
      same(log, ['outer', 'inner', 'teardown'], name + ' dependent signal abort order');
      check(await promise === reason, name + ' caller abort identity');
      same(log, ['outer', 'inner', 'teardown', 'outer job', 'reject', 'inner job'], name + ' dependent signal microtask order');
      for (const throws of [false, true]) {
        const ac = new AbortController(), later = {};
        const promise = call(Observable.from([1]), () => { ac.abort(reason); if (throws) throw later; return deciding; }, {signal: ac.signal});
        check(await rejected(promise) === reason, name + ' caller abort wins before predicate finishes');
      }
    });

    await test(name + ' shared observers', async () => {
      for (const throws of [false, true]) {
        let subscriber, starts = 0, calls = 0;
        const source = new Observable(s => { starts++; subscriber = s; }), marker = {};
        const all = source.toArray();
        const promise = call(source, () => { calls++; if (throws) throw marker; return deciding; });
        const handled = throws ? rejected(promise) : promise;
        subscriber.next(1);
        check(subscriber.active && await handled === (throws ? marker : decidedResult(1)), name + ' removes only deciding observer');
        subscriber.next(2); subscriber.complete();
        same(await all, [1, 2], name + ' other observer continues');
        check(starts === 1 && calls === 1, name + ' shares producer and removes callback');
      }
    });

    await test(name + ' reentrancy and notification snapshots', async () => {
      let subscriber;
      const source = new Observable(s => { subscriber = s; }), indices = [];
      const promise = call(source, (value, index) => { indices.push(index); if (value === 1) subscriber.next(2); return continuing; });
      subscriber.next(1); subscriber.next(3); subscriber.complete();
      same(indices, [0, 0, 2], name + ' draft reentrant index order');
      check(await promise === emptyResult, name + ' nondeciding reentrancy completes');
      const nested = call(new Observable(s => { subscriber = s; }), value => { if (value === 1) subscriber.next(2); return deciding; });
      subscriber.next(1);
      check(await nested === decidedResult(2) && !subscriber.active, name + ' nested decision wins');
      for (const terminal of ['abort', 'complete', 'error']) {
        const ac = new AbortController(), marker = {}, received = [];
        const source = new Observable(s => { subscriber = s; });
        source.subscribe({error() {}, next(value) {
          if (value !== 2) return;
          if (terminal === 'abort') ac.abort(marker);
          else if (terminal === 'complete') subscriber.complete();
          else subscriber.error(marker);
        }}, {signal: ac.signal});
        const promise = call(source, (value, index) => { received.push([value, index]); return value === 1 ? continuing : deciding; }, {signal: ac.signal});
        const handled = terminal === 'complete' ? promise : rejected(promise);
        subscriber.next(1); subscriber.next(2);
        same(received, [[1, 0], [2, 1]], name + ' captured next still runs after ' + terminal);
        check(await handled === (terminal === 'complete' ? emptyResult : marker), name + ' snapshot cannot overwrite settlement');
        subscriber.complete();
      }
      const completed = call(new Observable(s => { subscriber = s; }), () => { subscriber.complete(); return deciding; });
      subscriber.next(1);
      check(await completed === emptyResult, name + ' completion inside predicate wins');
    });
  }

  await test('find final value resolution', async () => {
    for (const value of [undefined, null, false, 0, -0, NaN, 1n, Symbol(), {}]) {
      check(Object.is(await Observable.from([value]).find(() => true), value), 'find preserves matching value');
    }
    let subscriber, resolveValue, reads = 0, calls = 0;
    const ac = new AbortController(), source = new Observable(s => { subscriber = s; });
    const selected = {get then() {
      reads++; subscriber.next(99); subscriber.complete(); ac.abort('late');
      return resolve => { resolveValue = resolve; };
    }};
    const promise = source.find(() => { calls++; return true; }, {signal: ac.signal});
    subscriber.next(selected);
    check(reads === 1 && calls === 2 && !subscriber.active, 'find locks resolution before reentrant then getter');
    await Promise.resolve(); resolveValue(42);
    check(await promise === 42, 'find selected thenable wins over later values and abort');
    const marker = {}, bad = {get then() { throw marker; }};
    check(await rejected(Observable.from([bad]).find(() => true)) === marker, 'find selected then getter rejection identity');
  });

  await test('iterator close after predicate decision', async () => {
    const closeError = {}, marker = {}, errors = [];
    const onerror = e => { if (e.error === closeError) { errors.push(e.error); e.preventDefault(); } };
    addEventListener('error', onerror);
    try {
      for (const name of names) for (const throws of [false, true]) {
        let closes = 0;
        const iterator = {next: () => ({value: 7}), return() { closes++; throw closeError; }};
        const source = Observable.from({[Symbol.iterator]: () => iterator});
        const promise = source[name](() => { if (throws) throw marker; return name !== 'every'; });
        const result = await (throws ? rejected(promise) : promise);
        check(result === (throws ? marker : name === 'find' ? 7 : name === 'some') && closes === 1, name + ' close exception preserves decision');
      }
      check(errors.length === 6, 'iterator close errors reported once per predicate consumer');
    } finally { removeEventListener('error', onerror); }
  });

  await test('intrinsic predicate consumers', async () => {
    const saved = [globalThis.Promise, globalThis.AbortController, Observable.prototype.subscribe, AbortSignal.any];
    const poison = () => { throw new Error('public implementation consulted'); };
    const source = Observable.from([1, 2]), promises = [];
    try {
      globalThis.Promise = globalThis.AbortController = Observable.prototype.subscribe = AbortSignal.any = poison;
      for (const name of names) promises.push(source[name](() => name !== 'every'));
    } finally { [globalThis.Promise, globalThis.AbortController, Observable.prototype.subscribe, AbortSignal.any] = saved; }
    same(await Promise.all(promises), [true, false, 1], 'predicate consumers bypass replaced globals and subscribe');
  });
  return {checks, failures};
})()
