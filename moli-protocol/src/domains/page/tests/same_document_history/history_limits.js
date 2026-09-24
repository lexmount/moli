function exhaustHistoryUpdates(target) {
  let admitted = 0;
  for (; admitted < 500; admitted++) {
    const before = target.location.href;
    const method = admitted % 2 ? 'pushState' : 'replaceState';
    // A borrowed method must charge the receiver's browsing context.
    History.prototype[method].call(target.history, { admitted }, '', `#budget-${admitted}`);
    if (target.location.href === before) break;
  }
  return admitted;
}

async function historyLimitFrame() {
  const frame = document.createElement('iframe');
  frame.src = '/history.html?child';
  const loaded = new Promise(resolve => frame.onload = resolve);
  document.body.append(frame);
  await loaded;
  return frame;
}

async function probeHistoryUpdateAdmission() {
  const frame = await historyLimitFrame();
  const admitted = exhaustHistoryUpdates(frame.contentWindow);
  frame.remove();
  let release;
  let aborted = false;
  navigation.onnavigate = event => {
    event.signal.addEventListener('abort', () => aborted = true);
    event.intercept({ handler: () => new Promise(resolve => release = resolve) });
  };
  const pending = navigation.navigate('#pending');
  await pending.committed;
  // Out-of-range traversal requests share the History budget, while leaving
  // the existing navigation alone. This makes the cancellation check reachable
  // even on browsers that also throttle navigation.navigate().
  for (let index = 0; index < 500; index++) history.go(1000000);
  let events = 0;
  const record = () => events++;
  navigation.addEventListener('navigate', record);
  navigation.addEventListener('currententrychange', record);
  const snapshot = () => JSON.stringify([
    location.href, history.state, history.length, navigation.currentEntry.id,
    navigation.entries().map(entry => entry.id),
  ]);
  const before = snapshot();
  const conversion = [];
  history.replaceState(
    { get value() { conversion.push('state'); return 1; } },
    { toString() { conversion.push('title'); return ''; } },
    { toString() { conversion.push('url'); return '#blocked'; } },
  );
  const marker = {};
  const errors = [];
  for (const operation of [
    () => history.replaceState(null, { toString() { throw marker; } }, ''),
    () => history.pushState(() => {}, '', ''),
    () => history.replaceState(null, '', 'https://other.invalid/'),
    () => history.go({ valueOf() { throw marker; } }),
  ]) {
    try { operation(); errors.push('none'); }
    catch (error) { errors.push(error === marker ? 'marker' : error.name); }
  }
  history.back();
  history.forward();
  history.go();
  let unchanged = snapshot() === before;
  release();
  await pending.finished;
  await new Promise(resolve => setTimeout(resolve, 0));
  unchanged &&= snapshot() === before;
  return { admitted, conversion, errors, unchanged, events, aborted, finished: true };
}

async function probeHistoryUpdateOwners() {
  const frame = await historyLimitFrame();
  const sibling = await historyLimitFrame();
  const childLimited = exhaustHistoryUpdates(frame.contentWindow) < 500;
  history.replaceState(null, '', '#parent-ok');
  frame.contentWindow.History.prototype.replaceState.call(sibling.contentWindow.history, null, '', '#sibling-ok');
  const popup = window.open('', '_blank');
  if (!popup) throw new Error('Popup was not created');
  History.prototype.replaceState.call(popup.history, null, '', 'about:blank#popup-ok');
  const popupAllowed = popup.location.hash === '#popup-ok';
  popup.close();
  const parentAllowed = location.hash === '#parent-ok';
  const siblingAllowed = sibling.contentWindow.location.hash === '#sibling-ok';
  const navigated = new Promise(resolve => frame.onload = resolve);
  frame.src = '/history.html?next-document';
  await navigated;
  const before = frame.contentWindow.location.href;
  frame.contentWindow.history.replaceState(null, '', '#still-blocked');
  const retainedAcrossNavigation = frame.contentWindow.location.href === before;
  frame.remove();
  const reattached = new Promise(resolve => frame.onload = resolve);
  document.body.append(frame);
  await reattached;
  frame.contentWindow.history.replaceState(null, '', '#fresh');
  const freshAfterRemoval = frame.contentWindow.location.hash === '#fresh';
  frame.remove();
  sibling.remove();
  return { childLimited, parentAllowed, siblingAllowed, popupAllowed, retainedAcrossNavigation, freshAfterRemoval };
}

async function probeHistoryUpdateRecursion() {
  const results = [];
  for (const mode of ['navigate-pushState', 'navigate-replaceState', 'currententrychange-pushState', 'currententrychange-replaceState', 'cross-window']) {
    const frame = await historyLimitFrame();
    const peer = mode === 'cross-window' ? await historyLimitFrame() : null;
    const child = frame.contentWindow;
    await child.navigation.navigate('#initial').finished;
    const eventType = mode.startsWith('currententrychange') ? 'currententrychange' : 'navigate';
    const method = mode.endsWith('pushState') ? 'pushState' : 'replaceState';
    let count = 0;
    const errors = [];
    const recurse = target => {
      // Also bound the probe on engines that do not implement this policy.
      if (++count > 300) return;
      try { target.history[method](null, '', `#recursive-${count}`); }
      catch (error) { errors.push(error.name); }
    };
    const listener = () => recurse(peer ? peer.contentWindow : child);
    const peerListener = () => recurse(child);
    child.navigation.addEventListener(eventType, listener);
    if (peer) peer.contentWindow.navigation.addEventListener(eventType, peerListener);
    let outcomes;
    if (peer) {
      const pending = child.navigation.back();
      outcomes = await Promise.all([pending.committed, pending.finished].map(promise =>
        promise.then(() => 'resolved', error => error.name)));
    } else {
      child.history[method](null, '', '#start');
    }
    child.navigation.removeEventListener(eventType, listener);
    if (peer) peer.contentWindow.navigation.removeEventListener(eventType, peerListener);
    child.history.replaceState(null, '', '#recovered');
    let recovered = child.location.hash === '#recovered';
    if (peer) {
      peer.contentWindow.history.replaceState(null, '', '#recovered');
      recovered &&= peer.contentWindow.location.hash === '#recovered';
    }
    results.push({ mode, count, errors, recovered, outcomes });
    frame.remove();
    if (peer) peer.remove();
  }
  return results;
}
