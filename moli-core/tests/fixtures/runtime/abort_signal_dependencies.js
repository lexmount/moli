function runAbortSignalDependencyProbe(scenario) {
  const errors = [];
  const events = [];
  const check = (condition, label) => { if (!condition) errors.push(label); };
  const checkReason = (signal, reason, label) => {
    check(signal.aborted && signal.reason === reason, label);
    let caught = false;
    try { signal.throwIfAborted(); }
    catch (error) { caught = true; check(error === reason, label + ': thrown reason'); }
    check(caught, label + ': throwIfAborted throws');
  };

  try {
    if (scenario === 'graph') {
      const first = new AbortController();
      const second = new AbortController();
      const empty = AbortSignal.any([]);
      const emptyClone = AbortSignal.any([empty]);
      const signals = [first.signal];
      signals.push(AbortSignal.any([first.signal, second.signal]));
      signals.push(AbortSignal.any([first.signal]));
      signals.push(AbortSignal.any([signals[1]]));
      signals.push(AbortSignal.any([signals[3], signals[1], first.signal, second.signal]));
      signals.push(AbortSignal.any([emptyClone, first.signal]));
      const reason = {source: 'first'};
      signals.forEach((signal, index) => signal.addEventListener('abort', event => {
        events.push(String(index));
        check(event.target === signal, 'target ' + index);
        signals.forEach((dependent, i) => checkReason(dependent, reason, index + '/' + i));
        checkReason(AbortSignal.any([emptyClone, signals[4], second.signal]), reason, 'late composite');
      }));
      first.abort(reason);
      second.abort({source: 'second'});
      signals.forEach((signal, index) => checkReason(signal, reason, 'final ' + index));
      check(!empty.aborted && empty.reason === undefined, 'empty signal stays pending');
      check(!emptyClone.aborted && emptyClone.reason === undefined, 'empty composite stays pending');
    } else if (scenario === 'reentrant') {
      const first = new AbortController();
      const second = new AbortController();
      const shared = AbortSignal.any([first.signal, second.signal]);
      const secondary = AbortSignal.any([second.signal]);
      const nested = AbortSignal.any([shared, secondary]);
      const firstReason = {source: 'first'};
      const secondReason = {source: 'second'};
      first.signal.addEventListener('abort', () => {
        events.push('first');
        checkReason(shared, firstReason, 'shared before reentry');
        checkReason(nested, firstReason, 'nested before reentry');
        second.abort(secondReason);
      });
      second.signal.addEventListener('abort', () => {
        events.push('second');
        checkReason(shared, firstReason, 'shared during reentry');
        checkReason(nested, firstReason, 'nested during reentry');
        checkReason(secondary, secondReason, 'secondary during reentry');
        for (const [sources, reason] of [
          [[secondary, shared], secondReason], [[shared, secondary], firstReason]
        ]) {
          const late = AbortSignal.any(sources);
          checkReason(late, reason, 'already aborted input order');
          late.onabort = () => errors.push('already aborted composite dispatched again');
        }
      });
      secondary.onabort = () => {
        events.push('secondary');
        first.abort({source: 'repeat'});
      };
      shared.onabort = () => events.push('shared');
      nested.onabort = () => events.push('nested');
      first.abort(firstReason);
      checkReason(shared, firstReason, 'shared final reason');
      checkReason(nested, firstReason, 'nested final reason');
      checkReason(secondary, secondReason, 'secondary final reason');
    } else if (scenario === 'listeners') {
      const controller = new AbortController();
      const first = AbortSignal.any([controller.signal]);
      const last = AbortSignal.any([first]);
      const targets = [new EventTarget()];
      if (typeof document !== 'undefined') targets.push(document.createElement('div'));
      const counts = targets.map(() => 0);
      targets.forEach((target, index) => target.addEventListener('probe', () => {
        counts[index]++;
      }, {signal: first}));
      const dispatch = () => targets.forEach(target => target.dispatchEvent(new Event('probe')));
      const removed = () => errors.push('removed dependent listener fired');
      const removedLast = () => errors.push('removed nested listener fired');
      first.addEventListener('abort', removed);
      last.addEventListener('abort', removedLast);
      last.addEventListener('abort', () => events.push('last'));
      first.addEventListener('abort', () => {
        events.push('first');
        dispatch();
        check(counts.every(count => count === 1), 'listener removal precedes dependent event');
        last.removeEventListener('abort', removedLast);
        last.addEventListener('abort', () => events.push('late-last'));
        first.addEventListener('abort', () => errors.push('listener added to active dispatch fired'));
      });
      controller.signal.addEventListener('abort', () => {
        events.push('source');
        check(first.aborted && last.aborted, 'dependent state precedes source event');
        dispatch();
        check(counts.every(count => count === 1), 'dependent algorithms wait for source event');
        first.removeEventListener('abort', removed);
        first.addEventListener('abort', () => events.push('added'));
        first.onabort = () => events.push('handler');
      });
      controller.abort('reason');
      dispatch();
      check(counts.every(count => count === 1), 'listeners remain removed');
    } else {
      errors.push('unknown scenario ' + scenario);
    }
  } catch (error) {
    errors.push(String(error));
  }
  return {errors, events};
}
