self.addEventListener('install', e=>e.waitUntil(self.skipWaiting()));
self.addEventListener('activate', e=>e.waitUntil(clients.claim()));
const counts={};
self.addEventListener('fetch', e=>{
 const u=new URL(e.request.url);
 if(u.pathname==='/probe-worker-counts'){e.respondWith(new Response(JSON.stringify(counts),{headers:{'Content-Type':'application/json'}}));return;}
 if(u.pathname!=='/probe-worker-asset')return;
 counts[u.href]=(counts[u.href]||0)+1;
 const kind=u.searchParams.get('source');
 const target=new URL('/probe-asset'+u.search,self.location.href);
 if(kind==='cors'||kind==='opaque')target.hostname=target.hostname==='localhost'?'127.0.0.1':'localhost';
 e.respondWith(kind==='default'?new Response('globalThis.preloadExecuted=(globalThis.preloadExecuted||0)+1;',{headers:{'Content-Type':'text/javascript'}}):fetch(target,{mode:kind==='opaque'?'no-cors':'cors'}));
});
