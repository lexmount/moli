
globalThis.referrerResult = null;
globalThis.referrerError = null;
(async () => {
  if (CONTROLLED) {
  await navigator.serviceWorker.register('/app/sw.js', {scope: '/app/'});
  await navigator.serviceWorker.ready;
  if (!navigator.serviceWorker.controller) await new Promise(resolve =>
    navigator.serviceWorker.addEventListener('controllerchange', resolve, {once: true}));
  }
  function take(kind, url) {
    if (kind === 'fetch') return fetch(url).then(response => response.json());
    if (kind === 'eventsource') return new Promise((resolve, reject) => {
      const source = new EventSource(url);
      source.onmessage = event => {source.close(); resolve(JSON.parse(event.data));};
      source.onerror = () => {source.close(); reject(new Error('EventSource failed: ' + url));};
    });
    if (kind === 'syncxhr') {
      const xhr = new XMLHttpRequest(); xhr.open('GET', url, false); xhr.send();
      if (xhr.status !== 200) throw new Error('sync XHR status ' + xhr.status);
      return Promise.resolve(JSON.parse(xhr.responseText));
    }
    return new Promise((resolve, reject) => {
      const xhr = new XMLHttpRequest(); xhr.open('GET', url, true);
      xhr.onload = () => xhr.status === 200 ? resolve(JSON.parse(xhr.responseText)) : reject(new Error('XHR status ' + xhr.status));
      xhr.onerror = () => reject(new Error('XHR failed: ' + url)); xhr.send();
    });
  }
  const routes = [
    ['synthetic-same', ORIGIN + '/app/echo'], ['synthetic-cross', CROSS + '/echo'],
    ['forward-same', ORIGIN + '/app/forward'], ['forward-cross', CROSS + '/forward'],
    ['fallback-same', ORIGIN + '/app/network'], ['fallback-cross', CROSS + '/network'],
    ['synthetic-redirect', ORIGIN + '/app/sw-redirect'],
    ['network-policy-redirect', ORIGIN + '/app/network-redirect']
  ];
  const rows = [];
  for (const kind of ['xhr', 'syncxhr', 'eventsource', 'fetch']) {
    for (const [route, url] of routes) {
      if (kind === 'syncxhr' && !route.startsWith('synthetic-')) continue;
      try { rows.push({kind, route, data: await take(kind, url + '?api=' + kind)}); }
      catch (error) { rows.push({kind, route, error: String(error)}); }
    }
  }
  referrerResult = {url: location.href, documentReferrerPolicy: document.referrerPolicy, rows};
})().catch(error => referrerError = String(error)); void 0;
