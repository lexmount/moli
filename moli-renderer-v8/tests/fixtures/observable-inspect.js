(async () => {
  'use strict';
  const failures = [];
  let checks = 0;
  const check = (value, label) => { checks++; if (!value) failures.push(label); };
  const same = (a, b, label) => check(JSON.stringify(a) === JSON.stringify(b), label);
  const thrown = fn => { try { fn(); } catch (e) { return e; } };
  const test = async (label, fn) => { try { await fn(); } catch (e) { check(false, label + ': ' + e); } };
  const method = Observable.prototype.inspect;
  check(typeof method === 'function', 'inspect exposed');
  if (failures.length) return {checks, failures};

  await test('receiver, union and dictionary conversion', async () => {
    const desc = Object.getOwnPropertyDescriptor(Observable.prototype, 'inspect');
    check(method.length === 0 && method.name === 'inspect', 'name and length');
    check(desc.enumerable && desc.writable && desc.configurable, 'descriptor');
    check(thrown(() => new method()) instanceof TypeError, 'not a constructor');
    const source = Observable.from([1]);
    let reads = 0, traps = 0;
    const input = {get next() { reads++; return () => {}; }};
    const revoked = Proxy.revocable(source, {}); revoked.revoke();
    for (const receiver of [undefined, null, false, 1, Symbol(), {}, Object.create(source), Object.create(Observable.prototype),
      new Proxy(source, {get() { traps++; }}), revoked.proxy]) {
      check(thrown(() => method.call(receiver, input)) instanceof TypeError, 'invalid receiver rejected');
    }
    check(reads === 0 && traps === 0, 'brand checked before inspector conversion without Proxy traps');
    for (const input of [false, true, 1, 1n, '', 'next', Symbol()]) check(thrown(() => source.inspect(input)) instanceof TypeError, 'primitive inspector rejected');
    for (const input of [undefined, null, {}, [], Object.create(null)]) same(await source.inspect(input).toArray(), [1], 'optional empty inspector');
    same(await source.inspect().toArray(), [1], 'omitted inspector');
    const names = ['abort', 'complete', 'error', 'next', 'subscribe'], order = [], calls = [];
    const prototype = {};
    for (const name of names) Object.defineProperty(prototype, name, {get() { order.push(name); return (...args) => calls.push([name, args.length]); }});
    const inspector = new Proxy(Object.create(prototype), {get(target, key, receiver) { return Reflect.get(target, key, receiver); }, has() { throw 'has consulted'; }});
    const result = source.inspect(inspector);
    same(order, names, 'inherited dictionary getters read lexicographically');
    same(calls, [], 'conversion is lazy');
    same(await result.toArray(), [1], 'dictionary inspector forwards');
    same(calls, [['subscribe', 0], ['next', 1], ['complete', 0]], 'callback argument counts');
    same(order, names, 'subscription does not reread dictionary');
    for (const name of names) {
      const log = [], marker = {};
      const bad = new Proxy({}, {get(_, key) { log.push(key); if (key === name) throw marker; }});
      check(thrown(() => source.inspect(bad)) === marker, name + ' getter exception identity');
      same(log, names.slice(0, names.indexOf(name) + 1), name + ' conversion stops at getter error');
      for (const value of [null, 1, {}]) check(thrown(() => source.inspect({[name]: value})) instanceof TypeError, name + ' must be callable when present');
      same(await source.inspect({[name]: undefined}).toArray(), [1], name + ' undefined is absent');
    }
    let invoked = 0;
    const callable = new Proxy(function(value) { invoked += value; check(this === undefined && arguments.length === 1, 'callable inspector uses undefined this'); }, {
      get() { throw 'callable dictionary lookup'; }
    });
    same(await source.inspect(callable).toArray(), [1], 'callable union bypasses dictionary lookup');
    check(invoked === 1, 'callable inspector invoked');
    const marker = {};
    Object.defineProperty(source, 'constructor', {get() { throw marker; }});
    Object.setPrototypeOf(source, null);
    const plain = method.call(source, undefined, {get signal() { throw marker; }});
    same(await plain.toArray(), [1], 'ignores source prototype, species and extra options');
    check(Observable.from(plain) === plain && Object.getPrototypeOf(plain) === Observable.prototype, 'intrinsic branded result');
    class Subclass extends Observable {}
    check(!(new Subclass(s => s.complete()).inspect() instanceof Subclass), 'result is not a species instance');
  });

  await test('snapshot callbacks, ordering and ignored results', async () => {
    const log = []; let starts = 0, reads = 0;
    const poison = {get then() { reads++; throw 'assimilated'; }};
    const source = new Observable(s => {
      starts++; log.push('source'); s.addTeardown(() => log.push('teardown')); s.next(7); s.complete();
    });
    const inspector = {};
    for (const name of ['subscribe', 'next', 'complete', 'error', 'abort']) inspector[name] = function(...args) {
      check(this === undefined, name + ' callback this'); log.push(name); return poison;
    };
    const result = source.inspect(inspector);
    for (const name of Object.keys(inspector)) inspector[name] = () => { throw 'mutated inspector consulted'; };
    check(starts === 0, 'creation does not subscribe');
    result.subscribe({next: () => log.push('down next'), complete: () => log.push('down complete')});
    same(log, ['subscribe', 'source', 'next', 'down next', 'teardown', 'complete', 'down complete'], 'inspection and source cleanup order');
    same(await result.toArray(), [7], 'second subscription uses stored callbacks');
    check(starts === 2 && reads === 0, 'new source subscription and no thenable assimilation');
  });

  await test('producer and inspector errors suppress abort hook', () => {
    const reports = [], onerror = e => { reports.push(e.error); e.preventDefault(); };
    addEventListener('error', onerror);
    try {
      for (const hook of ['subscribe', 'next', 'error', 'complete']) {
        for (const marker of [{}, null, undefined]) {
          const sourceError = {}, log = [], values = [], errors = [];
          let s, starts = 0, aborts = 0, completes = 0;
          const source = new Observable(subscriber => { starts++; s = subscriber; s.addTeardown(() => log.push('teardown')); });
          const result = source.inspect({[hook]() { throw marker; }, abort() { aborts++; }});
          result.subscribe({next: v => values.push(v), error: e => { errors.push(e); log.push('error'); }, complete: () => completes++});
          if (s) { s.next(1); if (hook === 'error') s.error(sourceError); else s.complete(); }
          check(errors.length === 1 && errors[0] === marker && completes === 0, hook + ' replacement error identity');
          same(values, hook === 'error' || hook === 'complete' ? [1] : [], hook + ' forwarding stops at exception');
          check(starts === (hook === 'subscribe' ? 0 : 1) && aborts === 0, hook + ' producer error does not call abort');
          same(log, hook === 'subscribe' ? ['error'] : ['teardown', 'error'], hook + ' cleanup precedes downstream error');
        }
      }
      same(reports, [], 'handled inspector errors do not report original errors globally');
      reports.length = 0;
      for (const complete of [false, true]) {
        const marker = {}, log = []; let s;
        new Observable(subscriber => { s = subscriber; }).inspect({abort: () => log.push('abort'), error: e => log.push(e), complete: () => log.push('complete')})
          .subscribe({error: e => log.push(e), complete: () => log.push('down complete')});
        if (complete) s.complete(); else s.error(marker);
        same(log, complete ? ['complete', 'down complete'] : [marker, marker], 'source terminal suppresses abort and forwards once');
      }
      const original = {};
      new Observable(s => s.error(original)).inspect({error() {}}).subscribe();
      check(reports.length === 1 && reports[0] === original, 'inspector does not consume an unhandled source error');
    } finally { removeEventListener('error', onerror); }
  });

  await test('consumer cancellation, abort exceptions and pre-abort', () => {
    const marker = {}, hookError = {}, reports = [], onerror = e => { reports.push(e.error); e.preventDefault(); };
    addEventListener('error', onerror);
    try {
      for (const throwing of [false, true]) {
        const log = [], ac = new AbortController(); let s;
        new Observable(subscriber => {
          s = subscriber; s.signal.addEventListener('abort', () => log.push('source abort')); s.addTeardown(() => log.push('teardown'));
        }).inspect({abort: function(reason) {
          check(this === undefined && arguments.length === 1 && reason === marker, 'abort this, argument count and reason');
          check(s.active, 'inspect abort precedes source cancellation'); log.push('inspect abort');
          if (throwing) throw hookError;
        }}).subscribe({complete: () => log.push('complete'), error: () => log.push('error')}, {signal: ac.signal});
        ac.abort(marker); ac.abort();
        same(log, ['inspect abort', 'source abort', 'teardown'], 'consumer abort is not a terminal notification');
        check(!s.active && s.signal.reason === marker, 'source cancellation reason');
      }
      check(reports.length === 1 && reports[0] === hookError, 'abort hook error reports once without interrupting cancellation');
      for (const mode of ['pre-aborted', 'abort in subscribe']) {
        const ac = new AbortController(), log = []; let sourceSubscriber;
        if (mode === 'pre-aborted') ac.abort(marker);
        new Observable(s => { sourceSubscriber = s; log.push('source'); s.addTeardown(() => log.push('teardown')); })
          .inspect({subscribe() { log.push('subscribe'); ac.abort(marker); }, abort() { log.push('abort'); }})
          .subscribe({next() { log.push('next'); }, error() { log.push('error'); }, complete() { log.push('complete'); }}, {signal: ac.signal});
        same(log, ['subscribe', 'source', 'teardown'], mode + ' invokes subscribe and inactive source without abort hook');
        check(!sourceSubscriber.active && sourceSubscriber.signal.reason === marker, mode + ' keeps cancellation reason');
      }
      let starts = 0;
      new Observable(() => starts++).inspect({subscribe() { throw marker; }, abort() { check(false, 'pre-aborted abort hook'); }})
        .subscribe({}, {signal: AbortSignal.abort()});
      check(starts === 0 && reports.length === 2 && reports[1] === marker, 'pre-aborted subscribe error is reported and skips source');
    } finally { removeEventListener('error', onerror); }
  });

  await test('shared subscriptions and independent branches', () => {
    let s, starts = 0, subscriptions = 0, aborts = 0, inspections = 0;
    const source = new Observable(subscriber => { s = subscriber; starts++; });
    const result = source.inspect({subscribe() { subscriptions++; }, next() { inspections++; }, abort() { aborts++; }});
    const a = new AbortController(), b = new AbortController(), received = [];
    result.subscribe(v => { received.push('a' + v); a.abort(); }, {signal: a.signal});
    result.subscribe(v => received.push('b' + v), {signal: b.signal});
    s.next(1); s.next(2);
    check(starts === 1 && subscriptions === 1 && inspections === 2 && aborts === 0 && s.active, 'shared downstream uses one inspector and one producer');
    same(received, ['a1', 'b1', 'b2'], 'one cancelled observer does not affect its sibling');
    b.abort(); check(aborts === 1 && !s.active, 'last observer triggers one abort hook');
    result.subscribe(); check(starts === 2 && subscriptions === 2, 'fresh subscription reinitializes inspector'); s.complete();
    check(aborts === 1, 'fresh source completion does not abort inspector');
    const first = new AbortController(), second = new AbortController(), log = [];
    source.inspect({abort: () => log.push('first')}).subscribe({}, {signal: first.signal});
    source.inspect({abort: () => log.push('second')}).subscribe({}, {signal: second.signal});
    first.abort(); check(s.active, 'independent inspector branch leaves source alive');
    second.abort(); check(!s.active, 'last branch cancels source'); same(log, ['first', 'second'], 'independent abort hooks');
  });

  await test('reentrant notifications and captured observers', () => {
    let s; const log = [], source = new Observable(subscriber => { s = subscriber; });
    source.inspect(value => { log.push('inspect' + value); if (value === 1) s.next(2); }).subscribe(value => log.push('down' + value));
    s.next(1); s.complete(); same(log, ['inspect1', 'inspect2', 'down2', 'down1'], 'reentrant values forward after inspector');
    const ac = new AbortController(), keeper = new AbortController(), captured = [];
    source.subscribe(() => ac.abort(), {signal: keeper.signal});
    source.inspect({next: v => captured.push('inspect' + v), abort: () => captured.push('abort')})
      .subscribe(v => captured.push('down' + v), {signal: ac.signal});
    s.next(3); keeper.abort();
    same(captured, ['abort', 'inspect3'], 'captured inspector still runs after earlier observer cancels it');
    const abort = new AbortController(), reentrant = [];
    source.inspect({next: v => reentrant.push('inspect' + v), abort() { reentrant.push('abort'); s.next(2); }})
      .subscribe(v => { reentrant.push('down' + v); abort.abort(); }, {signal: abort.signal});
    s.next(1);
    same(reentrant, ['inspect1', 'down1', 'abort', 'inspect2'], 'abort hook may reenter inspector before upstream cancellation');
    check(!s.active, 'reentrant abort ends source');
    const terminal = [];
    source.inspect({next() { s.complete(); }, complete() { terminal.push('inspect complete'); }, abort() { terminal.push('abort'); }})
      .subscribe({next() { terminal.push('next'); }, complete() { terminal.push('complete'); }});
    s.next(1); same(terminal, ['inspect complete', 'complete'], 'reentrant producer completion suppresses current value and abort hook');
  });

  await test('composition and iterator cleanup failures', async () => {
    const reports = [], onerror = e => { reports.push(e.error); e.preventDefault(); };
    addEventListener('error', onerror);
    try {
      const marker = {}, closeError = {}, abortError = {}, log = []; let reason, returns = 0;
      same(await Observable.from([1, 2]).inspect({abort: e => { reason = e; }}).take(1).toArray(), [1], 'take cancels inspect upstream');
      check(reason.name === 'AbortError', 'take completion supplies abort reason');
      const error = await Observable.from([1]).inspect({abort: e => { reason = e; }}).map(() => { throw marker; }).toArray().catch(e => e);
      check(error === marker && reason === marker, 'downstream map error is consumer cancellation of inspect');
      const iterator = {next: () => ({value: 1}), return() { returns++; log.push('return'); throw closeError; }};
      const caught = await Observable.from({[Symbol.iterator]: () => iterator})
        .inspect({next() { throw marker; }, abort() { log.push('abort'); }}).toArray().catch(e => e);
      check(caught === marker && returns === 1, 'inspector error survives IteratorClose failure');
      same(log, ['return'], 'inspector error suppresses abort hook during cleanup');
      check(reports.length === 1 && reports[0] === closeError, 'IteratorClose failure reports once');
      reports.length = 0; log.length = 0;
      const ac = new AbortController(); let cancelError;
      Observable.from({[Symbol.iterator]: () => iterator}).inspect({abort() { log.push('abort'); throw abortError; }})
        .subscribe(() => { cancelError = thrown(() => ac.abort(marker)); }, {signal: ac.signal});
      check(cancelError === closeError && reports.length === 1 && reports[0] === abortError, 'explicit cancellation rethrows close error and reports abort callback error');
      same(log, ['abort', 'return'], 'abort callback failure does not skip IteratorClose');
    } finally { removeEventListener('error', onerror); }
  });

  await test('raw values and intrinsic operations', async () => {
    let reads = 0;
    const poison = {get then() { reads++; throw 'then'; }}, rejected = Promise.reject('value'); rejected.catch(() => {});
    const revoked = Proxy.revocable({}, {}); revoked.revoke();
    const values = [undefined, null, -0, NaN, false, 1n, Symbol(), poison, rejected, revoked.proxy], visited = [];
    const received = await Observable.from(values).inspect(v => { visited.push(v); return poison; }).toArray();
    check(values.every((v, i) => Object.is(v, visited[i]) && Object.is(v, received[i])) && reads === 0, 'raw values and callback results are not assimilated');
    const Constructor = Observable, subscribe = Observable.prototype.subscribe, saved = [Observable, Observable.prototype.subscribe, Subscriber.prototype.next, Subscriber.prototype.error, Subscriber.prototype.complete];
    const source = Observable.from([1, 2]), log = [], bomb = () => { throw 'public method consulted'; };
    let result;
    try {
      globalThis.Observable = Constructor.prototype.subscribe = Subscriber.prototype.next = Subscriber.prototype.error = Subscriber.prototype.complete = bomb;
      result = method.call(source, v => log.push('inspect' + v)); subscribe.call(result, v => log.push(v));
    } finally { [globalThis.Observable, Constructor.prototype.subscribe, Subscriber.prototype.next, Subscriber.prototype.error, Subscriber.prototype.complete] = saved; }
    check(result instanceof Constructor, 'intrinsic result constructor');
    same(log, ['inspect1', 1, 'inspect2', 2], 'native subscription and notification operations');
  });
  return {checks, failures};
})()
