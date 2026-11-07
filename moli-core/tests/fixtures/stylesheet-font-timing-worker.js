self.addEventListener('install', event => event.waitUntil(self.skipWaiting()));
self.addEventListener('activate', event => event.waitUntil(clients.claim()));
const counts = {};
self.addEventListener('fetch', event => {
  const url = new URL(event.request.url);
  if (url.pathname === '/font-worker-counts') {
    event.respondWith(new Response(JSON.stringify(counts), {
      headers: {'Content-Type': 'application/json'}
    }));
    return;
  }
  if (url.pathname !== '/font-worker-asset') return;
  const token = url.searchParams.get('token');
  counts[token] = (counts[token] || 0) + 1;
  const source = url.searchParams.get('source');
  if (source === 'fallback') return;
  const response = fetch('/probe-asset' + url.search);
  event.respondWith(source === 'synthetic'
    ? response.then(response => response.arrayBuffer()).then(body =>
        new Response(body, {headers: {'Content-Type': 'font/ttf'}}))
    : response);
});
