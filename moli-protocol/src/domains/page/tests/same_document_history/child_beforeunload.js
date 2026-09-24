async function (method, action = 'none') {
  const tick = () => new Promise(resolve => setTimeout(resolve, 0));
  async function makeFrame(owner, url) {
    const frame = owner.document.createElement('iframe');
    const loaded = new Promise(resolve => frame.onload = resolve);
    frame.src = url;
    owner.document.body.append(frame);
    await loaded;
    await tick();
    return frame;
  }
  const frame = await makeFrame(window, '/beforeunload.html?initial');
  const win = frame.contentWindow;
  const doc = frame.contentDocument;
  const nested = await makeFrame(win, '/history.html?nested');
  const nestedDoc = nested.contentDocument;
  const events = [];
  const details = [];
  const record = (owner, label) => {
    for (const type of ['beforeunload', 'pagehide', 'unload']) {
      owner.addEventListener(type, event => {
        events.push(label + type);
        if (type === 'beforeunload') {
          details.push([event.isTrusted, event.target === owner,
            event.currentTarget === owner, owner.document.hidden]);
        }
      });
    }
  };
  record(win, '');
  record(nested.contentWindow, 'child:');
  let loads = 0;
  frame.onload = () => loads++;
  if (action === 'self') {
    win.addEventListener('beforeunload', () => {
      win.location.href = '/history.html?forbidden';
    });
  } else if (action === 'remove') {
    win.addEventListener('beforeunload', () => frame.remove(), {once: true});
  } else if (action === 'stop') {
    win.addEventListener('beforeunload', () => win.stop(), {once: true});
  } else if (action === 'supersede' || action === 'forged-flag') {
    nested.contentWindow.addEventListener('beforeunload', () => {
      if (action === 'forged-flag') win.__lmWindowUnloadEventActive = false;
      win.location.href = '/beforeunload.html?replacement';
    }, {once: true});
  }
  const snapshot = () => ({
    events: events.slice(), details: details.slice(), loads,
    sameDocument: frame.contentDocument === doc,
    nestedAlive: nested.contentDocument === nestedDoc,
    oldHidden: doc.hidden,
    url: frame.contentDocument ? new URL(frame.contentDocument.URL).search : null,
  });
  return {
    snapshot,
    start() {
      const url = '/beforeunload.html?next';
      if (method === 'href') win.location.href = url;
      else if (method === 'assign') win.location.assign(url);
      else if (method === 'replace') win.location.replace(url);
      else if (method === 'reload') win.location.reload();
      else if (method === 'navigation') {
        const result = win.navigation.navigate(url);
        result.committed.catch(() => {});
        result.finished.catch(() => {});
      } else if (method === 'src') frame.src = url;
      else if (method === 'anchor') {
        const anchor = doc.body.appendChild(doc.createElement('a'));
        anchor.href = url;
        anchor.click();
      } else if (method === 'form') {
        const form = doc.body.appendChild(doc.createElement('form'));
        form.action = url;
        form.submit();
      }
      return snapshot();
    },
    async waitForLoad() {
      for (let i = 0; i < 500 && loads === 0; i++) {
        await new Promise(resolve => setTimeout(resolve, 10));
      }
      await tick();
      return snapshot();
    },
    remove() { frame.remove(); },
  };
}
