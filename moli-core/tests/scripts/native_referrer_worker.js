const observations = new Map();
self.addEventListener('activate', event => event.waitUntil(clients.claim()));
self.addEventListener('fetch', event => {
  const url = new URL(event.request.url);
  if (!url.pathname.startsWith('/native-referrer/')) return;
  const id = url.searchParams.get('id');
  if (url.pathname === '/native-referrer/observations') {
    const value = observations.get(id);
    observations.delete(id);
    event.respondWith(new Response(JSON.stringify(value), {
      headers: {'Content-Type':'application/json'}
    }));
    return;
  }
  const entry = {
    referrer: event.request.referrer,
    policy: event.request.referrerPolicy,
    destination: event.request.destination,
  };
  observations.set(id, entry);
  const type = entry.destination === 'iframe' ? 'text/html' :
    entry.destination === 'style' ? 'text/css' : 'text/javascript';
  const body = entry.destination !== 'iframe' ? '' : url.searchParams.has('parser-css') ?
    '<!doctype html><meta name="referrer" content="no-referrer"><link rel="stylesheet" href="/native-referrer/resource?id=parser-css"><body>child' :
    '<!doctype html><body>child';
  event.respondWith(new Response(body, {
    headers: {'Content-Type':type}
  }));
});
