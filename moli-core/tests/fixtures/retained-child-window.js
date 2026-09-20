(async () => {
  const result = {checks: 0, failures: [], observations: []};
  const check = (name, action, expected) => {
    result.checks++;
    try {
      const actual = action();
      if (JSON.stringify(actual) !== JSON.stringify(expected)) result.failures.push({name, actual, expected});
    } catch (error) { result.failures.push({name, error: error.name, message: error.message}); }
  };
  const tick = () => new Promise(resolve => setTimeout(resolve, 0));
  const load = async (node, url) => {
    const loaded = new Promise(resolve => node.onload = resolve);
    node.src = url;
    await loaded;
  };
  const frame = async (owner, url) => {
    const node = owner.createElement('iframe');
    const loaded = new Promise(resolve => node.onload = resolve);
    if (url) node.src = url;
    owner.body.append(node);
    if (url) await loaded;
    return node;
  };
  for (const mode of ['parent-navigation', 'parent-open', 'remove-loaded', 'remove-blank', 'remove-reinsert']) {
    const parent = await frame(document, '/compat/child-dynamic-markup-document?markup=parent');
    const child = await frame(parent.contentDocument, mode === 'remove-blank' ? null : '/compat/child-dynamic-markup-document?markup=child#retained');
    const win = child.contentWindow, doc = child.contentDocument;
    const url = doc.URL;
    const eventConstructor = win.Event;
    win.retainedMarker = {value: mode};
    const marker = win.retainedMarker;
    let existingCalls = 0, newCalls = 0;
    const existing = () => existingCalls++;
    win.addEventListener('retained-test', existing);
    win.dispatchEvent(new Event('retained-test'));
    check(mode + ':live-event', () => existingCalls, 1);
    let liveTimerCalls = 0;
    await new Promise(resolve => win.setTimeout(() => { liveTimerCalls++; resolve(); }, 0));
    check(mode + ':live-timer', () => liveTimerCalls, 1);
    if (mode === 'parent-navigation') await load(parent, '/compat/child-dynamic-markup-document?markup=replacement');
    else if (mode === 'parent-open') { parent.contentDocument.open(); parent.contentDocument.close(); }
    else {
      child.remove();
      if (mode === 'remove-reinsert') {
        child.src = '/compat/child-dynamic-markup-document?markup=reinserted';
        const loaded = new Promise(resolve => child.onload = resolve);
        parent.contentDocument.body.append(child);
        await loaded;
      }
    }
    await tick();
    let timerCalls = 0;
    check(mode + ':retired-timer-registration', () => win.setTimeout(() => timerCalls++, 0), 0);
    check(mode + ':retired-source-timer', () => win.setTimeout('globalThis.retiredTimerRan = true', 0), 0);
    check(mode + ':retired-interval', () => win.setInterval(() => timerCalls++, 0), 0);
    check(mode + ':retired-source-interval', () => win.setInterval('globalThis.retiredTimerRan = true', 0), 0);
    await tick();
    check(mode + ':retired-timer-inactive', () => timerCalls, 0);
    check(mode + ':document', () => win.document === doc, true);
    // With no relevant Document, Location exposes about:blank; the retained Document keeps its URL.
    check(mode + ':location', () => win.location.href, 'about:blank');
    result.observations.push({mode, defaultViewIsNull: doc.defaultView === null});
    check(mode + ':own-data', () => win.retainedMarker === marker, true);
    check(mode + ':intrinsic', () => win.Event === eventConstructor, true);
    check(mode + ':self', () => win.self === win, true);
    check(mode + ':event-property', () => typeof win.addEventListener, 'function');
    check(mode + ':old-event', () => { win.dispatchEvent(new Event('retained-test')); return existingCalls; }, 1);
    check(mode + ':new-event', () => {
      win.addEventListener('retained-test', () => newCalls++);
      win.dispatchEvent(new Event('retained-test'));
      return [existingCalls, newCalls];
    }, [1, 0]);
    check(mode + ':remove-event', () => {
      win.removeEventListener('retained-test', existing);
      win.dispatchEvent(new Event('retained-test'));
      return [existingCalls, newCalls];
    }, [1, 0]);
    check(mode + ':open', () => doc.open() === doc, true);
    check(mode + ':open-url', () => doc.URL, url);
    check(mode + ':open-document', () => win.document === doc, true);
    check(mode + ':open-keeps-events-retired', () => {
      win.dispatchEvent(new Event('retained-test'));
      return newCalls;
    }, 0);
    if (mode === 'remove-reinsert') {
      check(mode + ':new-window', () => child.contentWindow !== win, true);
      check(mode + ':new-document', () => child.contentWindow.document !== doc, true);
      const replacement = child.contentWindow;
      const timer = replacement.setTimeout(() => liveTimerCalls++, 0);
      check(mode + ':retired-clear', () => {win.clearTimeout(timer); return true;}, true);
      await tick();
      check(mode + ':replacement-timer', () => liveTimerCalls, 2);
    }
    doc.close(); parent.remove();
  }
  const child = await frame(document, '/compat/child-dynamic-markup-document?markup=old');
  const win = child.contentWindow, oldDoc = child.contentDocument;
  const oldConstructor = win.Event;
  win.oldMarker = 'old';
  await load(child, '/compat/child-dynamic-markup-document?markup=new');
  check('navigation:same-proxy', () => child.contentWindow === win, true);
  check('navigation:new-document', () => win.document === child.contentDocument && win.document !== oldDoc, true);
  check('navigation:new-intrinsic', () => win.Event !== oldConstructor, true);
  check('navigation:old-data-gone', () => win.oldMarker, undefined);
  const foreign = new URL('/compat/child-dynamic-markup-document?markup=foreign', location.href);
  foreign.hostname = location.hostname === '127.0.0.1' ? 'localhost' : '127.0.0.1';
  await load(child, foreign.href);
  check('navigation:foreign-proxy', () => child.contentWindow === win, true);
  check('navigation:foreign-denied', () => {
    try { void win.document; return false; } catch (error) { return error.name === 'SecurityError'; }
  }, true);
  child.remove(); await tick();
  check('navigation:removed-foreign-denied', () => {
    try { void win.document; return false; } catch (error) { return ['SecurityError', 'TypeError'].includes(error.name); }
  }, true);
  return result;
})()
