(async () => {
  const checks = [];
  async function check(name, test) {
    try { checks.push({name, passed: (await test()) === true}); }
    catch (error) { checks.push({name, passed: false, error: String(error)}); }
  }
  const name = 'cache-request-state-173';
  const fields = ['method', 'url', 'destination', 'mode', 'credentials', 'cache',
    'redirect', 'referrer', 'referrerPolicy', 'integrity', 'keepalive', 'duplex',
    'isHistoryNavigation', 'isReloadNavigation'];
  const origin = new URL(globalThis.location.href).origin;
  const profiles = [
    ['default', {}],
    ['rich', {mode: 'same-origin', credentials: 'include', cache: 'only-if-cached',
      redirect: 'manual', referrer: origin + '/referrer', referrerPolicy: 'no-referrer',
      integrity: 'sha256-YWJj', keepalive: true, headers: {'x-shape': 'circle'}}],
    ['empty-referrer', {mode: 'cors', credentials: 'omit', cache: 'reload', redirect: 'error',
      referrer: '', referrerPolicy: 'same-origin', headers: {'x-shape': 'square'}}],
    ['no-cors', {mode: 'no-cors', credentials: 'omit', cache: 'no-store', redirect: 'manual',
      referrerPolicy: 'strict-origin', headers: {'accept': 'text/plain'}}],
  ];
  let frame;
  await caches.delete(name);
  const cache = await caches.open(name);
  try {
    for (const [label, init] of profiles) {
      const request = new Request(origin + '/request-state?kind=' + label, init);
      const expected = Object.fromEntries(fields.map(field => [field, request[field]]));
      const expectedHeaders = JSON.stringify([...request.headers]);
      const response = new Response(label);
      await cache.put(request, response);
      const [stored] = await cache.keys(request);
      for (const field of fields) await check(label + '/keys/' + field, () => stored[field] === expected[field]);
      await check(label + '/keys/headers', () => JSON.stringify([...stored.headers]) === expectedHeaders);
      await check(label + '/keys/native fresh request', () => stored instanceof Request && stored !== request && stored !== undefined);
      await check(label + '/keys/fresh independent signal', () => stored.signal instanceof AbortSignal && stored.signal !== request.signal && !stored.signal.aborted);
      await check(label + '/keys/null unused body', () => stored.body === null && stored.bodyUsed === false);
      await check(label + '/response content', async () => await (await cache.match(request)).text() === label);
    }
    await check('stored request ignores author getters', async () => {
      const input = new Request(origin + '/request-author', {mode: 'same-origin', credentials: 'omit', redirect: 'error'});
      let reads = 0;
      for (const key of ['url','method','headers','mode','credentials','redirect']) {
        Object.defineProperty(input, key, {get() {reads++; throw 42;}});
      }
      await cache.put(input, new Response('author'));
      const [stored] = await cache.keys(origin + '/request-author');
      return reads === 0 && stored.mode === 'same-origin' && stored.credentials === 'omit' && stored.redirect === 'error';
    });
    await check('cache signal does not follow input abort', async () => {
      const controller = new AbortController();
      const request = new Request(origin + '/request-abort', {signal: controller.signal});
      await cache.put(request, new Response('abort'));
      const [stored] = await cache.keys(request);controller.abort(42);
      const [again] = await cache.keys(request);
      return request.signal.aborted && !stored.signal.aborted && !again.signal.aborted && stored.signal !== again.signal;
    });
    await check('already aborted Request can be stored', async () => {
      const controller = new AbortController();controller.abort(42);
      const request = new Request(origin + '/request-already-aborted', {signal: controller.signal});
      await cache.put(request, new Response('already-aborted'));
      return !(await cache.keys(request))[0].signal.aborted;
    });
    for (const method of ['add', 'addAll']) {
      const request = new Request(origin + '/batch-resource?kind=' + method, {
        mode: 'same-origin', credentials: 'omit', cache: 'no-store', redirect: 'error',
        referrer: origin + '/batch-referrer', referrerPolicy: 'same-origin', headers: {'x-shape': method},
      });
      const expected = Object.fromEntries(fields.map(field => [field, request[field]]));
      const promise = method === 'add' ? cache.add(request) : cache.addAll([request]);
      await promise;
      const [stored] = await cache.keys(request);
      for (const field of fields) await check(method + '/keys/' + field, () => stored[field] === expected[field]);
      await check(method + '/response content', async () => await (await cache.match(request)).text() === method);
    }
    const reopened = await caches.open(name);
    await check('reopen retains stored metadata', async () => {
      const [stored] = await reopened.keys(origin + '/request-state?kind=rich');
      return stored.mode === 'same-origin' && stored.credentials === 'include' && stored.cache === 'only-if-cached' &&
        stored.redirect === 'manual' && stored.referrer === origin + '/referrer' && stored.keepalive;
    });
    await check('keys ignores inherited RequestInit getters', async () => {
      const keys = ['mode','cache','credentials','redirect','referrer','referrerPolicy','integrity','keepalive','headers','signal'];
      const previous = keys.map(key => Object.getOwnPropertyDescriptor(Object.prototype,key));let reads = 0;
      for (const key of keys) Object.defineProperty(Object.prototype,key,{configurable:true,get(){reads++;throw 42;}});
      try {return (await cache.keys()).length === 9 && reads === 0;}
      finally {for(let i=0;i<keys.length;i++){if(previous[i])Object.defineProperty(Object.prototype,keys[i],previous[i]);else delete Object.prototype[keys[i]];}}
    });
    if (typeof document !== 'undefined') {
      frame = document.createElement('iframe');document.body.append(frame);const child = frame.contentWindow;
      const childCache = await child.caches.open(name);
      for (const [ownerLabel, owner, receiver] of [['main',globalThis,cache],['child',child,childCache]]) {
        for (const [calleeLabel,callee] of [['main',globalThis],['child',child]]) {
          const label = 'put/realm/' + ownerLabel + '/' + calleeLabel;
          await check(label + '/ready success Promise', async () => {
            const response = new callee.Response('ready');
            const promise = callee.Cache.prototype.put.call(receiver,origin + '/put-ready-' + label,response);
            return Object.getPrototypeOf(promise) === owner.Promise.prototype && await promise === undefined;
          });
          await check(label + '/pending stream success Promise', async () => {
            const body = new callee.ReadableStream({start(controller){controller.enqueue(new callee.Uint8Array([65]));controller.close();}});
            const response = new callee.Response(body);
            const key = origin + '/put-stream-' + label;
            const promise = callee.Cache.prototype.put.call(receiver,key,response);
            return Object.getPrototypeOf(promise) === owner.Promise.prototype && await promise === undefined &&
              await (await receiver.match(key)).text() === 'A';
          });
          await check(label + '/stream rejection identity', async () => {
            const reason = {};const body = new callee.ReadableStream({start(controller){controller.error(reason);}});
            const promise = callee.Cache.prototype.put.call(receiver,origin + '/put-rejection-' + label,new callee.Response(body));
            let error;try{await promise;}catch(e){error=e;}
            return Object.getPrototypeOf(promise) === owner.Promise.prototype && error === reason;
          });
          for (const [kind,request,response] of [
            ['invalid URL','http://[',new callee.Response('invalid')],
            ['non-GET',new callee.Request(origin + '/post',{method:'POST'}),new callee.Response('post')],
            ['partial response',origin + '/partial',new callee.Response('partial',{status:206})],
            ['Vary star',origin + '/vary',new callee.Response('vary',{headers:{Vary:'*'}})],
            ['Response conversion',origin + '/response',{}],
          ]) await check(label + '/' + kind + ' callee rejection', async () => {
            const promise = callee.Cache.prototype.put.call(receiver,request,response);let error;
            try{await promise;}catch(e){error=e;}
            return Object.getPrototypeOf(promise) === callee.Promise.prototype && error instanceof callee.TypeError;
          });
          await check(label + '/Request conversion original reason', async () => {
            const reason = {};const value = {toString(){throw reason;}};
            const promise = callee.Cache.prototype.put.call(receiver,value,new callee.Response('conversion'));let error;
            try{await promise;}catch(e){error=e;}
            return Object.getPrototypeOf(promise) === callee.Promise.prototype && error === reason;
          });
          await check(label + '/snapshot before pending body completes', async () => {
            let controller;const body = new callee.ReadableStream({start(value){controller=value;}});
            const request = new callee.Request(origin + '/put-snapshot-' + label, {
              mode:'same-origin',credentials:'omit',cache:'no-store',redirect:'manual',headers:{'x-shape':'before'},
            });
            const promise = callee.Cache.prototype.put.call(receiver,request,new callee.Response(body));
            request.headers.set('x-shape','after');controller.enqueue(new callee.Uint8Array([66]));controller.close();
            await promise;const [stored] = await receiver.keys(request);
            return stored.headers.get('x-shape') === 'before' && stored.mode === 'same-origin' && stored.credentials === 'omit' &&
              stored.cache === 'no-store' && stored.redirect === 'manual' && await (await receiver.match(request)).text() === 'B';
          });
        }
      }
    }
  } finally {if(frame)frame.remove();await caches.delete(name);}
  globalThis.__uiEventResults = {checks,total:checks.length,passed:checks.filter(row=>row.passed).length,complete:true};
  return true;
})()
