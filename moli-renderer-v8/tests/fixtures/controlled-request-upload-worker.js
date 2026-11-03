self.oninstall = event => event.waitUntil(skipWaiting());
self.onactivate = event => event.waitUntil(clients.claim());
self.onfetch = event => {
    const url = new URL(event.request.url);
    if (url.pathname !== '/upload') return;
    const mode = url.searchParams.get('mode');
    if (mode === 'byob') {
        event.respondWith((async () => {
            const reader = event.request.body.getReader({mode: 'byob'});
            const bytes = [];
            while (true) {
                const {done, value} = await reader.read(new Uint8Array(3));
                bytes.push(...value);
                if (done) break;
            }
            return new Response(JSON.stringify(bytes));
        })());
    } else if (mode === 'constant') {
        event.respondWith(new Response('constant'));
    } else if (mode === 'echo') {
        event.respondWith(new Response(event.request.body));
    } else if (mode === 'used') {
        event.request.text().catch(() => {});
    } else if (mode === 'clone') {
        event.request.clone().text().catch(() => {});
    } else if (mode === 'forward') {
        event.respondWith(fetch(event.request));
    } else if (mode === 'redirect') {
        event.respondWith(Response.redirect(new URL('/done', url), +url.searchParams.get('status')));
    }
};
