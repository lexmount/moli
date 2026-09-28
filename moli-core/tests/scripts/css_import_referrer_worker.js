const observations = new Map();
self.addEventListener('activate', event => event.waitUntil(clients.claim()));
self.addEventListener('fetch', event => {
  const url = new URL(event.request.url);
  if (url.pathname === '/css-import-observations') {
    event.respondWith(new Response(JSON.stringify(observations.get(url.searchParams.get('case')) || []), {
      headers: {'Content-Type':'application/json'}
    }));
    return;
  }
  if (!url.pathname.startsWith('/css-referrer/')) return;
  const [, , name, file] = url.pathname.split('/');
  const rows = observations.get(name) || [];
  rows.push({file,referrer:event.request.referrer,policy:event.request.referrerPolicy});
  observations.set(name, rows);
  const resource = file => new URL('/css-referrer/' + name + '/' + file, url).href;
  const imports = file => '@import url("' + resource(file) + '");';
  if (file === 'start.css') {
    event.respondWith(Response.redirect(resource('root.css')));
    return;
  }
  let body = '';
  let policy;
  if (file === 'root.css') {
    policy = {
      'no-referrer':'no-referrer', origin:'origin', unsafe:'unsafe-url',
      multiple:'origin, invalid, unsafe-url', invalid:'not-a-policy',
      redirect:'unsafe-url', reset:'no-referrer', nested:'unsafe-url', 'synthetic-inherit':'unsafe-url',
      diamond:'unsafe-url', 'external-data':'unsafe-url',
      'insert-root':'no-referrer', 'insert-child':'unsafe-url'
    }[name];
    if (['reset','nested','insert-child','synthetic-inherit'].includes(name)) body=imports('mid.css');
    else if (name === 'diamond') body=imports('left.css')+imports('right.css');
    else if (name === 'external-data')
      body='@import url("data:text/css,' + encodeURIComponent(imports('leaf.css')) + '");';
    else if (name !== 'insert-root') body=imports('leaf.css');
  } else if (file === 'mid.css') {
    if (name === 'nested') policy='no-referrer';
    if (name === 'insert-child') policy='origin';
    else body=imports('leaf.css');
  } else if (file === 'left.css' || file === 'right.css') {
    policy=file === 'left.css' ? 'no-referrer' : 'unsafe-url';
    body=imports('leaf.css');
  }
  const headers={'Content-Type':'text/css','Cache-Control':'no-store'};
  if (policy !== undefined) headers['Referrer-Policy']=policy;
  event.respondWith(new Response(body,{headers}));
});
