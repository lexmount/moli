(async () => {
  const checks = [];
  async function check(name, test) {
    try { checks.push({name, passed: (await test()) === true}); }
    catch (error) { checks.push({name, passed: false, error: String(error)}); }
  }
  const names = ['cache-array-first', 'cache-array-z', 'cache-array-a'];
  const bucketNames = ['array-first', 'array-last'];
  const origin = new URL(location.href).origin;
  const urls = [origin + '/array-first', origin + '/array-second', origin + '/array-third'];
  const bodies = ['first', 'second', 'third'];
  const cache = await caches.open(names[0]);
  for (let i = 1; i < names.length; i++) await caches.open(names[i]);
  for (let i = 0; i < urls.length; i++) {
    await cache.put(urls[i], new Response(bodies[i], {headers: {'x-array': bodies[i]}}));
  }
  for (const name of bucketNames) await navigator.storageBuckets.open(name);
  let frame;
  const owners = [['main', globalThis, cache]];
  if (typeof document !== 'undefined') {
    frame = document.createElement('iframe');document.body.append(frame);
    const child = frame.contentWindow;
    owners.push(['child', child, await child.caches.open(names[0])]);
  }
  const poisons = ['setter', 'throwing-setter', 'getter-only', 'readonly-data'];
  async function query(owner, callee, receiver, api, request, prototype, index, poison) {
    const target = owner[prototype].prototype;
    const previous = Object.getOwnPropertyDescriptor(target, String(index));
    const reason = {arrayPoison: poison};
    let calls = 0, value, error, promise;
    const descriptor = poison === 'readonly-data'
      ? {configurable: true, value: 'inherited-poison', writable: false}
      : poison === 'getter-only'
        ? {configurable: true, get() {calls++; throw reason;}}
        : {configurable: true, set() {calls++; if (poison === 'throwing-setter') throw reason;}};
    Object.defineProperty(target, String(index), descriptor);
    try {
      if (api === 'CacheStorage.keys') promise = callee.CacheStorage.prototype.keys.call(receiver);
      else if (api === 'StorageBucketManager.keys') promise = callee.StorageBucketManager.prototype.keys.call(receiver);
      else promise = callee.Cache.prototype[api].call(receiver, request);
      value = await promise;
    } catch (caught) {error = caught;}
    finally {
      if (previous) Object.defineProperty(target, String(index), previous);
      else delete target[String(index)];
    }
    if (error !== undefined || calls !== 0 || !Array.isArray(value)) return false;
    if (Object.getPrototypeOf(promise) !== owner.Promise.prototype || Object.getPrototypeOf(value) !== owner.Array.prototype) return false;
    const frozen = api === 'keys' || api === 'matchAll';
    if (Object.isFrozen(value) !== frozen) return false;
    const expected = api === 'CacheStorage.keys' ? names : api === 'StorageBucketManager.keys' ? bucketNames
      : request === undefined ? urls : request === urls[1] ? [urls[1]] : [];
    if (value.length !== expected.length) return false;
    for (let i = 0; i < value.length; i++) {
      const own = Object.getOwnPropertyDescriptor(value, String(i));
      if (!own || !('value' in own) || !own.enumerable || own.writable === frozen || own.configurable === frozen) return false;
      if (api === 'CacheStorage.keys' || api === 'StorageBucketManager.keys') {
        if (own.value !== expected[i]) return false;
      } else {
        const C = api === 'keys' ? owner.Request : owner.Response;
        if (Object.getPrototypeOf(own.value) !== C.prototype) return false;
        if (api === 'keys') {
          if (own.value.url !== expected[i] || own.value.body !== null || own.value.signal.aborted) return false;
        } else {
          const n = urls.indexOf(expected[i]);
          if (await own.value.text() !== bodies[n] || own.value.headers.get('x-array') !== bodies[n]) return false;
        }
      }
    }
    const length = Object.getOwnPropertyDescriptor(value, 'length');
    return !!length && length.writable !== frozen && !length.enumerable && !length.configurable;
  }
  try {
    for (const [ownerLabel, owner, receiver] of owners) {
      for (const [calleeLabel, callee] of owners) {
        for (const prototype of ['Array', 'Object']) {
          for (const index of [0, 1]) {
            for (const poison of poisons) {
              for (const api of ['keys', 'matchAll']) {
                for (const [requestLabel, request] of [['all', undefined], ['filtered', urls[1]], ['empty', origin + '/absent']]) {
                  const label = `${ownerLabel}/${calleeLabel}/${api}/${requestLabel}/${prototype}/${index}/${poison}`;
                  await check(label, () => query(owner, callee, receiver, api, request, prototype, index, poison));
                }
              }
              const label = `${ownerLabel}/${calleeLabel}/CacheStorage.keys/${prototype}/${index}/${poison}`;
              await check(label, () => query(owner, callee, owner.caches, 'CacheStorage.keys', undefined, prototype, index, poison));
            }
          }
        }
      }
      for (const prototype of ['Array', 'Object']) for (const index of [0, 1]) for (const poison of poisons) {
        const label = `${ownerLabel}/StorageBucketManager.keys/${prototype}/${index}/${poison}`;
        await check(label, () => query(owner, owner, owner.navigator.storageBuckets, 'StorageBucketManager.keys', undefined, prototype, index, poison));
      }
    }
    for (const [label, owner, receiver] of owners) {
      for (const api of ['keys', 'matchAll']) {
        await check(`${label}/${api}/fresh dense results`, async () => {
          const a = await receiver[api](), b = await receiver[api]();
          return a !== b && a.length === 3 && b.length === 3 && a.every((value, i) => value !== b[i]);
        });
      }
      await check(`${label}/cache entries survive prototype poison`, async () => {
        for (let i = 0; i < urls.length; i++) if (await (await receiver.match(urls[i])).text() !== bodies[i]) return false;
        return true;
      });
      await check(`${label}/bucket and cache names survive prototype poison`, async () => {
        return JSON.stringify(await owner.caches.keys()) === JSON.stringify(names) &&
          JSON.stringify(await owner.navigator.storageBuckets.keys()) === JSON.stringify(bucketNames);
      });
    }
  } finally {
    if (frame) frame.remove();
    for (const name of names) await caches.delete(name);
    for (const name of bucketNames) await navigator.storageBuckets.delete(name);
  }
  globalThis.__uiEventResults = {checks, total: checks.length, passed: checks.filter(row => row.passed).length, complete: true};
  return true;
})()
