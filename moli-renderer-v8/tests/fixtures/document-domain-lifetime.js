async ({sameURL, crossURL, replacementURL}) => {
  document.domain = location.hostname;
  let checks = 0;
  const failures = [];
  const check = (label, predicate) => {
    checks++;
    try {
      if (!predicate()) failures.push(label);
    } catch (error) {
      failures.push(label + ': ' + error);
    }
  };
  const securityError = callback => {
    try { callback(); }
    catch (error) { return error instanceof DOMException && error.name === 'SecurityError'; }
    return false;
  };
  const create = async url => {
    const frame = document.createElement('iframe');
    const loaded = new Promise(resolve => { frame.onload = resolve; });
    frame.src = url;
    document.body.appendChild(frame);
    await loaded;
    return frame;
  };

  const openedFrame = await create(sameURL);
  const openedWindow = openedFrame.contentWindow;
  const openedDocument = openedWindow.document;
  check('explicit open returns the same Document', () => openedDocument.open() === openedDocument);
  check('explicit open keeps domain access', () => openedWindow.document === openedDocument);
  openedDocument.write('<!doctype html><body><p>explicit');
  openedDocument.close();
  check('closing the stream keeps domain access', () => openedWindow.document === openedDocument);
  check('explicit open replaces content', () => openedDocument.body.textContent === 'explicit');
  openedDocument.write('<!doctype html><body><p>implicit');
  openedDocument.close();
  check('implicit open keeps domain access', () => openedWindow.document === openedDocument);
  check('implicit open replaces content', () => openedDocument.body.textContent === 'implicit');
  openedFrame.remove();

  const frame = await create(crossURL);
  const window = frame.contentWindow;
  const original = window.document;
  const originalRoot = original.documentElement;
  const Exception = window.DOMException;
  const listen = window.addEventListener;
  const snapshot = phase => {
    check(phase + ': contentDocument', () => frame.contentDocument === original);
    check(phase + ': Window.document', () => window.document === original);
    check(phase + ': original realm', () => window.DOMException === Exception);
    check(phase + ': relaxed domain', () => original.domain === document.domain);
    check(phase + ': raw origins still differ', () => window.location.origin !== location.origin);
    check(phase + ': active Document preserves content', () => original.documentElement === originalRoot);
  };
  snapshot('loaded');

  const events = [];
  for (const type of ['beforeunload', 'pagehide', 'unload']) {
    listen.call(window, type, () => {
      events.push(type);
      snapshot(type);
    });
  }
  let loaded = new Promise(resolve => { frame.onload = resolve; });
  frame.src = replacementURL;
  snapshot('pending committed navigation');
  await loaded;
  for (const type of ['beforeunload', 'pagehide', 'unload']) {
    check(type + ' ran', () => events.filter(event => event === type).length === 1);
  }
  check('new network Document has no domain override', () => frame.contentDocument === null);
  check('new network Window denies domain access', () => securityError(() => window.document));

  loaded = new Promise(resolve => { frame.onload = resolve; });
  frame.src = 'about:blank';
  await loaded;
  const blank = frame.contentDocument;
  check('new about:blank inherits domain access', () => blank !== null && window.document === blank);
  check('new about:blank replaces the old Document', () => blank !== original);
  check('new about:blank inherits domain', () => blank.domain === document.domain);

  loaded = new Promise(resolve => { frame.onload = resolve; });
  frame.srcdoc = '<!doctype html><body>srcdoc';
  check('pending srcdoc keeps the current Document', () => window.document === blank);
  await loaded;
  check('new srcdoc inherits domain access', () => frame.contentDocument !== null && window.document === frame.contentDocument);
  check('new srcdoc replaces about:blank', () => frame.contentDocument !== blank);
  check('new srcdoc inherits domain', () => frame.contentDocument.domain === document.domain);
  frame.remove();
  return {checks, failures};
}
