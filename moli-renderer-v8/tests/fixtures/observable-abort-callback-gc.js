(() => {
  'use strict';
  const state = {abandoned: {}, live: {}};
  const observer = entry => {
    const token = {increment: 1};
    entry.token = new WeakRef(token);
    return () => { entry.events += token.increment; };
  };

  (() => {
    const target = new EventTarget(), entry = {events: 0};
    const source = target.when('tick');
    source.subscribe(observer(entry));
    state.abandoned.when = [new WeakRef(target), new WeakRef(source), entry.token];
  })();
  (() => {
    new Observable(outer => {
      new Observable(inner => {
        state.abandoned.nested = [outer, outer.signal, inner, inner.signal].map(value => new WeakRef(value));
      }).subscribe({}, {signal: outer.signal});
    }).subscribe();
  })();
  (() => {
    new Observable(outer => {
      new Observable(inner => {
        state.abandoned.first = [outer, inner].map(value => new WeakRef(value));
      }).first({signal: outer.signal}).catch(() => {});
    }).subscribe();
  })();

  // A retained target must keep an active event stream and its observer alive.
  (() => {
    const entry = {events: 0, target: new EventTarget()};
    entry.target.when('tick').subscribe(observer(entry));
    state.live.target = entry;
  })();

  // Only the outer producer survives. Its signal must retain both an ordinary
  // script subscription and a when() subscription, including their cleanup.
  (() => {
    const entry = {events: 0, teardowns: 0, abortEvents: 0};
    new Observable(outer => {
      entry.producer = outer;
      entry.signal = outer.signal;
      const target = new EventTarget();
      entry.target = new WeakRef(target);
      target.when('tick').subscribe(observer(entry), {signal: outer.signal});
      new Observable(inner => {
        entry.inner = new WeakRef(inner);
        inner.addTeardown(() => entry.teardowns++);
        inner.signal.addEventListener('abort', () => entry.abortEvents++);
      }).subscribe({}, {signal: outer.signal});
    }).subscribe();
    state.live.signal = entry;
  })();

  // Retaining a public cancellation handle keeps a pending Promise consumer
  // cancellable even if the Promise and Subscriber have no direct JS owner.
  (() => {
    const entry = {controller: new AbortController(), teardowns: 0};
    new Observable(inner => {
      entry.inner = new WeakRef(inner);
      inner.addTeardown(() => entry.teardowns++);
    }).first({signal: entry.controller.signal}).catch(reason => { entry.reason = reason; });
    state.live.caller = entry;
  })();

  // A kept dependent must not retain an otherwise unreachable source signal.
  // The pending Promise still keeps its own producer available for delivery.
  const pendingFirst = (signal, entry) => new Observable(inner => {
    entry.inner = new WeakRef(inner);
  }).first({signal});
  (() => {
    const entry = {};
    new Observable(outer => {
      outer.signal.owner = outer;
      entry.outer = new WeakRef(outer);
      entry.signal = new WeakRef(outer.signal);
      entry.promise = pendingFirst(outer.signal, entry);
      entry.promise.then(value => { entry.value = value; });
    }).subscribe();
    state.live.weakSource = entry;
  })();

  // Unregistering must release captured data even while the signal lives on.
  for (const mode of ['complete', 'remove']) {
    const entry = {events: 0, target: new EventTarget()};
    new Observable(outer => {
      entry.producer = outer;
      if (mode === 'complete') {
        new Observable(inner => inner.complete()).subscribe(observer(entry), {signal: outer.signal});
      } else {
        const listener = observer(entry);
        entry.target.addEventListener('tick', listener, {signal: outer.signal});
        entry.target.removeEventListener('tick', listener);
      }
    }).subscribe();
    state.live[mode] = entry;
  }
  globalThis.observableAbortCallbackGc = state;
  return 'ready';
})()
