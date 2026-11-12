(async function (classicOwner, apiOwner, mode) {
  const child = document.querySelector('iframe').contentWindow;
  history.replaceState('parent-zero', '', '#zero');
  child.history.replaceState('child-zero', '');
  const parentZero = navigation.currentEntry;
  const childZero = child.navigation.currentEntry;
  history.pushState('parent-one', '', '#one');
  child.history.pushState('child-one', '');
  const classic = classicOwner === 'parent' ? window : child;
  const api = apiOwner === 'parent' ? window : child;
  const target = api === window ? parentZero : childZero;
  const marker = new api.Error('precommit failure');
  const info = {marker: 'API tracker info'};
  const events = [];
  let reentrant, original, distinctPromises, blockedPrecommit;
  const observe = promise => promise.then(
    entry => ({status: 'fulfilled', identity: entry === target,
               realm: entry instanceof api.NavigationHistoryEntry}),
    error => ({status: 'rejected', marker: error === marker, name: error.name})
  );
  for (const [owner, w] of [['parent', window], ['child', child]]) {
    w.navigation.addEventListener('navigate', event => {
      if (event.navigationType !== 'traverse') return;
      events.push({owner, ownsInfo: event.info === info, emptyInfo: event.info == null});
      if (w !== api) return;
      if (!event.cancelable && (mode === 'held' || mode === 'reject')) {
        blockedPrecommit = {name: null, realm: false, called: false};
        try {
          event.intercept({precommitHandler: () => {blockedPrecommit.called = true;}});
        } catch (error) {
          blockedPrecommit.name = error.name;
          blockedPrecommit.realm = error instanceof api.DOMException;
        }
      }
      if (mode === 'held') {
        const wait = () => new Promise(resolve => {
          globalThis.releaseJointTrackerPrecommit = resolve;
        });
        event.intercept(event.cancelable ? {precommitHandler: wait} : {handler: wait});
      } else if (mode === 'reject') {
        const reject = () => {throw marker;};
        event.intercept(event.cancelable ? {precommitHandler: reject} : {handler: reject});
      } else if (mode === 'reentrant' && !reentrant) {
        const again = api.navigation.traverseTo(target.key, {info: 'later call'});
        distinctPromises = again.committed !== original.committed &&
                           again.finished !== original.finished;
        reentrant = Promise.all([observe(again.committed), observe(again.finished)]);
      }
    });
  }
  globalThis.jointTrackerPending = {committed: true, finished: true};
  classic.history.go(-2);
  original = api.navigation.traverseTo(target.key, {info});
  const committed = observe(original.committed).then(value => {
    jointTrackerPending.committed = false;
    return value;
  });
  const finished = observe(original.finished).then(value => {
    jointTrackerPending.finished = false;
    return value;
  });
  const [committedResult, finishedResult] = await Promise.all([committed, finished]);
  const later = reentrant ? await reentrant : null;
  return {committed: committedResult, finished: finishedResult, events, distinctPromises,
          blockedPrecommit,
          reentrant: later && {committed: later[0], finished: later[1]}};
})
