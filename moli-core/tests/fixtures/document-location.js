(async () => {
  const result = {checks: 0, failures: [], scenarios: [], observations: []};
  const check = (name, actual, expected = true) => {
    result.checks++;
    if (actual !== expected) result.failures.push({name, actual, expected});
  };
  const root = Object.getOwnPropertyDescriptor(document, 'location');
  const constructed = Object.getOwnPropertyDescriptor(new Document(), 'location');
  const url = name => '/compat/child-dynamic-markup-document?markup=' + name;
  const frame = async (owner, name) => {
    const node = owner.createElement('iframe');
    const loaded = new Promise(resolve => node.onload = resolve);
    if (name !== 'blank') node.src = url(name);
    owner.body.append(node);
    await loaded;
    return node;
  };
  function invalidReceivers(name, descriptor, errorConstructor, real) {
    let traps = 0, conversions = 0;
    const proxy = new Proxy(real, {get() { traps++; }, has() { traps++; }});
    const revoked = Proxy.revocable(real, {}); revoked.revoke();
    for (const [label, receiver] of [['plain', {}], ['derived', Object.create(real)], ['proxy', proxy], ['revoked', revoked.proxy]]) {
      for (const operation of ['get', 'set']) {
        let error;
        try { descriptor[operation].call(receiver, {toString() { conversions++; return '#forbidden'; }}); }
        catch (caught) { error = caught; }
        check(name + ':' + label + ':' + operation, error instanceof errorConstructor);
      }
    }
    check(name + ':no-proxy-traps', traps, 0);
    check(name + ':no-conversion', conversions, 0);
  }
  function inactive(name, doc, own, errorConstructor) {
    check(name + ':null', doc.location === null);
    const accessors = [['own', own, errorConstructor], ['constructed', constructed, TypeError]];
    // Chromium's HTMLDocument accessor does not accept an XML Document.
    if (doc.contentType === 'text/html') accessors.push(['root', root, TypeError]);
    for (const [label, descriptor, expectedError] of accessors) {
      check(name + ':' + label + ':descriptor', !!descriptor);
      if (!descriptor) continue;
      check(name + ':' + label + ':getter', descriptor.get.call(doc) === null);
      let conversions = 0, error;
      try { descriptor.set.call(doc, {toString() { conversions++; return '#forbidden'; }}); }
      catch (caught) { error = caught; }
      check(name + ':' + label + ':setter-error', error instanceof expectedError);
      check(name + ':' + label + ':no-conversion', conversions, 0);
    }
  }
  check('root-location', document.location === window.location);
  check('constructed-getter-on-root', constructed.get.call(document) === window.location);
  invalidReceivers('root-brand', root, TypeError, document);
  invalidReceivers('constructed-brand', constructed, TypeError, document);
  for (const [label, doc] of [
    ['constructor', new Document()],
    ['html', document.implementation.createHTMLDocument('')],
    ['parser', new DOMParser().parseFromString('<p>parsed</p>', 'text/html')],
    ['clone', document.cloneNode(false)],
  ]) {
    inactive(label, doc, Object.getOwnPropertyDescriptor(doc, 'location'), TypeError);
    result.scenarios.push(label);
  }
  for (const mode of ['remove', 'ancestor-remove', 'ancestor-navigation', 'ancestor-open', 'navigate', 'reinsert', 'initial-reuse']) {
    const parent = await frame(document, 'parent');
    const child = await frame(parent.contentDocument, mode === 'initial-reuse' ? 'blank' : 'child');
    const doc = child.contentDocument, win = child.contentWindow;
    const own = Object.getOwnPropertyDescriptor(doc, 'location'), errorConstructor = win.TypeError;
    if (mode === 'remove') invalidReceivers('child-brand', own, errorConstructor, doc);
    const location = win.location, documentURL = doc.URL, lifecycle = [];
    for (const type of ['beforeunload', 'pagehide', 'unload'])
      win.addEventListener(type, () => lifecycle.push([type, doc.location === location]));
    check(mode + ':active', doc.location === location);
    check(mode + ':borrowed-active', constructed.get.call(doc) === location);
    check(mode + ':child-getter-on-root', own.get.call(document) === window.location);
    if (mode === 'remove' || mode === 'reinsert') {
      child.remove();
      inactive(mode + ':removed', doc, own, errorConstructor);
      if (mode === 'reinsert') {
        const loaded = new Promise(resolve => child.onload = resolve);
        child.src = url('replacement'); parent.contentDocument.body.append(child);
        await loaded;
      }
    } else if (mode === 'ancestor-remove') parent.remove();
    else if (mode === 'ancestor-open') { parent.contentDocument.open(); parent.contentDocument.close(); }
    else {
      const target = mode === 'ancestor-navigation' ? parent : child;
      const loaded = new Promise(resolve => target.onload = resolve);
      target.src = url('replacement'); await loaded;
    }
    inactive(mode, doc, own, errorConstructor);
    check(mode + ':retained-url', doc.URL, documentURL);
    check(mode + ':child-getter-root-after-retirement', own.get.call(document) === window.location);
    doc.open(); doc.write('<p>retained</p>'); doc.close();
    check(mode + ':open-does-not-reactivate', doc.location === null);
    if (['navigate', 'reinsert', 'initial-reuse'].includes(mode)) {
      const current = child.contentDocument;
      check(mode + ':replacement', current !== doc);
      check(mode + ':replacement-location', current.location === child.contentWindow.location);
      const currentLocation = current.location;
      current.open(); current.write('<p>active replacement</p>'); current.close();
      check(mode + ':active-open-keeps-location', current.location === currentLocation);
    }
    const expectedEvents = ['ancestor-navigation', 'navigate', 'initial-reuse'].includes(mode)
      ? ['beforeunload', 'pagehide', 'unload'] : ['pagehide', 'unload'];
    check(mode + ':lifecycle-order', JSON.stringify(lifecycle.map(([type]) => type)), JSON.stringify(expectedEvents));
    check(mode + ':location-during-unload', lifecycle.every(([, active]) => active));
    result.observations.push({mode, lifecycle});
    result.scenarios.push(mode);
    parent.remove();
  }
  const popup = open('about:blank', 'document-location-probe');
  if (!popup) throw new Error('popup was blocked');
  const popupDoc = popup.document, popupLocation = popup.location;
  const popupOwn = Object.getOwnPropertyDescriptor(popupDoc, 'location');
  const popupError = popupOwn.set.constructor('return TypeError')();
  check('popup-active', popupDoc.location === popupLocation);
  popup.close();
  for (let i = 0; i < 100 && (!popup.closed || popupDoc.defaultView !== null); i++)
    await new Promise(resolve => setTimeout(resolve, 10));
  check('popup-closed', popup.closed);
  inactive('popup-closed', popupDoc, popupOwn, popupError);
  result.scenarios.push('popup-closed');
  return result;
})()
