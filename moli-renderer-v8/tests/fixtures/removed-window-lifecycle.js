async function removedWindowLifecycle({mode, shadow}) {
  const container = document.createElement('div');
  document.body.append(container);
  const mount = shadow ? container.attachShadow({mode: 'closed'}) : container;
  const frame = document.createElement('iframe');
  frame.src = '/history.html?root';
  let loaded = new Promise(resolve => frame.onload = resolve);
  mount.append(frame);
  await loaded;

  const child = frame.contentDocument.createElement('iframe');
  child.src = '/history.html?child';
  loaded = new Promise(resolve => child.onload = resolve);
  frame.contentDocument.body.append(child);
  await loaded;

  const windows = [frame.contentWindow, child.contentWindow];
  const observations = [];
  for (const [index, win] of windows.entries()) {
    // These callbacks must read the retiring Window from its own realm.
    win.Function('log', 'index', `
      const savedTop = top, savedParent = parent, savedFrame = frameElement;
      for (const type of ['pagehide', 'visibilitychange', 'unload']) {
        (type === 'visibilitychange' ? document : window).addEventListener(type, () => {
          log.push({index, type, top: top === savedTop, parent: parent === savedParent,
            frameElement: frameElement === savedFrame, closed, hidden: document.hidden});
        });
      }
    `)(observations, index);
  }

  const snapshot = win => ({top: win.top === null, parent: win.parent === null,
    frameElement: win.frameElement === null, closed: win.closed});
  if (mode === 'remove') frame.remove();
  else if (mode === 'replace') mount.replaceChildren();
  else if (mode === 'ancestor') container.remove();
  else if (mode === 'fragment') document.createDocumentFragment().append(frame);
  else throw new Error('Unknown removal operation');

  const after = windows.map(snapshot);
  if (mode === 'ancestor') {
    frame.remove();
    document.body.append(container);
  }
  frame.src = '/history.html?replacement';
  loaded = new Promise(resolve => frame.onload = resolve);
  mount.append(frame);
  await loaded;

  const reinserted = windows.map(snapshot);
  const fresh = {top: frame.contentWindow.top === window,
    parent: frame.contentWindow.parent === window,
    frameElement: frame.contentWindow.frameElement === frame,
    closed: frame.contentWindow.closed, different: windows[0] !== frame.contentWindow};
  const result = {observations: observations.slice(), after, reinserted, fresh};
  container.remove();
  return result;
}
