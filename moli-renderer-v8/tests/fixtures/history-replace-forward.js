async function historyReplaceForward(base, method) {
  const frame = document.createElement('iframe');
  const loaded = () => new Promise(resolve => frame.addEventListener('load', resolve, {once: true}));
  const settle = () => new Promise(resolve => setTimeout(resolve, 0));
  const initial = loaded();
  frame.src = base + '/a';
  document.body.appendChild(frame);
  await initial;
  async function navigate(action) {
    await settle();
    const completion = loaded();
    action(frame.contentWindow);
    await completion;
    await settle();
  }
  await navigate(w => w.location.href = base + '/b');
  await navigate(w => w.location.href = base + '/c');
  frame.contentWindow.history.replaceState({classic: 'forward'}, '');
  frame.contentWindow.navigation.updateCurrentEntry({state: {navigation: 'forward'}});
  const before = frame.contentWindow.navigation.entries().map(entry => ({
    id: entry.id, key: entry.key, url: entry.url,
  }));
  await navigate(w => w.history.back());
  await navigate(w => {
    if (method === 'location') w.location.replace(base + '/replaced');
    else w.navigation.navigate(base + '/replaced', {history: 'replace'});
  });
  const entries = frame.contentWindow.navigation.entries();
  const result = {
    paths: entries.map(entry => new URL(entry.url).pathname),
    backwardIdentity: entries[0].id === before[0].id && entries[0].key === before[0].key,
    replacementIdentity: entries[1].id !== before[1].id && entries[1].key === before[1].key,
    forwardIdentity: entries.length === 3 && entries[2].id === before[2].id && entries[2].key === before[2].key,
  };
  if (!result.forwardIdentity) {
    frame.remove();
    return result;
  }
  result.savedState = entries[2].getState();
  await navigate(w => w.history.forward());
  result.forwardPath = new URL(frame.contentWindow.location.href).pathname;
  result.classicState = frame.contentWindow.history.state;
  result.navigationState = frame.contentWindow.navigation.currentEntry.getState();
  await navigate(w => w.history.back());
  result.replacementPath = new URL(frame.contentWindow.location.href).pathname;
  frame.remove();
  return result;
}
