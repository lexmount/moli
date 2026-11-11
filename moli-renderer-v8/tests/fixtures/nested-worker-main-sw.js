self.oninstall = event => event.waitUntil(self.skipWaiting());
self.onactivate = event => event.waitUntil(self.clients.claim());
const requests = [];
self.onmessage = event => event.waitUntil(
  self.clients.matchAll({type: 'all', includeUncontrolled: true}).then(clients => {
    event.ports[0].postMessage({clients: clients.map(client => ({
      id: client.id, url: client.url, type: client.type,
    })), requests});
  })
);
self.onfetch = event => {
  const url = new URL(event.request.url);
  if (url.pathname === '/child/probe') {
    event.respondWith(Response.json({clientId: event.clientId, source: 'serviceworker'}));
    return;
  }
  if (url.pathname !== '/child/script.js') return;
  const meta = {
    clientId: event.clientId,
    resultingClientId: event.resultingClientId,
    destination: event.request.destination,
    mode: event.request.mode,
    credentials: event.request.credentials,
    referrer: event.request.referrer,
    url: event.request.url,
  };
  requests.push(meta);
  if (url.searchParams.get('route') === 'rewritten') {
    event.respondWith(fetch('/final/script.js' + url.search));
    return;
  }
  event.respondWith(new Response(
    'postMessage({kind:"child",source:"serviceworker",meta:' + JSON.stringify(meta) + '});' +
    'onmessage=e=>postMessage({kind:"echo",value:e.data.value,bytes:Array.from(new Uint8Array(e.data.bytes))});',
    {headers: {'Content-Type': 'text/javascript'}}
  ));
};
