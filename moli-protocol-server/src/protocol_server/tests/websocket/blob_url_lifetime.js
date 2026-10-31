async function probeBlobLifetime(kind, revoke) {
  const events = [];
  globalThis.__blobLifetimePhase = 'starting';
  const id = `blob-lifetime-${Date.now()}-${Math.random()}`;
  let settle;
  const done = new Promise(resolve => { settle = resolve; });
  const channel = new BroadcastChannel(id);
  channel.onmessage = event => {
    events.push(['channel', event.data]);
    if (event.data === id) settle('loaded');
  };
  const source = (kind === 'script' || kind === 'module')
    ? `globalThis.__blobLifetimeMarker = ${JSON.stringify(id)}; globalThis.__blobLifetimeExecutedPhase = globalThis.__blobLifetimePhase;`
    : `<!doctype html><meta charset="utf-8"><body>${id}<script>window.test_result = ${JSON.stringify(id)}; ${kind === 'popup-noopener' || kind === 'anchor' ? `new BroadcastChannel(${JSON.stringify(id)}).postMessage(${JSON.stringify(id)}); self.close();` : ''}</script>`;
  const url = URL.createObjectURL(new Blob([source], {type: kind === 'module' ? 'text/javascript' : kind === 'script' ? '' : 'text/html'}));
  let element;
  let popup;
  const timeout = setTimeout(() => settle('timeout'), 5000);
  try {
    if ((kind === 'script' || kind === 'module')) {
      element = document.createElement('script');
      element.src = url;
      if (kind === 'module') element.type = 'module';
      element.onload = () => {
        events.push(['load', globalThis.__blobLifetimeMarker]);
        settle(globalThis.__blobLifetimeMarker === id && globalThis.__blobLifetimeExecutedPhase === 'after-start' ? 'loaded' : 'wrong-marker-or-order');
      };
      element.onerror = () => { events.push(['error']); settle('error'); };
      document.body.appendChild(element);
    } else if (kind === 'iframe' || kind === 'iframe-navigation') {
      element = document.createElement('iframe');
      if (kind === 'iframe-navigation') {
        element.src = '/blank';
        await new Promise((resolve, reject) => {
          element.onload = resolve;
          element.onerror = reject;
          document.body.appendChild(element);
        });
      } else {
        element.src = url;
      }
      element.onload = () => {
        let marker;
        try { marker = element.contentWindow.test_result; }
        catch (error) { events.push(['access-error', error.name]); }
        events.push(['load', marker || null]);
        if (marker === id) settle('loaded');
      };
      if (kind === 'iframe-navigation') element.contentWindow.location = url;
      else document.body.appendChild(element);
    } else if (kind === 'popup') {
      popup = window.open(url);
      popup.onload = () => {
        const marker = popup.test_result;
        events.push(['load', marker || null]);
        if (marker === id) settle('loaded');
      };
    } else if (kind === 'popup-noopener') {
      popup = window.open();
      popup.opener = null;
      popup.location = url;
    } else if (kind === 'anchor') {
      element = document.createElement('a');
      element.href = url;
      element.target = '_blank';
      document.body.appendChild(element);
      element.click();
    } else {
      throw new Error(`unknown kind ${kind}`);
    }
    globalThis.__blobLifetimePhase = 'after-start';
    events.push(['started']);
    if (revoke) URL.revokeObjectURL(url);
    events.push(['after-revoke', revoke]);
    const outcome = await done;
    return {kind, revoke, outcome, events};
  } finally {
    clearTimeout(timeout);
    channel.close();
    element?.remove();
    if (popup && !popup.closed) popup.close();
    URL.revokeObjectURL(url);
  }
}
