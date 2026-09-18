(async () => {
  'use strict';
  const failures = [];
  let checks = 0;
  const check = (value, label) => { checks++; if (!value) failures.push(label); };
  const same = (a, b, label) => check(JSON.stringify(a) === JSON.stringify(b), label);
  const thrown = fn => { try { fn(); } catch (e) { return e; } };
  const test = async (label, fn) => { try { await fn(); } catch (e) { check(false, label + ': ' + e); } };
  const method = Observable.prototype.flatMap;
  check(typeof method === 'function', 'flatMap exposed');
  if (failures.length) return {checks, failures};
  function subject() {
    let subscriber, starts = 0;
    return {source: new Observable(s => { subscriber = s; starts++; }),
      get subscriber() { return subscriber; }, get starts() { return starts; }};
  }

  await test('Web IDL conversion and native operations', async () => {
    const desc = Object.getOwnPropertyDescriptor(Observable.prototype, 'flatMap');
    check(method.name === 'flatMap' && method.length === 1, 'name and length');
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
    check(thrown(() => source.flatMap()) instanceof TypeError, 'required mapper');
    for (const callback of [undefined, null, false, 1, 1n, '', Symbol(), {}, [], {handleEvent() {}}]) {
      check(thrown(() => source.flatMap(callback)) instanceof TypeError, 'non-callable mapper rejected');
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
    check(!(new Derived(s => s.complete()).flatMap(mapper) instanceof Derived), 'does not use source species');
    const saved = [];
    for (const [object, key] of [[Observable, 'from'], [Observable.prototype, 'subscribe'],
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
      same(await Observable.from([1]).flatMap(() => inner).toArray(), [3], 'maps convertible input');
    }
    for (const value of [undefined, null, false, 1, 1n, '', 'abc', Symbol(), {}, {then() { throw 'assimilated'; }}]) {
      check(await Observable.from([1]).flatMap(() => value).toArray().catch(e => e) instanceof TypeError, 'invalid result rejects through observer');
    }
    const marker = {}, log = [];
    const iterable = {get [Symbol.asyncIterator]() { log.push('async'); return undefined; },
      get [Symbol.iterator]() { log.push('sync'); return function* () { log.push('open'); yield marker; }; }};
    const values = await Observable.from([0]).flatMap(() => iterable).toArray();
    check(values.length === 1 && values[0] === marker, 'inner value identity');
    same(log, ['async', 'sync', 'sync', 'open'], 'conversion probes before subscription obtains iterator');
    let thenReads = 0;
    const preferred = {[Symbol.iterator]: function* () { yield 5; }, get then() { thenReads++; throw 'then read'; }};
    same(await Observable.from([0]).flatMap(() => preferred).toArray(), [5], 'iterable result bypasses then property');
    check(thenReads === 0, 'does not assimilate arbitrary thenables');
  });

  await test('raw queue, delayed mapper and synchronous completion order', () => {
    const outer = subject(), inners = [], log = [], indices = [];
    const result = outer.source.flatMap((value, index) => {
      const id = value.id;
      indices.push(index); log.push('map' + id);
      return new Observable(s => {
        inners.push(s); log.push('start' + id); s.addTeardown(() => log.push('cleanup' + id)); s.next(id);
        if (id > 1) { s.complete(); log.push('after' + id); }
      });
    });
    const values = [];
    result.subscribe({next: v => values.push(v), complete: () => log.push('complete')});
    const queued = {id: 20};
    outer.subscriber.next({id: 1}); outer.subscriber.next(queued); outer.subscriber.next({id: 3});
    same(log, ['map1', 'start1'], 'queued values do not invoke mapper early');
    queued.id = 2; outer.subscriber.complete();
    check(inners.length === 1 && inners[0].active, 'outer completion waits for active inner');
    inners[0].complete(); log.push('after1');
    same(values, [1, 2, 3], 'queue retains raw value identity');
    same(indices, [0, 1, 2], 'serial mapper indices');
    same(log, ['map1', 'start1', 'cleanup1', 'map2', 'start2', 'cleanup2', 'map3', 'start3', 'cleanup3',
      'complete', 'after3', 'after2', 'after1'], 'drains next inner before previous complete returns');
  });

  await test('reentrant mapper, conversion and inner delivery', async () => {
    const outer = subject(), mapped = [], values = [];
    outer.source.flatMap((value, index) => {
      mapped.push([value, index]);
      if (value === 1) outer.subscriber.next(2);
      return {[Symbol.iterator]() {
        if (value === 1) outer.subscriber.next(3);
        return [value][Symbol.iterator]();
      }};
    }).subscribe(value => { values.push(value); if (value === 1) outer.subscriber.next(4); });
    outer.subscriber.next(1); outer.subscriber.complete();
    same(mapped, [[1, 0], [2, 1], [3, 2], [4, 3]], 'reentrant pushes wait until mapper and inner finish');
    same(values, [1, 2, 3, 4], 'reentrant output order');
    const pending = subject(), gate = subject(), queued = [];
    const result = pending.source.flatMap(value => { queued.push(value); return value === 0 ? gate.source : [value]; });
    const promise = result.toArray();
    for (let i = 0; i < 128; i++) pending.subscriber.next(i);
    pending.subscriber.complete();
    same(queued, [0], 'large queue maps lazily');
    gate.subscriber.next(0); gate.subscriber.complete();
    const collected = await promise;
    check(collected.length === 128 && collected.every((v, i) => v === i), 'drains buffered synchronous inputs in FIFO order');
  });

  await test('sharing, distinct branches and last-consumer cancellation', () => {
    const outer = subject(), inner = subject(), ac1 = new AbortController(), ac2 = new AbortController();
    let maps = 0, otherMaps = 0;
    const result = outer.source.flatMap(() => { maps++; return inner.source; });
    const first = [], second = [];
    result.subscribe(v => first.push(v), {signal: ac1.signal});
    result.subscribe(v => second.push(v), {signal: ac2.signal});
    outer.source.flatMap(() => { otherMaps++; return []; }).subscribe();
    outer.subscriber.next(1); inner.subscriber.next(2); ac1.abort('first'); inner.subscriber.next(3);
    check(maps === 1 && otherMaps === 1 && outer.starts === 1 && inner.starts === 1, 'shared result maps once and distinct branch separately');
    check(outer.subscriber.active && inner.subscriber.active, 'first cancellation keeps both subscriptions');
    same(first, [2], 'first consumer removed'); same(second, [2, 3], 'second consumer survives');
    const reason = {}; ac2.abort(reason);
    check(outer.subscriber.active && !inner.subscriber.active && inner.subscriber.signal.reason === reason, 'last result consumer only cancels its branch');
    const oldInner = inner.subscriber;
    result.subscribe(); outer.subscriber.next(2);
    check(maps === 2 && otherMaps === 2 && inner.starts === 2 && inner.subscriber !== oldInner, 'resubscription uses fresh queue and inner');
    inner.subscriber.complete(); outer.subscriber.complete();
  });

  await test('original errors, queue disposal and synchronous cancellation', () => {
    const reports = [], onerror = e => { reports.push(e.error); e.preventDefault(); };
    addEventListener('error', onerror);
    try {
      for (const mode of ['outer', 'inner', 'mapper', 'conversion', 'initializer']) {
        for (const marker of [{}, null, undefined]) {
          const outer = subject(), inner = subject(), log = [], errors = [];
          let maps = 0, complete = 0;
          outer.source.flatMap(() => {
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
            outer.subscriber.next(2);
            if (mode === 'outer') outer.subscriber.error(marker); else inner.subscriber.error(marker);
          }
          check(errors.length === 1 && errors[0] === marker && complete === 0, mode + ' error identity');
          check(!outer.subscriber.active && (!inner.subscriber || !inner.subscriber.active), mode + ' closes both subscriptions');
          check(maps === 1, mode + ' abandons queued values');
          same(log, mode === 'outer' ? ['outer', 'inner', 'error'] : mode === 'inner' ? ['inner', 'outer', 'error'] : ['outer', 'error'], mode + ' cleanup precedes observer error');
        }
      }
      same(reports, [], 'handled errors are not reported globally');
      const outer = subject(), inner = subject(), ac = new AbortController(), reason = {}, cleanupError = {};
      let maps = 0;
      outer.source.flatMap(() => { maps++; return inner.source; }).subscribe({}, {signal: ac.signal});
      outer.subscriber.next(1); outer.subscriber.next(2);
      outer.subscriber.addTeardown(() => { throw cleanupError; });
      ac.abort(reason);
      check(!outer.subscriber.active && !inner.subscriber.active && maps === 1, 'explicit cancellation closes both and drops queue');
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
      Observable.from(outer).flatMap(() => inner).finally(() => log.push('finally')).subscribe(() => {
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
    same(log, ['third'], 'later signal algorithm runs after failure without flatMap');
  });

  await test('pre-abort and cancellation inside mapper', () => {
    const ac = new AbortController(), reason = {}, pre = subject(); let maps = 0, inactive;
    pre.source.flatMap(() => { maps++; return []; }).subscribe({}, {signal: AbortSignal.abort(reason)});
    check(pre.starts === 1 && !pre.subscriber.active && pre.subscriber.signal.reason === reason && maps === 0, 'pre-aborted source initialized inactive without mapping');
    const outer = subject();
    outer.source.flatMap(() => { ac.abort(reason); return new Observable(s => { inactive = s; }); }).subscribe({}, {signal: ac.signal});
    outer.subscriber.next(1);
    check(!outer.subscriber.active && inactive && !inactive.active && inactive.signal.reason === reason, 'mapper cancellation still initializes returned Observable inactive');
    const captured = subject(), controller = new AbortController(); let calls = 0, inner;
    captured.source.subscribe(() => controller.abort(reason));
    captured.source.flatMap(() => { calls++; return new Observable(s => { inner = s; }); }).subscribe({}, {signal: controller.signal});
    captured.subscriber.next(1);
    check(calls === 1 && inner && !inner.active, 'captured next notification still maps after an earlier observer cancels');
    captured.subscriber.complete();
  });
  return {checks, failures};
})()
