(async () => {
  const checks = [];
  const check = async (name, fn) => {
    try {checks.push({name, passed: await fn() === true});}
    catch (error) {checks.push({name, passed: false, error: String(error)});}
  };
  const realms = [window, document.querySelector('iframe').contentWindow];
  const names = ['ignoreMethod', 'ignoreSearch', 'ignoreVary', 'cacheName'];
  for (const [ownerIndex, owner] of realms.entries()) for (const [calleeIndex, callee] of realms.entries()) {
    const prefix = `cache-inherit/${ownerIndex}/${calleeIndex}`;
    const call = options => callee.CacheStorage.prototype.match.call(owner.caches, location.origin + '/never-cached', options);
    await check(prefix + '/base-before-own', async () => {
      const order = [], options = {};
      for (const key of names) Object.defineProperty(options, key, {get() {
        order.push(key);
        return key === 'cacheName' ? {toString() {order.push('string'); return 'dictionary-order-empty';}} : false;
      }});
      const result = await call(options);
      return result === undefined && order.join() === names.concat('string').join();
    });
    for (const proxy of [false, true]) await check(prefix + '/inherited-or-proxy/' + proxy, async () => {
      const order = [], target = {};
      for (const key of names) Object.defineProperty(target, key, {get() {order.push(key); return key === 'cacheName' ? 'dictionary-order-empty' : false;}});
      const result = await call(proxy ? new owner.Proxy(target, {}) : owner.Object.create(target));
      return result === undefined && order.join() === names.join();
    });
    for (const key of names) await check(prefix + '/getter-exception/' + key, async () => {
      const sentinel = {}, order = [], options = {};
      for (const member of names) Object.defineProperty(options, member, {get() {
        order.push(member);
        if (key === member) throw sentinel;
        return member === 'cacheName' ? 'dictionary-order-empty' : false;
      }});
      let promise;
      try {promise = call(options);} catch (_) {return false;}
      try {await promise;} catch (error) {return error === sentinel && order.join() === names.slice(0, names.indexOf(key) + 1).join();}
      return false;
    });
    await check(prefix + '/conversion-exception', async () => {
      const sentinel = {}, order = [];
      const options = new owner.Proxy({}, {get(_, key) {
        order.push(key);
        return key === 'cacheName' ? {toString() {order.push('string'); throw sentinel;}} : false;
      }});
      let promise;
      try {promise = call(options);} catch (_) {return false;}
      try {await promise;} catch (error) {return error === sentinel && order.join() === names.concat('string').join();}
      return false;
    });
    await check(prefix + '/no-base-property', async () => {
      const order = [];
      const result = await call(new owner.Proxy({}, {get(_, key) {
        order.push(key);
        if (!names.includes(key)) throw Error('unexpected dictionary member: ' + String(key));
        return undefined;
      }}));
      return result === undefined && order.join() === names.join();
    });
    for (const [index, cacheName] of ['\ud800', '\udc00', 'x\ud800y', '𝌆'].entries()) {
      await check(prefix + '/utf16-name/' + index, async () => {
        let conversions = 0;
        const result = await call({cacheName: {toString() {conversions++; return cacheName;}}});
        return result === undefined && conversions === 1;
      });
    }
  }
  globalThis.__cacheDictionaryInheritanceResults = {complete: true, total: checks.length, passed: checks.filter(row => row.passed).length, checks};
  return true;
})()
