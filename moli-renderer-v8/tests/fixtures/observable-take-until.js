(async () => {
  'use strict';
  const failures = [];
  let checks = 0;
  const check = (value, label) => { checks++; if (!value) failures.push(label); };
  const same = (a, b, label) => check(JSON.stringify(a) === JSON.stringify(b), label);
  const thrown = fn => { try { fn(); } catch (e) { return e; } };
  const test = async (label, fn) => { try { await fn(); } catch (e) { check(false, label + ': ' + e); } };
  const method = Observable.prototype.takeUntil;
  check(typeof method === 'function', 'takeUntil exposed');
  if (failures.length) return {checks, failures};

  await test('receiver and notifier conversion', async () => {
    const desc = Object.getOwnPropertyDescriptor(Observable.prototype, 'takeUntil');
    check(method.name === 'takeUntil' && method.length === 1, 'name and length');
    check(desc.enumerable && desc.writable && desc.configurable, 'descriptor');
    check(thrown(() => new method([])) instanceof TypeError, 'not a constructor');
    let starts = 0, reads = 0, traps = 0;
    const source = new Observable(s => { starts++; s.next(1); s.complete(); });
    const input = {get [Symbol.asyncIterator]() { reads++; return undefined; }, get [Symbol.iterator]() { reads++; return [][Symbol.iterator]; }};
    const revoked = Proxy.revocable(source, {}); revoked.revoke();
    for (const receiver of [undefined, null, false, 1, Symbol(), {}, Object.create(source), Object.create(Observable.prototype),
      new Proxy(source, {get() { traps++; }}), revoked.proxy]) {
      check(thrown(() => method.call(receiver, input)) instanceof TypeError, 'receiver rejected before notifier conversion');
    }
    check(reads === 0 && traps === 0, 'invalid receiver does not probe notifier or Proxy');
    check(thrown(() => method.call(source)) instanceof TypeError, 'notifier argument required');
    for (const value of [undefined, null, false, 1, 1n, '', 'iterable primitive', Symbol(), {}, {then() {}}]) {
      check(thrown(() => method.call(source, value)) instanceof TypeError, 'unsupported notifier rejected');
    }
    const marker = {};
    for (const symbol of [Symbol.asyncIterator, Symbol.iterator]) {
      check(thrown(() => method.call(source, {get [symbol]() { throw marker; }})) === marker, 'protocol getter error identity');
      check(thrown(() => method.call(source, {[symbol]: 1})) instanceof TypeError, 'non-callable notifier protocol');
    }
    check(starts === 0, 'conversion failures do not initialize source');
    const result = method.call(source, input);
    check(reads === 2 && starts === 0, 'conversion probes protocols without subscribing');
    check(result !== source && Object.getPrototypeOf(result) === Observable.prototype && Observable.from(result) === result, 'new intrinsic branded result');
    same(await result.toArray(), [1], 'empty converted notifier does not stop source');
    same(await result.toArray(), [1], 'fresh subscription');
    check(starts === 2 && reads === 4, 'subscription rereads selected protocol without repeating conversion');
    let notifierStarts = 0;
    const notifier = new Observable(s => { notifierStarts++; s.complete(); });
    Object.defineProperty(notifier, Symbol.asyncIterator, {get() { throw marker; }});
    same(await method.call(source, notifier).toArray(), [1], 'native notifier bypasses protocol lookup');
    check(notifierStarts === 1, 'native notifier initializes once');
    Object.defineProperty(source, 'constructor', {get() { throw marker; }});
    Object.setPrototypeOf(source, null);
    same(await method.call(source, [], {get signal() { throw marker; }}).toArray(), [1], 'ignores source prototype, species and extra options');
    class Subclass extends Observable {}
    const derived = method.call(new Subclass(s => s.complete()), []);
    check(Object.getPrototypeOf(derived) === Observable.prototype && !(derived instanceof Subclass), 'result uses base intrinsic prototype');
  });

  await test('synchronous notifier notifications', async () => {
    for (const action of ['next', 'error', 'throw', 'complete']) {
      const log = [], marker = {};
      let notifierSubscriber;
      const source = new Observable(s => { log.push('source'); s.next(1); s.complete(); });
      const notifier = new Observable(s => {
        notifierSubscriber = s; log.push('notifier'); s.addTeardown(() => log.push('notifier teardown'));
        if (action === 'throw') throw marker;
        s[action](marker); log.push('after notifier');
      });
      const result = source.takeUntil(notifier);
      same(log, [], 'takeUntil creation is lazy');
      result.subscribe({next: value => log.push(value), error: e => log.push(e), complete: () => log.push('complete')});
      const expected = action === 'complete' ? ['notifier', 'notifier teardown', 'after notifier', 'source', 1, 'complete']
        : action === 'throw' ? ['notifier', 'notifier teardown', 'complete'] : ['notifier', 'notifier teardown', 'complete', 'after notifier'];
      same(log, expected, action + ' subscription and teardown order');
      check(!notifierSubscriber.active, action + ' leaves notifier inactive');
      check(action === 'error' || action === 'throw' ? notifierSubscriber.signal.reason === marker : notifierSubscriber.signal.reason.name === 'AbortError', action + ' notifier abort reason');
    }
    let starts = 0;
    const source = new Observable(s => { starts++; s.next(1); s.complete(); });
    same(await source.takeUntil([1, 2]).toArray(), [], 'nonempty iterable prevents source initialization');
    same(await source.takeUntil(new String('stop')).toArray(), [], 'boxed string notifier');
    check(starts === 0, 'synchronous iterable notifier skips source');
    same(await source.takeUntil([]).toArray(), [1], 'empty iterable permits source');
  });

  await test('dual upstream terminal ordering and reasons', () => {
    for (const terminal of ['notifier next', 'notifier error', 'source complete', 'source error', 'abort']) {
      const log = [], marker = {}, ac = new AbortController();
      let s, n;
      const source = new Observable(subscriber => {
        s = subscriber; s.signal.addEventListener('abort', () => log.push('source abort'));
        s.addTeardown(() => log.push('source teardown'));
      });
      const notifier = new Observable(subscriber => {
        n = subscriber; n.signal.addEventListener('abort', () => log.push('notifier abort'));
        n.addTeardown(() => log.push('notifier teardown'));
      });
      const received = [], errors = [];
      source.takeUntil(notifier).subscribe({next: value => received.push(value), error: e => { errors.push(e); log.push('error'); }, complete: () => log.push('complete')}, {signal: ac.signal});
      s.next(1);
      if (terminal === 'notifier next') n.next(marker);
      else if (terminal === 'notifier error') n.error(marker);
      else if (terminal === 'source complete') s.complete();
      else if (terminal === 'source error') s.error(marker);
      else ac.abort(marker);
      s.next(2); n.next(3);
      same(received, [1], terminal + ' stops further values');
      check(!s.active && !n.active, terminal + ' closes both upstreams');
      const notifications = terminal === 'abort' ? [] : [terminal === 'source error' ? 'error' : 'complete'];
      const order = terminal.startsWith('source') ? ['source abort', 'source teardown', 'notifier abort', 'notifier teardown']
        : ['notifier abort', 'notifier teardown', 'source abort', 'source teardown'];
      same(log, [...order, ...notifications], terminal + ' cleanup precedes downstream notification');
      check(terminal === 'source error' ? errors.length === 1 && errors[0] === marker : errors.length === 0, terminal + ' only source error is forwarded');
      if (terminal === 'abort' || terminal === 'source error') check(s.signal.reason === marker && n.signal.reason === marker, terminal + ' reason propagates to both inputs');
      else check(s.signal.reason.name === 'AbortError' && (terminal === 'notifier error' ? n.signal.reason === marker : n.signal.reason.name === 'AbortError'), terminal + ' completion uses AbortError');
    }
  });

  await test('notifier completion, pre-abort and initialization cancellation', async () => {
    let s, n, starts = 0;
    const source = new Observable(subscriber => { starts++; s = subscriber; });
    const notifier = new Observable(subscriber => { n = subscriber; });
    const promise = source.takeUntil(notifier).toArray();
    n.complete(); check(s.active && !n.active, 'notifier completion leaves source active');
    s.next(1); s.next(2); s.complete(); same(await promise, [1, 2], 'values continue after notifier completion');
    const marker = {}, preaborted = new AbortController(); preaborted.abort(marker);
    let notifications = 0;
    source.takeUntil(notifier).subscribe({next() { notifications++; }, error() { notifications++; }, complete() { notifications++; }}, {signal: preaborted.signal});
    check(starts === 1 && !n.active && n.signal.reason === marker, 'pre-abort initializes inactive notifier and skips source');
    check(notifications === 0, 'pre-aborted downstream does not receive completion');
    const ac = new AbortController(), log = [];
    source.takeUntil(new Observable(subscriber => {
      subscriber.addTeardown(() => log.push('notifier teardown')); ac.abort(marker); log.push('after abort');
    })).subscribe({complete: () => log.push('complete')}, {signal: ac.signal});
    check(starts === 1, 'abort during notifier initializer skips source');
    same(log, ['notifier teardown', 'after abort'], 'initializer cancellation is not completion');
  });

  await test('shared downstream and independent branches', async () => {
    let s, n, sources = 0, notifiers = 0;
    const source = new Observable(subscriber => { sources++; s = subscriber; });
    const notifier = new Observable(subscriber => { notifiers++; n = subscriber; });
    const result = source.takeUntil(notifier), ac = new AbortController(), left = [];
    result.subscribe(value => { left.push(value); ac.abort(); }, {signal: ac.signal});
    const right = result.toArray(); s.next(1); s.next(2);
    check(s.active && n.active && sources === 1 && notifiers === 1, 'remaining observer keeps both shared subscriptions');
    n.next('stop'); same(left, [1], 'one downstream cancellation'); same(await right, [1, 2], 'remaining downstream values');
    const fresh = result.toArray();
    check(sources === 2 && notifiers === 2, 'later subscription initializes both inputs again');
    n.complete(); s.next(3); s.complete(); same(await fresh, [3], 'fresh subscription after previous notifier cancellation');
    let n1, n2;
    const a = source.takeUntil(new Observable(subscriber => { n1 = subscriber; })).toArray();
    const b = source.takeUntil(new Observable(subscriber => { n2 = subscriber; })).toArray();
    s.next(4); n1.next('stop'); check(s.active && n2.active, 'ending one branch keeps shared source and other notifier');
    s.next(5); n2.error({});
    same(await a, [4], 'first independent branch'); same(await b, [4, 5], 'second independent branch');
    check(!s.active && !n1.active && !n2.active, 'last branch releases all inputs');
  });

  await test('shared producer with notifier and reentrant snapshots', async () => {
    let s, n;
    const source = new Observable(subscriber => { s = subscriber; });
    const sameSource = source.takeUntil(source).toArray();
    s.next(1); same(await sameSource, [], 'source used as its own notifier completes before forwarding');
    check(!s.active, 'source/notifier identity closes one producer');
    const notifier = new Observable(subscriber => { n = subscriber; });
    const result = source.takeUntil(notifier), log = [];
    result.subscribe({next: value => { log.push('a' + value); n.next('stop'); }, complete: () => log.push('a complete')});
    result.subscribe({next: value => log.push('b' + value), complete: () => log.push('b complete')});
    s.next(1); s.next(2);
    same(log, ['a1', 'a complete', 'b complete', 'b1'], 'downstream snapshot continues after reentrant notifier termination');
    const keeper = new AbortController(), received = [];
    source.subscribe(() => n.next('stop'), {signal: keeper.signal});
    source.takeUntil(notifier).subscribe({next: value => received.push(value), complete: () => received.push('complete')});
    s.next(3); same(received, ['complete'], 'notifier termination rejects captured source value');
    keeper.abort();
  });

  await test('Promise and asynchronous iterable notifiers', async () => {
    for (const rejected of [false, true]) {
      let s;
      const marker = {}, notifier = rejected ? Promise.reject(marker) : Promise.resolve(marker);
      notifier.catch(() => {});
      const result = new Observable(subscriber => { s = subscriber; s.next(1); }).takeUntil(notifier).toArray();
      check(s.active, 'Promise notifier does not synchronously complete downstream');
      await Promise.resolve(); await Promise.resolve();
      check(!s.active, 'Promise fulfillment and rejection both stop source');
      s.complete(); same(await result, [1], 'Promise notifier error becomes completion');
    }
    let s, pulls = 0, closes = 0;
    const iterator = {next() { pulls++; return Promise.resolve({value: 'stop'}); }, return() { closes++; return Promise.resolve({}); }};
    const result = new Observable(subscriber => { s = subscriber; s.next(2); })
      .takeUntil({[Symbol.asyncIterator]: () => iterator}).toArray();
    same(await result, [2], 'async notifier allows source to start before first value');
    check(pulls === 1 && closes === 1 && !s.active, 'async notifier cancels both inputs on first value');
    const log = [];
    const promise = new Observable(subscriber => { log.push('source'); subscriber.complete(); })
      .takeUntil({[Symbol.asyncIterator]: () => ({next: () => new Promise(() => {}), return() { log.push('return'); return Promise.resolve({}); }})}).toArray();
    same(await promise, [], 'source completion closes a pending async notifier');
    same(log, ['source', 'return'], 'pending async notifier return called');
    let starts = 0;
    const notifier = {[Symbol.iterator]: () => [][Symbol.iterator]()};
    const changed = new Observable(() => { starts++; }).takeUntil(notifier);
    for (const value of [undefined, 1, () => { throw new Error('open'); }]) {
      notifier[Symbol.iterator] = value;
      same(await changed.toArray(), [], 'invalid protocol at subscription becomes notifier error/completion');
    }
    check(starts === 0, 'failed notifier opening skips source');
  });

  await test('raw source values and ignored notifier values', async () => {
    let n, reads = 0;
    const poison = {get then() { reads++; throw 1; }};
    const rejected = Promise.reject('raw value'); rejected.catch(() => {});
    const revoked = Proxy.revocable({}, {}); revoked.revoke();
    const values = [undefined, null, false, -0, NaN, 1n, Symbol(), poison, rejected, revoked.proxy];
    const result = await Observable.from(values).takeUntil(new Observable(subscriber => { n = subscriber; })).toArray();
    check(result.length === values.length && result.every((v, i) => Object.is(v, values[i])), 'source values forwarded without conversion or assimilation');
    check(!n.active && reads === 0, 'source exhaustion cancels notifier without reading values');
    for (const value of values) {
      let starts = 0;
      same(await new Observable(() => { starts++; }).takeUntil(new Observable(s => s.next(value))).toArray(), [], 'any notifier value completes');
      check(starts === 0, 'ignored notifier value prevents source start');
    }
    check(reads === 0, 'notifier values are not assimilated');
    for (const error of [{}, null, undefined]) {
      const received = [];
      new Observable(() => { throw error; }).takeUntil(new Observable(() => {})).subscribe({error: e => received.push(e)});
      check(received.length === 1 && received[0] === error, 'source initializer exception identity');
    }
  });

  await test('iterator close errors and late notifier errors', async () => {
    const closeError = {}, lateError = {}, firstError = {}, reports = [];
    const onerror = e => { reports.push(e.error); e.preventDefault(); };
    addEventListener('error', onerror);
    try {
      let starts = 0, closes = 0;
      const notifier = {next: () => ({value: 1}), return() { closes++; throw closeError; }};
      same(await new Observable(() => { starts++; }).takeUntil({[Symbol.iterator]: () => notifier}).toArray(), [], 'notifier close failure does not replace completion');
      check(starts === 0 && closes === 1 && reports.length === 1 && reports[0] === closeError, 'sync notifier failure closes once and skips source');
      reports.length = 0;
      let n, pulls = 0;
      const iterator = {next() { if (++pulls === 3) n.next('stop'); return {value: pulls}; }, return() { throw closeError; }};
      same(await Observable.from({[Symbol.iterator]: () => iterator}).takeUntil(new Observable(s => { n = s; })).toArray(), [1, 2], 'source close failure does not replace notifier completion');
      check(reports.length === 1 && reports[0] === closeError && !n.active, 'source close error reported once after both inputs close');
      reports.length = 0;
      const ac = new AbortController(); let caught;
      Observable.from({[Symbol.iterator]: () => notifier}).takeUntil(new Observable(s => { n = s; }))
        .subscribe(() => { caught = thrown(() => ac.abort()); }, {signal: ac.signal});
      check(caught === closeError && !n.active && reports.length === 0, 'explicit cancellation preserves iterator-close exception');
      let completions = 0, errors = 0;
      new Observable(() => {}).takeUntil(new Observable(s => { n = s; })).subscribe({error: () => errors++, complete: () => completions++});
      n.error(firstError); n.error(lateError);
      check(completions === 1 && errors === 0, 'notifier error completes only once');
      check(reports.length === 1 && reports[0] === lateError, 'second notifier error reports globally');
      reports.length = 0;
      const rejected = Promise.reject(lateError); rejected.catch(() => {});
      new Observable(() => check(false, 'pre-aborted source started')).takeUntil(rejected).subscribe({}, {signal: AbortSignal.abort()});
      await Promise.resolve(); await Promise.resolve();
      check(reports.length === 1 && reports[0] === lateError, 'pre-aborted notifier still handles and reports Promise rejection');
    } finally { removeEventListener('error', onerror); }
  });

  await test('intrinsic conversion and subscription operations', () => {
    const Constructor = Observable, subscribe = Observable.prototype.subscribe;
    const originals = [globalThis.Observable, Observable.from, Observable.prototype.subscribe, Subscriber.prototype.next, Subscriber.prototype.error, Subscriber.prototype.complete];
    const source = Observable.from([1, 2]), values = [];
    const poison = () => { throw new Error('public implementation consulted'); };
    let result;
    try {
      globalThis.Observable = Constructor.from = Constructor.prototype.subscribe = Subscriber.prototype.next = Subscriber.prototype.error = Subscriber.prototype.complete = poison;
      result = method.call(source, []); subscribe.call(result, value => values.push(value));
    } finally { [globalThis.Observable, Constructor.from, Constructor.prototype.subscribe, Subscriber.prototype.next, Subscriber.prototype.error, Subscriber.prototype.complete] = originals; }
    check(result instanceof Constructor, 'takeUntil uses intrinsic Observable prototype');
    same(values, [1, 2], 'takeUntil bypasses public from/subscribe/Subscriber methods');
  });

  return {checks, failures};
})()
