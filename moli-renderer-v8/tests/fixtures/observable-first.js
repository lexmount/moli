(async () => {
  'use strict';
  const failures = [];
  let checks = 0;
  const check = (value, label) => { checks++; if (!value) failures.push(label); };
  const same = (actual, expected, label) => check(JSON.stringify(actual) === JSON.stringify(expected), label);
  const thrown = fn => { try { fn(); } catch (error) { return error; } };
  const rejected = promise => promise.then(() => { throw new Error('expected rejection'); }, error => error);
  const test = async (label, fn) => { try { await fn(); } catch (error) { check(false, label + ': ' + error); } };
  const first = Observable.prototype.first;
  if (typeof first !== 'function') {
    check(false, 'Observable.first is exposed');
    return {checks, failures};
  }

  await test('receiver and conversion', async () => {
    const descriptor = Object.getOwnPropertyDescriptor(Observable.prototype, 'first');
    check(first.name === 'first' && first.length === 0, 'name and length');
    check(descriptor.enumerable && descriptor.writable && descriptor.configurable, 'method descriptor');
    check(thrown(() => new first()) instanceof TypeError, 'not constructible');
    const source = new Observable(subscriber => subscriber.next(7));
    let reads = 0, traps = 0;
    const options = {get signal() { reads++; return undefined; }};
    const revoked = Proxy.revocable(source, {}); revoked.revoke();
    const forged = [undefined, null, false, 1, 'x', Symbol(), {}, Object.create(Observable.prototype),
      Object.create(source), new Proxy(source, {get() { traps++; throw 1; }}), revoked.proxy];
    for (const receiver of forged) {
      const promise = first.call(receiver, options);
      check(promise instanceof Promise && await rejected(promise) instanceof TypeError, 'invalid receiver rejects a Promise');
    }
    check(reads === 0 && traps === 0, 'brand check precedes option conversion and does not inspect proxies');
    Object.setPrototypeOf(source, null);
    check(await first.call(source) === 7, 'native brand survives prototype replacement');
    for (const value of [1, true, 'x', Symbol(), 1n]) {
      check(await rejected(first.call(source, value)) instanceof TypeError, 'non-object options reject');
    }
    for (const value of [null, undefined, {}, {signal: undefined}]) {
      check(await first.call(source, value) === 7, 'empty dictionary accepted');
    }
    const ac = new AbortController();
    for (const signal of [null, {}, Object.create(AbortSignal.prototype), new Proxy(ac.signal, {})]) {
      check(await rejected(first.call(source, {signal})) instanceof TypeError, 'invalid signal rejects');
    }
    const marker = {};
    check(await rejected(first.call(source, {get signal() { throw marker; }})) === marker, 'option getter rejection identity');
    check(await first.call(source, options) === 7 && reads === 1, 'signal read exactly once');
  });

  await test('lifecycle', async () => {
    const log = [], value = {};
    let subscriber;
    const source = new Observable(s => {
      subscriber = s;
      s.signal.addEventListener('abort', () => log.push('abort'));
      s.addTeardown(() => log.push('teardown'));
      log.push('before'); s.next(value);
      log.push(s.active ? 'active' : 'inactive');
      s.next('ignored'); s.complete();
    });
    const promise = source.first().then(result => { log.push('resolved'); return result; });
    same(log, ['before', 'abort', 'teardown', 'inactive'], 'synchronous cancellation order');
    check(subscriber.signal.aborted && subscriber.signal.reason.name === 'AbortError', 'upstream receives default abort reason');
    check(await promise === value, 'first value identity');
    same(log, ['before', 'abort', 'teardown', 'inactive', 'resolved'], 'Promise reaction runs after teardown');
    check(await rejected(new Observable(s => s.complete()).first()) instanceof RangeError, 'empty source rejects with RangeError');
    const marker = {};
    check(await rejected(new Observable(s => s.error(marker)).first()) === marker, 'source error identity');
    check(await rejected(new Observable(() => { throw marker; }).first()) === marker, 'initializer exception identity');
  });

  await test('abort and sharing', async () => {
    const marker = {}, ac = new AbortController(), log = [];
    let subscriber, starts = 0;
    const source = new Observable(s => {
      starts++; subscriber = s;
      s.addTeardown(() => log.push('teardown'));
    });
    const aborted = AbortSignal.abort(marker);
    check(await rejected(source.first({signal: aborted})) === marker && starts === 0, 'pre-abort skips initializer');
    const promise = source.first({signal: ac.signal});
    const rejection = rejected(promise);
    ac.abort(marker);
    check(!subscriber.active && subscriber.signal.reason === marker, 'input abort closes upstream with same reason');
    same(log, ['teardown'], 'input abort runs teardown once');
    check(await rejection === marker, 'input abort rejects with reason identity');
    const values = [], shared = new AbortController();
    source.subscribe(value => values.push(value), {signal: shared.signal});
    check(await rejected(source.first({signal: aborted})) === marker && subscriber.active, 'pre-aborted first leaves shared producer active');
    const firstValue = source.first();
    subscriber.next(1);
    check(subscriber.active && starts === 2, 'first only removes its observer from a shared subscription');
    subscriber.next(2);
    check(await firstValue === 1, 'shared first value');
    same(values, [1, 2], 'other observer continues receiving');
    shared.abort();
    check(!subscriber.active && log.length === 2, 'last observer cancellation closes shared producer');
    for (const terminal of ['complete', 'error']) {
      const duringTeardown = new AbortController(), reason = {};
      const source = new Observable(s => {
        s.addTeardown(() => duringTeardown.abort(reason));
        s[terminal]('source error');
      });
      check(await rejected(source.first({signal: duringTeardown.signal})) === reason, 'teardown abort wins before ' + terminal + ' notification');
    }
  });

  await test('thenable reentrancy', async () => {
    for (const reenter of ['next', 'complete', 'abort']) {
      const ac = new AbortController(), log = [], marker = {};
      let subscriber, thenReads = 0, secondReads = 0;
      const source = new Observable(s => { subscriber = s; s.addTeardown(() => log.push('teardown')); });
      const promise = source.first({signal: ac.signal});
      const value = {get then() {
        thenReads++; log.push('get then');
        if (reenter === 'next') subscriber.next({get then() { secondReads++; }});
        if (reenter === 'complete') subscriber.complete();
        if (reenter === 'abort') ac.abort(marker);
        check(!subscriber.active, 'reentrant ' + reenter + ' cancels synchronously');
        return resolve => { log.push('then'); resolve(42); };
      }};
      subscriber.next(value);
      check(thenReads === 1 && secondReads === 0, 'first resolve locks before then getter reentrancy');
      check(await promise === 42, 'thenable result survives reentrant ' + reenter);
      same(log, ['get then', 'teardown', 'then'], 'thenable cancellation order for ' + reenter);
    }
    let subscriber, resolveValue;
    const ac = new AbortController(), value = new Promise(resolve => { resolveValue = resolve; });
    const promise = new Observable(s => { subscriber = s; }).first({signal: ac.signal});
    subscriber.next(value);
    ac.abort('too late'); resolveValue(9);
    check(await promise === 9, 'late input abort cannot replace pending assimilation');
    const marker = {};
    let closed = false;
    const throwing = new Observable(s => {
      s.addTeardown(() => { closed = true; });
      s.next({get then() { throw marker; }});
    });
    check(await rejected(throwing.first()) === marker && closed, 'throwing then getter still cancels source');
  });

  await test('dependent signal ordering', async () => {
    const ac = new AbortController(), reason = {}, value = {};
    let subscriber;
    const promise = new Observable(s => { subscriber = s; }).first({signal: ac.signal});
    ac.signal.addEventListener('abort', () => subscriber.next(value));
    ac.abort(reason);
    check(await promise === value, 'source abort event precedes dependent signal algorithms');
    check(!subscriber.active, 'dependent abort still removes observer');
  });

  await test('iterable cancellation', async () => {
    for (const symbol of [Symbol.iterator, Symbol.asyncIterator]) {
      let pulls = 0, returns = 0;
      const iterator = {next() { pulls++; return {value: 8}; }, return() { returns++; return {}; }};
      check(await Observable.from({[symbol]: () => iterator}).first() === 8, 'first from iterable ' + String(symbol));
      check(pulls === 1 && returns === 1, 'one pull and one close ' + String(symbol));
    }
    const marker = {}, errors = [];
    const onerror = event => { if (event.error === marker) { errors.push(event.error); event.preventDefault(); } };
    addEventListener('error', onerror);
    try {
      const iterator = {next: () => ({value: 5}), return() { throw marker; }};
      check(await Observable.from({[Symbol.iterator]: () => iterator}).first() === 5, 'close exception cannot replace resolved value');
      check(errors.length === 1 && errors[0] === marker, 'close exception reported globally once');
    } finally { removeEventListener('error', onerror); }
  });

  await test('intrinsics and internal subscription', async () => {
    const source = Observable.from([17]);
    const globals = ['Observable', 'Promise', 'AbortController'];
    const saved = globals.map(name => globalThis[name]);
    const methods = [[Observable.prototype, 'subscribe'], [AbortSignal, 'any'],
      [Subscriber.prototype, 'next'], [Subscriber.prototype, 'error'], [Subscriber.prototype, 'complete']];
    const descriptors = methods.map(([object, name]) => Object.getOwnPropertyDescriptor(object, name));
    const poison = () => { throw new Error('public implementation consulted'); };
    let promise;
    try {
      globals.forEach(name => { globalThis[name] = poison; });
      methods.forEach(([object, name]) => { object[name] = poison; });
      promise = first.call(source);
    } finally {
      globals.forEach((name, index) => { globalThis[name] = saved[index]; });
      methods.forEach(([object, name], index) => Object.defineProperty(object, name, descriptors[index]));
    }
    check(promise instanceof Promise && await promise === 17, 'native first ignores replaced public constructors and methods');
  });
  return {checks, failures};
})()
