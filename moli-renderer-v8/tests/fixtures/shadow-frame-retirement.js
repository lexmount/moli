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
  const snapshot = win => win.closed;
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
  const result = {after, reinserted, fresh};
  container.remove();
  return result;
}
