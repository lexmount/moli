globalThis.navigationCancellationProbe = async function(kind, cause, phase) {
  const frame = document.createElement('iframe');
  frame.src = new URL('/child', location.href).href;
  const loaded = new Promise(resolve => frame.onload = resolve);
  (document.body || document.documentElement || document).appendChild(frame);
  await loaded;
  const child = frame.contentWindow;
  const navigation = child.navigation;
  const Exception = child.DOMException;
  const documentURL = child.location.href;
  if (kind === 'traverse') child.history.pushState(null, '', documentURL + '#pushed');
  const key = navigation.entries()[0].key;
  const abortStates = [];
  let event;
  let afterPreventDefault;
  const cancel = () => {
    if (cause === 'detach') frame.remove();
    if (cause === 'stop') child.stop();
    if (cause === 'nested') {
      const replacement = navigation.navigate(documentURL + '#replacement');
      replacement.committed.catch(() => {});
      replacement.finished.catch(() => {});
    }
  };
  navigation.addEventListener('navigate', e => {
    if (event) return;
    event = e;
    if (kind === 'traverse') {
      e.preventDefault();
      afterPreventDefault = e.defaultPrevented;
    }
    e.signal.addEventListener('abort', () => {
      abortStates.push([e.cancelable, e.defaultPrevented]);
    });
    if (cause === 'preventDefault') return;
    if (phase === 'precommit') e.intercept({precommitHandler: cancel});
    else {
      e.intercept();
      cancel();
    }
  });
  const result = kind === 'traverse' ? navigation.traverseTo(key) : navigation.navigate(documentURL + '#destination');
  const outcome = await Promise.allSettled([result.committed, result.finished]);
  const record = {
    cancelable: event.cancelable,
    defaultPrevented: event.defaultPrevented,
    afterPreventDefault: afterPreventDefault ?? null,
    aborted: event.signal.aborted,
    abortStates,
    promises: outcome.map(value => value.status === 'fulfilled' ? 'fulfilled' : value.reason.name),
    sameReason: outcome.every(value => value.status === 'fulfilled' || value.reason === event.signal.reason),
    calleeRealm: outcome.every(value => value.status === 'fulfilled' || value.reason instanceof Exception)
  };
  frame.remove();
  return record;
};
