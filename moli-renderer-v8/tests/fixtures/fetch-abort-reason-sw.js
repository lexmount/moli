self.oninstall = event => event.waitUntil(self.skipWaiting());
self.onactivate = event => event.waitUntil(self.clients.claim());
self.onfetch = event => {
  const url = new URL(event.request.url);
  if (url.pathname !== '/controlled/data') return;
  const id = url.searchParams.get('case');
  let finish;
  event.respondWith(new Promise(resolve => finish = resolve));
  event.waitUntil(new Promise(resolve => {
    event.request.signal.addEventListener('abort', async () => {
      const report = {aborted: event.request.signal.aborted, reason: snapshot(event.request.signal.reason), clientId: event.clientId, url: event.request.url};
      const client = await clients.get(event.clientId);
      report.client = client ? {id: client.id, url: client.url, type: client.type} : null;
      fetch('/report?case=' + id, {method: 'POST', body: JSON.stringify(report)}).then(() => {
        finish(new Response('after-abort'));
        resolve();
      });
    }, {once: true});
    fetch('/started?case=' + id);
  }));
};
