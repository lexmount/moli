(() => {
  let checks = 0;
  const check = (condition, message) => {
    checks++;
    if (!condition) throw new Error(message);
  };
  const target = () => typeof document === 'undefined' ? new EventTarget() : document.createElement('div');
  for (const listenerFirst of [true, false]) {
    const ac = new AbortController(), el = target(), log = [];
    const listen = () => el.addEventListener('x', () => log.push('listener'), {signal: ac.signal});
    if (listenerFirst) listen();
    new Observable(s => s.addTeardown(() => {
      log.push('teardown');
      el.dispatchEvent(new Event('x'));
    })).subscribe({}, {signal: ac.signal});
    if (!listenerFirst) listen();
    ac.abort();
    check(JSON.stringify(log) === JSON.stringify(listenerFirst ? ['teardown'] : ['teardown', 'listener']),
      'listener removal keeps its position among abort steps');
    el.dispatchEvent(new Event('x'));
    check(log.length === (listenerFirst ? 1 : 2), 'listener remains removed after abort');
  }
  for (const marker of [{}, undefined]) {
    const ac = new AbortController(), el = target(), log = [];
    let caught, didThrow = false, pulls = 0;
    el.addEventListener('x', () => log.push('earlier listener'), {signal: ac.signal});
    ac.signal.addEventListener('abort', () => log.push('source abort'));
    const dependent = AbortSignal.any([ac.signal]);
    dependent.addEventListener('abort', () => log.push('dependent abort'));
    const iterable = {[Symbol.iterator]: () => ({
      next: () => ++pulls === 1 ? {value: 1} : {done: true},
      return() { log.push('return'); throw marker; }
    })};
    Observable.from(iterable).subscribe(() => {
      el.addEventListener('x', () => log.push('later listener'), {signal: ac.signal});
      try { ac.abort('stop'); } catch (error) { caught = error; didThrow = true; }
      el.dispatchEvent(new Event('x'));
    }, {signal: ac.signal});
    check(didThrow && caught === marker, 'first IteratorClose exception is preserved');
    check(JSON.stringify(log) === JSON.stringify(['return', 'source abort', 'dependent abort']),
      'throwing IteratorClose still removes listeners and dispatches source/dependent abort');
    ac.abort();
    check(log.length === 3, 'second abort does not replay completed steps');
  }
  return {checks};
})()
