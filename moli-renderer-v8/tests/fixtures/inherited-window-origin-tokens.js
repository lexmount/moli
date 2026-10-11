(() => {
  const checks = [];
  const check = (name, passed) => checks.push({name, passed: !!passed});
  const denied = action => {
    try { action(); } catch (error) { return error.name === 'SecurityError'; }
    return false;
  };
  const invalidReceiver = receiver => {
    try { getDocument.call(receiver); } catch (error) { return error instanceof TypeError; }
    return false;
  };
  const popup = open();
  const getDocument = Object.getOwnPropertyDescriptor(window, 'document').get;
  const getFrameElement = Object.getOwnPropertyDescriptor(window, 'frameElement').get;
  try {
    check('popup-document-accessible', popup.document.defaultView === popup);
    check('borrowed-popup-document-getter', getDocument.call(popup) === popup.document);
    let parent = popup;
    for (let depth = 0; depth < 3; depth++) {
      const frame = parent.document.createElement('iframe');
      parent.document.body.appendChild(frame);
      const child = frame.contentWindow;
      const prefix = `depth-${depth}:`;
      check(prefix + 'inherited-document', child.document === frame.contentDocument);
      check(prefix + 'parent', child.parent === parent);
      check(prefix + 'top', child.top === popup);
      check(prefix + 'borrowed-document-getter', getDocument.call(child) === child.document);
      check(prefix + 'borrowed-frame-element-getter', getFrameElement.call(child) === frame);
      child.__originTokenMarker = depth;
      check(prefix + 'shared-window-property', child.__originTokenMarker === depth);
      parent = child;
    }
    for (const [label, owner] of [['root', window], ['popup', popup], ['nested', parent]]) {
      const frame = owner.document.createElement('iframe');
      frame.setAttribute('sandbox', 'allow-scripts');
      owner.document.body.appendChild(frame);
      const child = frame.contentWindow;
      check(label + ':sandbox-content-document', frame.contentDocument === null);
      check(label + ':sandbox-direct-getter-denied', denied(() => child.document));
      check(label + ':sandbox-borrowed-getter-denied', denied(() => getDocument.call(child)));
      const revoked = Proxy.revocable(child, {});
      revoked.revoke();
      for (const [kind, receiver] of [
        ['plain', {}],
        ['inherited', Object.create(child)],
        ['author-proxy', new Proxy(child, {})],
        ['revoked-proxy', revoked.proxy],
      ]) {
        check(`${label}:${kind}-receiver-rejected`, invalidReceiver(receiver));
      }
      frame.remove();
    }
  } catch (error) {
    checks.push({name: 'fixture-completed-without-exception', passed: false, details: String(error.stack || error)});
  } finally {
    popup?.close();
  }
  globalThis.__uiEventResults = {complete: true, checks, total: checks.length, passed: checks.filter(row => row.passed).length};
  return checks.every(row => row.passed);
})()
