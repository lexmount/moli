async function iframeSrcdocHistory(root, api, originalURL) {
  const frame = root.document.createElement('iframe');
  const loaded = () => new Promise(resolve => frame.addEventListener('load', resolve, {once: true}));
  const settle = () => new Promise(resolve => setTimeout(resolve, 0));
  let ready = loaded();
  frame.src = originalURL;
  root.document.body.append(frame);
  await ready;
  await settle();
  const initialLength = root.history.length;
  const steps = [];
  const navigate = async (label, attribute, values) => {
    const beforeDocument = frame.contentDocument;
    const beforeEntry = frame.contentWindow.navigation.currentEntry;
    const beforeKey = beforeEntry.key;
    const beforeId = beforeEntry.id;
    const beforeLength = root.history.length;
    ready = loaded();
    for (const value of values) {
      if (api === 'property') frame[attribute] = value;
      else frame.setAttribute(attribute, value);
    }
    const during = {
      sameDocument: frame.contentDocument === beforeDocument,
      sameEntry: frame.contentWindow.navigation.currentEntry === beforeEntry,
      delta: root.history.length - beforeLength,
    };
    await ready;
    await settle();
    const child = frame.contentWindow;
    const entry = child.navigation.currentEntry;
    steps.push({
      label, during,
      delta: root.history.length - initialLength,
      childDelta: child.history.length - initialLength,
      index: entry.index,
      type: child.navigation.activation.navigationType,
      sameKey: entry.key === beforeKey,
      newId: entry.id !== beforeId,
      text: attribute === 'srcdoc' ? child.document.querySelector('p').textContent : null,
      entries: child.navigation.entries().map(entry => entry.url.startsWith('about:srcdoc') ? entry.url : 'ordinary'),
    });
  };
  try {
    await navigate('same-url-src', 'src', [originalURL]);
    await navigate('first-srcdoc', 'srcdoc', ['<p>discarded</p>', '<p>one</p>']);
    await navigate('same-url-srcdoc', 'srcdoc', ['<p>two</p>']);
    const child = frame.contentWindow;
    child.history.pushState(null, '', 'about:srcdoc#part');
    const fragment = {
      delta: root.history.length - initialLength,
      childDelta: child.history.length - initialLength,
      index: child.navigation.currentEntry.index,
      url: child.location.href,
    };
    await navigate('after-fragment', 'srcdoc', ['<p>three</p>']);
    return {steps, fragment};
  } finally {
    frame.remove();
  }
}
