(() => {
  const refs = [];
  for (const mode of ['subscribe', 'first', 'listener']) (() => {
    const source = new Observable(s => {
      refs.push(new WeakRef(s), new WeakRef(s.signal));
      if (mode === 'listener') s.signal.addEventListener('abort', () => s.active);
      else s.signal.owner = s;
    });
    source[mode === 'first' ? 'first' : 'subscribe']();
  })();
  globalThis.observableSignalGc = refs;
  return 'ready';
})()
