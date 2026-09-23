async function navigationReentrantTraversal(base, target, scenario) {
  let frame, owner = window;
  if (target === 'child') {
    frame = document.createElement('iframe');
    frame.src = base + '/common/blank.html';
    await new Promise(resolve => { frame.onload = resolve; document.body.appendChild(frame); });
    owner = frame.contentWindow;
  } else if (target === 'popup') {
    owner = window.open(base + '/common/blank.html');
    await new Promise(resolve => owner.onload = resolve);
  }
  const calleeRealm = owner.navigation.back.constructor('return globalThis')();
  const entries = [owner.navigation.currentEntry];
  for (let i = 1; i <= 3; ++i) {
    // The intercepted step changes state without changing the fragment.
    const fragment = scenario === 'handler' && i === 3 ? 2 : i;
    owner.history.pushState(i, '', '#' + fragment);
    entries.push(owner.navigation.currentEntry);
  }
  await new Promise(resolve => setTimeout(resolve, 0));
  const index = () => owner.navigation.currentEntry.index - entries[0].index;
  const failures = [], events = [], outcomes = [], settlements = [];
  let checks = 0, first = true, originalSignal, hashWaiter;
  const check = (value, label) => { ++checks; if (!value) failures.push(label); };
  const equal = (actual, expected, label) =>
    check(JSON.stringify(actual) === JSON.stringify(expected), label + ': ' + JSON.stringify(actual));
  let requestsReady;
  const requested = new Promise(resolve => requestsReady = resolve);
  const track = (result, name, expectedIndex, rejected) => {
    for (const field of ['committed', 'finished']) {
      check(result[field] instanceof calleeRealm.Promise, name + ' ' + field + ' promise realm');
      settlements.push(result[field].then(entry => {
        const value = entry.index - entries[0].index;
        outcomes.push([name, field, 'resolved', value]);
        check(!rejected && value === expectedIndex, name + ' ' + field + ' resolves its own entry');
      }, error => {
        outcomes.push([name, field, 'rejected', error.name]);
        check(rejected && error.name === 'InvalidStateError' && error instanceof calleeRealm.DOMException,
          name + ' ' + field + ' rejects in the callee realm');
      }));
    }
  };
  const duplicate = (firstResult, secondResult) => {
    check(firstResult !== secondResult, 'fresh result dictionary');
    check(firstResult.committed === secondResult.committed, 'shared committed promise');
    check(firstResult.finished === secondResult.finished, 'shared finished promise');
  };
  const request = () => {
    if (scenario === 'same' || scenario === 'same-then-new') {
      const result = owner.navigation.back({info: 'same'});
      duplicate(result, owner.navigation.back({info: 'ignored'}));
      track(result, 'same', 2, true);
    }
    if (scenario !== 'same') {
      const result = owner.navigation.traverseTo(entries[0].key, {info: 'first'});
      track(result, 'first', 0, false);
      if (scenario === 'multiple') {
        track(owner.navigation.traverseTo(entries[1].key, {info: 'second'}), 'second', 1, false);
      }
      duplicate(result, owner.navigation.traverseTo(entries[0].key, {info: 'ignored'}));
    }
    requestsReady();
  };
  owner.navigation.onnavigate = event => {
    events.push(['navigate', index(), event.destination.index - entries[0].index, event.info ?? null]);
    if (!first) return;
    first = false;
    originalSignal = event.signal;
    if (scenario === 'handler') event.intercept({handler: request});
    else request();
  };
  owner.navigation.oncurrententrychange = () => events.push(['currententrychange', index()]);
  owner.navigation.onnavigateerror = event => events.push(['navigateerror', index(), event.error.name]);
  owner.onpopstate = () => events.push(['popstate', index()]);
  owner.onhashchange = event => {
    events.push(['hashchange', index(), event.newURL]);
    if (hashWaiter && hashWaiter.url === event.newURL) hashWaiter.resolve();
  };
  try {
    owner.history.back();
    await requested;
    await Promise.all(settlements);
    const finalUrl = owner.navigation.currentEntry.url;
    if (!events.some(event => event[0] === 'hashchange' && event[2] === finalUrl)) {
      await new Promise(resolve => hashWaiter = {url: finalUrl, resolve});
    }
    const expected = scenario === 'same' ? [2] : scenario === 'multiple' ? [2, 0, 1] : [2, 0];
    equal(events.filter(event => event[0] === 'hashchange').map(event => event[1]),
      scenario === 'handler' ? [0] : expected,
      'each hashchange observes its committed entry before the next traversal');
    equal(events.filter(event => event[0] === 'popstate').map(event => event[1]), expected,
      'popstate order');
    equal(events.filter(event => event[0] === 'currententrychange').map(event => event[1]), expected,
      'currententrychange order');
    equal(events.filter(event => event[0] === 'navigate').map(event => event[2]), expected,
      'each distinct queued target dispatches once');
    const infos = expected.map(value => value === 2 ? null : value === 0 ? 'first' : 'second');
    equal(events.filter(event => event[0] === 'navigate').map(event => event[3]), infos,
      'repeated requests preserve the first info');
    check(index() === expected[expected.length - 1], 'final entry');
    if (scenario !== 'handler') {
      check(!originalSignal.aborted, 'later requests do not abort the completed traversal');
      check(!events.some(event => event[0] === 'navigateerror'), 'redundant request rejection emits no navigateerror');
    }
    return {checks, failures, events, outcomes};
  } finally {
    owner.navigation.onnavigate = null;
    owner.navigation.oncurrententrychange = null;
    owner.navigation.onnavigateerror = null;
    owner.onpopstate = null;
    owner.onhashchange = null;
    if (frame) frame.remove();
    else if (target === 'popup') owner.close();
  }
}
