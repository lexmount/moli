(async () => {
  'use strict';
  const failures = [];
  let checks = 0;
  const check = (value, label) => { checks++; if (!value) failures.push(label); };
  const same = (a, b, label) => check(JSON.stringify(a) === JSON.stringify(b), label);
  const thrown = fn => { try { fn(); } catch (e) { return e; } };
  const test = async (label, fn) => { try { await fn(); } catch (e) { check(false, label + ': ' + e); } };
  const names = ['map', 'filter'];
  for (const name of names) check(typeof Observable.prototype[name] === 'function', name + ' exposed');
  if (failures.length) return {checks, failures};

  for (const name of names) {
    const method = Observable.prototype[name];
    const identity = value => name === 'map' ? value : true;
    await test(name + ' conversion and lazy creation', async () => {
      const desc = Object.getOwnPropertyDescriptor(Observable.prototype, name);
      check(method.name === name && method.length === 1, name + ' metadata');
      check(desc.enumerable && desc.writable && desc.configurable, name + ' descriptor');
      check(thrown(() => new method(() => {})) instanceof TypeError, name + ' not a constructor');
      let starts = 0, calls = 0, traps = 0;
      const source = new Observable(s => { starts++; s.next(1); s.complete(); });
      const revoked = Proxy.revocable(source, {}); revoked.revoke();
      for (const receiver of [undefined, null, false, 1, Symbol(), {}, Object.create(source),
        Object.create(Observable.prototype), new Proxy(source, {get() { traps++; }}), revoked.proxy]) {
        check(thrown(() => method.call(receiver, identity)) instanceof TypeError, name + ' rejects invalid receiver synchronously');
      }
      for (const callback of [undefined, null, 1, {}, {handleEvent() {}}, Object.create(Function.prototype)]) {
        check(thrown(() => method.call(source, callback)) instanceof TypeError, name + ' rejects invalid callback synchronously');
      }
      check(thrown(() => method.call(source)) instanceof TypeError, name + ' requires callback');
      const mapped = method.call(source, value => { calls++; return identity(value); });
      check(starts === 0 && calls === 0 && traps === 0, name + ' conversion and creation do not subscribe');
      check(mapped !== source && Object.getPrototypeOf(mapped) === Observable.prototype && Observable.from(mapped) === mapped, name + ' intrinsic branded result');
      check(method.call(source, identity) !== mapped, name + ' always returns a new Observable');
      same(await mapped.toArray(), [1], name + ' first subscription');
      same(await mapped.toArray(), [1], name + ' later subscription');
      check(starts === 2 && calls === 2, name + ' completed subscription starts fresh');
      Object.setPrototypeOf(source, null);
      same(await method.call(source, identity).toArray(), [1], name + ' genuine receiver with replaced prototype');
      const errors = [], callback = Proxy.revocable(() => true, {});
      const lazy = method.call(Observable.from([1]), callback.proxy); callback.revoke();
      lazy.subscribe({error: e => errors.push(e)});
      method.call(Observable.from([1]), class Callback {}).subscribe({error: e => errors.push(e)});
      check(errors.length === 2 && errors.every(e => e instanceof TypeError), name + ' callable conversion defers invocation errors');
      let reads = 0;
      Object.defineProperty(source, 'constructor', {get() { reads++; throw 1; }});
      same(await method.call(source, identity, {get signal() { reads++; throw 2; }}).toArray(), [1], name + ' constructor and extra arguments ignored');
      check(reads === 0, name + ' no species or options lookup');
    });

    await test(name + ' invocation and fresh indices', async () => {
      const args = [];
      const callback = new Proxy(function (...values) {
        check(this === undefined, name + ' strict callback this');
        args.push(values); return name === 'map' ? values[0] * 2 : values[0] % 2;
      }, {});
      const result = method.call(Observable.from([1, 2, 3]), callback);
      same(await result.toArray(), name === 'map' ? [2, 4, 6] : [1, 3], name + ' output values');
      same(await result.toArray(), name === 'map' ? [2, 4, 6] : [1, 3], name + ' resubscribed output');
      same(args, [[1, 0], [2, 1], [3, 2], [1, 0], [2, 1], [3, 2]], name + ' two arguments and index reset');
    });

    await test(name + ' errors and source completion', async () => {
      for (const error of [{}, null, undefined]) {
        let subscriber, teardowns = 0, calls = 0;
        const source = new Observable(s => {
          subscriber = s; s.addTeardown(() => teardowns++);
          s.next(1); s.next(2); s.next(3); s.complete();
        });
        const received = [], errors = [];
        method.call(source, value => { calls++; if (value === 2) throw error; return identity(value); }).subscribe({
          next: value => received.push(value), error: e => errors.push(e), complete: () => errors.push('complete'),
        });
        same(received, [1], name + ' values stop after callback failure');
        check(errors.length === 1 && errors[0] === error && calls === 2 && !subscriber.active && teardowns === 1, name + ' error identity and cancellation');
        check(error === undefined ? subscriber.signal.reason.name === 'AbortError' : subscriber.signal.reason === error, name + ' upstream abort reason');
      }
      const marker = {}, errors = [];
      method.call(new Observable(s => s.error(marker)), identity).subscribe({error: e => errors.push(e)});
      method.call(new Observable(() => { throw marker; }), identity).subscribe({error: e => errors.push(e)});
      check(errors.length === 2 && errors.every(e => e === marker), name + ' forwards source and initializer errors');
      const log = [];
      method.call(new Observable(s => {
        s.signal.addEventListener('abort', () => log.push('source abort'));
        s.addTeardown(() => log.push('source teardown')); s.complete();
      }), () => { log.push('callback'); }).subscribe({complete: () => log.push('complete')});
      same(log, ['source abort', 'source teardown', 'complete'], name + ' source closes before downstream completion');
    });

    await test(name + ' cancellation and pre-abort', async () => {
      const marker = {}, ac = new AbortController();
      let subscriber, starts = 0, calls = 0, teardowns = 0;
      const source = new Observable(s => { starts++; subscriber = s; s.addTeardown(() => teardowns++); });
      const received = [];
      method.call(source, value => { calls++; return identity(value); }).subscribe({
        next: value => { received.push(value); ac.abort(marker); subscriber.next(2); },
        error: () => received.push('error'), complete: () => received.push('complete'),
      }, {signal: ac.signal});
      subscriber.next(1);
      same(received, [1], name + ' cancellation does not notify error or complete');
      check(!subscriber.active && calls === 1 && teardowns === 1 && subscriber.signal.reason === marker, name + ' cancellation reaches upstream');
      method.call(source, () => { calls++; }).subscribe({}, {signal: ac.signal});
      check(starts === 2 && calls === 1 && !subscriber.active && teardowns === 2, name + ' pre-aborted subscription initializes inactive source');
      check(subscriber.signal.reason === marker, name + ' pre-aborted source keeps reason identity');
    });

    await test(name + ' shared downstream and independent branches', async () => {
      let subscriber, starts = 0, calls = 0;
      const source = new Observable(s => { starts++; subscriber = s; });
      const transformed = method.call(source, value => { calls++; return identity(value); });
      const a = new AbortController(), left = [], right = [];
      transformed.subscribe(value => left.push(value), {signal: a.signal});
      const rightPromise = transformed.toArray().then(value => right.push(...value));
      subscriber.next(1); a.abort(); subscriber.next(2);
      check(subscriber.active && starts === 1 && calls === 2, name + ' downstream observers share one transform');
      subscriber.complete(); await rightPromise;
      same(left, [1], name + ' cancelled downstream removed'); same(right, [1, 2], name + ' remaining downstream receives values');
      const all = source.toArray(), marker = {}, errors = [];
      method.call(source, () => { throw marker; }).subscribe({error: e => errors.push(e)});
      subscriber.next(3);
      check(subscriber.active && errors.length === 1 && errors[0] === marker, name + ' failing branch does not cancel other source observers');
      subscriber.next(4); subscriber.complete(); same(await all, [3, 4], name + ' other branch survives');
      const keep = new AbortController(), indices = [];
      source.subscribe(() => {}, {signal: keep.signal});
      for (let i = 0; i < 2; i++) {
        const cancel = new AbortController();
        transformed.subscribe(() => cancel.abort(), {signal: cancel.signal});
        const branch = method.call(source, (value, index) => { indices.push(index); return identity(value); });
        branch.subscribe(() => {}, {signal: cancel.signal}); subscriber.next(5);
      }
      // Source stays alive across separate subscriptions; each transform branch
      // starts at zero even though another observer has already received values.
      same(indices, [0, 0], name + ' fresh branch index on shared source');
      keep.abort(); check(!subscriber.active, name + ' last source observer tears down');
    });

    await test(name + ' reentrant callbacks and snapshots', async () => {
      let subscriber;
      const source = new Observable(s => { subscriber = s; }), indices = [];
      const transformed = method.call(source, (value, index) => { indices.push(index); if (value === 1) subscriber.next(2); return identity(value); });
      const result = transformed.toArray(); subscriber.next(1); subscriber.next(3); subscriber.complete();
      same(await result, [2, 1, 3], name + ' nested values delivered before outer result');
      same(indices, [0, 0, 2], name + ' draft reentrant index order');
      for (const terminal of ['abort', 'complete']) {
        const ac = new AbortController(), received = [], callbacks = [];
        const source = new Observable(s => { subscriber = s; });
        source.subscribe(() => { if (terminal === 'abort') ac.abort(); else subscriber.complete(); });
        method.call(source, value => { callbacks.push(value); return identity(value); }).subscribe({
          next: value => received.push(value), complete: () => received.push('complete'),
        }, {signal: ac.signal});
        subscriber.next(1);
        same(callbacks, [1], name + ' captured upstream next invokes callback after ' + terminal);
        same(received, terminal === 'abort' ? [] : ['complete'], name + ' closed downstream rejects late next');
        subscriber.complete();
      }
      const ac = new AbortController(), received = [];
      method.call(source, value => { ac.abort(); return identity(value); }).subscribe(value => received.push(value), {signal: ac.signal});
      subscriber.next(1); same(received, [], name + ' cancellation inside callback suppresses result');
    });
  }

  await test('raw mapper values and predicate booleans', async () => {
    let reads = 0;
    const poison = {get then() { reads++; throw 1; }, [Symbol.toPrimitive]() { reads++; throw 2; }};
    const rejected = Promise.reject('mapper value'); rejected.catch(() => {});
    const revoked = Proxy.revocable({}, {}); revoked.revoke();
    const values = [undefined, null, false, 0, -0, NaN, 0n, 1n, '', 'text', Symbol(), {}, [], new Boolean(false), poison, Promise.resolve(false), rejected, revoked.proxy];
    const mapped = await Observable.from(values).map(value => value).toArray();
    check(mapped.length === values.length && values.every((value, index) => Object.is(mapped[index], value)), 'map keeps raw values including thenables');
    const filtered = await Observable.from(values.keys()).filter(index => values[index]).toArray();
    same(filtered, [...values.keys()].filter(index => Boolean(values[index])), 'filter uses boolean conversion only');
    check(reads === 0, 'neither transform reads then or conversion methods');
    const indices = [[], [], []];
    const chain = Observable.from([1, 2, 3, 4]).map((v, i) => { indices[0].push(i); return v * 2; })
      .filter((v, i) => { indices[1].push(i); return v % 4 === 0; })
      .map((v, i) => { indices[2].push(i); return v * 10; });
    same(await chain.toArray(), [40, 80], 'mixed native transform chain');
    same(indices, [[0, 1, 2, 3], [0, 1, 2, 3], [0, 1]], 'each transform counts its own input');
  });

  await test('iterator close and late callback errors', async () => {
    const closeError = {}, lateError = {}, errors = [];
    const onerror = e => { if (e.error === closeError || e.error === lateError) { errors.push(e.error); e.preventDefault(); } };
    addEventListener('error', onerror);
    try {
      for (const name of names) {
        const iterator = {next: () => ({value: 7}), return() { throw closeError; }};
        check(await Observable.from({[Symbol.iterator]: () => iterator})[name](value => name === 'map' ? value : true).first() === 7, name + ' iterator close error cannot replace first value');
        let subscriber;
        const source = new Observable(s => { subscriber = s; }), ac = new AbortController();
        source.subscribe(() => ac.abort());
        source[name](() => { throw lateError; }).subscribe({error: () => check(false, name + ' closed observer error delivery')}, {signal: ac.signal});
        subscriber.next(1); subscriber.complete();
      }
      check(errors.filter(e => e === closeError).length === 2, 'close errors reported once per transform');
      check(errors.filter(e => e === lateError).length === 2, 'callback failures after cancellation reported globally');
    } finally { removeEventListener('error', onerror); }
  });

  await test('callback failures survive upstream close failures', async () => {
    for (const name of names) for (const consumer of ['subscribe', 'toArray']) for (const wrapped of [false, true]) {
      const callbackError = {}, closeError = {}, teardownError = {}, errors = [], reports = [], teardowns = [];
      const onerror = e => { reports.push(e.error); e.preventDefault(); };
      addEventListener('error', onerror);
      try {
        let closes = 0;
        const iterator = {next: () => ({value: 1}), return() { closes++; throw closeError; }};
        const upstream = Observable.from({[Symbol.iterator]: () => iterator});
        const source = wrapped ? new Observable(s => {
          s.addTeardown(() => teardowns.push(1));
          s.addTeardown(() => { teardowns.push(2); throw teardownError; });
          upstream.subscribe(value => s.next(value), {signal: s.signal});
        }) : upstream;
        const transformed = source[name](() => { throw callbackError; });
        const escaped = thrown(() => {
          if (consumer === 'subscribe') transformed.subscribe({error: e => errors.push(e)});
          else transformed.toArray().then(value => errors.push(value), e => errors.push(e));
        });
        await Promise.resolve(); await Promise.resolve();
        check(escaped === undefined, name + ' callback and close failures do not escape subscribe');
        check(errors.length === 1 && errors[0] === callbackError, name + ' ' + consumer + ' retains callback error despite close failure');
        check(closes === 1, name + ' upstream iterator closes once on callback failure');
        check(reports.filter(e => e === closeError).length === 1 && reports.length === (wrapped ? 2 : 1)
          && (!wrapped || reports.includes(teardownError)), name + ' cleanup errors reported separately');
        same(teardowns, wrapped ? [2, 1] : [], name + ' close failure does not skip teardowns');
      } finally { removeEventListener('error', onerror); }
    }
  });

  await test('explicit cancellation still propagates iterator close errors', () => {
    for (const name of names) {
      const closeError = {}, ac = new AbortController(), log = [];
      let caught;
      const iterator = {next: () => ({value: 1}), return() { log.push('return'); throw closeError; }};
      new Observable(s => {
        s.addTeardown(() => log.push('teardown'));
        Observable.from({[Symbol.iterator]: () => iterator}).subscribe(value => s.next(value), {signal: s.signal});
      })[name](value => value).subscribe(() => {
        caught = thrown(() => ac.abort()); log.push('after');
      }, {signal: ac.signal});
      check(caught === closeError, name + ' author abort keeps close exception identity');
      same(log, ['return', 'teardown', 'after'], name + ' explicit cancellation finishes teardown before throwing');
    }
  });

  await test('terminal notifications survive iterator close errors', () => {
    for (const terminal of ['error', 'complete']) {
      const original = {}, closeError = {}, log = [], reports = [], received = [];
      const onerror = e => { reports.push(e.error); e.preventDefault(); };
      addEventListener('error', onerror);
      try {
        let subscriber, escaped;
        new Observable(s => { subscriber = s; s.addTeardown(() => log.push('teardown')); })
          .subscribe({error: e => { received.push(e); log.push('error'); }, complete: () => log.push('complete')});
        const iterator = {next: () => ({value: 1}), return() { log.push('return'); throw closeError; }};
        Observable.from({[Symbol.iterator]: () => iterator}).subscribe(() => {
          escaped = thrown(() => subscriber[terminal](original)); log.push('after');
        }, {signal: subscriber.signal});
        check(escaped === undefined, terminal + ' does not propagate cleanup error');
        same(log, ['return', 'teardown', terminal, 'after'], terminal + ' terminal delivery follows cleanup');
        check(reports.length === 1 && reports[0] === closeError, terminal + ' reports cleanup error');
        check(terminal === 'complete' ? received.length === 0 : received.length === 1 && received[0] === original, terminal + ' preserves original notification');
      } finally { removeEventListener('error', onerror); }
    }
  });

  await test('intrinsic transform operations', async () => {
    const Constructor = Observable, subscribe = Observable.prototype.subscribe, map = Observable.prototype.map, filter = Observable.prototype.filter;
    const original = [globalThis.Observable, Observable.prototype.subscribe, Subscriber.prototype.next, Subscriber.prototype.error, Subscriber.prototype.complete];
    const source = Observable.from([1, 2]), values = [];
    const poison = () => { throw new Error('mutable public implementation consulted'); };
    let result;
    try {
      globalThis.Observable = Constructor.prototype.subscribe = Subscriber.prototype.next = Subscriber.prototype.error = Subscriber.prototype.complete = poison;
      result = filter.call(map.call(source, value => value * 2), () => true);
      subscribe.call(result, value => values.push(value));
    } finally { [globalThis.Observable, Constructor.prototype.subscribe, Subscriber.prototype.next, Subscriber.prototype.error, Subscriber.prototype.complete] = original; }
    check(result instanceof Constructor, 'transforms use intrinsic Observable prototype');
    same(values, [2, 4], 'transforms bypass public subscribe and Subscriber methods');
  });
  return {checks, failures};
})()
