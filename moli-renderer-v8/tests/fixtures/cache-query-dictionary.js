(async () => {
  const assert = (ok, label) => { if (!ok) throw Error(label); };
  const name = 'derive-cache-dictionary';
  await caches.delete(name);
  const cache = await caches.open(name);
  try {
    await cache.put('/one', new Response('one'));
    await cache.put('/two', new Response('two'));
    const base = ['ignoreMethod','ignoreSearch','ignoreVary'];
    const operations = [
      ['Cache.match', (key, options) => cache.match(key, options), base],
      ['Cache.matchAll', (key, options) => cache.matchAll(key, options), base],
      ['Cache.keys', (key, options) => cache.keys(key, options), base],
      ['Cache.delete', (key, options) => cache.delete(key, options), base],
      ['CacheStorage.match', (key, options) => caches.match(key, options), [...base, 'cacheName']]
    ];
    for (const [label, operation, members] of operations) {
      const trace = [], options = {};
      for (const key of members) Object.defineProperty(options, key, {
        configurable: true, get() { trace.push(key); }
      });
      await operation('/missing', options);
      assert(trace.join() === members.join(), label + ' inheritance order: ' + trace);
      for (const stop of members) for (const sentinel of [undefined, null, false, 0, 'sentinel', {}]) {
        trace.length = 0;
        const bad = Object.create(options);
        Object.defineProperty(bad, stop, {get() { trace.push(stop); throw sentinel; }});
        let promise;
        try { promise = operation('/missing', bad); } catch (error) { throw Error(label + ' synchronous conversion throw'); }
        assert(promise instanceof Promise, label + ' returns Promise');
        let rejected = false, error;
        await promise.then(() => {}, caught => { rejected = true; error = caught; });
        assert(rejected && Object.is(error, sentinel), label + '/' + stop + ' original rejection');
        assert(trace.join() === members.slice(0, members.indexOf(stop) + 1).join(), label + '/' + stop + ' short circuit');
      }
      for (const options of [1, true, 'options', 1n, Symbol('options')]) {
        let error;
        await operation('/missing', options).catch(caught => { error = caught; });
        assert(error instanceof TypeError, label + ' primitive dictionary');
      }
      const sentinel = {};
      let error;
      await operation('http://[', {get ignoreMethod() { throw sentinel; }}).catch(caught => { error = caught; });
      assert(error === sentinel, label + ' options conversion precedes URL parsing');
      error = undefined;
      await operation({toString() { throw sentinel; }}, {get ignoreMethod() { throw Error('late options'); }})
        .catch(caught => { error = caught; });
      assert(error === sentinel, label + ' RequestInfo string conversion precedes options');
      if (label !== 'CacheStorage.match') {
        await operation('/missing', {get cacheName() { throw Error('unknown cacheName'); }});
      }
    }
    assert((await cache.keys()).length === 2, 'failed conversions do not change cache entries');
    const trace = [];
    await cache.match({toString() { trace.push('url'); return '/missing'; }}, {
      get ignoreMethod() { trace.push('ignoreMethod'); },
      get ignoreSearch() { trace.push('ignoreSearch'); },
      get ignoreVary() { trace.push('ignoreVary'); }
    });
    assert(trace.join() === 'url,ignoreMethod,ignoreSearch,ignoreVary', 'argument conversion order');
    assert((await cache.matchAll(undefined, {})).length === 2, 'explicit undefined omits matchAll request');
    assert((await cache.keys(undefined, {})).length === 2, 'explicit undefined omits keys request');
    await cache.put(new Request('/vary', {headers: {'X-Mode': 'after'}}), new Response('vary', {headers: {Vary: 'X-Mode'}}));
    const request = new Request('/vary', {headers: {'X-Mode': 'before'}});
    const response = await cache.match(request, {get ignoreVary() { request.headers.set('X-Mode', 'after'); return false; }});
    assert(response && await response.text() === 'vary', 'native Request state read after options conversion');
    const native = new Request('/one');
    Object.defineProperty(native, 'toString', {value() { throw Error('native Request stringification'); }});
    Object.setPrototypeOf(native, null);
    assert(await (await cache.match(native)).text() === 'one', 'Request native identity survives prototype changes');
    let conversions = 0, error;
    await cache.put({toString() { conversions++; return '/missing'; }}).catch(caught => { error = caught; });
    assert(error instanceof TypeError && conversions === 0, 'put arity checked before earlier argument conversion');
    const body = new Response('kept');
    error = undefined;
    await cache.put('http://[', body).catch(caught => { error = caught; });
    assert(error instanceof TypeError && !body.bodyUsed && await body.text() === 'kept', 'failed put conversion does not consume response body');
    for (const response of [{}, new Proxy(new Response('body'), {})]) {
      let error;
      await cache.put('/missing', response).catch(caught => { error = caught; });
      assert(error instanceof TypeError, 'put Response interface conversion');
    }
    return true;
  } finally {
    await caches.delete(name);
  }
})()
