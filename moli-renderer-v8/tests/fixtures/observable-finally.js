(async () => {
  'use strict';
  const failures = [];
  let checks = 0;
  const check = (value, label) => { checks++; if (!value) failures.push(label); };
  const same = (a, b, label) => check(JSON.stringify(a) === JSON.stringify(b), label);
  const thrown = fn => { try { fn(); } catch (e) { return e; } };
  const test = async (label, fn) => { try { await fn(); } catch (e) { check(false, label + ': ' + e); } };
  const method = Observable.prototype.finally;
  check(typeof method === 'function', 'finally exposed');
  if (failures.length) return {checks, failures};

  await test('Web IDL conversion and native branding', async () => {
    const desc = Object.getOwnPropertyDescriptor(Observable.prototype, 'finally');
    check(method.name === 'finally' && method.length === 1, 'name and length');
    check(desc.enumerable && desc.writable && desc.configurable, 'descriptor');
    check(thrown(() => new method(() => {})) instanceof TypeError, 'not a constructor');
    const source = Observable.from([1]);
    let traps = 0;
    const revoked = Proxy.revocable(source, {}); revoked.revoke();
    for (const receiver of [undefined, null, false, 1, Symbol(), {}, Object.create(source),
      Object.create(Observable.prototype), new Proxy(source, {get() { traps++; }}), revoked.proxy]) {
      check(thrown(() => method.call(receiver, () => {})) instanceof TypeError, 'invalid receiver rejected');
    }
    check(traps === 0, 'receiver check does not invoke Proxy traps');
    check(thrown(() => source.finally()) instanceof TypeError, 'required callback');
    for (const callback of [undefined, null, false, 1, 1n, '', Symbol(), {}, [], {handleEvent() {}}]) {
      check(thrown(() => source.finally(callback)) instanceof TypeError, 'non-callable callback rejected');
    }
    let calls = 0;
    const callback = new Proxy(function() {
      calls++;
      check(this === undefined && arguments.length === 0, 'undefined receiver and zero arguments');
      return {get then() { throw 'return value assimilated'; }};
    }, {get() { throw 'callback property read'; }});
    Object.defineProperty(source, 'constructor', {get() { throw 'constructor read'; }});
    Object.setPrototypeOf(source, null);
    const result = method.call(source, callback, {get signal() { throw 'extra argument read'; }});
    check(calls === 0, 'creation is lazy');
    check(Object.getPrototypeOf(result) === Observable.prototype && Observable.from(result) === result, 'intrinsic branded result');
    same(await result.toArray(), [1], 'forwards without reading source prototype or species');
    same(await result.toArray(), [1], 'reusable result');
    check(calls === 2, 'one callback per subscription');
    class Derived extends Observable {}
    check(!(new Derived(s => s.complete()).finally(callback) instanceof Derived), 'does not use source species');
  });

  await test('termination order and original error identity', () => {
    const reports = [], onerror = e => { reports.push(e.error); e.preventDefault(); };
    addEventListener('error', onerror);
    try {
      for (const mode of ['complete', 'error', 'throw', 'abort', 'pre-abort']) {
        for (const fails of [false, true]) {
          const log = [], reason = {}, callbackError = {}, ac = new AbortController();
          let sourceSubscriber, error, completions = 0, next = 0, calls = 0;
          if (mode === 'pre-abort') ac.abort(reason);
          const source = new Observable(s => {
            sourceSubscriber = s;
            log.push('source');
            s.signal.addEventListener('abort', () => log.push('source abort'));
            s.addTeardown(() => log.push('source teardown 1'));
            s.addTeardown(() => log.push('source teardown 2'));
            s.next(reason);
            if (mode === 'complete') s.complete();
            if (mode === 'error') s.error(reason);
            if (mode === 'throw') throw reason;
          });
          source.finally(function() {
            calls++; log.push('finally');
            check(this === undefined && arguments.length === 0, mode + ' callback call shape');
            if (sourceSubscriber) check(!sourceSubscriber.active && sourceSubscriber.signal.aborted, mode + ' source already closed');
            if (fails) throw callbackError;
          }).subscribe({next: v => { next++; check(v === reason, 'value identity'); },
            error: e => { error = e; log.push('error'); }, complete: () => { completions++; log.push('complete'); }}, {signal: ac.signal});
          if (mode === 'abort') ac.abort(reason);
          ac.abort('again'); sourceSubscriber.complete();
          const terminal = mode === 'complete' ? ['complete'] : mode === 'error' || mode === 'throw' ? ['error'] : [];
          same(log, mode === 'pre-abort' ? ['finally', 'source', 'source teardown 1', 'source teardown 2'] :
            ['source', 'source abort', 'source teardown 2', 'source teardown 1', 'finally', ...terminal], mode + ' cleanup order');
          check(calls === 1 && next === (mode === 'pre-abort' ? 0 : 1), mode + ' exactly once');
          check(completions === (mode === 'complete' ? 1 : 0) && error === (terminal[0] === 'error' ? reason : undefined), mode + ' original terminal outcome');
          if (mode === 'abort' || mode === 'pre-abort') check(sourceSubscriber.signal.reason === reason, 'abort reason identity');
          check(reports.length === (fails ? 1 : 0) && (!fails || reports[0] === callbackError), mode + ' callback exception reported');
          reports.length = 0;
        }
      }
    } finally { removeEventListener('error', onerror); }
  });

  await test('shared consumers and independent branches', () => {
    let s, starts = 0, cleanup = 0, finalizers = 0;
    const a = new AbortController(), b = new AbortController(), direct = new AbortController();
    const source = new Observable(subscriber => { s = subscriber; starts++; s.addTeardown(() => cleanup++); });
    const result = source.finally(() => finalizers++), first = [], second = [];
    source.subscribe({}, {signal: direct.signal});
    result.subscribe(v => first.push(v), {signal: a.signal});
    result.subscribe(v => second.push(v), {signal: b.signal});
    s.next(1); a.abort('a'); s.next(2);
    check(starts === 1 && finalizers === 0 && cleanup === 0, 'removing one consumer keeps subscription');
    b.abort('b');
    check(finalizers === 1 && cleanup === 0 && s.active, 'finalizes branch while another source observer remains');
    same(first, [1], 'cancelled observer stops'); same(second, [1, 2], 'remaining observer receives values');
    result.subscribe();
    check(starts === 1, 'resubscription joins existing source');
    direct.abort(); s.complete();
    check(finalizers === 2 && cleanup === 1, 'resubscribed branch finalizes independently');
    let left = 0, right = 0;
    source.finally(() => left++).subscribe();
    source.finally(() => right++).subscribe();
    s.complete();
    check(left === 1 && right === 1 && starts === 2 && cleanup === 2, 'each derived Observable owns its finalizer');
  });

  await test('composition, short circuit and reentrant subscription', async () => {
    for (const mode of ['complete', 'error', 'abort']) {
      const log = [], ac = new AbortController(); let s;
      const source = new Observable(subscriber => { s = subscriber; s.addTeardown(() => log.push('source')); });
      source.finally(() => log.push('inner')).finally(() => log.push('outer'))
        .subscribe({complete: () => log.push('complete'), error: () => log.push('error')}, {signal: ac.signal});
      if (mode === 'abort') ac.abort(); else s[mode]('error');
      same(log, ['source', 'inner', 'outer', ...(mode === 'abort' ? [] : [mode])], mode + ' composition order');
    }
    const log = [];
    function* values() { try { yield 1; yield 2; } finally { log.push('iterator'); } }
    const value = await Observable.from(values()).finally(() => log.push('finally')).first();
    check(value === 1, 'first resolves original value'); same(log, ['iterator', 'finally'], 'short circuit closes iterator before finally');
    let starts = 0, finalizers = 0;
    const order = [];
    const result = new Observable(s => {
      const id = ++starts; order.push('start' + id); s.addTeardown(() => order.push('cleanup' + id)); s.complete();
    }).finally(() => {
      order.push('finally' + ++finalizers);
      if (finalizers === 1) result.subscribe({complete: () => order.push('complete2')});
    });
    result.subscribe({complete: () => order.push('complete1')});
    same(order, ['start1', 'cleanup1', 'finally1', 'start2', 'cleanup2', 'finally2', 'complete2', 'complete1'], 'reentrant finalizer starts a fresh subscription');
    check(starts === 2 && finalizers === 2, 'reentrancy does not reuse closed Subscriber');
  });

  await test('cancellation failures still finalize', () => {
    const marker = {}, callbackError = {}, log = [], reports = [], ac = new AbortController();
    const onerror = e => { reports.push(e.error); e.preventDefault(); };
    addEventListener('error', onerror);
    try {
      const iterable = {[Symbol.iterator]() { return {
        next() { return {value: 1, done: false}; },
        return() { log.push('return'); throw marker; }
      }; }};
      Observable.from(iterable).finally(() => { log.push('finally'); throw callbackError; }).subscribe(() => {
        log.push('next'); check(thrown(() => ac.abort()) === marker, 'IteratorClose failure propagates from explicit cancellation');
      }, {signal: ac.signal});
      same(log, ['next', 'return', 'finally'], 'finally still runs after failing IteratorClose');
      check(reports.length === 1 && reports[0] === callbackError, 'finally exception reported separately');
    } finally { removeEventListener('error', onerror); }
  });

  await test('internal steps ignore overridden public methods', async () => {
    const source = Observable.from([7]); let calls = 0;
    const saved = [];
    for (const [object, key] of [[Observable.prototype, 'subscribe'], [Subscriber.prototype, 'addTeardown'],
      [Subscriber.prototype, 'next'], [Subscriber.prototype, 'error'], [Subscriber.prototype, 'complete']]) {
      saved.push([object, key, Object.getOwnPropertyDescriptor(object, key)]);
      Object.defineProperty(object, key, {value() { throw key + ' invoked'; }, configurable: true});
    }
    try { same(await source.finally(() => calls++).toArray(), [7], 'native pass-through does not call author methods'); }
    finally { for (const [object, key, descriptor] of saved) Object.defineProperty(object, key, descriptor); }
    check(calls === 1, 'native teardown invoked once');
  });
  return {checks, failures};
})()
