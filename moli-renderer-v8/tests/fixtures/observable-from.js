(async () => {
  'use strict';
  const failures = [];
  let checks = 0;
  const check = (value, label) => { checks++; if (!value) failures.push(label); };
  const same = (actual, expected, label) => check(JSON.stringify(actual) === JSON.stringify(expected), label);
  const thrown = fn => { try { fn(); } catch (error) { return error; } };
  const test = async (label, fn) => { try { await fn(); } catch (error) { check(false, label + ': ' + error); } };
  if (typeof Observable.from !== 'function') {
    check(false, 'Observable.from is exposed');
    return {checks, failures};
  }
  const from = Observable.from;
  const collect = source => new Promise((resolve, reject) => {
    const values = [];
    source.subscribe({next: value => values.push(value), complete: () => resolve(values), error: reject});
  });

  await test('conversion', () => {
    const descriptor = Object.getOwnPropertyDescriptor(Observable, 'from');
    check(from.length === 1 && from.name === 'from', 'static name and length');
    check(descriptor.writable && descriptor.enumerable && descriptor.configurable, 'static descriptor');
    check(thrown(() => new from([])) instanceof TypeError, 'from is not constructible');
    check(thrown(() => from()) instanceof TypeError, 'required argument');
    for (const value of [undefined, null, false, 0, 1n, '', 'abc', Symbol(), {}, {then() {}}]) {
      check(thrown(() => from(value)) instanceof TypeError, 'reject unsupported value ' + String(value));
    }
    check(from.call(null, []) instanceof Observable, 'static receiver ignored');
    const marker = {}, native = new Observable(() => {});
    Object.defineProperty(native, Symbol.asyncIterator, {get() { throw marker; }});
    check(from(native) === native, 'native Observable returned before protocol lookup');
    const fake = Object.create(Observable.prototype);
    check(thrown(() => from(fake)) instanceof TypeError, 'forged Observable rejected');
    check(thrown(() => from(new Proxy(native, {}))) === marker, 'author Proxy does not inherit Observable brand');
    const boxed = []; from(new String('ab')).subscribe(value => boxed.push(value));
    same(boxed, ['a', 'b'], 'boxed string iterable');

    const reads = [], input = {
      get [Symbol.asyncIterator]() { reads.push('async'); return null; },
      get [Symbol.iterator]() { reads.push('sync'); return Array.prototype[Symbol.iterator]; },
      0: 7, length: 1,
    };
    const source = from(input);
    same(reads, ['async', 'sync'], 'conversion probes without calling');
    const values = []; source.subscribe(value => values.push(value));
    same(reads, ['async', 'sync', 'sync'], 'subscription rereads selected sync protocol');
    same(values, [7], 'sync method receiver');
    for (const symbol of [Symbol.asyncIterator, Symbol.iterator]) {
      check(thrown(() => from({[symbol]: 3})) instanceof TypeError, 'non-callable protocol');
      check(thrown(() => from({get [symbol]() { throw marker; }})) === marker, 'protocol getter error identity');
    }
    input[Symbol.iterator];
    Object.defineProperty(input, Symbol.iterator, {value: undefined});
    let error; source.subscribe({error: value => { error = value; }});
    check(error instanceof TypeError, 'missing protocol at subscription is observer error');
  });

  await test('sync iteration', () => {
    const log = [], marker = {}, promisedValue = Promise.resolve(2);
    let getterReads = 0, returns = 0;
    const iterator = {
      count: 0,
      get next() {
        getterReads++;
        return new Proxy(function () {
          check(this === iterator && arguments.length === 0, 'cached next receiver and arguments');
          return this.count++ ? {done: true, get value() { throw marker; }} : {value: promisedValue};
        }, {});
      },
      return() { returns++; return {}; },
    };
    const input = {[Symbol.iterator]: new Proxy(function () { return iterator; }, {})};
    from(input).subscribe({next: value => log.push(value === promisedValue), complete: () => log.push('done')});
    same(log, [true, 'done'], 'sync values are not awaited and final value is not read');
    check(getterReads === 1 && returns === 0, 'next cached and exhaustion does not close');
    const repeated = from([1, 2]);
    const values = []; repeated.subscribe(value => values.push(value)); repeated.subscribe(value => values.push(value));
    same(values, [1, 2, 1, 2], 'fresh subscription gets fresh iterator');
    for (const bad of [
      {next() { throw marker; }},
      {get next() { throw marker; }},
      {next() { return {get done() { throw marker; }}; }},
      {next() { return {done: false, get value() { throw marker; }}; }},
    ]) {
      bad.return = () => { returns++; return {}; };
      let error; from({[Symbol.iterator]: () => bad}).subscribe({error: value => { error = value; }});
      check(error === marker, 'sync iterator error identity');
    }
    check(returns === 0, 'failed iteration does not call return');
    for (const bad of [{next: null}, {next: () => 1}]) {
      let error; from({[Symbol.iterator]: () => bad}).subscribe({error: value => { error = value; }});
      check(error instanceof TypeError, 'invalid next or result');
    }
  });

  await test('sync cancellation', () => {
    const ac = new AbortController(), reason = {}, closeError = {};
    let nextCalls = 0, returnCalls = 0, caught;
    const iterator = {
      next() { nextCalls++; return {value: 1, done: false}; },
      return() {
        returnCalls++;
        check(this === iterator && arguments.length === 0, 'sync return receiver and zero arguments');
        throw closeError;
      },
    };
    from({[Symbol.iterator]: () => iterator}).subscribe(() => { caught = thrown(() => ac.abort(reason)); }, {signal: ac.signal});
    check(caught === closeError, 'sync return exception escapes abort with identity');
    check(ac.signal.aborted && ac.signal.reason === reason, 'abort state visible despite close error');
    ac.abort();
    check(nextCalls === 1 && returnCalls === 1, 'cancel stops pulling and closes once');
    const invalid = new AbortController();
    let error;
    from({[Symbol.iterator]: () => ({next: () => ({value: 1}), return: () => 0})})
      .subscribe(() => { error = thrown(() => invalid.abort()); }, {signal: invalid.signal});
    check(error instanceof TypeError && error.message.includes('return()') && error.message.includes('Object'), 'sync close result must be Object');
    let reads = 0;
    const source = from({get [Symbol.iterator]() { reads++; return () => iterator; }});
    source.subscribe({next() { throw closeError; }}, {signal: AbortSignal.abort()});
    check(reads === 1, 'pre-abort skips protocol read');
    const duringOpen = new AbortController();
    const opening = from({[Symbol.iterator]() { duringOpen.abort(); return iterator; }});
    opening.subscribe({}, {signal: duringOpen.signal});
    check(nextCalls === 1 && returnCalls === 1, 'abort during open neither pulls nor closes');
  });

  await test('async timing', async () => {
    const marker = {}, log = [];
    from({[Symbol.asyncIterator]() { throw marker; }}).subscribe({error: error => log.push(error === marker)});
    same(log.splice(0), [true], 'async protocol call errors are synchronous');
    from({[Symbol.asyncIterator]: () => ({get next() { throw marker; }})}).subscribe({error: error => log.push(error === marker)});
    same(log.splice(0), [true], 'GetIterator caches next synchronously');
    from({[Symbol.asyncIterator]: () => ({next() { throw marker; }})}).subscribe({error: error => log.push(error === marker)});
    same(log, [], 'async next call exception is deferred');
    await Promise.resolve();
    same(log.splice(0), [true], 'async next call rejection delivery');
    const source = from({[Symbol.asyncIterator]: () => ({next: () => ({get done() { log.push('done'); return true; }})})});
    source.subscribe({complete: () => log.push('complete')});
    same(log, [], 'raw async result is wrapped');
    await Promise.resolve();
    same(log.splice(0), ['done', 'complete'], 'done read and completion at microtask');
    const values = await collect(from((async function* () { yield 1; yield 2; })()));
    same(values, [1, 2], 'async generator values');
    for (const result of [0, Promise.resolve(0), {get done() { throw marker; }}, {done: false, get value() { throw marker; }}]) {
      let error, closes = 0;
      const done = new Promise(resolve => from({[Symbol.asyncIterator]: () => ({next: () => result, return() { closes++; return {}; }})})
        .subscribe({error: value => { error = value; resolve(); }}));
      check(error === undefined, 'async iteration errors are deferred');
      await done;
      check(error === marker || error instanceof TypeError, 'async iteration error');
      check(closes === 0, 'async iteration errors do not close');
    }
  });

  await test('async cancellation and multicast', async () => {
    let resolve, calls = 0, closed = 0;
    const pending = new Promise(r => { resolve = r; });
    const first = new AbortController(), last = new AbortController(), reason = {}, events = [];
    const source = from({[Symbol.asyncIterator]: () => ({
      next() { calls++; return pending; },
      return(value) { check(value === reason && arguments.length === 1, 'async return gets abort reason'); closed++; return {}; },
    })});
    source.subscribe(value => events.push(value), {signal: first.signal});
    source.subscribe(value => events.push(value), {signal: last.signal});
    check(calls === 1, 'concurrent observers share async iterator');
    first.abort();
    check(closed === 0, 'one remaining observer keeps iterator');
    last.abort(reason);
    check(closed === 1, 'last observer closes iterator');
    resolve({get done() { events.push('done'); return false; }, get value() { events.push('value'); return 9; }});
    await Promise.resolve();
    same(events, ['done', 'value'], 'queued getters still run after abort without delivering value');
    check(calls === 1, 'no next after cancellation');
  });

  await test('Promise conversion and native entry points', async () => {
    const promise = Promise.resolve(8), marker = {}, log = [];
    Object.defineProperty(promise, 'then', {get() { throw marker; }});
    from(promise).subscribe({next: value => log.push(value), complete: () => log.push('complete')});
    same(log, [], 'Promise conversion is asynchronous');
    await Promise.resolve();
    same(log, [8, 'complete'], 'native Promise reactions ignore replaced then');
    const failed = Promise.reject(marker);
    let error; from(failed).subscribe({error: value => { error = value; }});
    await Promise.resolve();
    check(error === marker, 'Promise rejection identity');
    const preferred = Promise.resolve('promise');
    preferred[Symbol.iterator] = function* () { yield 'iterable'; };
    same(await collect(from(preferred)), ['iterable'], 'iterable precedes Promise');
    const Constructor = Observable, saved = [Subscriber.prototype.next, Subscriber.prototype.complete];
    try {
      globalThis.Observable = Subscriber.prototype.next = Subscriber.prototype.complete = () => { throw marker; };
      same(await collect(from([3])), [3], 'native sync producer bypasses replaced JS methods');
      same(await collect(from(Promise.resolve(4))), [4], 'native Promise producer bypasses replaced JS methods');
    } finally {
      globalThis.Observable = Constructor;
      [Subscriber.prototype.next, Subscriber.prototype.complete] = saved;
    }
  });

  // Current Chromium does not implement this GetIterator(async) fallback. Keep
  // it in the shared fixture so a browser comparison records that difference.
  const fallback = iterator => {
    let probes = 0;
    return from({get [Symbol.asyncIterator]() { return ++probes === 1 ? () => {} : undefined; }, [Symbol.iterator]: () => iterator});
  };
  await test('async-from-sync fallback', async () => {
    const events = []; let count = 0;
    const source = fallback({next() {
      const done = count++ > 0;
      return {done, get value() { events.push(done ? 'final' : 'value'); return Promise.resolve(5); }};
    }});
    same(await collect(source), [5], 'fallback awaits yielded value');
    same(events, ['value', 'final'], 'fallback also awaits final value');
  });
  await test('fallback rejected value closes iterator', async () => {
    const marker = {}, closeError = {}; let closed = 0, error;
    await new Promise(resolve => fallback({
      next: () => ({done: false, value: Promise.reject(marker)}),
      return() { closed++; throw closeError; },
    }).subscribe({error: value => { error = value; resolve(); }}));
    check(error === marker && closed === 1, 'fallback rejection preserves value error through close');
  });
  await test('fallback PromiseResolve constructor failure closes iterator', async () => {
    const marker = {}, value = Promise.resolve(1); let closed = 0, error;
    Object.defineProperty(value, 'constructor', {get() { throw marker; }});
    await new Promise(resolve => fallback({
      next: () => ({done: false, value}), return() { closed++; return {}; },
    }).subscribe({error: value => { error = value; resolve(); }}));
    check(error === marker && closed === 1, 'fallback uses ECMAScript PromiseResolve');
  });
  return {checks, failures};
})()
