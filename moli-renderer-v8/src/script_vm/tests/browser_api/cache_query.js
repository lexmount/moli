(async () => {
  const checks = [];
  async function check(name, test) {
    try { checks.push({name, passed: (await test()) === true}); }
    catch (error) { checks.push({name, passed: false, error: String(error)}); }
  }
  const name = 'query-fixture'; await caches.delete(name); const cache = await caches.open(name);
  await cache.put('/undefined', new Response('undefined'));
  await cache.put('/null', new Response('null'));
  await cache.put('/ordinary', new Response('ordinary'));
  for (const method of ['matchAll', 'keys']) {
    for (const options of [undefined, null, {}]) {
      await check(method + ' optional undefined/' + String(options), async () => (await cache[method](undefined, options)).length === 3);
    }
    await check(method + ' null converts', async () => (await cache[method](null)).length === 1);
  }
  for (const method of ['match', 'matchAll', 'keys', 'delete']) {
    const args = () => method === 'delete' ? '/absent' : '/ordinary';
    await check(method + ' ignores cacheName getter', async () => {
      let reads = 0; await cache[method](args(), {get cacheName() { reads++; throw 42; }}); return reads === 0;
    });
    await check(method + ' dictionary getter order', async () => {
      const order = []; const options = Object.create(null);
      for (const member of ['ignoreSearch', 'ignoreMethod', 'ignoreVary']) Object.defineProperty(options, member, {get() {order.push(member); return false;}});
      await cache[method](args(), options); return order.join() === 'ignoreMethod,ignoreSearch,ignoreVary';
    });
    for (const primitive of [1, false, 'x', Symbol('x')]) {
      await check(method + ' rejects primitive ' + String(primitive), async () => {
        const p = cache[method](args(), primitive); let error; try { await p; } catch (e) {error = e;}
        return p instanceof Promise && error instanceof TypeError;
      });
    }
    for (const member of ['ignoreMethod', 'ignoreSearch', 'ignoreVary']) {
      await check(method + ' getter exception ' + member, async () => {
        const reason = {}; let error; try {await cache[method](args(), {[member]: false, get [member]() {throw reason;}});} catch (e) {error = e;} return error === reason;
      });
    }
    await check(method + ' request conversion exception', async () => {
      const reason = {}; let reads = 0, error; const input = {toString() {throw reason;}};
      try { await cache[method](input, {get ignoreMethod() {reads++; return true;}}); } catch (e) {error = e;}
      return error === reason && reads === 0;
    });
    await check(method + ' invalid URL after options', async () => {
      let reads = 0, error; try {await cache[method]('http://[', {get ignoreMethod() {reads++; return false;}});} catch (e) {error = e;}
      return error instanceof TypeError && reads === 1;
    });
    await check(method + ' Symbol request rejects', async () => {
      let error; try {await cache[method](Symbol('x'));} catch (e) {error = e;} return error instanceof TypeError;
    });
  }
  await check('required undefined request converts', async () => await (await cache.match(undefined)).text() === 'undefined');
  await check('CacheStorage inherited dictionary order', async () => {
    const order = []; const options = Object.create(null);
    for (const member of ['cacheName', 'ignoreSearch', 'ignoreMethod', 'ignoreVary']) Object.defineProperty(options, member, {get() {order.push(member); return member === 'cacheName' ? name : false;}});
    await caches.match('/ordinary', options); return order.join() === 'ignoreMethod,ignoreSearch,ignoreVary,cacheName';
  });
  await check('CacheStorage cacheName getter exception', async () => {
    const reason = {}; let error; try {await caches.match('/ordinary', {get cacheName() {throw reason;}});} catch (e) {error = e;} return error === reason;
  });
  for (const method of ['open', 'has', 'delete']) {
    await check('CacheStorage.' + method + ' name conversion exception', async () => {
      const reason = {}; let error; const promise = caches[method]({toString() {throw reason;}}); try {await promise;} catch (e) {error = e;}
      return promise instanceof Promise && error === reason;
    });
    await check('CacheStorage.' + method + ' missing name', async () => {
      let error; try {await caches[method]();} catch (e) {error = e;} return error instanceof TypeError;
    });
  }
  const names = ['\ud800', '\ud801', '\udc00', '\ufffd', '\ud83d\ude00', 'x\0y', '~u16-d800', ''];
  for (let i = 0; i < names.length; i++) {
    await check('DOMString name roundtrip ' + i, async () => {
      await caches.delete(names[i]); const c = await caches.open(names[i]);
      await c.put('/name', new Response(String(i))); return (await caches.keys()).includes(names[i]);
    });
  }
  for (let i = 0; i < names.length; i++) {
    await check('DOMString name identity ' + i, async () => {
      const c = await caches.open(names[i]); return await (await c.match('/name')).text() === String(i) && await (await caches.match('/name', {cacheName: names[i]})).text() === String(i);
    });
  }
  await check('DOMString deletion distinct', async () => {
    await caches.delete(names[0]); return !(await caches.has(names[0])) && await caches.has(names[1]) && await caches.has(names[3]);
  });
  for (const url of ['data:text/plain,x', 'ftp://example.test/x', 'blob:https://example.test/uuid']) {
    await check('put rejects scheme ' + url.split(':')[0], async () => {
      const response = new Response('unconsumed'); let error;
      try {await cache.put(new Request(url), response);} catch (e) {error = e;}
      return error instanceof TypeError && response.bodyUsed === false && await cache.match(url) === undefined;
    });
  }
  for (const n of [name, ...names]) await caches.delete(n);
  globalThis.__uiEventResults = {total: checks.length, passed: checks.filter(c => c.passed).length, checks, complete: true};
  return true;
})()
