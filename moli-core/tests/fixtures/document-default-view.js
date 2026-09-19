(async () => {
  const result = {checks: 0, failures: [], observations: []};
  const check = (name, action, expected = true) => {
    result.checks++;
    try {
      const actual = action();
      if (actual !== expected) result.failures.push({name, actual, expected});
    } catch (error) { result.failures.push({name, error: error.name, message: error.message}); }
  };
  const tick = () => new Promise(resolve => setTimeout(resolve, 0));
  const url = name => '/compat/child-dynamic-markup-document?markup=' + name;
  const frame = async (owner, name) => {
    const node = owner.createElement('iframe');
    const loaded = new Promise(resolve => node.onload = resolve);
    node.src = url(name);
    owner.body.append(node);
    await loaded;
    return node;
  };
  const getter = Object.getOwnPropertyDescriptor(Document.prototype, 'defaultView').get;
  const popup = window.open('about:blank', 'default-view-probe');
  if (!popup) {result.failures.push({name: 'popup-open', message: 'popup was blocked'}); return result;}
  const popupDoc = popup.document;
  check('popup-live', () => popupDoc.defaultView === popup);
  check('root', () => document.defaultView === window);
  for (const [label, doc] of [
    ['constructor', new Document()],
    ['html-document', document.implementation.createHTMLDocument('')],
    ['xml-document', document.implementation.createDocument(null, 'root')],
    ['parser', new DOMParser().parseFromString('<p>text</p>', 'text/html')],
    ['clone', document.cloneNode(false)],
  ]) check(label, () => doc.defaultView === null);

  const blank = document.createElement('iframe');
  document.body.append(blank);
  const blankDoc = blank.contentDocument, blankForms = blankDoc.forms;
  const blankWin = blank.contentWindow;
  check('initial-blank-view', () => blankDoc.defaultView === blankWin);
  check('initial-blank-collection-realm', () => Object.getPrototypeOf(blankForms) === blankWin.HTMLCollection.prototype);
  blank.remove();
  check('initial-blank-removed-null', () => blankDoc.defaultView === null);

  for (const mode of ['remove', 'parent-navigation', 'parent-open', 'self-navigation', 'reinsert']) {
    const parentFrame = await frame(document, 'parent');
    const child = await frame(parentFrame.contentDocument, 'child');
    const doc = child.contentDocument, win = child.contentWindow;
    const childGetter = Object.getOwnPropertyDescriptor(win.Document.prototype, 'defaultView').get;
    const collectionPrototype = win.HTMLCollection.prototype;
    const elementPrototype = win.HTMLDivElement.prototype;
    check(mode + ':live', () => doc.defaultView === win);
    check(mode + ':borrowed-live', () => getter.call(doc) === win);
    check(mode + ':child-getter-root', () => childGetter.call(document) === window);
    if (mode === 'remove' || mode === 'reinsert') {
      child.remove();
      check(mode + ':synchronous-null', () => doc.defaultView === null);
      if (mode === 'reinsert') {
        const loaded = new Promise(resolve => child.onload = resolve);
        child.src = url('replacement'); parentFrame.contentDocument.body.append(child);
        await loaded;
      }
    } else if (mode === 'parent-open') {
      parentFrame.contentDocument.open(); parentFrame.contentDocument.close();
    } else {
      const target = mode === 'self-navigation' ? child : parentFrame;
      const loaded = new Promise(resolve => target.onload = resolve);
      target.src = url('replacement'); await loaded;
    }
    await tick();
    check(mode + ':discarded-null', () => doc.defaultView === null);
    check(mode + ':borrowed-null', () => getter.call(doc) === null);
    check(mode + ':old-realm-getter-null', () => childGetter.call(doc) === null);
    check(mode + ':old-realm-getter-root', () => childGetter.call(document) === window);
    check(mode + ':collection-realm', () => Object.getPrototypeOf(doc.forms) === collectionPrototype);
    check(mode + ':element-realm', () => Object.getPrototypeOf(doc.createElement('div')) === elementPrototype);
    check(mode + ':open-identity', () => doc.open() === doc);
    check(mode + ':open-remains-null', () => doc.defaultView === null);
    doc.close();
    if (mode === 'self-navigation' || mode === 'reinsert') {
      check(mode + ':new-document', () => child.contentDocument !== doc);
      check(mode + ':new-defaultView', () => child.contentDocument.defaultView === child.contentWindow);
    }
    parentFrame.remove();
  }
  const live = await frame(document, 'stream');
  const liveDoc = live.contentDocument, liveWin = live.contentWindow;
  liveDoc.open(); liveDoc.write('<p>replacement</p>'); liveDoc.close();
  check('document-open-keeps-view', () => liveDoc.defaultView === liveWin);
  live.remove(); await tick();
  check('document-open-removed-null', () => liveDoc.defaultView === null);

  popup.close();
  // Closing a top-level browsing context is asynchronous, even after closed becomes true.
  for (let i = 0; i < 100 && (!popup.closed || popupDoc.defaultView !== null); i++) {
    await new Promise(resolve => setTimeout(resolve, 10));
  }
  check('popup-closed', () => popup.closed);
  check('popup-discarded-null', () => popupDoc.defaultView === null);
  return result;
})()
