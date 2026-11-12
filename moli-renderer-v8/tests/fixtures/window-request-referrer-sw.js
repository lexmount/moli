
self.oninstall = event => event.waitUntil(self.skipWaiting());
self.onactivate = event => event.waitUntil(self.clients.claim());
self.onfetch = event => {
  const url = new URL(event.request.url), path = url.pathname;
  const kind = url.searchParams.get('api');
  if (path.endsWith('/echo')) {
    const result = {source: 'serviceworker', referrer: event.request.referrer,
      referrerPolicy: event.request.referrerPolicy, cache: event.request.cache,
      method: event.request.method, mode: event.request.mode,
      credentials: event.request.credentials, destination: event.request.destination,
      bodyIsNull: event.request.body === null, clientId: event.clientId,
      resultingClientId: event.resultingClientId};
    event.respondWith(new Response(kind === 'eventsource' ? 'data: ' + JSON.stringify(result) + '\n\n' : JSON.stringify(result),
      {headers: {'Content-Type': kind === 'eventsource' ? 'text/event-stream' : 'application/json'}}));
  } else if (path.endsWith('/forward')) {
    event.respondWith(fetch(event.request));
  } else if (path.endsWith('/sw-redirect')) {
    event.respondWith(Response.redirect(CROSS + '/network?api=' + kind, 307));
  }
};
