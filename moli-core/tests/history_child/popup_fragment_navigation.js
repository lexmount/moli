function navigatePopupFragment() {
  const finish = opener.finish;
  if (MODE === 'repeat') {
    const loads = opener.popupLoads = (opener.popupLoads || 0) + 1;
    if (loads === 1) {
      opener.beforePopupLength = history.length;
      opener.beforePopupDocument = document;
      location.assign(location.href);
    } else {
      finish({loads, delta: history.length - opener.beforePopupLength,
        documentChanged: document !== opener.beforePopupDocument,
        entryIndex: navigation.currentEntry.index});
    }
    return;
  }
  const beforeLength = history.length;
  const beforeDocument = document;
  history.replaceState({classic: 1}, '');
  navigation.updateCurrentEntry({state: {navigation: 1}});
  const beforeEntry = navigation.currentEntry;
  const events = [];
  navigation.addEventListener('navigate', event => {
    events.push(`navigate:${event.navigationType}:${event.destination.sameDocument}`);
    if (MODE === 'cancel') event.preventDefault();
    if (MODE === 'close') close();
  });
  navigation.addEventListener('currententrychange', event => {
    events.push(`currententrychange:${event.navigationType}`);
    if (event.from !== beforeEntry || document.URL !== location.href) {
      events.push('invalid-currententrychange');
    }
  });
  addEventListener('popstate', event => {
    events.push(`popstate:${event.state}:${event.target === window}`);
  });
  addEventListener('hashchange', event => {
    events.push(`hashchange:${new URL(event.oldURL).hash}:${new URL(event.newURL).hash}:${event.target === window}`);
  });
  if (API === 'replace') location.replace('#fragment');
  else if (API === 'assign') location.assign('#fragment');
  else if (API === 'open') open('#fragment', '_self');
  else location.hash = 'fragment';
  if (MODE === 'close') {
    finish({closed: closed, events});
    return;
  }
  const entry = navigation.currentEntry;
  const result = {
    delta: history.length - beforeLength,
    sameDocument: document === beforeDocument,
    documentURL: document.URL === location.href,
    hash: location.hash,
    state: history.state,
    navigationState: entry.getState(),
    index: entry.index,
    currentURL: entry.url === location.href,
    sameKey: entry.key === beforeEntry.key,
    sameId: entry.id === beforeEntry.id,
    entries: navigation.entries().map(item => ({index: item.index, sameDocument: item.sameDocument})),
    synchronousEvents: events.slice()
  };
  setTimeout(() => finish({...result, events}), 0);
}

if (PHASE === 'parser') navigatePopupFragment();
else if (PHASE === 'load') addEventListener('load', navigatePopupFragment, {once: true});
else addEventListener('load', () => setTimeout(() => {
  if (PHASE === 'reopen') document.open();
  navigatePopupFragment();
}, 0), {once: true});
