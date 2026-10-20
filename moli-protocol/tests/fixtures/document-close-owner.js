function installDocumentCloseSource() {
  addEventListener('message', event => {
    if (event.data?.tag !== 'document-close-owner-command') return;
    const controller = opener || parent;
    const target = event.data.self ? globalThis : controller.__documentCloseOwnerTarget;
    const result = performDocumentClose(target, event.data.operation);
    controller.postMessage({tag: 'document-close-owner-result', result}, '*');
  });
}

function performDocumentClose(target, operation) {
  const doc = target.document;
  if (operation !== 'close') {
    doc.open();
    if (target === globalThis) installDocumentCloseSource();
    doc.write('<!doctype html><body>replacement');
    target.__documentCloseOwnerEvents = [];
    const note = event => target.__documentCloseOwnerEvents.push(event.type + ':' + doc.readyState);
    doc.addEventListener('readystatechange', note);
    doc.addEventListener('DOMContentLoaded', note);
    target.addEventListener('load', note);
  }
  if (operation === 'reopen') {
    doc.close();
    return performDocumentClose(target, 'same-turn');
  }
  if (operation !== 'open') doc.close();
  return {readyState: doc.readyState, sameDocument: doc === target.document};
}

async function documentCloseOwner(base, sourceKind, operation) {
  const pause = () => new Promise(resolve => setTimeout(resolve, 5));
  const ready = async (win, url) => {
    const deadline = performance.now() + 2000;
    while (win.location.href !== url || win.document.readyState !== 'complete') {
      if (performance.now() > deadline) throw Error('initial page did not load: ' + win.location.href);
      await pause();
    }
  };
  const targetURL = base + (sourceKind === 'self' ? '/close-source' : '/close-target');
  const target = open(targetURL, 'document-close-owner-target');
  let source;
  let frame;
  try {
    await ready(target, targetURL);
    const original = target.document;
    globalThis.__documentCloseOwnerTarget = target;
    if (sourceKind === 'iframe') {
      frame = document.createElement('iframe');
      frame.src = base + '/close-source';
      document.body.appendChild(frame);
      source = frame.contentWindow;
    } else if (sourceKind === 'popup') source = open(base + '/close-source');
    else if (sourceKind === 'self') source = target;
    if (source && source !== target) await ready(source, base + '/close-source');
    async function invoke(command) {
      if (!source) return performDocumentClose(target, command);
      const response = new Promise(resolve => {
        function receive(event) {
          if (event.source === source && event.data?.tag === 'document-close-owner-result') {
            removeEventListener('message', receive);
            resolve(event.data.result);
          }
        }
        addEventListener('message', receive);
      });
      source.postMessage({tag: 'document-close-owner-command', operation: command,
        self: source === target}, '*');
      return response;
    }
    let opened;
    if (operation === 'later-close') {
      opened = await invoke('open');
      await pause();
    }
    const closed = await invoke(operation === 'later-close' ? 'close' : operation);
    const deadline = performance.now() + 2000;
    while (target.document.readyState !== 'complete' && performance.now() < deadline) await pause();
    const expected = ['readystatechange:interactive', 'DOMContentLoaded:interactive',
      'readystatechange:complete', 'load:complete'];
    const facts = {opened, closed, state: target.document.readyState,
      sameDocument: target.document === original, text: target.document.body?.textContent,
      events: [...target.__documentCloseOwnerEvents]};
    return {sourceKind, operation, facts, checks: {
      completes: facts.state === 'complete',
      events: JSON.stringify(facts.events) === JSON.stringify(expected),
      documentRetained: facts.sameDocument && closed.sameDocument,
      writtenContent: facts.text === 'replacement',
      openWaitsForClose: !opened || opened.readyState === 'loading'
    }};
  } finally {
    if (frame) frame.remove();
    else if (source && source !== target) source.close();
    target.close();
    delete globalThis.__documentCloseOwnerTarget;
  }
}
