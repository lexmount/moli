function replaceDocumentFromEntry(target, method, options = {}) {
  const getURL = Object.getOwnPropertyDescriptor(Document.prototype, 'URL').get;
  const doc = target.document;
  const beforeURL = getURL.call(doc);
  const state = target.history.state;
  const length = target.history.length;
  const sourceURLAtOpen = location.href;
  let sourceGetterCalls = 0;
  const descriptor = Object.getOwnPropertyDescriptor(Document.prototype, 'URL');
  if (options.poison) {
    Object.defineProperty(Document.prototype, 'URL', {
      configurable: true,
      get() { sourceGetterCalls++; throw Error('author URL getter'); }
    });
  }
  const controller = opener || parent;
  try {
    let result = doc;
    const markup = '<!doctype html><body>replacement-content';
    if (method === 'borrowed') result = controller.Document.prototype.open.call(doc);
    else if (method === 'open') result = doc.open();
    if (method === 'write' || method === 'writeln') {
      doc[method](markup);
    } else {
      doc.write(markup);
    }
    doc.close();
    const expected = new URL(sourceURLAtOpen);
    if (target !== globalThis) expected.hash = '';
    return {
      beforeURL, expectedURL: expected.href,
      url: getURL.call(doc), location: target.location.href, baseURI: doc.baseURI,
      sameDocument: target.document === doc && result === doc,
      stateRetained: target.history.state === state,
      lengthRetained: target.history.length === length,
      written: doc.body?.textContent === 'replacement-content' + (method === 'writeln' ? '\n' : ''),
      sourceGetterCalls
    };
  } finally {
    if (options.poison) Object.defineProperty(Document.prototype, 'URL', descriptor);
  }
}

function installDocumentOpenEntrySource() {
  addEventListener('message', event => {
    if (event.data?.tag !== 'document-open-entry-run') return;
    const command = event.data;
    const controller = opener || parent;
    const execute = () => {
      try {
        const target = command.self ? globalThis : controller.__documentOpenEntryTarget;
        const facts = replaceDocumentFromEntry(target, command.method, command.options);
        controller.postMessage({tag: 'document-open-entry-result', facts}, '*');
      } catch (error) {
        controller.postMessage({tag: 'document-open-entry-result', error: error.name, message: error.message}, '*');
      }
    };
    if (command.microtask) queueMicrotask(execute);
    else execute();
  });
}

async function documentOpenEntry(base, sourceKind, method, options = {}) {
  const pause = () => new Promise(resolve => setTimeout(resolve, 5));
  const ready = async (window, url) => {
    for (let i = 0; i < 400; i++) {
      if (window.location.href === url && window.document.readyState === 'complete') return;
      await pause();
    }
    throw Error('source or target failed to finish loading: ' + JSON.stringify({
      expected: url, actual: window.location.href, documentURL: window.document.URL,
      readyState: window.document.readyState, closed: window.closed
    }));
  };
  const fragment = async (window, value) => {
    const changed = new Promise(resolve => window.addEventListener('hashchange', resolve, {once: true}));
    window.location.hash = value;
    await changed;
    await pause();
  };
  const self = sourceKind === 'self';
  const targetURL = base + (self ? '/entry-source' : '/entry-target');
  const target = open(targetURL, 'document-open-entry-target');
  let source;
  let frame;
  try {
    await ready(target, targetURL);
    await fragment(target, 'target-fragment');
    await ready(target, targetURL + '#target-fragment');
    globalThis.__documentOpenEntryTarget = target;
    let facts;
    if (sourceKind === 'root') {
      history.replaceState(null, '', location.pathname + location.search + '#entry-fragment');
      await pause();
      facts = replaceDocumentFromEntry(target, method, options);
    } else {
      if (self) source = target;
      else if (sourceKind === 'iframe') {
        frame = document.createElement('iframe');
        frame.src = base + '/entry-source';
        document.body.appendChild(frame);
        source = frame.contentWindow;
      } else {
        source = open(base + '/entry-source');
      }
      if (!self) {
        await ready(source, base + '/entry-source');
        await fragment(source, 'source-fragment');
        await ready(source, base + '/entry-source#source-fragment');
      }
      const result = new Promise(resolve => {
        function receive(event) {
          if (event.source === source && event.data?.tag === 'document-open-entry-result') {
            removeEventListener('message', receive);
            resolve(event.data);
          }
        }
        addEventListener('message', receive);
      });
      source.postMessage({tag: 'document-open-entry-run', self, method,
        microtask: sourceKind === 'popup-microtask', options}, '*');
      const response = await result;
      if (response.error) throw Error(response.error + ': ' + response.message);
      facts = response.facts;
    }
    facts.readyStateAfterClose = target.document.readyState;
    const checks = {
      sameDocument: facts.sameDocument,
      urlUsesEntryDocument: facts.url === facts.expectedURL,
      locationUsesEntryDocument: facts.location === facts.expectedURL,
      baseUsesEntryDocument: facts.baseURI === facts.expectedURL,
      stateRetained: facts.stateRetained,
      lengthRetained: facts.lengthRetained,
      written: facts.written,
      ignoresAuthorURLGetter: facts.sourceGetterCalls === 0
    };
    return {sourceKind, method, options, facts, checks,
      passed: Object.values(checks).filter(Boolean).length,
      total: Object.keys(checks).length};
  } finally {
    if (frame) frame.remove();
    else if (source && source !== target) source.close();
    target.close();
    delete globalThis.__documentOpenEntryTarget;
  }
}
