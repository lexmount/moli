async function(mode, method = 'src', depth = 0) {
  const delay = () => new Promise(resolve => setTimeout(resolve, 100));
  const loaded = frame => new Promise(resolve => frame.addEventListener('load', () => setTimeout(resolve, 0), {once: true}));
  const set = (frame, url) => {
    switch (method) {
      case 'src': frame.src = url; break;
      case 'attribute': frame.setAttribute('src', url); break;
      case 'namespace': frame.setAttributeNS(null, 'src', url); break;
      case 'value': frame.getAttributeNode('src').value = url; break;
      case 'node': {
        const attr = frame.ownerDocument.createAttribute('src');
        attr.value = url;
        frame.setAttributeNode(attr);
        break;
      }
    }
  };
  let owner = document;
  const ancestors = [];
  for (let i = 0; i < depth; i++) {
    const frame = owner.createElement('iframe');
    const ready = loaded(frame);
    frame.srcdoc = '<!doctype html><body>intermediate</body>';
    owner.body.append(frame);
    await ready;
    ancestors.push(frame);
    owner = frame.contentDocument;
  }
  const frame = owner.createElement(mode === 'frame' ? 'frame' : 'iframe');
  const target = location.href + (mode === 'srcdoc' ? '?src-fallback' : '#ignored-fragment');
  let initial = loaded(frame);
  frame.src = mode === 'insert' ? target : '/frame-child.html?initial';
  if (mode === 'srcdoc' || mode === 'remove-srcdoc') frame.srcdoc = '<p>srcdoc</p>';
  let loads = 0;
  frame.addEventListener('load', () => loads++);
  owner.body.append(frame);
  if (mode === 'insert') await delay();
  else await initial;
  const original = frame.contentDocument;
  const historyLength = frame.contentWindow.history.length;
  const initialLoads = loads;
  loads = 0;
  const events = [];
  for (const name of ['beforeunload', 'pagehide', 'unload'])
    frame.contentWindow.addEventListener(name, () => events.push(name));
  const snapshot = () => ({
    sameDocument: frame.contentDocument === original,
    url: frame.contentDocument.URL.replace(location.origin, ''),
    readyState: frame.contentDocument.readyState,
    historyDelta: frame.contentWindow.history.length - historyLength,
    reflected: frame.getAttribute('src') === target,
    events: events.slice(), loads
  });
  try {
    if (mode === 'location') {
      const ready = loaded(frame);
      frame.contentWindow.location.href = target;
      await ready;
    } else if (mode === 'query') {
      const ready = loaded(frame);
      set(frame, location.href + '?different');
      await ready;
    } else if (mode === 'pending') {
      const ready = loaded(frame);
      set(frame, '/frame-child.html?pending');
      await fetch('/pending-started');
      set(frame, target);
      await fetch('/release-pending');
      await ready;
    } else if (mode !== 'insert') {
      set(frame, target);
      set(frame, target);
      if (mode === 'remove-srcdoc') frame.removeAttribute('srcdoc');
      await delay();
    }
    const result = snapshot();
    if (mode === 'insert') result.initialLoads = initialLoads;
    if (mode === 'ancestor-change') {
      const previous = location.href;
      history.replaceState(null, '', '/history.html?changed');
      // Reading the wrapper must not retry a previously rejected attribute.
      frame.contentWindow;
      await delay();
      result.afterAncestorChange = snapshot();
      const ready = loaded(frame);
      set(frame, target);
      await ready;
      result.afterRetry = snapshot();
      history.replaceState(null, '', previous);
    }
    return result;
  } finally {
    frame.remove();
    for (const ancestor of ancestors.reverse()) ancestor.remove();
  }
}
