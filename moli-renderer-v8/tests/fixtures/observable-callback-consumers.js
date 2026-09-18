(async () => {
  'use strict';
  const failures = [];
  let checks = 0;
  const check = (value, label) => { checks++; if (!value) failures.push(label); };
  const same = (a, b, label) => check(JSON.stringify(a) === JSON.stringify(b), label);
  const rejected = p => p.then(() => { throw new Error('expected rejection'); }, e => e);
  const thrown = fn => { try { fn(); } catch (e) { return e; } };
  const test = async (label, fn) => { try { await fn(); } catch (e) { check(false, label + ': ' + e); } };
  const names = ['forEach', 'reduce'];
  for (const name of names) check(typeof Observable.prototype[name] === 'function', name + ' exposed');
  if (failures.length) return {checks, failures};

  for (const name of names) {
    const method = Observable.prototype[name];
    const call = (source, callback, options) => Reflect.apply(method, source,
      name === 'reduce' ? [callback, 10, options] : [callback, options]);
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

    await test(name + ' invocation', async () => {
      const log = [], marker = {}, returned = {get then() { throw marker; }};
      let subscriber;
      const source = new Observable(s => { subscriber = s; });
      const callback = new Proxy(function (...args) {
        check(this === undefined, name + ' strict callback this is undefined');
        log.push(args);
        return name === 'reduce' ? args[0] + args[1] : returned;
      }, {});
      const promise = call(source, callback);
      subscriber.next(1); subscriber.next(2);
      same(log, name === 'reduce' ? [[10, 1, 0], [11, 2, 1]] : [[1, 0], [2, 1]], name + ' synchronous arguments');
      check(subscriber.active, name + ' success keeps producer active');
      subscriber.complete();
      check(await promise === (name === 'reduce' ? 13 : undefined), name + ' completion result');
      check(await rejected(call(new Observable(s => s.error(marker)), () => {})) === marker, name + ' source error identity');
      check(await rejected(call(new Observable(() => { throw marker; }), () => {})) === marker, name + ' initializer error identity');
    });

    await test(name + ' callback errors', async () => {
      for (const marker of [{}, null, undefined]) {
        let subscriber, visits = 0, teardowns = 0;
        const source = new Observable(s => {
          subscriber = s; s.addTeardown(() => teardowns++);
          s.next(1); s.next(2); s.complete();
        });
        const error = await rejected(call(source, () => { visits++; throw marker; }));
        check(error === marker && visits === 1 && teardowns === 1 && !subscriber.active, name + ' callback throw rejects and cancels once');
        check(marker === undefined ? subscriber.signal.reason.name === 'AbortError' : subscriber.signal.reason === marker, name + ' callback throw abort reason');
      }
      const ac = new AbortController(), reason = {}, later = {};
      const promise = call(Observable.from([1]), () => { ac.abort(reason); throw later; }, {signal: ac.signal});
      check(await rejected(promise) === reason, name + ' reentrant caller abort wins before callback throw');
    });

    await test(name + ' cancellation timing', async () => {
      const reason = {}, preaborted = AbortSignal.abort(reason);
      let starts = 0, calls = 0, subscriber;
      const source = new Observable(s => { starts++; subscriber = s; });
      check(await rejected(call(source, () => calls++, {signal: preaborted})) === reason && starts === 0 && calls === 0, name + ' pre-abort skips producer');
      const ac = new AbortController(), log = [];
      ac.signal.addEventListener('abort', () => { log.push('outer'); queueMicrotask(() => log.push('outer job')); });
      const promise = call(source, () => {}, {signal: ac.signal}).catch(e => { log.push('reject'); return e; });
      subscriber.signal.addEventListener('abort', () => { log.push('inner'); queueMicrotask(() => log.push('inner job')); });
      subscriber.addTeardown(() => log.push('teardown'));
      ac.abort(reason);
      same(log, ['outer', 'inner', 'teardown'], name + ' dependent signal abort order');
      check(await promise === reason, name + ' caller reason identity');
      same(log, ['outer', 'inner', 'teardown', 'outer job', 'reject', 'inner job'], name + ' dependent signal microtask order');
    });

    await test(name + ' reentrant dispatch', async () => {
      let subscriber;
      const source = new Observable(s => { subscriber = s; }), indices = [], values = [];
      const promise = call(source, (...args) => {
        const value = args[name === 'reduce' ? 1 : 0], idx = args.at(-1);
        indices.push(idx); values.push(value);
        if (value === 1) subscriber.next(2);
        return name === 'reduce' ? args[0] + value : undefined;
      });
      subscriber.next(1); subscriber.next(3); subscriber.complete();
      same(values, [1, 2, 3], name + ' nested emission order');
      // The draft increments after callback invocation; Chromium currently
      // increments before it and returns [0, 1, 2] in this case.
      same(indices, [0, 0, 2], name + ' draft reentrant index order');
      check(await promise === (name === 'reduce' ? 14 : undefined), name + ' outer reducer result replaces nested result');
      for (const terminal of ['abort', 'complete']) {
        const ac = new AbortController(), marker = {}, received = [];
        const source = new Observable(s => { subscriber = s; });
        source.subscribe(() => { if (terminal === 'abort') ac.abort(marker); else subscriber.complete(); });
        const promise = call(source, (...args) => { received.push(args); return 99; }, {signal: ac.signal});
        const handled = terminal === 'abort' ? rejected(promise) : promise;
        subscriber.next(1);
        same(received, name === 'reduce' ? [[10, 1, 0]] : [[1, 0]], name + ' already captured next runs after ' + terminal);
        check(await handled === (terminal === 'abort' ? marker : name === 'reduce' ? 10 : undefined), name + ' snapshot cannot replace settled result');
        subscriber.complete();
      }
    });
  }

  await test('reduce seeds and raw results', async () => {
    const empty = Observable.from([]);
    check(await rejected(empty.reduce(() => {})) instanceof TypeError, 'empty reduction without seed rejects');
    check(await rejected(empty.reduce(() => {}, undefined)) instanceof TypeError, 'optional undefined seed is missing under Web IDL');
    check(await Observable.from([undefined]).reduce(() => { throw 1; }) === undefined, 'emitted undefined is a real accumulator');
    check(await empty.reduce(() => {}, null) === null, 'null is a real seed');
    const reason = {};
    check(await rejected(empty.reduce(() => {}, undefined, {signal: AbortSignal.abort(reason)})) === reason, 'optional missing seed still converts later options');
    const seed = {}, values = [];
    check(await empty.reduce(() => { throw 1; }, seed) === seed, 'empty reduction preserves seed identity');
    const sum = await Observable.from([1, 2, 3]).reduce((a, v, i) => { values.push([a, v, i]); return a + v; });
    check(sum === 6, 'seedless reduction result');
    same(values, [[1, 2, 1], [3, 3, 2]], 'first emitted value seeds without callback');
    const thenable = {get then() { throw new Error('intermediate result assimilated'); }};
    let calls = 0;
    check(await Observable.from([1, 2]).reduce((a, v) => { calls++; if (v === 1) return thenable; check(a === thenable, 'raw intermediate accumulator'); return 42; }, 0) === 42 && calls === 2, 'intermediate thenables not assimilated');
    let subscriber, reads = 0, resolveValue;
    const ac = new AbortController(), value = {get then() { reads++; ac.abort('late'); return resolve => { resolveValue = resolve; }; }};
    const promise = new Observable(s => { subscriber = s; }).reduce(() => value, 0, {signal: ac.signal});
    subscriber.next(1); check(reads === 0, 'final thenable not read before completion');
    subscriber.complete(); await Promise.resolve(); resolveValue(8);
    check(await promise === 8 && reads === 1, 'final assimilation locks before then getter abort');
    const ignored = Promise.reject('visitor promise'); ignored.catch(() => {});
    check(await Observable.from([1]).forEach(() => ignored) === undefined, 'visitor return Promise is ignored');
  });

  await test('shared subscription and iterator close', async () => {
    let subscriber;
    const source = new Observable(s => { subscriber = s; }), marker = {};
    const all = source.toArray(), completion = rejected(source.forEach(() => { throw marker; }));
    subscriber.next(1); check(subscriber.active && await completion === marker, 'callback error removes only its own observer');
    subscriber.next(2); subscriber.complete();
    same(await all, [1, 2], 'other observer continues after visitor error');
    const closeError = {}, errors = [];
    const onerror = e => { if (e.error === closeError) { errors.push(e.error); e.preventDefault(); } };
    addEventListener('error', onerror);
    try {
      for (const name of names) {
        const iterator = {next: () => ({value: 1}), return() { throw closeError; }};
        const source = Observable.from({[Symbol.iterator]: () => iterator});
        const promise = name === 'reduce' ? source.reduce(() => { throw marker; }, 0) : source.forEach(() => { throw marker; });
        check(await rejected(promise) === marker, name + ' close error cannot replace callback error');
      }
      check(errors.length === 2, 'iterator close errors reported once per consumer');
    } finally { removeEventListener('error', onerror); }
  });

  await test('intrinsic operations', async () => {
    const source = Observable.from([1, 2]), forEach = Observable.prototype.forEach, reduce = Observable.prototype.reduce;
    const saved = [globalThis.Promise, globalThis.AbortController, Observable.prototype.subscribe, AbortSignal.any];
    const poison = () => { throw new Error('public native implementation consulted'); };
    let each, reduced;
    try {
      globalThis.Promise = globalThis.AbortController = Observable.prototype.subscribe = AbortSignal.any = poison;
      each = forEach.call(source, () => {}); reduced = reduce.call(source, (a, v) => a + v, 0);
    } finally {
      [globalThis.Promise, globalThis.AbortController, Observable.prototype.subscribe, AbortSignal.any] = saved;
    }
    check(each instanceof Promise && await each === undefined, 'forEach bypasses mutable globals and subscribe');
    check(reduced instanceof Promise && await reduced === 3, 'reduce bypasses mutable globals and subscribe');
  });
  return {checks, failures};
})()
