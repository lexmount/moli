globalThis.navigationDocumentOpenProbe = async function(kind) {
  const frame = kind === 'iframe' ? document.createElement('iframe') : null;
  if (frame) (document.body || document.documentElement || document).appendChild(frame);
  const child = frame ? frame.contentWindow : window.open('about:blank');
  try {
    const doc = child.document;
    const nav = child.navigation;
    const events = [];
    for (const type of ['navigate', 'currententrychange', 'navigatesuccess', 'navigateerror']) {
      nav.addEventListener(type, () => events.push(type));
    }
    const snapshots = [];
    const snapshot = label => {
      let update;
      try {
        nav.updateCurrentEntry({state: {value: 3}});
        update = 'ok';
      } catch (error) { update = error.name; }
      snapshots.push({label, entries: nav.entries().length, current: nav.currentEntry,
        update, events: events.slice(), sameNavigation: child.navigation === nav});
    };
    snapshot('initial');
    doc.open();
    doc.write('<p>first stream</p>');
    doc.close();
    snapshot('opened');
    doc.open();
    doc.write('<p>second stream</p>');
    doc.close();
    snapshot('reopened');
    child.history.pushState({}, '', '#push');
    snapshot('pushed');
    child.location.hash = 'fragment';
    await new Promise(resolve => setTimeout(resolve, 20));
    snapshot('fragment');

    // Only a navigation initializes the entry list. Opening the resulting
    // Document must then retain the initialized Navigation object and entries.
    child.location.href = 'about:blank';
    for (let i = 0; i < 200 && (child.document === doc || child.document.readyState !== 'complete'); i++) {
      await new Promise(resolve => setTimeout(resolve, 5));
    }
    if (child.document === doc) throw new Error('navigation did not replace the Document');
    const loadedNav = child.navigation;
    loadedNav.updateCurrentEntry({state: {value: 4}});
    const navigated = {
      current: loadedNav.currentEntry.url,
      state: loadedNav.currentEntry.getState().value
    };
    const key = loadedNav.currentEntry.key;
    const id = loadedNav.currentEntry.id;
    child.document.open();
    child.document.write('<p>loaded stream</p>');
    child.document.close();
    loadedNav.updateCurrentEntry({state: {value: 5}});
    const loadedOpen = {
      sameNavigation: child.navigation === loadedNav,
      sameEntryKey: loadedNav.currentEntry.key === key,
      newPublicId: loadedNav.currentEntry.id !== id && /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/.test(loadedNav.currentEntry.id),
      state: loadedNav.currentEntry.getState().value
    };
    return {snapshots, navigated, loadedOpen};
  } finally {
    if (frame) frame.remove();
    else child.close();
  }
};
