(() => {
  const cases = {};
  for (const cancellable of [true, false]) (() => {
    const entry = {closes: 0};
    const iterator = {
      next: () => new Promise(() => {}),
      return() { entry.closes++; return {done: true}; }
    };
    entry.iterator = new WeakRef(iterator);
    const source = Observable.from({[Symbol.asyncIterator]: () => iterator});
    if (cancellable) {
      entry.controller = new AbortController();
      source.subscribe({}, {signal: entry.controller.signal});
    } else {
      source.subscribe();
    }
    cases[cancellable ? 'live' : 'abandoned'] = entry;
  })();
  globalThis.observableIteratorGc = cases;
  return 'ready';
})()
