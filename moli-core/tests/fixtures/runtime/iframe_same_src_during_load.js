({ phase, write, childURL, crossOrigin }) => new Promise((resolve, reject) => {
  const frame = document.createElement('iframe');
  const documents = [];
  const checks = [];
  let windowProxy;
  let rounds = 0;
  const timeout = setTimeout(() => finish(new Error(`Only ${rounds} load callbacks ran`)), 3000);

  function finish(error) {
    clearTimeout(timeout);
    frame.remove();
    delete globalThis.reloadFrameInWindowLoad;
    if (error) reject(error);
    else resolve(checks);
  }

  function check(condition, label) {
    checks.push({ label: `${phase}/${write}/${crossOrigin ? 'cross' : 'same'}/${rounds}: ${label}`, pass: condition });
  }

  function writeSrc() {
    if (write === 'property') frame.src = childURL;
    else frame.setAttribute('src', childURL);
  }

  function reload() {
    rounds++;
    check(frame.contentWindow === windowProxy, 'WindowProxy identity survives reload');
    check(frame.contentWindow.length === 2, 'descendant frames finished loading');
    if (!crossOrigin) {
      const document = frame.contentDocument;
      check(document.readyState === 'complete', 'load sees a complete document');
      check(!documents.includes(document), 'each load owns a fresh document');
      documents.push(document);
    }
    if (rounds < 3) {
      const before = rounds;
      const document = frame.contentDocument;
      writeSrc();
      check(rounds === before, 'same-src navigation does not dispatch load synchronously');
      check(frame.contentDocument === document, 'same-src assignment leaves the old document until commit');
    }
  }

  // The child calls this synchronously from its Window load handler. The owner
  // variant instead reloads in the iframe element's load handler, like WPT's
  // cross-origin-objects.html reload barrier.
  globalThis.reloadFrameInWindowLoad = phase === 'window-load' ? reload : () => {};
  frame.onload = () => {
    if (phase === 'owner-load') reload();
    if (rounds === 3) setTimeout(() => finish(), 0);
  };
  writeSrc();
  document.body.append(frame);
  windowProxy = frame.contentWindow;
})
