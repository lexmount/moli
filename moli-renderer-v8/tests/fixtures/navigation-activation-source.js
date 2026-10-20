async function navigationActivationSource(base, mode) {
  const foreignURL = new URL(base);
  foreignURL.hostname = foreignURL.hostname === 'localhost' ? '127.0.0.1' : 'localhost';
  const foreign = foreignURL.origin;
  const frame = document.createElement('iframe');
  document.body.appendChild(frame);
  const initialDocument = frame.contentDocument;
  const settle = () => new Promise(resolve => setTimeout(resolve, 0));
  await settle();
  async function load(action) {
    const loaded = new Promise(resolve => frame.addEventListener('load', resolve, {once: true}));
    action();
    await loaded;
    await settle();
  }
  if (mode.startsWith('noninitial-blank')) {
    await load(() => frame.contentWindow.location.replace('about:blank'));
  } else if (mode.startsWith('srcdoc')) {
    await load(() => { frame.srcdoc = '<!doctype html><body>source</body>'; });
  } else if (mode.startsWith('http')) {
    await load(() => { frame.src = base + '/common/blank.html?source'; });
  } else if (mode.startsWith('opened')) {
    const doc = frame.contentDocument;
    doc.open();
    doc.write('<!doctype html><body>opened</body>');
    doc.close();
    await settle();
  } else if (mode === 'opaque-source') {
    frame.sandbox = 'allow-scripts';
    await load(() => { frame.src = base + '/common/blank.html?opaque'; });
    frame.removeAttribute('sandbox');
  } else if (mode === 'cross-origin-source' || mode === 'cross-origin-to-blank') {
    await load(() => { frame.src = foreign + '/common/blank.html?foreign'; });
  } else if (mode === 'traverse-middle-origin') {
    await load(() => { frame.src = base + '/common/blank.html?first'; });
    await load(() => { frame.src = foreign + '/common/blank.html?middle'; });
    await load(() => { frame.src = base + '/common/blank.html?last'; });
  }
  if (mode.includes('-fragment-')) {
    const child = frame.contentWindow;
    const changed = new Promise(resolve => child.addEventListener('hashchange', resolve, {once: true}));
    child.location.hash = 'source-fragment';
    await changed;
    await settle();
  }
  const child = frame.contentWindow;
  const crossOrigin = mode === 'cross-origin-source' || mode === 'cross-origin-to-blank' || mode === 'opaque-source';
  const sourceDocument = crossOrigin ? null : child.document;
  const sourceURL = crossOrigin ? frame.src : child.location.href;
  const sourceEntry = crossOrigin ? null : child.navigation.currentEntry;
  const before = {
    sourceURL, currentURL: sourceEntry?.url ?? null,
    currentId: sourceEntry?.id ?? null,
    sameInitialDocument: sourceDocument === initialDocument,
    activationNull: crossOrigin ? null : child.navigation.activation === null,
    historyLength: crossOrigin ? null : child.history.length
  };
  const destination = mode.endsWith('-to-blank') ? 'about:blank?destination' : base + '/common/blank.html?destination';
  await load(() => {
    if (crossOrigin || mode.endsWith('-src')) frame.src = destination;
    else if (mode === 'traverse-middle-origin') child.history.go(-2);
    else if (mode.endsWith('-navigate')) child.navigation.navigate(destination).finished.catch(() => {});
    else if (mode.endsWith('-replace')) child.location.replace(destination);
    else if (mode.endsWith('-form')) {
      const form = child.document.createElement('form');
      form.action = destination;
      form.method = 'GET';
      (child.document.body || child.document.documentElement).appendChild(form);
      form.submit();
    } else if (mode.endsWith('-reload')) child.location.reload();
    else child.location.assign(destination);
  });
  const target = frame.contentWindow;
  const activation = target.navigation.activation;
  const after = {
    url: target.location.href,
    documentChanged: target.document !== sourceDocument,
    activationNull: activation === null,
    fromNull: activation?.from === null,
    fromURL: activation?.from?.url ?? null,
    fromId: activation?.from?.id ?? null,
    fromIndex: activation?.from?.index ?? null,
    entryURL: activation?.entry?.url ?? null,
    entryCurrent: activation?.entry === target.navigation.currentEntry,
    type: activation?.navigationType ?? null,
    historyLength: target.history.length
  };
  const checks = [];
  const check = (name, pass) => checks.push({name, pass: !!pass});
  const noFrom = mode.startsWith('initial-') || crossOrigin || mode === 'traverse-middle-origin';
  check('navigation commits a new Document', after.documentChanged);
  check('committed Document has activation', !after.activationNull);
  check('activation entry is the current entry', after.entryCurrent);
  check('entry URL describes the committed Document', after.entryURL === after.url);
  check('source visibility follows Document lifecycle and origin', after.fromNull === noFrom);
  check('activation has the destination interface brand', activation instanceof target.NavigationActivation);
  check('activation entry has the destination entry brand', activation.entry instanceof target.NavigationHistoryEntry);
  if (noFrom) {
    check('hidden source has no URL', after.fromURL === null);
    check('hidden source has no identity', after.fromId === null);
  } else {
    check('visible source has the destination entry brand', activation.from instanceof target.NavigationHistoryEntry);
    if (sourceEntry) {
      check('source URL is retained', after.fromURL === before.currentURL);
      check('source entry identity is retained', after.fromId === before.currentId);
    }
  }
  check('activation type describes the operation', after.type === (mode.startsWith('initial-') || mode.endsWith('-replace') ? 'replace' : mode.endsWith('-reload') ? 'reload' : mode === 'traverse-middle-origin' ? 'traverse' : 'push'));
  frame.remove();
  return {mode, before, after, checks, passed: checks.filter(row => row.pass).length, total: checks.length};
}
