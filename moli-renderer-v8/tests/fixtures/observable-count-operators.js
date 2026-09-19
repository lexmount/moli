(async () => {
  'use strict';
  const failures = [];
  let checks = 0;
  const check = (value, label) => { checks++; if (!value) failures.push(label); };
  const same = (a, b, label) => check(JSON.stringify(a) === JSON.stringify(b), label);
  const thrown = fn => { try { fn(); } catch (e) { return e; } };
  const test = async (label, fn) => { try { await fn(); } catch (e) { check(false, label + ': ' + e); } };
  const names = ['take', 'drop'];
  for (const name of names) check(typeof Observable.prototype[name] === 'function', name + ' exposed');
  if (failures.length) return {checks, failures};

  for (const name of names) {
    const method = Observable.prototype[name];
    await test(name + ' conversion and intrinsic creation', async () => {
      const desc = Object.getOwnPropertyDescriptor(Observable.prototype, name);
      check(method.name === name && method.length === 1, name + ' name and length');
      check(desc.enumerable && desc.writable && desc.configurable, name + ' descriptor');
      check(thrown(() => new method(1)) instanceof TypeError, name + ' non-constructible');
      let starts = 0, conversions = 0, traps = 0;
      const source = new Observable(s => { starts++; s.next(1); s.next(2); s.complete(); });
      const amount = {[Symbol.toPrimitive](hint) { conversions++; check(hint === 'number', name + ' numeric conversion hint'); return 1; }};
      const revoked = Proxy.revocable(source, {}); revoked.revoke();
      for (const receiver of [undefined, null, false, 1, Symbol(), {}, Object.create(source),
        Object.create(Observable.prototype), new Proxy(source, {get() { traps++; }}), revoked.proxy]) {
        check(thrown(() => method.call(receiver, amount)) instanceof TypeError, name + ' invalid receiver');
      }
      check(conversions === 0 && traps === 0, name + ' receiver validation precedes conversion and traps');
      check(thrown(() => method.call(source)) instanceof TypeError, name + ' required argument');
      for (const value of [1n, Symbol(), {[Symbol.toPrimitive]: () => 1n}, Object.create(null)]) {
        check(thrown(() => method.call(source, value)) instanceof TypeError, name + ' invalid numeric conversion');
      }
      const marker = {};
      check(thrown(() => method.call(source, {valueOf() { throw marker; }})) === marker, name + ' conversion exception identity');
      check(starts === 0, name + ' invalid conversion does not subscribe');
      const result = method.call(source, amount);
      check(starts === 0 && conversions === 1, name + ' converts once at creation without subscribing');
      check(result !== source && Object.getPrototypeOf(result) === Observable.prototype && Observable.from(result) === result, name + ' new branded intrinsic result');
      same(await result.toArray(), name === 'take' ? [1] : [2], name + ' first result');
      same(await result.toArray(), name === 'take' ? [1] : [2], name + ' count resets for next subscription');
      check(starts === 2 && conversions === 1, name + ' subscriptions reuse converted amount');
      const order = [];
      const numeric = {valueOf() { order.push('valueOf'); return {}; }, toString() { order.push('toString'); return '1.9'; }};
      same(await method.call(source, numeric).toArray(), name === 'take' ? [1] : [2], name + ' ordinary number conversion');
      same(order, ['valueOf', 'toString'], name + ' conversion order');
      let reads = 0;
      Object.defineProperty(source, 'constructor', {get() { reads++; throw marker; }});
      Object.setPrototypeOf(source, null);
      same(await method.call(source, 1, {get signal() { reads++; throw marker; }}).toArray(), name === 'take' ? [1] : [2], name + ' genuine receiver ignores public prototype and extra options');
      check(reads === 0, name + ' no species or options lookup');
      class Subclass extends Observable {}
      const derived = method.call(new Subclass(s => s.complete()), 1);
      check(Object.getPrototypeOf(derived) === Observable.prototype && !(derived instanceof Subclass), name + ' result does not inherit source subclass');
    });

    await test(name + ' unsigned 64-bit amount', async () => {
      const cases = [
        [undefined, 0], [null, 0], [false, 0], [-0, 0], [-0.9, 0], [NaN, 0], [Infinity, 0], [-Infinity, 0],
        ['', 0], ['not numeric', 0], [2 ** 64, 0], [-(2 ** 64), 0],
        [true, 1], [1.9, 1], ['1.9', 1], [new Number(1), 1], [2.9, 2], ['2', 2],
        [-1, 3], [-2, 3], [-3.9, 3], [2 ** 32, 3], [2 ** 53, 3], [2 ** 64 - 2048, 3],
        [2 ** 64 + 4096, 3], [-(2 ** 64) + 2048, 3],
      ];
      for (const [amount, count] of cases) {
        const values = [1, 2, 3];
        same(await method.call(Observable.from(values), amount).toArray(),
          name === 'take' ? values.slice(0, count) : values.slice(count), name + ' converts ' + String(amount));
      }
    });

    await test(name + ' sharing, cancellation and fresh counts', () => {
      let subscriber, starts = 0, teardowns = 0;
      const source = new Observable(s => { starts++; subscriber = s; s.addTeardown(() => teardowns++); });
      const keeper = new AbortController(); source.subscribe({}, {signal: keeper.signal});
      const result = method.call(source, 2);
      for (let round = 0; round < 2; round++) {
        const left = [], right = [], a = new AbortController(), b = new AbortController();
        let completions = 0;
        result.subscribe(value => { left.push(value); a.abort(); }, {signal: a.signal});
        result.subscribe({next: value => right.push(value), complete: () => completions++}, {signal: b.signal});
        for (const value of [1, 2, 3, 4]) subscriber.next(value);
        same(left, name === 'take' ? [1] : [3], name + ' cancelled observer stops receiving');
        same(right, name === 'take' ? [1, 2] : [3, 4], name + ' observers share one count');
        check(completions === (name === 'take' ? 1 : 0), name + ' completion only at take limit');
        check(subscriber.active && starts === 1 && teardowns === 0, name + ' other source subscription stays alive');
        b.abort();
      }
      keeper.abort(); check(!subscriber.active && teardowns === 1, name + ' last observer closes source');
    });

    await test(name + ' errors, completion and pre-abort', async () => {
      for (const error of [{}, null, undefined]) {
        const received = [], errors = [];
        const source = new Observable(s => { s.next(1); s.error(error); s.next(2); });
        method.call(source, 2).subscribe({next: value => received.push(value), error: e => errors.push(e), complete: () => errors.push('complete')});
        same(received, name === 'take' ? [1] : [], name + ' error before count reached');
        check(errors.length === 1 && errors[0] === error, name + ' source error identity');
      }
      const marker = {}, errors = [];
      method.call(new Observable(() => { throw marker; }), 2).subscribe({error: e => errors.push(e)});
      check(errors.length === 1 && errors[0] === marker, name + ' initializer exception forwarded');
      same(await method.call(Observable.from([]), 2).toArray(), [], name + ' empty source');
      same(await method.call(Observable.from([1]), 2).toArray(), name === 'take' ? [1] : [], name + ' source ends before amount reached');
      for (const amount of [0, 2]) {
        let starts = 0, notifications = 0, subscriber;
        const source = new Observable(s => { starts++; subscriber = s; });
        method.call(source, amount).subscribe({next() { notifications++; }, error() { notifications++; }, complete() { notifications++; }}, {signal: AbortSignal.abort(marker)});
        check(starts === (name === 'take' && amount === 0 ? 0 : 1), name + ' pre-aborted source initialization');
        check(!subscriber || !subscriber.active && subscriber.signal.reason === marker, name + ' pre-aborted Subscriber keeps reason');
        check(notifications === 0, name + ' pre-aborted observer receives no notifications');
      }
    });

    await test(name + ' iterator cancellation and raw values', async () => {
      const ac = new AbortController(), marker = {}, values = [];
      let pulls = 0, closes = 0;
      const iterator = {next: () => ({value: ++pulls}), return() { closes++; return {}; }};
      method.call(Observable.from({[Symbol.iterator]: () => iterator}), 2).subscribe({
        next: value => { values.push(value); ac.abort(marker); }, error: () => values.push('error'), complete: () => values.push('complete'),
      }, {signal: ac.signal});
      same(values, name === 'take' ? [1] : [3], name + ' cancellation inside next suppresses terminal notifications');
      check(pulls === (name === 'take' ? 1 : 3) && closes === 1, name + ' cancellation closes iterator exactly once');
      let reads = 0;
      const poison = {get then() { reads++; throw marker; }};
      const rejected = Promise.reject(marker); rejected.catch(() => {});
      const revoked = Proxy.revocable({}, {}); revoked.revoke();
      const raw = [undefined, null, false, -0, NaN, 1n, Symbol(), poison, Promise.resolve(1), rejected, revoked.proxy];
      const result = await method.call(Observable.from(raw), name === 'take' ? raw.length : 0).toArray();
      check(result.length === raw.length && result.every((value, i) => Object.is(value, raw[i])), name + ' values are not converted or assimilated');
      check(reads === 0, name + ' raw values do not read then');
    });
  }

  await test('zero count and teardown ordering', () => {
    const log = [];
    const source = new Observable(s => {
      log.push('start'); s.signal.addEventListener('abort', () => log.push('abort'));
      s.addTeardown(() => log.push('teardown')); s.next(1); log.push('after 1'); s.next(2); log.push('after 2'); s.complete();
    });
    source.take(0).subscribe({complete: () => log.push('empty')});
    same(log.splice(0), ['empty'], 'take(0) completes without starting source');
    source.take(1).subscribe({next: value => log.push(value), complete: () => log.push('complete')});
    same(log.splice(0), ['start', 1, 'abort', 'teardown', 'complete', 'after 1', 'after 2'], 'take closes upstream before downstream completion');
    source.drop(0).subscribe({next: value => log.push(value), complete: () => log.push('complete')});
    same(log.splice(0), ['start', 1, 'after 1', 2, 'after 2', 'abort', 'teardown', 'complete'], 'drop(0) mirrors full source');
    let reads = 0;
    const iterable = {get [Symbol.iterator]() { reads++; return () => { throw new Error('opened'); }; }};
    Observable.from(iterable).take(0).subscribe();
    check(reads === 1, 'take(0) does not open iterator after Observable.from conversion');
  });

  await test('reentrancy and notification snapshots', () => {
    let subscriber;
    const source = new Observable(s => { subscriber = s; });
    const taken = source.take(1), log = [];
    taken.subscribe({next: value => { log.push('a' + value); if (value === 1) subscriber.next(2); }, complete: () => log.push('a complete')});
    taken.subscribe({next: value => log.push('b' + value), complete: () => log.push('b complete')});
    subscriber.next(1); subscriber.next(3);
    same(log, ['a1', 'a2', 'b2', 'a complete', 'b complete', 'b1'], 'take decrements after delivery and preserves captured downstream next');
    const values = [];
    source.take(2).subscribe({next: value => { values.push(value); if (value === 1) { subscriber.next(2); subscriber.next(3); } }, complete: () => values.push('complete')});
    subscriber.next(1); subscriber.next(4);
    same(values, [1, 2, 3, 'complete'], 'nested take decrements are read after reentrant delivery');
    const dropped = [];
    source.drop(1).subscribe(value => { dropped.push(value); if (value === 2) subscriber.next(3); });
    subscriber.next(1); subscriber.next(2); subscriber.complete();
    same(dropped, [2, 3], 'drop count stays zero during reentrant delivery');
    for (const name of names) {
      const ac = new AbortController(), received = [];
      source.subscribe(() => ac.abort());
      source[name](name === 'take' ? 1 : 0).subscribe({next: v => received.push(v), complete: () => received.push('complete')}, {signal: ac.signal});
      subscriber.next(1); subscriber.complete();
      same(received, [], name + ' cancelled branch rejects captured upstream notification');
    }
  });

  await test('iterator close errors and async sources', async () => {
    const closeError = {}, reports = [];
    const onerror = e => { reports.push(e.error); e.preventDefault(); };
    addEventListener('error', onerror);
    try {
      const iterator = {next: () => ({value: 7}), return() { throw closeError; }};
      same(await Observable.from({[Symbol.iterator]: () => iterator}).take(1).toArray(), [7], 'take completion survives iterator close error');
      check(reports.length === 1 && reports[0] === closeError, 'take reports close error once');
      reports.length = 0;
      for (const name of names) {
        const ac = new AbortController(); let caught;
        Observable.from({[Symbol.iterator]: () => iterator})[name](name === 'take' ? 2 : 0)
          .subscribe(() => { caught = thrown(() => ac.abort()); }, {signal: ac.signal});
        check(caught === closeError, name + ' explicit abort preserves close exception');
      }
      check(reports.length === 0, 'caught author abort errors are not reported');
    } finally { removeEventListener('error', onerror); }
    let pulls = 0, closes = 0;
    const iterator = {next: () => Promise.resolve({value: ++pulls}), return() { closes++; return Promise.resolve({}); }};
    same(await Observable.from({[Symbol.asyncIterator]: () => iterator}).drop(1).take(2).toArray(), [2, 3], 'async drop/take chain');
    check(pulls === 3 && closes === 1, 'async chain cancels at last selected value');
    async function* numbers() { yield 1; yield 2; yield 3; yield 4; }
    const source = Observable.from(numbers());
    const results = await Promise.all([source.take(1).toArray(), source.drop(1).take(2).toArray()]);
    same(results, [[1], [2, 3]], 'shared async producer continues after shorter branch completes');
  });

  await test('intrinsic operations and mixed transforms', async () => {
    same(await Observable.from([1, 2, 3, 4, 5]).map(v => v * 2).drop(1).filter(v => v % 4 === 0).take(2).toArray(), [4, 8], 'mixed callback and count operators');
    const Constructor = Observable, take = Observable.prototype.take, drop = Observable.prototype.drop, subscribe = Observable.prototype.subscribe;
    const originals = [globalThis.Observable, Observable.prototype.subscribe, Subscriber.prototype.next, Subscriber.prototype.error, Subscriber.prototype.complete];
    const source = Observable.from([1, 2, 3, 4]), values = [];
    const poison = () => { throw new Error('public implementation consulted'); };
    let result;
    try {
      globalThis.Observable = Constructor.prototype.subscribe = Subscriber.prototype.next = Subscriber.prototype.error = Subscriber.prototype.complete = poison;
      result = drop.call(take.call(source, 3), 1); subscribe.call(result, value => values.push(value));
    } finally { [globalThis.Observable, Constructor.prototype.subscribe, Subscriber.prototype.next, Subscriber.prototype.error, Subscriber.prototype.complete] = originals; }
    check(result instanceof Constructor, 'count operators use intrinsic Observable prototype');
    same(values, [2, 3], 'count operators bypass public methods');
  });
  return {checks, failures};
})()
