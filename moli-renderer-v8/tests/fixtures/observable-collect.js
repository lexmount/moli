(async () => {
  'use strict';
  const failures = [];
  let checks = 0;
  const check = (value, label) => { checks++; if (!value) failures.push(label); };
  const same = (actual, expected, label) => check(JSON.stringify(actual) === JSON.stringify(expected), label);
  const thrown = fn => { try { fn(); } catch (error) { return error; } };
  const rejected = promise => promise.then(() => { throw new Error('expected rejection'); }, error => error);
  const test = async (label, fn) => { try { await fn(); } catch (error) { check(false, label + ': ' + error); } };
  const methods = ['last', 'toArray'];
  for (const name of methods) check(typeof Observable.prototype[name] === 'function', name + ' exposed');
  if (failures.length) return {checks, failures};

  for (const name of methods) {
    const method = Observable.prototype[name];
    await test(name + ' conversion', async () => {
      const descriptor = Object.getOwnPropertyDescriptor(Observable.prototype, name);
      check(method.name === name && method.length === 0, name + ' name and length');
      check(descriptor.enumerable && descriptor.writable && descriptor.configurable, name + ' descriptor');
      check(thrown(() => new method()) instanceof TypeError, name + ' not constructible');
      const source = Observable.from([1, 2]);
      let conversions = 0, traps = 0;
      const options = {get signal() { conversions++; }};
      const revoked = Proxy.revocable(source, {}); revoked.revoke();
      for (const receiver of [undefined, null, false, 1, 'x', Symbol(), {},
        Object.create(Observable.prototype), Object.create(source),
        new Proxy(source, {get() { traps++; throw 1; }}), revoked.proxy]) {
        const promise = method.call(receiver, options);
        check(promise instanceof Promise && await rejected(promise) instanceof TypeError, name + ' receiver rejection');
      }
      check(conversions === 0 && traps === 0, name + ' brand before conversions without Proxy traps');
      for (const options of [true, 1, 'x', Symbol(), {signal: null}, {signal: {}},
        {signal: new Proxy(new AbortController().signal, {})}]) {
        check(await rejected(method.call(source, options)) instanceof TypeError, name + ' invalid options rejection');
      }
      const marker = {};
      check(await rejected(method.call(source, {get signal() { throw marker; }})) === marker, name + ' getter error identity');
      Object.setPrototypeOf(source, null);
      for (const options of [undefined, null, {}, {signal: undefined}]) {
        const value = await method.call(source, options);
        check(name === 'last' ? value === 2 : value.length === 2 && value[1] === 2, name + ' genuine receiver with changed prototype');
      }
      await method.call(source, options);
      check(conversions === 1, name + ' signal read once');
    });

    await test(name + ' lifecycle', async () => {
      const log = [];
      let subscriber, settled = false;
      const source = new Observable(s => {
        subscriber = s;
        s.signal.addEventListener('abort', () => log.push('abort'));
        s.addTeardown(() => log.push('teardown'));
      });
      const promise = source[name]().then(value => { settled = true; log.push('resolved'); return value; });
      subscriber.next(1); subscriber.next(2);
      await Promise.resolve();
      check(!settled && subscriber.active, name + ' waits for completion');
      subscriber.complete();
      same(log, ['abort', 'teardown'], name + ' closes before resolving');
      const value = await promise;
      same(value, name === 'last' ? 2 : [1, 2], name + ' result');
      same(log, ['abort', 'teardown', 'resolved'], name + ' reaction timing');
      const error = {};
      check(await rejected(new Observable(s => { s.next(1); s.error(error); })[name]()) === error, name + ' error overrides retained values');
      check(await rejected(new Observable(() => { throw error; })[name]()) === error, name + ' initializer exception');
      for (const terminal of ['complete', 'error']) {
        const ac = new AbortController(), reason = {};
        const source = new Observable(s => { s.next(7); s.addTeardown(() => ac.abort(reason)); s[terminal](error); });
        check(await rejected(source[name]({signal: ac.signal})) === reason, name + ' teardown abort before ' + terminal);
      }
    });

    await test(name + ' direct abort ordering', async () => {
      let starts = 0, subscriber;
      const reason = {}, source = new Observable(s => { starts++; subscriber = s; });
      const log = [];
      const preaborted = source[name]({signal: AbortSignal.abort(reason)}).catch(e => { log.push('reject'); return e; });
      Promise.resolve().then(() => log.push('later'));
      check(await preaborted === reason && starts === 0, name + ' pre-abort skips subscription');
      same(log, ['reject', 'later'], name + ' pre-abort is immediately rejected');
      const ac = new AbortController();
      ac.signal.addEventListener('abort', () => { log.push('outer'); Promise.resolve().then(() => log.push('outer job')); });
      const pending = source[name]({signal: ac.signal}).catch(e => { log.push('reject'); return e; });
      subscriber.signal.addEventListener('abort', () => { log.push('inner'); Promise.resolve().then(() => log.push('inner job')); });
      subscriber.addTeardown(() => { log.push('teardown'); Promise.resolve().then(() => log.push('teardown job')); });
      log.length = 0;
      ac.abort(reason);
      same(log, ['inner', 'teardown', 'outer'], name + ' abort algorithms precede caller event');
      check(await pending === reason && !subscriber.active && subscriber.signal.reason === reason, name + ' abort preserves reason');
      same(log, ['inner', 'teardown', 'outer', 'reject', 'inner job', 'teardown job', 'outer job'], name + ' abort microtask order');
    });

    await test(name + ' snapshot reentrancy', async () => {
      let subscriber;
      const source = new Observable(s => { subscriber = s; });
      source.subscribe(() => subscriber.complete());
      const result = source[name]();
      const handled = name === 'last' ? rejected(result) : result;
      subscriber.next('after completion');
      const value = await handled;
      check(name === 'last' ? value instanceof RangeError : value.length === 0, name + ' stale next snapshot cannot change completed result');
    });
  }

  await test('last values and thenables', async () => {
    for (const value of [undefined, null, false, NaN, -0, 1n]) {
      check(Object.is(await Observable.from([value]).last(), value), 'last distinguishes emitted value from empty');
    }
    check(await rejected(Observable.from([]).last()) instanceof RangeError, 'empty last RangeError');
    const discarded = {get then() { throw new Error('discarded value inspected'); }};
    check(await Observable.from([discarded, 3]).last() === 3, 'last never assimilates overwritten values');
    let subscriber, reads = 0, resolveValue;
    const ac = new AbortController(), source = new Observable(s => { subscriber = s; });
    const promise = source.last({signal: ac.signal});
    const value = {get then() { reads++; ac.abort('too late'); return resolve => { resolveValue = resolve; }; }};
    subscriber.next(value);
    check(reads === 0, 'last then getter waits until completion');
    subscriber.complete();
    check(reads === 1 && !subscriber.active, 'last assimilates after source closure');
    await Promise.resolve(); resolveValue(41);
    check(await promise === 41, 'last locks result before reentrant then getter abort');
  });

  await test('arrays preserve values and own properties', async () => {
    const marker = {}, thenable = {get then() { throw marker; }}, promise = Promise.resolve(9), symbol = Symbol();
    const values = [thenable, promise, undefined, null, symbol, 7n];
    const result = await Observable.from(values).toArray();
    check(result.length === values.length && result.every((value, i) => value === values[i]), 'toArray does not assimilate individual values');
    check(Object.getPrototypeOf(result) === Array.prototype, 'toArray result uses intrinsic Array prototype');
    const index = Object.getOwnPropertyDescriptor(result, '0');
    check(index.writable && index.enumerable && index.configurable, 'result element is own data property');
    const source = Observable.from([]), a = await source.toArray(), b = await source.toArray();
    check(a.length === 0 && b.length === 0 && a !== b, 'each empty subscription returns a fresh array');
    let subscriber, reads = 0;
    const ac = new AbortController(), pending = new Observable(s => { subscriber = s; }).toArray({signal: ac.signal});
    const saved = Object.getOwnPropertyDescriptor(Array.prototype, 'then');
    let resultArray;
    try {
      Object.defineProperty(Array.prototype, 'then', {configurable: true, get() { reads++; resultArray = this; ac.abort('too late'); }});
      subscriber.next(1); subscriber.next(2);
      check(reads === 0, 'private collection does not read then');
      subscriber.complete();
    } finally {
      if (saved) Object.defineProperty(Array.prototype, 'then', saved); else delete Array.prototype.then;
    }
    check(await pending === resultArray && reads === 1 && resultArray.length === 2, 'result array assimilation locks before caller abort');
  });

  await test('shared producer', async () => {
    let starts = 0, subscriber;
    const source = new Observable(s => { starts++; subscriber = s; });
    const ac = new AbortController(), last = rejected(source.last({signal: ac.signal})), all = source.toArray(), first = source.first();
    subscriber.next(1);
    check(subscriber.active && starts === 1 && await first === 1, 'first leaves collecting observers active');
    subscriber.next(2); ac.abort('cancel last');
    check(await last === 'cancel last' && subscriber.active, 'last cancellation preserves other observers');
    subscriber.next(3); subscriber.complete();
    same(await all, [1, 2, 3], 'toArray keeps collecting shared values');
  });

  await test('native intrinsics', async () => {
    for (const name of methods) {
      const source = Observable.from([1, 2]), method = Observable.prototype[name];
      const globals = ['Promise', 'Array', 'Observable'], saved = globals.map(key => globalThis[key]);
      const targets = [[Observable.prototype, 'subscribe'], [Array.prototype, 'push'], [Array.prototype, '0']];
      const descriptors = targets.map(([object, key]) => Object.getOwnPropertyDescriptor(object, key));
      const poison = () => { throw new Error('author method or setter invoked'); };
      let promise;
      try {
        globals.forEach(key => { globalThis[key] = poison; });
        targets.forEach(([object, key]) => Object.defineProperty(object, key, {configurable: true, set: poison}));
        promise = method.call(source);
      } finally {
        globals.forEach((key, i) => { globalThis[key] = saved[i]; });
        targets.forEach(([object, key], i) => {
          if (descriptors[i]) Object.defineProperty(object, key, descriptors[i]); else delete object[key];
        });
      }
      check(promise instanceof Promise, name + ' intrinsic Promise');
      same(await promise, name === 'last' ? 2 : [1, 2], name + ' bypasses public subscription and inherited setters');
    }
  });
  return {checks, failures};
})()
