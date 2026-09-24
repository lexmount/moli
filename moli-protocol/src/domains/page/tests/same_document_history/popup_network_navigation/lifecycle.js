async function(method, action = 'none', destination = 'network') {
  const tick = () => new Promise(resolve => setTimeout(resolve, 0));
  const traversing = method === 'back' || method === 'navigation-back';
  const win = open(traversing ? '/history.html?previous' : '/popup-unload.html?initial', 'popup-lifecycle');
  await new Promise(resolve => win.addEventListener('load', resolve, {once: true}));
  await tick();
  if (traversing) {
    const previous = win.document;
    win.location.href = '/popup-unload.html?initial';
    for (let i = 0; i < 500 && (win.document === previous || win.document.readyState !== 'complete'); i++)
      await new Promise(resolve => setTimeout(resolve, 10));
    await tick();
  }
  const doc = win.document;
  async function child(owner, name) {
    const frame = owner.createElement('iframe');
    frame.src = '/history.html?' + name;
    const loaded = new Promise(resolve => frame.onload = resolve);
    owner.body.append(frame);
    await loaded;
    await tick();
    return frame;
  }
  const frame = await child(doc, 'child');
  const childDoc = frame.contentDocument;
  const grandchild = await child(childDoc, 'grandchild');
  const events = [], details = [], writes = [], microtasks = [];
  const nodes = [[win, doc, ''], [frame.contentWindow, childDoc, 'child:'],
    [grandchild.contentWindow, grandchild.contentDocument, 'grandchild:']];
  function attemptWrite(target) {
    const body = target.body;
    const text = body.textContent;
    const same = target.open() === target;
    target.write('<p>unexpected destructive write</p>');
    target.close();
    return same && target.body === body && body.textContent === text;
  }
  for (const [owner, original, label] of nodes) {
    for (const type of ['beforeunload', 'pagehide', 'unload']) {
      owner.addEventListener(type, event => {
        events.push(label + type);
        if (type === 'beforeunload')
          details.push([event.isTrusted, event.target === owner, event.currentTarget === owner, original.hidden]);
        if (action === 'writes') {
          const ancestors = type === 'beforeunload' ? [original] : nodes.slice(0, nodes.findIndex(n => n[0] === owner) + 1).map(n => n[1]);
          writes.push([label + type, ...ancestors.map(attemptWrite)]);
        }
      });
    }
  }
  if (action === 'self') win.addEventListener('beforeunload', () => {
    win.location.href = '/history.html?forbidden';
  });
  if (action === 'ancestor' || action.startsWith('named-') || action === 'stop')
    grandchild.contentWindow.addEventListener('beforeunload', () => {
      if (action === 'stop') win.stop();
      else if (action === 'named-ancestor') open('/history.html?forbidden', 'popup-lifecycle');
      else if (action === 'named-anchor') {
        const anchor = document.body.appendChild(document.createElement('a'));
        anchor.href = '/history.html?forbidden'; anchor.target = 'popup-lifecycle'; anchor.click();
      } else if (action === 'named-form') {
        const form = document.body.appendChild(document.createElement('form'));
        form.action = '/history.html'; form.target = 'popup-lifecycle';
        const input = form.appendChild(document.createElement('input'));
        input.name = 'forbidden'; form.submit();
      }
      else win.location.href = '/history.html?forbidden';
    });
  if (action === 'close-before' || action === 'close-pagehide') {
    win.addEventListener(action === 'close-before' ? 'beforeunload' : 'pagehide', () => {
      win.close();
      Promise.resolve().then(() => microtasks.push('close'));
    }, {once: true});
  }
  const snapshot = () => ({
    events: events.slice(), details: details.slice(), writes: writes.slice(), microtasks: microtasks.slice(),
    closed: win.closed, sameDocument: !win.closed && win.document === doc,
    nestedAlive: frame.contentDocument === childDoc, oldHidden: doc.hidden,
    url: win.closed ? null : win.document.URL.replace(location.origin, ''),
  });
  return {
    snapshot,
    start() {
      const url = destination === 'blank' ? 'about:blank' : '/popup-unload.html?step=next';
      if (method === 'href') win.location.href = url;
      else if (method === 'assign') win.location.assign(url);
      else if (method === 'replace') win.location.replace(url);
      else if (method === 'reload') win.location.reload();
      else if (method === 'navigation' || method === 'navigation-back') {
        const result = method === 'navigation-back' ? win.navigation.back() : win.navigation.navigate(url);
        result.committed.catch(() => {}); result.finished.catch(() => {});
      } else if (method === 'named') open(url, 'popup-lifecycle');
      else if (method === 'anchor') {
        const anchor = doc.body.appendChild(doc.createElement('a'));
        anchor.href = url; anchor.click();
      } else if (method === 'form') {
        const form = doc.body.appendChild(doc.createElement('form'));
        form.action = '/popup-unload.html';
        const input = form.appendChild(doc.createElement('input'));
        input.name = 'step'; input.value = 'next'; form.submit();
      } else if (method === 'close') win.close();
      else if (method === 'back') win.history.back();
      return snapshot();
    },
    async waitForCommit() {
      for (let i = 0; i < 500 && !win.closed && (win.document === doc || win.document.readyState !== 'complete'); i++)
        await new Promise(resolve => setTimeout(resolve, 10));
      await tick();
      return snapshot();
    },
    close() { win.close(); },
  };
}
