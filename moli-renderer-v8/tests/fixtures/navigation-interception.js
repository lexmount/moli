globalThis.navigationInterceptionProbe = async function(api, cause) {
  const frame = document.createElement('iframe');
  frame.src = new URL('/child', location.href).href;
  const loaded = new Promise(resolve => frame.onload = resolve);
  (document.body || document.documentElement || document).appendChild(frame);
  await loaded;
  const child = frame.contentWindow;
  const nav = child.navigation;
  const url = child.location.href;
  const Exception = child.DOMException;
  const failure = new child.Error('handler failure');
  const log = [];
  const done = Promise.withResolvers();
  let event;
  let transition;
  let observedTransition;
  let committedEntry = nav.currentEntry;
  let entryChanges = 0;
  let error;
  nav.addEventListener('currententrychange', () => {
    committedEntry = nav.currentEntry;
    entryChanges++;
  });
  nav.addEventListener('navigateerror', e => {
    error = e.error;
    log.push('error:' + error.name);
    done.resolve();
  });
  nav.addEventListener('navigatesuccess', () => {
    log.push('success');
    done.resolve();
  });
  nav.addEventListener('navigate', e => {
    event = e;
    log.push('navigate');
    e.signal.addEventListener('abort', () => log.push('abort'));
    const options = {handler() {
      transition = nav.transition;
      observedTransition = Promise.allSettled([transition.committed, transition.finished]);
      log.push('first');
      if(cause === 'detach') frame.remove();
      if(cause === 'stop') child.stop();
      if(cause === 'throw') throw failure;
      log.push('first-after');
    }};
    if(api === 'precommit') options.precommitHandler = () => Promise.resolve();
    e.intercept(options);
    e.intercept({handler() {log.push('second');}});
  }, {once: true});
  let result;
  if(api === 'location-fragment') child.location.href = url + '#destination';
  else if(api === 'location-cross-document') child.location.href = '/intercepted';
  else if(api === 'location-replace') child.location.replace('/intercepted');
  else if(api === 'location-reload') child.location.reload();
  else if(api === 'reload') result = nav.reload();
  else result = nav.navigate(api === 'navigate-cross-document' ? '/intercepted' : url + '#destination');
  if(result) {
    result.committed.catch(() => {});
    result.finished.catch(() => {});
  }
  await done.promise;
  const transitionResults = await observedTransition;
  const apiResults = result ? await Promise.allSettled([result.committed, result.finished]) : [];
  const rejected = [...transitionResults, ...apiResults].filter(value => value.status === 'rejected');
  const record = {
    log,
    entryChanges,
    defaultPrevented: event.defaultPrevented,
    aborted: event.signal.aborted,
    transition: transitionResults.map(value => value.status === 'fulfilled' ? 'fulfilled' : value.reason.name),
    promises: apiResults.map(value => value.status === 'fulfilled' ? 'fulfilled' : value.reason.name),
    committedEntry: !result || apiResults[0].value === committedEntry,
    sameReason: rejected.every(value => value.reason === error && value.reason === event.signal.reason),
    errorKind: error === undefined || (cause === 'throw' ? error === failure : error instanceof Exception),
    transitionCleared: nav.transition === null
  };
  frame.remove();
  return record;
};

globalThis.navigationReentrantInterceptionProbe = async function(throwsAfterReplacement) {
  const frame = document.createElement('iframe');
  frame.src = new URL('/child', location.href).href;
  const loaded = new Promise(resolve => frame.onload = resolve);
  (document.body || document.documentElement || document).appendChild(frame);
  await loaded;
  const child = frame.contentWindow;
  const nav = child.navigation;
  const url = child.location.href;
  const gate = Promise.withResolvers();
  const log = [];
  let firstEvent, firstEntry, firstTransitionResults, replacement, replacementTransition;
  let successes = 0;
  const errors = [];
  nav.addEventListener('navigatesuccess', () => successes++);
  nav.addEventListener('navigateerror', e => errors.push(e.error));
  nav.addEventListener('navigate', e => {
    if(firstEvent) {
      e.intercept({handler() {
        log.push('replacement-handler');
        replacementTransition = nav.transition;
        return gate.promise;
      }});
      return;
    }
    firstEvent = e;
    e.signal.addEventListener('abort', () => log.push('abort'));
    e.intercept({handler() {
      firstEntry = nav.currentEntry;
      const transition = nav.transition;
      firstTransitionResults = Promise.allSettled([transition.committed, transition.finished]);
      log.push('first');
      replacement = nav.navigate(url + '#replacement');
      replacement.committed.catch(() => {});
      replacement.finished.catch(() => {});
      if(throwsAfterReplacement) throw new Error('old handler rejection');
      log.push('first-after');
    }});
    e.intercept({handler() {log.push('second');}});
  });
  const first = nav.navigate(url + '#first');
  const results = await Promise.allSettled([first.committed, first.finished]);
  const transitions = await firstTransitionResults;
  const before = {
    activeTransition: nav.transition === replacementTransition && replacementTransition !== null,
    promises: results.map(value => value.status === 'fulfilled' ? 'fulfilled' : value.reason.name),
    transition: transitions.map(value => value.status === 'fulfilled' ? 'fulfilled' : value.reason.name),
    committedEntry: results[0].value === firstEntry,
    sameReason: results[1].reason === firstEvent.signal.reason && transitions[1].reason === firstEvent.signal.reason,
    errors: errors.length,
    successes
  };
  gate.resolve();
  const entry = await replacement.finished;
  const record = {
    before, log,
    replacementEntry: entry === nav.currentEntry && entry.url === url + '#replacement',
    transitionCleared: nav.transition === null,
    errors: errors.length,
    successes
  };
  frame.remove();
  return record;
};
