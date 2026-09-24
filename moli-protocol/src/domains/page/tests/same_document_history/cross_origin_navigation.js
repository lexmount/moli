async function crossOriginNavigationProbe(api, crossOrigin, destination, action) {
  const frame = document.createElement('iframe');
  frame.name = 'navigation-target';
  const url = new URL('/cross-origin-navigation.html?', location.href);
  if (crossOrigin) url.hostname = location.hostname === 'localhost' ? '127.0.0.1' : 'localhost';
  const initialURL = url.href;
  frame.src = initialURL;
  const records = [], sourceEvents = [], queue = [];
  let resolveMessage, timer, form;
  const onMessage = event => {
    if (event.source !== frame.contentWindow || !event.data?.kind) return;
    if (event.data.kind === 'navigate') records.push(event.data);
    if (resolveMessage) {
      const resolve = resolveMessage;
      resolveMessage = null;
      resolve(event.data);
    } else queue.push(event.data);
  };
  const nextMessage = async () => {
    if (queue.length) return queue.shift();
    try {
      return await Promise.race([
        new Promise(resolve => { resolveMessage = resolve; }),
        new Promise((_, reject) => {
          timer = setTimeout(() => reject(new Error('missing child message')), 2000);
        })
      ]);
    } finally { clearTimeout(timer); }
  };
  const waitFor = async (...kinds) => {
    for (;;) {
      const message = await nextMessage();
      if (kinds.includes(message.kind)) return message;
    }
  };
  const onNavigate = event => sourceEvents.push(event.navigationType);
  addEventListener('message', onMessage);
  navigation.addEventListener('navigate', onNavigate);
  try {
    document.body.append(frame);
    await waitFor('load');
    frame.contentWindow.postMessage({kind: 'configure', action}, '*');
    const initial = await waitFor('configured');
    if (destination === 'fragment') url.hash = 'fragment';
    else url.pathname = '/cross-origin-navigation-destination.html';
    if (destination === 'cross-origin-document') {
      url.hostname = url.hostname === 'localhost' ? '127.0.0.1' : 'localhost';
    }
    let returnedExisting = null;
    if (api === 'href') frame.contentWindow.location.href = url.href;
    else if (api === 'window-location') frame.contentWindow.location = url.href;
    else if (api === 'replace') frame.contentWindow.location.replace(url.href);
    else if (api === 'open') returnedExisting = open(url.href, frame.name) === frame.contentWindow;
    else {
      form = document.createElement('form');
      form.action = url.href;
      form.target = frame.name;
      if (api === 'post') {
        form.method = 'POST';
        form.innerHTML = '<input name="secret" value="source-only">';
      }
      document.body.append(form);
      if (api === 'requestSubmit') form.requestSubmit();
      else form.submit();
    }
    const terminal = action === 'cancel'
      ? await waitFor('navigate', 'load')
      : await waitFor('success', 'load');
    frame.contentWindow.postMessage({kind: 'report'}, '*');
    const report = await waitFor('report');
    return {terminal: terminal.kind, records, sourceEvents, returnedExisting,
      unchanged: report.href === initialURL,
      reached: report.href === url.href,
      retained: report.token === initial.token,
      entriesDelta: report.entries - initial.entries,
      currentEntryMatches: report.currentEntryMatches};
  } finally {
    clearTimeout(timer);
    form?.remove();
    frame.remove();
    removeEventListener('message', onMessage);
    navigation.removeEventListener('navigate', onNavigate);
  }
}
