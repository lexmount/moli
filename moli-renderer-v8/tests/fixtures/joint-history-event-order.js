async function jointHistoryEventOrder(base, initiator, intercept) {
  const frames = [], trace = [], failures = [];
  let checks = 0;
  const check = (condition, message) => {
    checks++;
    if (!condition) failures.push(message);
  };
  const tick = () => new Promise(resolve => setTimeout(resolve, 0));
  for (const name of ['a', 'b']) {
    const frame = document.createElement('iframe');
    frame.src = base + '/common/blank.html?' + name;
    await new Promise(resolve => {
      frame.onload = resolve;
      document.body.appendChild(frame);
    });
    await tick();
    frames.push(frame);
  }
  const windows = frames.map(frame => frame.contentWindow);
  const source = initiator === 'top' ? window : windows[initiator === 'first' ? 0 : 1];
  const sourceName = initiator === 'top' ? 'top' : initiator === 'first' ? 'a' : 'b';
  const participants = initiator === 'top' ? [window, ...windows] : windows;
  const names = initiator === 'top' ? ['top', 'a', 'b'] : ['a', 'b'];
  const original = source.navigation.currentEntry;
  const pushes = [source, ...participants.filter(win => win !== source)];
  for (const win of pushes) {
    await win.navigation.navigate('#one').finished;
  }
  await tick();
  const reason = new Error('joint history handler rejected');
  let release;
  const gate = new Promise(resolve => release = resolve);
  let committed = false;
  const records = participants.map((win, index) => {
    const name = names[index], nav = win.navigation;
    const record = {name, handlers: 0, popstates: 0, errors: 0, successes: 0};
    record.finished = new Promise(resolve => record.resolve = resolve);
    const log = what => trace.push(name + ':' + what);
    nav.oncurrententrychange = () => {
      log('currententrychange');
      check(win.location.hash === '', name + ' committed URL before currententrychange');
      if (intercept !== 'none') {
        const transition = nav.transition;
        check(transition instanceof win.NavigationTransition, name + ' transition realm');
        check(transition.committed instanceof win.Promise, name + ' transition committed realm');
        check(transition.finished instanceof win.Promise, name + ' transition finished realm');
        check(transition.from === record.from, name + ' transition from identity');
        check(transition.to === record.destination, name + ' transition destination identity');
      }
      queueMicrotask(() => {
        log('currententrychange microtask');
        if (intercept !== 'none')
          check(record.handlers === 2, name + ' handlers before listener microtasks');
      });
    };
    nav.onnavigate = event => {
      log('navigate');
      if (intercept === 'none') return;
      record.from = nav.currentEntry;
      record.destination = event.destination;
      event.intercept({handler() {
        record.handlers++;
        log('handler 1');
        queueMicrotask(() => {
          log('handler microtask');
          check(record.handlers === 2, name + ' complete handler list before microtasks');
        });
        if (intercept === 'reject') return Promise.reject(reason);
        if (intercept === 'stop') return gate;
      }});
      event.intercept({handler() { record.handlers++; log('handler 2'); }});
    };
    win.onpopstate = event => {
      log('popstate');
      record.popstates++;
      check(event.constructor === win.PopStateEvent, name + ' popstate realm');
      if (win === source) check(committed, name + ' committed reactions before popstate');
      if (intercept !== 'none')
        check(record.handlers === 2, name + ' handlers before popstate');
    };
    nav.onnavigatesuccess = () => {
      record.successes++;
      log('success');
      record.resolve();
    };
    nav.onnavigateerror = event => {
      record.errors++;
      log('error');
      check(intercept === 'reject' ? event.error === reason :
        intercept === 'stop' && win === source && event.error.name === 'AbortError',
        name + ' navigation error identity');
      record.resolve();
    };
    return record;
  });
  const result = source.navigation.traverseTo(original.key);
  const observeCommit = result.committed.then(entry => {
    trace.push('committed');
    committed = true;
    check(entry === source.navigation.currentEntry, 'committed entry identity');
    const record = records.find(record => record.name === sourceName);
    check(record.popstates === 0, 'committed before initiating popstate');
    check(intercept === 'none' || record.handlers === 2, 'initiating handlers before committed');
    if (intercept === 'stop') source.stop();
    release();
  });
  const observeFinish = result.finished.then(() => {
    trace.push('finished');
    check(intercept !== 'reject' && intercept !== 'stop', 'finished fulfillment');
  }, error => {
    trace.push('finished rejected');
    check(intercept === 'reject' ? error === reason :
      intercept === 'stop' && error.name === 'AbortError', 'finished rejection');
  });
  await Promise.all([observeCommit, observeFinish, ...records.map(record => record.finished)]);
  await tick();
  for (const record of records) {
    check(record.popstates === 1, record.name + ' one committed popstate');
    const rejected = intercept === 'reject' || intercept === 'stop' && record.name === sourceName;
    check(record.errors === (rejected ? 1 : 0), record.name + ' error count');
    check(record.successes === (rejected ? 0 : 1), record.name + ' success count');
  }
  for (const win of participants) {
    win.onpopstate = null;
    win.navigation.onnavigate = win.navigation.oncurrententrychange = null;
    win.navigation.onnavigatesuccess = win.navigation.onnavigateerror = null;
  }
  for (const frame of frames) frame.remove();
  return {checks, failures, trace};
}
