async function iframeSrcdocTraversal(root, api, route, originalURL) {
  const frame = root.document.createElement('iframe');
  const loaded = () => new Promise(resolve => frame.addEventListener('load', resolve, {once: true}));
  const settle = () => new Promise(resolve => setTimeout(resolve, 0));
  const steps = [];
  const check = (condition, message) => { if (!condition) throw new Error(message); };
  const source = label => '<!doctype html><meta charset="utf-8"><base href="/historical/">' +
    '<p>' + label + '</p><script>window.sourceMarker=' + JSON.stringify(label) + ';<' + '/script>';
  const change = async action => {
    const ready = loaded();
    action();
    await ready;
    await settle();
  };
  const setSource = value => api === 'property' ? frame.srcdoc = value : frame.setAttribute('srcdoc', value);
  const remember = () => {
    const child = frame.contentWindow;
    const entry = child.navigation.currentEntry;
    return {key: entry.key, id: entry.id, url: child.location.href, charset: child.document.characterSet};
  };
  let initialLength;
  const restored = (label, expected, text, state, navigationState, activation = expected) => {
    const child = frame.contentWindow;
    check(child.location.href === expected.url, label + ': URL');
    check(child.navigation.currentEntry.key === expected.key, label + ': key');
    check(child.navigation.currentEntry.id === expected.id, label + ': id');
    check(child.navigation.activation.navigationType === 'traverse', label + ': activation');
    check(child.navigation.activation.entry.id === activation.id, label + ': activation entry');
    check(child.document.querySelector('p').textContent === text, label + ': source');
    check(child.sourceMarker === text, label + ': script execution');
    check(child.document.characterSet === expected.charset, label + ': charset');
    check(child.document.baseURI === new URL('/historical/', originalURL).href, label + ': base URL');
    check(JSON.stringify(child.history.state) === JSON.stringify(state), label + ': classic state');
    check(JSON.stringify(child.navigation.currentEntry.getState()) === JSON.stringify(navigationState), label + ': navigation state');
    check(frame.getAttribute('srcdoc') === source('new'), label + ': attribute changed during traversal');
    check(root.history.length === initialLength + 3, label + ': joint history length');
    steps.push(label);
  };
  try {
    await change(() => { frame.src = originalURL; root.document.body.append(frame); });
    initialLength = root.history.length;
    await change(() => setSource(source('discarded')));
    await change(() => setSource(source('old')));
    frame.contentWindow.history.replaceState({label: 'old'}, '');
    frame.contentWindow.navigation.updateCurrentEntry({state: {label: 'old-navigation'}});
    const old = remember();
    if (route === 'fragment') {
      frame.contentWindow.history.pushState({label: 'fragment'}, '', 'about:srcdoc#old');
      frame.contentWindow.navigation.updateCurrentEntry({state: {label: 'fragment-navigation'}});
    } else {
      await change(() => frame.contentWindow.location.href = originalURL + '&middle');
    }
    const middle = remember();
    await change(() => setSource(source('new')));
    const newer = remember();
    await change(() => root.history.back());
    if (route === 'fragment') {
      restored('back-middle', middle, 'old', {label: 'fragment'}, {label: 'fragment-navigation'});
      const documentBefore = frame.contentDocument;
      const popped = new Promise(resolve => frame.contentWindow.addEventListener('popstate', resolve, {once: true}));
      root.history.back();
      await popped;
      check(frame.contentDocument === documentBefore, 'same-document back replaced the Document');
    } else {
      check(frame.contentWindow.location.href === middle.url, 'ordinary URL was replaced by srcdoc');
      check(frame.contentWindow.sourceMarker === undefined, 'ordinary URL reused srcdoc source');
      steps.push('back-middle');
      await change(() => root.history.back());
    }
    restored('back-old', old, 'old', {label: 'old'}, {label: 'old-navigation'}, route === 'fragment' ? middle : old);
    await change(() => root.history.go(2));
    restored('forward-new', newer, 'new', null, undefined);
    await change(() => frame.contentWindow.history.go(-2));
    restored('child-back-old', old, 'old', {label: 'old'}, {label: 'old-navigation'});
    return steps;
  } finally {
    frame.remove();
  }
}
