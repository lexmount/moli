(async () => {
  'use strict';
  const failures = [];
  let checks = 0;
  const check = (value, label) => { checks++; if (!value) failures.push(label); };
  const same = (a, b, label) => check(JSON.stringify(a) === JSON.stringify(b), label);
  const thrown = fn => { try { fn(); } catch (e) { return e; } };
  const test = async (label, fn) => { try { await fn(); } catch (e) { check(false, label + ': ' + e); } };
  const method = Observable.prototype.switchMap;
  check(typeof method === 'function', 'switchMap exposed');
  if (failures.length) return {checks, failures};
  function subject() {
    let subscriber, starts = 0;
    return {source: new Observable(s => { subscriber = s; starts++; }),
      get subscriber() { return subscriber; }, get starts() { return starts; }};
  }

  await test('Web IDL conversion and native operations', async () => {
    const desc = Object.getOwnPropertyDescriptor(Observable.prototype, 'switchMap');
    check(method.name === 'switchMap' && method.length === 1, 'name and length');
    check(desc.enumerable && desc.writable && desc.configurable, 'descriptor');
    check(thrown(() => new method(() => [])) instanceof TypeError, 'not a constructor');
    const source = Observable.from([1]);
    let traps = 0;
    const revoked = Proxy.revocable(source, {}); revoked.revoke();
    for (const receiver of [undefined, null, false, 1, Symbol(), {}, Object.create(source), Object.create(Observable.prototype),
      new Proxy(source, {get() { traps++; }}), revoked.proxy]) {
      check(thrown(() => method.call(receiver, () => [])) instanceof TypeError, 'invalid receiver rejected');
    }
    check(traps === 0, 'receiver check does not invoke Proxy traps');
    check(thrown(() => source.switchMap()) instanceof TypeError, 'required mapper');
    for (const callback of [undefined, null, false, 1, 1n, '', Symbol(), {}, [], {handleEvent() {}}]) {
      check(thrown(() => source.switchMap(callback)) instanceof TypeError, 'non-callable mapper rejected');
    }
    let calls = 0;
    const mapper = new Proxy(function(value, index) {
      calls++;
      check(this === undefined && arguments.length === 2, 'mapper receiver and arguments');
      check(index === 0, 'fresh subscription resets index');
      return [value, value + 1];
    }, {get() { throw 'mapper property read'; }});
    Object.defineProperty(source, 'constructor', {get() { throw 'constructor read'; }});
    Object.setPrototypeOf(source, null);
    const result = method.call(source, mapper, {get signal() { throw 'extra argument read'; }});
    check(calls === 0, 'creation is lazy');
    check(Object.getPrototypeOf(result) === Observable.prototype && Observable.from(result) === result, 'intrinsic branded result');
    same(await result.toArray(), [1, 2], 'source prototype and species ignored');
    same(await result.toArray(), [1, 2], 'reusable result');
    check(calls === 2, 'one mapper call per source value');
    class Derived extends Observable {}
    check(!(new Derived(s => s.complete()).switchMap(mapper) instanceof Derived), 'does not use source species');
    const saved = [];
    for (const [object, key] of [[Observable, 'from'], [Observable.prototype, 'subscribe'],
      [AbortSignal, 'any'], [AbortController.prototype, 'abort'], [globalThis, 'AbortController'],
      [Subscriber.prototype, 'next'], [Subscriber.prototype, 'error'], [Subscriber.prototype, 'complete']]) {
      saved.push([object, key, Object.getOwnPropertyDescriptor(object, key)]);
      Object.defineProperty(object, key, {value() { throw key + ' invoked'; }, configurable: true});
    }
    try { same(await result.toArray(), [1, 2], 'internal conversion and notifications ignore public methods'); }
    finally { for (const [object, key, descriptor] of saved) Object.defineProperty(object, key, descriptor); }
  });

  await test('conversion of mapper results', async () => {
    for (const inner of [Observable.from([3]), [3], new Set([3]), Promise.resolve(3),
      (async function* () { yield 3; })()]) {
      same(await Observable.from([1]).switchMap(() => inner).toArray(), [3], 'maps convertible input');
    }
    for (const value of [undefined, null, false, 1, 1n, '', 'abc', Symbol(), {}, {then() { throw 'assimilated'; }}]) {
      check(await Observable.from([1]).switchMap(() => value).toArray().catch(e => e) instanceof TypeError, 'invalid result rejects through observer');
    }
    const marker = {}, log = [];
    const iterable = {get [Symbol.asyncIterator]() { log.push('async'); return undefined; },
      get [Symbol.iterator]() { log.push('sync'); return function* () { log.push('open'); yield marker; }; }};
    const values = await Observable.from([0]).switchMap(() => iterable).toArray();
    check(values.length === 1 && values[0] === marker, 'inner value identity');
    same(log, ['async', 'sync', 'sync', 'open'], 'conversion probes before subscription obtains iterator');
    let thenReads = 0;
    const preferred = {[Symbol.iterator]: function* () { yield 5; }, get then() { thenReads++; throw 'then read'; }};
    same(await Observable.from([0]).switchMap(() => preferred).toArray(), [5], 'iterable result bypasses then property');
    check(thenReads === 0, 'does not assimilate arbitrary thenables');
  });

  await test('switch cancellation and completion ordering', () => {
    const outer = subject(), inners = [], order = [], values = [], indices = [];
    const result = outer.source.switchMap((value, index) => {
      indices.push(index); order.push('map' + value);
      if (inners.length) check(!inners.at(-1).active, 'old inner closes before next mapper');
      return new Observable(s => {
        inners.push(s); order.push('start' + value);
        s.signal.addEventListener('abort', () => order.push('abort' + value));
        s.addTeardown(() => order.push('cleanup' + value)); s.next(value);
      });
    });
    result.subscribe({next: v => values.push(v), complete: () => order.push('complete')});
    outer.subscriber.next(1); outer.subscriber.next(2); inners[0].next('stale'); inners[0].complete();
    same(indices, [0, 1], 'indices begin at zero per subscription');
    same(values, [1, 2], 'cancelled inner no longer emits');
    check(inners[0].signal.reason instanceof DOMException && inners[0].signal.reason.name === 'AbortError', 'switch reason defaults to AbortError');
    same(order, ['map1', 'start1', 'abort1', 'cleanup1', 'map2', 'start2'], 'old abort and cleanup precede mapper');
    outer.subscriber.complete();
    check(inners[1].active && !order.includes('complete'), 'outer completion waits for current inner');
    inners[1].next(3); inners[1].complete();
    same(values, [1, 2, 3], 'last inner survives outer completion');
    same(order.slice(-3), ['abort2', 'cleanup2', 'complete'], 'final inner cleanup precedes downstream complete');
    const empty = subject(); let completes = 0;
    empty.source.switchMap(() => []).subscribe({complete: () => completes++});
    empty.subscriber.complete(); check(completes === 1, 'empty outer completes immediately');
    const sync = subject(); let syncCompletes = 0;
    sync.source.switchMap(v => [v]).subscribe({complete: () => syncCompletes++});
    sync.subscriber.next(1); check(syncCompletes === 0, 'completed inner waits for outer');
    sync.subscriber.complete(); check(syncCompletes === 1, 'completed inner releases completion gate');
  });

  await test('reentrant mapper and conversion use shared controller reference', () => {
    for (const mode of ['mapper', 'conversion']) {
      const outer = subject(), inners = {}, calls = [], values = [];
      outer.source.switchMap((value, index) => {
        calls.push([value, index]);
        const inner = new Observable(s => { inners[value] = s; });
        if (value === 1 && mode === 'mapper') outer.subscriber.next(2);
        if (value === 1 && mode === 'conversion') return {
          get [Symbol.asyncIterator]() { outer.subscriber.next(2); return undefined; },
          [Symbol.iterator]() { return [value][Symbol.iterator](); }
        };
        return inner;
      }).subscribe({next: v => values.push(v), complete: () => values.push('complete')});
      outer.subscriber.next(1);
      same(calls, mode === 'mapper' ? [[1, 0], [2, 0]] : [[1, 0], [2, 1]], mode + ' invocation/index update order');
      if (mode === 'mapper') {
        check(inners[1].active && inners[2].active, 'mapper reentrancy retains both inner observers');
        inners[1].next('one'); inners[2].next('two');
        outer.subscriber.next(3);
        check(!inners[1].active && !inners[2].active && inners[3].active, 'next switch cancels both observers sharing current controller');
        inners[3].complete();
      } else {
        check(inners[2].active, 'conversion reentrancy keeps earlier inner active');
        inners[2].next('two');
      }
      outer.subscriber.complete();
      same(values, mode === 'mapper' ? ['one', 'two', 'complete'] : [1, 'two', 'complete'], mode + ' output and completion');
      check(Object.values(inners).every(s => !s.active), mode + ' leaves no live inner after completion');
    }
  });

  await test('teardown and delivery reentrancy preserve all subscriptions', () => {
    const outer = subject(), inners = {}, log = [], values = [];
    outer.source.switchMap(v => new Observable(s => {
      inners[v] = s; log.push('start' + v);
      s.addTeardown(() => { log.push('close' + v); if (v === 1) outer.subscriber.next(3); });
    })).subscribe({next: v => values.push(v), complete: () => values.push('complete')});
    outer.subscriber.next(1); outer.subscriber.next(2);
    same(log, ['start1', 'close1', 'start3', 'start2'], 'reentrant teardown initializes before interrupted switch resumes');
    check(inners[2].active && inners[3].active, 'both reentrant inners stay live');
    inners[2].next(2); inners[3].next(3); outer.subscriber.complete();
    check(inners[2].active && inners[3].active, 'source completion still waits');
    inners[2].complete();
    same(values, [2, 3, 'complete'], 'inner completion closes remaining reentrant inner');
    check(!inners[3].active, 'dependent signal cancels reentrant inner');
    const delivery = subject(), seen = [], created = [];
    delivery.source.switchMap(v => new Observable(s => {
      created.push(s); s.next(v); if (v === 1) s.next('stale');
    })).subscribe(v => { seen.push(v); if (v === 1) delivery.subscriber.next(2); });
    delivery.subscriber.next(1);
    same(seen, [1, 2], 'delivery switches synchronously and suppresses later old notifications');
    check(!created[0].active && created[1].active, 'delivery cancels old inner before initializer resumes');
    delivery.subscriber.complete(); created[1].complete();
  });

  await test('asynchronous switching ignores superseded resolutions', async () => {
    const outer = subject(), resolvers = [], values = [];
    const result = outer.source.switchMap(() => new Promise(resolve => resolvers.push(resolve)));
    const promise = result.toArray();
    outer.subscriber.next(1); outer.subscriber.next(2); outer.subscriber.complete();
    resolvers[0]('old'); await Promise.resolve();
    promise.then(v => values.push(...v));
    check(values.length === 0, 'old promise does not complete result');
    resolvers[1]('new'); same(await promise, ['new'], 'only active Promise delivers');
    const asyncOuter = subject(), asyncLog = [], gates = [];
    const asyncResult = asyncOuter.source.switchMap(v => ({[Symbol.asyncIterator]() { return {
      next() { return new Promise(resolve => gates.push(resolve)); },
      return(reason) { asyncLog.push([v, reason.name]); return Promise.resolve({done: true}); }
    }; }}));
    const ac = new AbortController(); asyncResult.subscribe({}, {signal: ac.signal});
    asyncOuter.subscriber.next(1); asyncOuter.subscriber.next(2);
    check(asyncLog.length === 1 && asyncLog[0][0] === 1, 'switch closes async iterator');
    gates[0]({value: 'old'}); await Promise.resolve(); await Promise.resolve();
    check(gates.length === 2, 'cancelled async iterator is not pulled again');
    ac.abort(); check(asyncLog.length === 2 && asyncLog[1][0] === 2, 'cancel closes latest async iterator');
  });

  await test('switch only removes its own observer from a shared inner', () => {
    const outer = subject(), shared = subject(), keep = new AbortController(), values = [], other = [];
    shared.source.subscribe(v => other.push(v), {signal: keep.signal});
    outer.source.switchMap(v => v === 1 ? shared.source : [v]).subscribe(v => values.push(v));
    outer.subscriber.next(1); shared.subscriber.next('a'); outer.subscriber.next(2);
    check(shared.subscriber.active, 'other consumer keeps replaced producer alive');
    shared.subscriber.next('b'); same(values, ['a', 2], 'switched observer detached'); same(other, ['a', 'b'], 'other observer unaffected');
    keep.abort(); check(!shared.subscriber.active, 'last independent consumer closes producer');
    outer.subscriber.complete();
  });


  await test('sharing, distinct branches and last-consumer cancellation', () => {
    const outer = subject(), inner = subject(), ac1 = new AbortController(), ac2 = new AbortController();
    let maps = 0, otherMaps = 0;
    const result = outer.source.switchMap(() => { maps++; return inner.source; });
    const first = [], second = [];
    result.subscribe(v => first.push(v), {signal: ac1.signal});
    result.subscribe(v => second.push(v), {signal: ac2.signal});
    outer.source.switchMap(() => { otherMaps++; return []; }).subscribe();
    outer.subscriber.next(1); inner.subscriber.next(2); ac1.abort('first'); inner.subscriber.next(3);
    check(maps === 1 && otherMaps === 1 && outer.starts === 1 && inner.starts === 1, 'shared result maps once and distinct branch separately');
    check(outer.subscriber.active && inner.subscriber.active, 'first cancellation keeps both subscriptions');
    same(first, [2], 'first consumer removed'); same(second, [2, 3], 'second consumer survives');
    const reason = {}; ac2.abort(reason);
    check(outer.subscriber.active && !inner.subscriber.active && inner.subscriber.signal.reason === reason, 'last result consumer only cancels its branch');
    const oldInner = inner.subscriber;
    result.subscribe(); outer.subscriber.next(2);
    check(maps === 2 && otherMaps === 2 && inner.starts === 2 && inner.subscriber !== oldInner, 'resubscription uses fresh inner');
    inner.subscriber.complete(); outer.subscriber.complete();
  });

  await test('original errors, inner disposal and synchronous cancellation', () => {
    const reports = [], onerror = e => { reports.push(e.error); e.preventDefault(); };
    addEventListener('error', onerror);
    try {
      for (const mode of ['outer', 'inner', 'mapper', 'conversion', 'initializer']) {
        for (const marker of [{}, null, undefined]) {
          const outer = subject(), inner = subject(), log = [], errors = [];
          let maps = 0, complete = 0;
          outer.source.switchMap(() => {
            maps++;
            if (mode === 'mapper') throw marker;
            if (mode === 'conversion') return {get [Symbol.asyncIterator]() { throw marker; }};
            if (mode === 'initializer') return new Observable(() => { throw marker; });
            return inner.source;
          }).subscribe({error: e => { errors.push(e); log.push('error'); }, complete: () => complete++});
          outer.subscriber.addTeardown(() => log.push('outer'));
          outer.subscriber.next(1);
          if (inner.subscriber) {
            inner.subscriber.addTeardown(() => log.push('inner'));
            if (mode === 'outer') outer.subscriber.error(marker); else inner.subscriber.error(marker);
          }
          check(errors.length === 1 && errors[0] === marker && complete === 0, mode + ' error identity');
          check(!outer.subscriber.active && (!inner.subscriber || !inner.subscriber.active), mode + ' closes both subscriptions');
          check(maps === 1, mode + ' maps once');
          same(log, mode === 'outer' ? ['outer', 'inner', 'error'] : mode === 'inner' ? ['inner', 'outer', 'error'] : ['outer', 'error'], mode + ' cleanup precedes observer error');
        }
      }
      same(reports, [], 'handled errors are not reported globally');
      const outer = subject(), inner = subject(), ac = new AbortController(), reason = {}, cleanupError = {};
      let maps = 0;
      outer.source.switchMap(() => { maps++; return inner.source; }).subscribe({}, {signal: ac.signal});
      outer.subscriber.next(1);
      outer.subscriber.addTeardown(() => { throw cleanupError; });
      ac.abort(reason);
      check(!outer.subscriber.active && !inner.subscriber.active && maps === 1, 'explicit cancellation closes both subscriptions');
      check(outer.subscriber.signal.reason === reason && inner.subscriber.signal.reason === reason, 'both receive same cancellation reason');
      check(reports.length === 1 && reports[0] === cleanupError, 'teardown exception does not skip inner cancellation');
    } finally { removeEventListener('error', onerror); }
  });

  await test('throwing IteratorClose still cancels both producers', () => {
    for (const firstError of [{}, undefined]) {
      const secondError = {}, ac = new AbortController(), log = [], reason = {};
      let outerClosed = 0, innerClosed = 0, innerPulls = 0, caught, didThrow = false;
      const outer = {[Symbol.iterator]() { return {
        next() { return {value: 1}; },
        return() { outerClosed++; log.push('outer return'); throw firstError; }
      }; }};
      const inner = {[Symbol.iterator]() { return {
        next() { return ++innerPulls > 3 ? {done: true} : {value: innerPulls}; },
        return() { innerClosed++; log.push('inner return'); throw secondError; }
      }; }};
      Observable.from(outer).switchMap(() => inner).finally(() => log.push('finally')).subscribe(() => {
        try { ac.abort(reason); } catch (e) { didThrow = true; caught = e; }
        log.push('after abort');
      }, {signal: ac.signal});
      check(didThrow && caught === firstError, 'first IteratorClose failure preserved, including undefined');
      check(outerClosed === 1 && innerClosed === 1 && innerPulls === 1, 'both iterators close once before any extra pulls');
      same(log, ['outer return', 'inner return', 'finally', 'after abort'], 'cleanup and finalizer finish before abort rethrows');
    }
    const ac = new AbortController(), marker = {}, first = subject(), second = subject(), log = [];
    first.source.subscribe({}, {signal: ac.signal}); second.source.subscribe({}, {signal: ac.signal});
    Observable.from({[Symbol.iterator]() { return {
      next() { return {value: 1}; }, return() { throw marker; }
    }; }}).subscribe(() => {
      const third = new Observable(s => s.addTeardown(() => log.push('third')));
      third.subscribe({}, {signal: ac.signal});
      check(thrown(() => ac.abort()) === marker, 'shared direct signal preserves failing cancellation');
    }, {signal: ac.signal});
    check(!first.subscriber.active && !second.subscriber.active, 'earlier consumers also closed');
    same(log, ['third'], 'later signal algorithm runs after failure without switchMap');
  });

  await test('dependent cancellation finishes after source IteratorClose failure', () => {
    for (const marker of [{}, undefined]) {
      const ac = new AbortController(), log = [];
      let inner, caught, didThrow = false, pulls = 0;
      Observable.from({[Symbol.iterator]() { return {
        next() { return ++pulls === 1 ? {value: 1} : {done: true}; },
        return() { log.push('outer'); throw marker; }
      }; }}).subscribe(() => {
        new Observable(s => { inner = s; s.addTeardown(() => log.push('inner')); })
          .subscribe({}, {signal: AbortSignal.any([ac.signal])});
        try { ac.abort('stop'); } catch (e) { didThrow = true; caught = e; }
      }, {signal: ac.signal});
      check(didThrow && caught === marker, 'dependent dispatch preserves first exception including undefined');
      check(!inner.active && inner.signal.reason === 'stop', 'dependent observer is cancelled despite earlier failure');
      same(log, ['outer', 'inner'], 'dependent cleanup finishes before rethrow');
    }
  });

  await test('captured inner notification survives observer removal', () => {
    const outer = subject(), inner = subject(), keep = new AbortController(), seen = [];
    inner.source.subscribe(() => outer.subscriber.next(2), {signal: keep.signal});
    outer.source.switchMap(value => value === 1 ? inner.source : [value]).subscribe(v => seen.push(v));
    outer.subscriber.next(1); inner.subscriber.next('captured');
    same(seen, [2, 'captured'], 'already captured next steps still run after reentrant switch');
    keep.abort(); outer.subscriber.complete();
  });

  await test('pre-abort and cancellation inside mapper', () => {
    const ac = new AbortController(), reason = {}, pre = subject(); let maps = 0, inactive;
    pre.source.switchMap(() => { maps++; return []; }).subscribe({}, {signal: AbortSignal.abort(reason)});
    check(pre.starts === 1 && !pre.subscriber.active && pre.subscriber.signal.reason === reason && maps === 0, 'pre-aborted source initialized inactive without mapping');
    const outer = subject();
    outer.source.switchMap(() => { ac.abort(reason); return new Observable(s => { inactive = s; }); }).subscribe({}, {signal: ac.signal});
    outer.subscriber.next(1);
    check(!outer.subscriber.active && inactive && !inactive.active && inactive.signal.reason === reason, 'mapper cancellation still initializes returned Observable inactive');
    const captured = subject(), controller = new AbortController(); let calls = 0, inner;
    captured.source.subscribe(() => controller.abort(reason));
    captured.source.switchMap(() => { calls++; return new Observable(s => { inner = s; }); }).subscribe({}, {signal: controller.signal});
    captured.subscriber.next(1);
    check(calls === 1 && inner && !inner.active, 'captured next notification still maps after an earlier observer cancels');
    captured.subscriber.complete();
  });
  return {checks, failures};
})()
