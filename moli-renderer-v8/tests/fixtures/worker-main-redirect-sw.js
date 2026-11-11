self.oninstall = event => event.waitUntil(self.skipWaiting());
self.onactivate = event => event.waitUntil(self.clients.claim());
const requests = [];
self.onmessage = event => event.waitUntil(self.clients.matchAll({type: 'all', includeUncontrolled: true}).then(clients => event.ports[0].postMessage({
  clients: clients.map(client => ({id: client.id, url: client.url, type: client.type})), requests,
})));
self.onfetch = event => {
  const url = new URL(event.request.url);
  if (!['/child/child.js', '/child/redirect.js', '/other/child.js'].includes(url.pathname)) return;
  const meta = {
    clientId: event.clientId, resultingClientId: event.resultingClientId,
    destination: event.request.destination, mode: event.request.mode,
    credentials: event.request.credentials, referrer: event.request.referrer, url: event.request.url,
  };
  requests.push(meta);
  if (url.pathname === '/child/redirect.js') {
    event.respondWith(Response.redirect(new URL((url.searchParams.get('route') === 'exit' ? '/outside/' : '/other/') + 'child.js' + url.search, url)));
    return;
  }
  event.respondWith(new Response('const SOURCE="serviceworker",META=' + JSON.stringify(meta) + ';' + CONNECT + (event.request.destination === 'sharedworker' ? 'onconnect=e=>connect(e.ports[0]);' : 'connect(self);'), {headers: {'Content-Type': 'text/javascript'}}));
};
