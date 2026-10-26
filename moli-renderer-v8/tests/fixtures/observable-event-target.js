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
  if (typeof EventTarget.prototype.when !== 'function') {
    check(false, 'EventTarget.when is exposed');
    return {checks, failures};
  }
  try {
    const when = EventTarget.prototype.when;
    const descriptor = Object.getOwnPropertyDescriptor(EventTarget.prototype, 'when');
    check(when.length === 1 && when.name === 'when', 'method name/length');
    check(descriptor.enumerable && descriptor.configurable && descriptor.writable, 'method descriptor');
    throws(() => new when('test'), 'method is not a constructor');
    const target = new EventTarget();
    const revoked = Proxy.revocable(target, {}); revoked.revoke();
    let conversions = 0, traps = 0;
    for (const fake of [1, {}, Object.create(target),
      new Proxy(target, {get() { traps++; }}), revoked.proxy]) {
      throws(() => when.call(fake, {toString() { conversions++; return 'test'; }},
        {get capture() { conversions++; }}), 'receiver validation');
    }
    check(conversions === 0 && traps === 0, 'receiver precedes conversion/proxy traps');
    check(when.call(null, 'test') instanceof Observable && when.call(undefined, 'test') instanceof Observable,
      'null and undefined this use the global EventTarget');
    throws(() => target.when(), 'event type is required');
    throws(() => target.when(Symbol()), 'event type uses DOMString');
    for (const options of [true, false, 1, 'capture', Symbol()]) {
      throws(() => target.when('test', options), 'options is a dictionary, not a boolean union');
    }
    const converted = [];
    const options = {
      get capture() { converted.push('capture'); return false; },
      get passive() { converted.push('passive'); return undefined; },
      get once() { throw new Error('must not read once'); },
      get signal() { throw new Error('must not read signal'); },
    };
    const source = target.when({toString() { converted.push('type'); return 'test'; }}, options);
    same(converted, ['type', 'capture', 'passive'], 'dictionary conversion order');
    check(source instanceof Observable && Object.getPrototypeOf(source) === Observable.prototype,
      'native Observable instance');
    check(target.when('test', null) instanceof Observable, 'null dictionary');
    check(target.when('test') !== source, 'each when call creates an Observable');
    const marker = {};
    try { target.when('test', {get capture() { throw marker; }}); check(false, 'getter exception'); }
    catch (error) { check(error === marker, 'getter exception identity'); }

    const order = [], first = new AbortController(), second = new AbortController();
    const before = () => order.push('before'), after = () => order.push('after');
    target.dispatchEvent(new Event('test'));
    target.addEventListener('test', before);
    let currentEvent;
    source.subscribe(function (event) {
      check(this === undefined && arguments.length === 1 && event === currentEvent, 'original event callback');
      check(event.currentTarget === target && event.eventPhase === Event.AT_TARGET, 'event dispatch state');
      order.push('first');
    }, {signal: first.signal});
    target.addEventListener('test', after);
    source.subscribe(() => order.push('second'), {signal: second.signal});
    currentEvent = new Event('test');
    target.dispatchEvent(currentEvent);
    same(order.splice(0), ['before', 'first', 'second', 'after'], 'lazy single listener multicasts in registration order');
    first.abort();
    target.dispatchEvent(new Event('test'));
    same(order.splice(0), ['before', 'second', 'after'], 'first abort keeps shared listener');
    second.abort();
    source.subscribe(() => order.push('restart'), {signal: AbortSignal.abort()});
    target.dispatchEvent(new Event('test'));
    same(order.splice(0), ['before', 'after'], 'last abort and pre-aborted subscription stop delivery');
    const restarted = new AbortController();
    source.subscribe(() => order.push('restart'), {signal: restarted.signal});
    target.dispatchEvent(new Event('test'));
    same(order.splice(0), ['before', 'after', 'restart'], 'restart installs a new listener');
    restarted.abort();
    same(converted, ['type', 'capture', 'passive'], 'subscription never rereads options');
    target.removeEventListener('test', before);
    target.removeEventListener('test', after);

    // stopImmediatePropagation affects subsequent listeners, not the other
    // observers sharing this one event listener.
    const multicast = target.when('stop');
    const stopped = new AbortController();
    multicast.subscribe(event => { order.push('a'); event.stopImmediatePropagation(); }, {signal: stopped.signal});
    multicast.subscribe(() => order.push('b'), {signal: stopped.signal});
    target.addEventListener('stop', () => order.push('unexpected'), {signal: stopped.signal});
    target.dispatchEvent(new Event('stop'));
    same(order.splice(0), ['a', 'b'], 'one physical listener despite multiple observers');
    stopped.abort();

    // Closing and reopening during a notification must not deliver the same
    // event to the newly installed listener.
    const reentrant = target.when('reentrant'), cancelled = new AbortController(), fresh = new AbortController();
    reentrant.subscribe(() => {
      order.push('old'); cancelled.abort();
      reentrant.subscribe(() => order.push('new'), {signal: fresh.signal});
    }, {signal: cancelled.signal});
    target.dispatchEvent(new Event('reentrant'));
    target.dispatchEvent(new Event('reentrant'));
    same(order.splice(0), ['old', 'new'], 'reentrant cancel/restart');
    fresh.abort();

    const savedObservable = globalThis.Observable, savedNext = Subscriber.prototype.next;
    const savedAdd = EventTarget.prototype.addEventListener, savedRemove = EventTarget.prototype.removeEventListener;
    const tampered = new AbortController();
    let nativeDeliveries = 0;
    try {
      globalThis.Observable = function () { throw marker; };
      Subscriber.prototype.next = EventTarget.prototype.addEventListener = EventTarget.prototype.removeEventListener = () => { throw marker; };
      target.addEventListener = target.removeEventListener = () => { throw marker; };
      const native = target.when('native');
      check(Object.getPrototypeOf(native) === savedObservable.prototype, 'intrinsic constructor under tampering');
      native.subscribe(() => nativeDeliveries++, {signal: tampered.signal});
      target.dispatchEvent(new Event('native'));
      tampered.abort();
      target.dispatchEvent(new Event('native'));
      check(nativeDeliveries === 1, 'native subscription/delivery/abort bypass public methods');
    } finally {
      globalThis.Observable = savedObservable;
      Subscriber.prototype.next = savedNext;
      EventTarget.prototype.addEventListener = savedAdd;
      EventTarget.prototype.removeEventListener = savedRemove;
      delete target.addEventListener; delete target.removeEventListener;
    }

    const targets = [new EventTarget(), globalThis, new AbortController().signal];
    if (typeof document !== 'undefined') {
      const detached = document.implementation.createHTMLDocument('');
      targets.push(document, document.body, document.createElement('div'),
        detached, detached.createElement('select'));
    }
    for (const [index, eventTarget] of targets.entries()) {
      const cancellation = new AbortController(), event = new Event('when-target');
      let seen;
      when.call(eventTarget, 'when-target').subscribe(value => { seen = value; }, {signal: cancellation.signal});
      eventTarget.dispatchEvent(event);
      check(seen === event, 'delivery on target ' + index);
      cancellation.abort(); seen = undefined;
      eventTarget.dispatchEvent(event);
      check(seen === undefined, 'abort on target ' + index);
    }

    for (const passive of [true, false, undefined]) {
      const cancellation = new AbortController();
      target.when('passive', {passive}).subscribe(event => event.preventDefault(), {signal: cancellation.signal});
      const event = new Event('passive', {cancelable: true});
      check(target.dispatchEvent(event) === (passive === true), 'passive dispatch return');
      check(event.defaultPrevented === (passive !== true), 'passive preventDefault');
      cancellation.abort();
    }
    if (typeof document !== 'undefined') {
      const node = document.body.appendChild(document.createElement('div'));
      try {
        const cancellation = new AbortController(), phases = [];
        document.body.when('phases', {capture: true}).subscribe(event => phases.push(event.eventPhase), {signal: cancellation.signal});
        document.body.when('phases').subscribe(event => phases.push(event.eventPhase), {signal: cancellation.signal});
        node.dispatchEvent(new Event('phases', {bubbles: true}));
        same(phases, [Event.CAPTURING_PHASE, Event.BUBBLING_PHASE], 'DOM propagation');
        cancellation.abort();
        for (const eventTarget of [globalThis, document, document.documentElement, document.body, node]) {
          for (const passive of [undefined, false, true]) {
            const ac = new AbortController(), event = new Event('wheel', {cancelable: true});
            eventTarget.when('wheel', {passive}).subscribe(value => value.preventDefault(), {signal: ac.signal});
            eventTarget.dispatchEvent(event);
            const expected = passive === false || (passive === undefined && eventTarget === node);
            check(event.defaultPrevented === expected, 'DOM default passive ' + passive);
            ac.abort();
          }
        }
      } finally { node.remove(); }
    }
  } catch (error) { failures.push('unexpected: ' + error.stack); }
  return {checks, failures};
})()
