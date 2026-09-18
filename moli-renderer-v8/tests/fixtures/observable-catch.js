(async () => {
  'use strict';
  const failures = []; let checks = 0;
  const check = (v, name) => { checks++; if (!v) failures.push(name); };
  const same = (a, b, name) => check(JSON.stringify(a) === JSON.stringify(b), name);
  const thrown = fn => { try { fn(); } catch (e) { return e; } };
  const test = async (name, fn) => { try { await fn(); } catch (e) { check(false, name + ': ' + e); } };
  const method = Observable.prototype.catch;
  check(typeof method === 'function', 'catch exposed');
  if (failures.length) return {checks, failures};
  const fail = error => new Observable(s => s.error(error));
  function subject() {
    let subscriber, starts = 0;
    return {source: new Observable(s => { subscriber = s; starts++; }),
      get subscriber() { return subscriber; }, get starts() { return starts; }};
  }

  await test('binding and intrinsic operations', async () => {
    const desc = Object.getOwnPropertyDescriptor(Observable.prototype, 'catch');
    check(method.length === 1 && method.name === 'catch', 'name and length');
    check(desc.enumerable && desc.writable && desc.configurable, 'descriptor');
    check(thrown(() => new method(() => [])) instanceof TypeError, 'not constructible');
    const source = fail(1), revoked = Proxy.revocable(source, {}); revoked.revoke(); let traps = 0;
    for (const receiver of [undefined, null, false, 1, Symbol(), {}, Object.create(source), Object.create(Observable.prototype),
      new Proxy(source, {get() { traps++; }}), revoked.proxy]) {
      check(thrown(() => method.call(receiver, () => [])) instanceof TypeError, 'invalid receiver');
    }
    check(traps === 0, 'native receiver check bypasses author traps');
    check(thrown(() => source.catch()) instanceof TypeError, 'required callback');
    for (const callback of [undefined, null, false, 1, 1n, '', Symbol(), {}, [], {handleEvent() {}}]) {
      check(thrown(() => source.catch(callback)) instanceof TypeError, 'non-callable callback');
    }
    let calls = 0;
    const callback = new Proxy(function(error) {
      calls++; check(this === undefined && arguments.length === 1 && error === 1, 'callback receiver and exact argument');
      return [2];
    }, {get() { throw 'callback property read'; }});
    Object.defineProperty(source, 'constructor', {get() { throw 'source constructor'; }});
    Object.setPrototypeOf(source, null);
    const result = method.call(source, callback, {get signal() { throw 'extra argument'; }});
    check(calls === 0, 'creation is lazy');
    check(Object.getPrototypeOf(result) === Observable.prototype && Observable.from(result) === result, 'intrinsic branded result');
    same(await result.toArray(), [2], 'native source ignores prototype and constructor');
    same(await result.toArray(), [2], 'fresh subscription can recover again');
    check(calls === 2, 'one catch per subscription');
    class Derived extends Observable {}
    check(!(new Derived(s => s.complete()).catch(callback) instanceof Derived), 'source species ignored');
    const saved = [];
    for (const [object, key] of [[Observable, 'from'], [Observable.prototype, 'subscribe'], [Subscriber.prototype, 'next'],
      [Subscriber.prototype, 'complete'], [Subscriber.prototype, 'error']]) {
      saved.push([object, key, Object.getOwnPropertyDescriptor(object, key)]);
      Object.defineProperty(object, key, {value() { throw key; }, configurable: true});
    }
    // A retained source Subscriber emits through its saved intrinsic method.
    const nativeError = saved.find(([object, key]) => object === Subscriber.prototype && key === 'error')[2].value;
    const primitiveSource = new Observable(s => nativeError.call(s, 1));
    try { same(await primitiveSource.catch(callback).toArray(), [2], 'internal conversion and subscription ignore public methods'); }
    finally { for (const [object, key, descriptor] of saved) Object.defineProperty(object, key, descriptor); }
  });

  await test('pass through and replacement conversion', async () => {
    let calls = 0;
    same(await Observable.from([1, 2]).catch(() => { calls++; return []; }).toArray(), [1, 2], 'normal values pass through');
    check(calls === 0, 'completion does not call catcher');
    const marker = {};
    for (const input of [Observable.from([marker]), [marker], new Set([marker]), Promise.resolve(marker),
      (async function* () { yield marker; })()]) {
      const values = await fail('source').catch(() => input).toArray();
      check(values.length === 1 && values[0] === marker, 'converts recovery result and preserves value identity');
    }
    for (const input of [undefined, null, false, 1, 1n, '', 'abc', Symbol(), {}, {then() { throw 'then'; }}]) {
      check(await fail('source').catch(() => input).toArray().catch(e => e) instanceof TypeError, 'invalid recovery result rejects downstream');
    }
    const log = [];
    same(await fail(1).catch(() => ({
      get [Symbol.asyncIterator]() { log.push('async'); return undefined; },
      get [Symbol.iterator]() { log.push('sync'); return function* () { log.push('open'); yield 2; }; },
      get then() { throw 'then getter'; }
    })).toArray(), [2], 'iterable conversion bypasses then property');
    same(log, ['async', 'sync', 'sync', 'open'], 'conversion probes precede obtaining iterator');
    const revoked = Proxy.revocable(() => [], {}); revoked.revoke();
    check(await fail(1).catch(revoked.proxy).toArray().catch(e => e) instanceof TypeError, 'revoked callable throws at invocation');
  });

  await test('cleanup precedes recovery and recovery errors are not caught twice', async () => {
    const marker = {}, log = [], outer = subject(), inner = subject(); let calls = 0;
    const promise = outer.source.finally(() => log.push('source finally')).catch(error => {
      calls++; check(error === marker && !outer.subscriber.active, 'catch sees original error after source closure');
      log.push('catch'); return inner.source;
    }).finally(() => log.push('result finally')).toArray();
    outer.subscriber.signal.addEventListener('abort', () => log.push('source abort'));
    outer.subscriber.addTeardown(() => log.push('source teardown'));
    outer.subscriber.next(1); outer.subscriber.error(marker);
    same(log, ['source abort', 'source teardown', 'source finally', 'catch'], 'source cleanup finishes before recovery callback');
    check(inner.subscriber.active && calls === 1, 'replacement remains active');
    inner.subscriber.addTeardown(() => log.push('inner teardown')); inner.subscriber.next(2); inner.subscriber.complete();
    same(await promise, [1, 2], 'preserves prefix and recovery values');
    same(log.slice(-2), ['inner teardown', 'result finally'], 'recovery cleanup precedes result finalizer');
    for (const mode of ['callback', 'conversion', 'initializer', 'inner']) for (const error of [{}, null, undefined]) {
      let recovered = 0; const errors = [], values = [];
      fail('original').catch(() => {
        recovered++;
        if (mode === 'callback') throw error;
        if (mode === 'conversion') return {get [Symbol.asyncIterator]() { throw error; }};
        if (mode === 'initializer') return new Observable(() => { throw error; });
        return new Observable(s => { s.next(1); s.error(error); });
      }).subscribe({next: v => values.push(v), error: e => errors.push(e), complete: () => values.push('complete')});
      check(errors.length === 1 && errors[0] === error, mode + ' preserves replacement error identity');
      check(recovered === 1, mode + ' does not invoke catcher recursively');
      same(values, mode === 'inner' ? [1] : [], mode + ' does not complete after error');
    }
    for (const error of [{}, null, undefined]) {
      let seen, count = 0;
      same(await new Observable(() => { throw error; }).catch(e => { seen = e; count++; return [3]; }).toArray(), [3], 'initializer errors recover');
      check(count === 1 && seen === error, 'initializer error identity reaches catcher');
    }
    let starts = 0, recovered = 0;
    const retry = new Observable(s => { starts++; s.next(starts); s.error(starts); });
    const events = [];
    retry.catch(() => { recovered++; return retry; }).subscribe({next: v => events.push(v), error: e => events.push('error' + e)});
    same(events, [1, 2, 'error2'], 'returning source retries once then forwards second error');
    check(starts === 2 && recovered === 1, 'no implicit retry loop');
  });

  await test('sharing, branches and later subscriptions', () => {
    const outer = subject(), inner = subject(), ac1 = new AbortController(), ac2 = new AbortController();
    const a = [], b = [], branch = []; let calls = 0, branchCalls = 0;
    const result = outer.source.catch(() => { calls++; return inner.source; });
    result.subscribe(v => a.push(v), {signal: ac1.signal}); result.subscribe(v => b.push(v), {signal: ac2.signal});
    outer.source.catch(() => { branchCalls++; return [9]; }).subscribe(v => branch.push(v));
    outer.subscriber.next(1); outer.subscriber.error('source');
    check(outer.starts === 1 && inner.starts === 1 && calls === 1 && branchCalls === 1, 'shared source and distinct catch branches');
    inner.subscriber.next(2); ac1.abort(); inner.subscriber.next(3);
    check(inner.subscriber.active, 'first consumer removal keeps recovery alive');
    same(a, [1, 2], 'first consumer removed'); same(b, [1, 2, 3], 'second consumer survives'); same(branch, [1, 9], 'other branch recovers separately');
    const reason = {}; ac2.abort(reason);
    check(!inner.subscriber.active && inner.subscriber.signal.reason === reason, 'last consumer cancels recovery with same reason');
    result.subscribe(); outer.subscriber.error('again');
    check(outer.starts === 2 && inner.starts === 2 && calls === 2, 'resubscription uses fresh source and replacement');
    inner.subscriber.complete();
  });

  await test('cancellation before recovery, during callback, and in conversion', () => {
    const pre = subject(); let calls = 0;
    const reason = {};
    pre.source.catch(() => { calls++; return []; }).subscribe({}, {signal: AbortSignal.abort(reason)});
    check(pre.starts === 1 && !pre.subscriber.active && pre.subscriber.signal.reason === reason && calls === 0, 'pre-aborted source initialized inactive');
    for (const phase of ['before', 'callback', 'conversion']) {
      const outer = subject(), ac = new AbortController(), log = []; let inactive, count = 0, iteratorCalls = 0;
      outer.source.catch(() => {
        count++;
        if (phase === 'callback') { ac.abort(reason); return new Observable(s => { inactive = s; }); }
        return {get [Symbol.asyncIterator]() { ac.abort(reason); return undefined; },
          [Symbol.iterator]() { iteratorCalls++; return [1][Symbol.iterator](); }};
      }).subscribe(v => log.push(v), {signal: ac.signal});
      if (phase === 'before') ac.abort(reason); else outer.subscriber.error('source');
      check(!outer.subscriber.active && log.length === 0, phase + ' cancellation suppresses output');
      check(count === (phase === 'before' ? 0 : 1), phase + ' catcher count');
      if (phase === 'callback') check(inactive && !inactive.active && inactive.signal.reason === reason, 'cancelled callback still initializes inactive replacement');
      if (phase === 'conversion') check(iteratorCalls === 0, 'cancelled conversion does not obtain an iterator');
    }
    const captured = subject(), ac = new AbortController(); let count = 0, inner;
    captured.source.subscribe({error: () => ac.abort(reason)});
    captured.source.catch(() => { count++; return new Observable(s => { inner = s; }); }).subscribe({}, {signal: ac.signal});
    captured.subscriber.error('captured');
    check(count === 1 && inner && !inner.active, 'captured error still calls catcher after earlier observer cancels');
  });

  await test('cancellation order and IteratorClose failures', () => {
    const log = [], ac = new AbortController(), reason = {}; let inner;
    ac.signal.addEventListener('abort', () => log.push('consumer abort'));
    fail('source').catch(() => new Observable(s => {
      inner = s; s.signal.addEventListener('abort', () => log.push('inner abort')); s.addTeardown(() => log.push('inner teardown'));
    })).finally(() => log.push('finally')).subscribe({}, {signal: ac.signal});
    ac.abort(reason);
    same(log, ['inner abort', 'inner teardown', 'finally', 'consumer abort'], 'nested cancellation algorithms precede outer abort event');
    check(inner.signal.reason === reason, 'replacement receives cancellation identity');
    for (const marker of [{}, undefined]) {
      const controller = new AbortController(), log = []; let pulls = 0, returns = 0, caught, didThrow = false;
      fail(1).catch(() => ({[Symbol.iterator]() { return {
        next() { return ++pulls < 3 ? {value: 1} : {done: true}; }, return() { returns++; log.push('return'); throw marker; }
      }; }})).finally(() => log.push('finally')).subscribe(() => {
        try { controller.abort(); } catch (e) { caught = e; didThrow = true; }
        log.push('after abort');
      }, {signal: controller.signal});
      check(didThrow && caught === marker, 'recovery IteratorClose exception propagates including undefined');
      check(pulls === 1 && returns === 1, 'cancelled recovery closes once without extra pulls');
      same(log, ['return', 'finally', 'after abort'], 'finalizer still runs before abort rethrows');
    }
  });

  await test('reentrancy and compositions', async () => {
    const outer = subject(), order = []; let calls = 0;
    const result = outer.source.catch(() => { calls++; order.push('catch'); return [2]; });
    const values = []; result.subscribe(v => values.push(v));
    outer.subscriber.addTeardown(() => {
      order.push('teardown'); result.subscribe(v => order.push('late' + v));
    });
    outer.subscriber.error('source');
    same(order, ['teardown', 'catch', 'late2'], 'reentrant result subscription joins recovery');
    same(values, [2], 'original consumer receives recovery'); check(calls === 1, 'reentrant join does not invoke callback twice');
    same(await Observable.from([1, 2, 3]).flatMap(v => v === 2 ? fail('two').catch(() => []) : [v]).toArray(), [1, 3], 'flatMap continues after recovered inner');
    same(await fail('a').catch(() => fail('b')).catch(e => [e]).toArray(), ['b'], 'outer catch can recover replacement errors');
    let resolve; const pending = new Promise(r => { resolve = r; }); const ac = new AbortController();
    const promise = fail('source').catch(() => pending).toArray({signal: ac.signal}); const outcome = promise.catch(e => e);
    const reason = {}; ac.abort(reason); resolve('late');
    check(await outcome === reason, 'promise recovery cancellation keeps consumer rejection');
    let source; const reports = [], cleanupError = {}, observerError = {}, onerror = e => { reports.push(e.error); e.preventDefault(); }; let caught = 0;
    addEventListener('error', onerror);
    try {
      new Observable(s => { source = s; }).catch(() => { caught++; return []; }).subscribe(() => { throw observerError; });
      source.addTeardown(() => { throw cleanupError; }); source.next(1); source.complete();
      check(caught === 0, 'observer and teardown exceptions do not trigger catch');
      check(reports.length === 2 && reports[0] === observerError && reports[1] === cleanupError, 'observer and teardown exceptions report separately');
    } finally { removeEventListener('error', onerror); }
  });
  return {checks, failures};
})()
