(() => {
  'use strict';
  const failures = [];
  let checks = 0;
  const check = (ok, label) => { checks++; if (!ok) failures.push(label); };
  const same = (actual, expected, label) => check(JSON.stringify(actual) === JSON.stringify(expected), label);
  const throws = (fn, label, expected = TypeError) => {
    try { fn(); check(false, label); }
    catch (error) { check(error instanceof expected, label); }
  };
  const reported = [];
  const onError = event => { reported.push(event.error); event.preventDefault(); };
  addEventListener('error', onError);
  try {
    for (const make of [() => Observable(() => {}), () => new Observable(),
      () => new Observable({}), () => new Subscriber()]) throws(make, 'constructor validation');
    let calls = 0, subscriber;
    const source = new Observable(function (value) {
      check(this === undefined && arguments.length === 1, 'initializer callback this/arguments');
      calls++;
      subscriber = value;
    });
    check(calls === 0, 'construction is lazy');
    const order = [];
    const controller1 = new AbortController(), controller2 = new AbortController();
    source.subscribe({ next(value) { check(this === undefined, 'observer this'); order.push('a:' + value); } }, {signal: controller1.signal});
    const signal = subscriber.signal;
    check(signal !== controller1.signal && signal === subscriber.signal, 'stable independent signal');
    check(subscriber.active && !signal.aborted, 'active subscription');
    source.subscribe(value => order.push('b:' + value), {signal: controller2.signal});
    check(calls === 1, 'shared producer');
    subscriber.addTeardown(() => { order.push('first teardown'); check(!subscriber.active && signal.aborted, 'teardown sees closed state'); });
    subscriber.addTeardown(() => order.push('second teardown'));
    signal.addEventListener('abort', () => order.push('abort'));
    subscriber.next(1);
    controller1.abort('one');
    check(subscriber.active && !signal.aborted, 'first cancellation keeps producer active');
    subscriber.next(2);
    controller2.abort('two');
    check(!subscriber.active && signal.reason === 'two', 'last cancellation closes with reason');
    subscriber.next(3);
    subscriber.complete();
    same(order, ['a:1', 'b:1', 'b:2', 'abort', 'second teardown', 'first teardown'], 'cancellation/teardown order');
    source.subscribe();
    check(calls === 2 && subscriber.signal !== signal, 'restart after cancellation');
    subscriber.complete();

    const converted = [];
    const observer = {};
    for (const name of ['next', 'error', 'complete']) Object.defineProperty(observer, name, {get() { converted.push(name); return undefined; }});
    source.subscribe(observer, {get signal() { converted.push('signal'); return undefined; }});
    same(converted, ['complete', 'error', 'next', 'signal'], 'dictionary conversion order');
    subscriber.complete();
    for (const observer of [1, true, 'text', {next: null}, {error: 1}, {complete: {}}]) throws(() => source.subscribe(observer), 'invalid observer');
    for (const signal of [null, {}, new Proxy(new AbortController().signal, {})]) throws(() => source.subscribe({}, {signal}), 'signal brand');
    for (const options of [1, 'text', true]) throws(() => source.subscribe({}, options), 'invalid options');
    const marker = {marker: true};
    try { source.subscribe({get next() { throw marker; }}); check(false, 'getter throws'); }
    catch (error) { check(error === marker, 'getter preserves exception'); }
    let conversions = 0, traps = 0;
    const revoked = Proxy.revocable(source, {}); revoked.revoke();
    for (const fake of [{}, Object.create(source), new Proxy(source, {get() { traps++; }}), revoked.proxy]) {
      throws(() => Observable.prototype.subscribe.call(fake, {get next() { conversions++; }}), 'Observable receiver');
    }
    check(conversions === 0 && traps === 0, 'receiver precedes conversion and proxy traps');
    for (const fake of [{}, Object.create(subscriber), new Proxy(subscriber, {})]) {
      for (const name of ['next', 'error', 'complete', 'addTeardown']) throws(() => Subscriber.prototype[name].call(fake, () => {}), 'Subscriber method receiver');
      for (const name of ['active', 'signal']) throws(() => Object.getOwnPropertyDescriptor(Subscriber.prototype, name).get.call(fake), 'Subscriber getter receiver');
    }
    throws(() => subscriber.next(), 'next requires value');
    throws(() => subscriber.error(), 'error requires value');
    throws(() => subscriber.addTeardown(null), 'teardown requires callback');

    const preAborted = AbortSignal.abort(marker), closed = [];
    new Observable(s => {
      check(!s.active && s.signal.aborted && s.signal.reason === marker, 'pre-aborted initializer runs inactive');
      s.addTeardown(() => closed.push('a'));
      s.addTeardown(() => closed.push('b'));
      s.next(1); s.complete();
    }).subscribe(() => closed.push('unexpected'), {signal: preAborted});
    same(closed, ['a', 'b'], 'inactive teardown runs synchronously');

    const notifications = [];
    let shared;
    const multicast = new Observable(s => { shared = s; });
    const cancelled = new AbortController();
    multicast.subscribe(value => {
      notifications.push('a' + value);
      if (value === 1) {
        multicast.subscribe(value => notifications.push('c' + value));
        cancelled.abort();
      }
    });
    multicast.subscribe(value => notifications.push('b' + value), {signal: cancelled.signal});
    shared.next(1); shared.next(2);
    same(notifications, ['a1', 'b1', 'a2', 'c2'], 'notification uses observer snapshot');
    shared.complete();

    const exceptions = [new Error('initializer'), new Error('late'), new Error('observer'), new Error('teardown')];
    let handled;
    new Observable(() => { throw exceptions[0]; }).subscribe({error(error) { handled = error; }});
    check(handled === exceptions[0] && reported.length === 0, 'initializer exception goes to observer');
    new Observable(s => { s.complete(); s.error(exceptions[1]); }).subscribe();
    check(reported.pop() === exceptions[1], 'late error is reported');
    const lifecycle = [];
    new Observable(s => {
      s.addTeardown(() => lifecycle.push('first'));
      s.addTeardown(() => { lifecycle.push('throwing'); throw exceptions[3]; });
      s.addTeardown(() => { lifecycle.push('last'); s.addTeardown(() => lifecycle.push('nested')); s.complete(); });
      s.next(1);
      s.complete();
    }).subscribe({next() { throw exceptions[2]; }, complete() { lifecycle.push('complete'); }});
    same(lifecycle, ['last', 'nested', 'throwing', 'first', 'complete'], 'reentrant teardown and exceptions');
    check(reported.shift() === exceptions[2] && reported.shift() === exceptions[3], 'callback exceptions reported in order');

    // Author changes to the public constructors/methods must not redirect the
    // native close algorithm or creation of its signal.
    const savedAbortController = globalThis.AbortController;
    const savedAbort = AbortController.prototype.abort;
    const savedComplete = Subscriber.prototype.complete;
    const savedError = Subscriber.prototype.error;
    const savedSignal = Object.getOwnPropertyDescriptor(AbortController.prototype, 'signal');
    try {
      globalThis.AbortController = () => { throw marker; };
      savedAbortController.prototype.abort = () => { throw marker; };
      Object.defineProperty(savedAbortController.prototype, 'signal', {configurable: true, get() { throw marker; }});
      Subscriber.prototype.complete = Subscriber.prototype.error = () => { throw marker; };
      let completed = false, seen;
      new Observable(s => { savedComplete.call(s); }).subscribe({complete() { completed = true; }});
      new Observable(() => { throw marker; }).subscribe({error(value) { seen = value; }});
      check(completed && seen === marker, 'internal algorithms bypass public methods');
    } finally {
      globalThis.AbortController = savedAbortController;
      savedAbortController.prototype.abort = savedAbort;
      Object.defineProperty(savedAbortController.prototype, 'signal', savedSignal);
      Subscriber.prototype.complete = savedComplete;
      Subscriber.prototype.error = savedError;
    }
    check(reported.length === 0, 'no unexpected callback errors');
  } catch (error) {
    failures.push('uncaught: ' + error);
  } finally {
    removeEventListener('error', onError);
  }
  return {checks, failures};
})()
