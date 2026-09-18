(async () => {
  let checks = 0;
  const failures = [], observations = [];
  const check = (name, actual, expected) => {
    checks++;
    if (JSON.stringify(actual) !== JSON.stringify(expected)) failures.push({name, actual, expected});
  };
  const frame = async (src) => {
    const node = document.createElement('iframe');
    if (src) node.src = src;
    const loaded = new Promise(resolve => node.onload = resolve);
    document.body.append(node);
    await loaded;
    return node;
  };
  const snapshot = (doc, win) => ({
    url: doc.URL, uri: doc.documentURI, base: doc.baseURI,
    location: win.location.href, length: win.history.length, state: win.history.state,
    navigation: win.navigation?.currentEntry?.url ?? null,
    link: new win.URL('relative', doc.baseURI).href,
  });
  const parentURL = new URL(document.URL); parentURL.hash = '';
  for (const mode of ['blank-open', 'loaded-open', 'loaded-write', 'base-open', 'borrowed-open', 'self-open']) {
    const f = await frame(mode === 'blank-open' ? null : '/compat/child-dynamic-markup-document?markup=%3C!doctype%20html%3E%3Cbody%3Etarget#target');
    const d = f.contentDocument, w = f.contentWindow;
    if (mode !== 'blank-open') w.history.replaceState({kept: true, map: new w.Map([['key', 'value']])}, '');
    const before = snapshot(d,w);
    const base = document.createElement('base');
    if (mode === 'base-open') {base.href = '/different-base/'; document.head.append(base);}
    let returned;
    if (mode === 'loaded-write') {d.write('<p>replacement</p>');}
    else if (mode === 'borrowed-open') {returned = Document.prototype.open.call(d);}
    else if (mode === 'self-open') {
      await new Promise(resolve => {
        w.done = result => {returned = result; resolve();};
        w.setTimeout(w.Function('done(document.open())'), 0);
      });
    } else {returned = d.open();}
    const after = snapshot(d,w);
    const expectedURL = mode === 'self-open' ? before.url : parentURL.href;
    for (const field of ['url','uri','location']) check(mode + ':' + field, after[field], expectedURL);
    check(mode + ':base', after.base, expectedURL);
    check(mode + ':history-length', after.length, before.length);
    check(mode + ':history-state', after.state, before.state);
    if (mode !== 'loaded-write') check(mode + ':identity', returned === d, true);
    check(mode + ':state-identity', after.state === before.state, true);
    if (mode !== 'blank-open') check(mode + ':structured-state', after.state.map.get('key'), 'value');
    observations.push({mode,before,after});
    d.close();
    if (mode === 'loaded-open') {
      w.history.pushState(null, '', new URL('?after-open', expectedURL).href);
      await new Promise(resolve => {
        w.addEventListener('popstate', resolve, {once: true});
        w.history.back();
      });
      check(mode + ':back-url', d.URL, expectedURL);
      check(mode + ':back-location', w.location.href, expectedURL);
      check(mode + ':back-structured-state', w.history.state.map.get('key'), 'value');
    }
    f.remove(); base.remove();
  }
  for (const mode of ['nested-call', 'timer', 'microtask']) {
    const source = await frame('/compat/child-dynamic-markup-document?markup=%3Cbody%3Esource#source-fragment');
    const target = await frame('/compat/child-dynamic-markup-document?markup=%3Cbody%3Etarget#target-fragment');
    const w = source.contentWindow, d = target.contentDocument;
    w.target = d;
    const expected = mode === 'nested-call' ? parentURL.href : w.document.URL.split('#')[0];
    if (mode === 'nested-call') {
      w.Function('target.open()')();
    } else {
      await new Promise(resolve => {
        w.done = resolve;
        const action = w.Function('target.open(); done();');
        if (mode === 'timer') w.setTimeout(action, 0);
        else w.Promise.resolve().then(action);
      });
    }
    const observed = snapshot(d, target.contentWindow);
    for (const field of ['url', 'uri', 'base', 'location']) check(mode + ':' + field, observed[field], expected);
    d.close(); source.remove(); target.remove();
  }
  for (const mode of ['removed', 'windowless', 'old-document']) {
    const f = await frame();
    let d = f.contentDocument;
    if (mode === 'removed') f.remove();
    if (mode === 'windowless') d = d.implementation.createHTMLDocument('');
    if (mode === 'old-document') {
      const loaded = new Promise(resolve => f.onload = resolve);
      f.src = '/compat/child-dynamic-markup-document?markup=replacement'; await loaded;
    }
    const before = d.URL;
    check(mode + ':identity', d.open() === d, true);
    check(mode + ':url', d.URL, before);
    d.close(); f.remove();
  }
  return {checks, failures, observations};
})()
