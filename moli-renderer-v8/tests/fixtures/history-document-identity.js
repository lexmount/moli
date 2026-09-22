async function historyDocumentIdentity(base, other, mode, method) {
  const crossOrigin = mode.startsWith('cross-');
  const firstURL = base + '/common/blank.html?first#start';
  const middleURL = (crossOrigin ? other : base) + '/common/blank.html?middle';
  const lastURL = mode.endsWith('-same') ? firstURL :
    mode.endsWith('-fragment') ? firstURL.replace('#start', '#last') :
    base + '/common/blank.html?last';
  const frame = document.createElement('iframe');
  let loads = 0;
  frame.addEventListener('load', () => loads++);
  const settle = () => new Promise(resolve => setTimeout(resolve, 0));
  async function load(action) {
    const completion = new Promise(resolve => frame.addEventListener('load', resolve, {once: true}));
    action();
    await completion;
    await settle();
  }
  await load(() => {
    frame.src = firstURL;
    document.body.appendChild(frame);
  });
  const initialLength = history.length;
  const save = value => {
    const w = frame.contentWindow;
    w.history.replaceState({classic: value}, '');
    w.navigation.updateCurrentEntry({state: {navigation: value}});
    return {id: w.navigation.currentEntry.id, key: w.navigation.currentEntry.key};
  };
  const first = save('first');
  await load(() => frame.contentWindow.location.href = middleURL);
  await load(() => frame.contentWindow.location.href = lastURL);
  const last = save('last');
  const rows = [];
  const normalize = value => value?.replace(other, '<other>').replace(base, '<base>');
  function record(stage, identity) {
    const w = frame.contentWindow, n = w.navigation;
    rows.push({
      stage, loads, history: history.length - initialLength,
      url: normalize(w.location.href),
      entries: n.entries().map(e => normalize(e.url)),
      sameDocument: n.entries().map(e => e.sameDocument),
      index: n.currentEntry.index,
      identity: n.currentEntry.id === identity.id && n.currentEntry.key === identity.key,
      classicState: w.history.state,
      navigationState: n.currentEntry.getState(),
      from: normalize(n.activation.from?.url) ?? null,
      type: n.activation.navigationType,
      activationEntry: n.activation.entry === n.currentEntry,
    });
  }
  record('last', last);
  function traverse(delta, key) {
    if (method === 'navigation') frame.contentWindow.navigation.traverseTo(key);
    else if (method === 'child') frame.contentWindow.history.go(delta);
    else history.go(delta);
  }
  await load(() => traverse(-2, first.key));
  record('back', first);
  await load(() => traverse(2, last.key));
  record('forward', last);

  // A same-Document entry can have a different path or query. Traversing it
  // must preserve the Document and activation, without another load event.
  const w = frame.contentWindow, activeDocument = w.document;
  const activation = w.navigation.activation;
  w.history.pushState({classic: 'same-document'}, '', '/common/blank.html?state');
  const popped = new Promise(resolve => w.addEventListener('popstate', resolve, {once: true}));
  w.history.back();
  await popped;
  await settle();
  record('same-document', last);
  const result = {
    rows,
    sameDocumentPreserved: w.document === activeDocument,
    activationPreserved: w.navigation.activation === activation,
  };
  frame.remove();
  return result;
}
