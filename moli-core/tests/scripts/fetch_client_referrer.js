(async () => {
  const origins = __clientReferrerOrigins;
  const explicit = origins[0] + '/explicit-referrer?private';
  const initial = location.href.split('#')[0];
  let requestId = 0;
  async function check(owner, expected, label) {
    for (const origin of origins) {
      const url = origin + '/meta-policy-fetch?client-' + ++requestId;
      const requests = [
        ['default', () => owner.fetch(url), expected],
        ['client', () => owner.fetch(url, {referrer:'about:client'}), expected],
        ['Request', () => owner.fetch(new owner.Request(url)), expected],
        ['empty', () => owner.fetch(url, {referrer:''}), null],
        ['explicit', () => owner.fetch(url, {referrer:explicit,referrerPolicy:'unsafe-url'}), explicit],
      ];
      for (const [mode, send, wanted] of requests) {
        const actual = await (await send()).json();
        if (actual !== wanted) throw new Error(label + ' ' + mode + ': ' + actual + ' != ' + wanted);
      }
    }
  }
  async function frame(owner, kind, url) {
    const element = owner.document.createElement('iframe');
    if (kind === 'srcdoc') element.srcdoc = '<!doctype html><body>srcdoc';
    else element.src = url || 'about:blank';
    await new Promise(resolve => {element.onload=resolve;owner.document.body.append(element);});
    return element.contentWindow;
  }
  if (__opaqueReferrerProbe) {
    const child = document.createElement('iframe');
    child.sandbox = 'allow-scripts';
    const source = `(async () => {
      const values = [];
      for (const origin of ${JSON.stringify(origins)}) {
        for (const referrer of [undefined, 'about:client']) {
          const request = new Request(origin + '/meta-policy-fetch?opaque', {referrer,referrerPolicy:'unsafe-url'});
          if (request.referrer !== 'about:client') throw new Error('public Request referrer was resolved too early');
          values.push(await (await fetch(request)).json());
        }
      }
      parent.postMessage({values}, '*');
    })().catch(error => parent.postMessage({error:String(error)}, '*'));`;
    child.srcdoc = '<!doctype html><script>' + source + '<' + '/script>';
    const result = await new Promise(resolve => {
      addEventListener('message', event => {if (event.source === child.contentWindow) resolve(event.data);});
      document.body.append(child);
    });
    if (result.error || result.values.length !== 4 || result.values.some(value => value !== null)) {
      throw new Error('opaque srcdoc: ' + JSON.stringify(result));
    }
    return true;
  }

  // An author-controlled base URL affects resolution, never the referrer source.
  const base = document.createElement('base');
  base.href = origins[1] + '/unrelated-base/'; document.head.append(base);
  await check(window, initial, 'main');
  const srcdoc = await frame(window, 'srcdoc');
  await check(srcdoc, initial, 'srcdoc');
  const nested = await frame(srcdoc, 'srcdoc');
  await check(nested, initial, 'nested srcdoc');
  const blank = await frame(window, 'blank');
  await check(blank, null, 'about:blank');
  await check(await frame(blank, 'srcdoc'), null, 'srcdoc under about:blank');
  const networkURL = origins[0] + '/document-referrer.html?response=unsafe-url&child';
  const network = await frame(window, 'network', networkURL);
  await check(network, networkURL, 'network child');
  await check(await frame(network, 'srcdoc'), networkURL, 'srcdoc under network child');

  for (const policy of ['unsafe-url', 'no-referrer']) {
    const current = await frame(window, 'srcdoc');
    const element = current.frameElement;
    const meta = current.document.createElement('meta');
    meta.name='referrer'; meta.content=policy; current.document.head.append(meta); meta.remove();
    const replaced = new Promise(resolve => {
      function onMessage(event) {
        if (event.data !== 'client-referrer-replacement') return;
        removeEventListener('message', onMessage); resolve();
      }
      addEventListener('message', onMessage);
    });
    const markup = '<script>parent.postMessage("client-referrer-replacement", "*")<' + '/script>';
    current.location = 'javascript:' + JSON.stringify(markup);
    await replaced;
    await check(element.contentWindow, policy === 'no-referrer' ? null : initial,
      'javascript srcdoc ' + policy);
  }

  const updated = origins[0] + '/history-referrer?updated';
  history.replaceState(null, '', updated + '#excluded');
  try {
    await check(window, updated, 'main after history');
    await check(nested, updated, 'srcdoc after ancestor history');
    // Selecting an ancestor URL must still use the requesting Document's policy.
    const meta = nested.document.createElement('meta');
    meta.name='referrer'; meta.content='no-referrer'; nested.document.head.append(meta);
    await check(nested, null, 'srcdoc own policy');
  } finally {
    history.replaceState(null, '', initial);
  }
  return true;
})();
