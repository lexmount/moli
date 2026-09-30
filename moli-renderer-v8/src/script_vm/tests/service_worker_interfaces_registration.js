(async () => {
  const rows = [];
  const assert = (ok, message) => { if (!ok) throw Error(message); };
  const check = async (name, run) => {
    try { await run(); rows.push({ name, pass: true }); }
    catch (error) { rows.push({ name, pass: false, message: String(error) }); }
  };
  let registration;
  try {
    const originals = ['ServiceWorker', 'ServiceWorkerRegistration', 'NavigationPreloadManager'].map(name => [name, globalThis[name], Object.getOwnPropertyDescriptor(globalThis, name)]);
    let calls = 0;
    for (const [name] of originals) Object.defineProperty(globalThis, name, { configurable: true, get() { calls++; throw Error('author constructor getter'); } });
    try { registration = await navigator.serviceWorker.register('/app/worker.js', { scope: '/app/' }); }
    finally { for (const [name, , descriptor] of originals) { if (descriptor) Object.defineProperty(globalThis, name, descriptor); else delete globalThis[name]; } }
    await check('registration/intrinsic-factories', () => assert(calls === 0, 'constructors not read during native creation'));
    const worker = registration.installing || registration.waiting || registration.active;
    const manager = registration.navigationPreload;
    await check('registration/native-prototypes', () => {
      assert(Object.getPrototypeOf(registration) === ServiceWorkerRegistration.prototype && registration instanceof EventTarget, 'registration prototype');
      assert(Object.getPrototypeOf(worker) === ServiceWorker.prototype && worker instanceof EventTarget, 'worker prototype');
      assert(Object.getPrototypeOf(manager) === NavigationPreloadManager.prototype, 'navigation preload prototype');
      for (const [object, property] of [[registration, 'scope'], [registration, 'active'], [registration, 'navigationPreload'], [worker, 'state'], [worker, 'scriptURL'], [manager, 'getState']]) assert(!Object.hasOwn(object, property), 'shared ' + property);
      assert(registration.navigationPreload === manager, 'SameObject navigation preload');
      assert(registration.scope === new URL('/app/', location.href).href && registration.updateViaCache === 'imports', 'registration values');
      assert(worker.scriptURL === new URL('/app/worker.js', location.href).href, 'worker script URL');
    });
    await check('registration/activation-handler', async () => {
      if (worker.state === 'activated') return;
      await new Promise((resolve, reject) => {
        const timeout = setTimeout(() => { worker.onstatechange = null; reject(Error('activation timeout')); }, 10000);
        worker.onstatechange = function(event) {
          try {
            assert(this === worker && event instanceof Event && event.isTrusted && event.target === worker, 'native statechange event');
            if (worker.state === 'activated') { clearTimeout(timeout); worker.onstatechange = null; resolve(); }
          } catch (error) { clearTimeout(timeout); worker.onstatechange = null; reject(error); }
        };
      });
    });
    await check('registration/preload-native-state', async () => {
      await manager.enable(); await manager.setHeaderValue('from-window');
      const state = await manager.getState();
      assert(state.enabled && state.headerValue === 'from-window', 'native preload state');
      await manager.disable(); assert(!(await manager.getState()).enabled, 'native disable');
    });
    await check('registration/cross-realm-preload', async () => {
      const other = document.getElementById('child').contentWindow;
      const promise = other.NavigationPreloadManager.prototype.getState.call(manager);
      assert(promise instanceof Promise && !(promise instanceof other.Promise) && (await promise).headerValue === 'from-window', 'cross-realm genuine receiver');
      let error; try { await other.NavigationPreloadManager.prototype.setHeaderValue.call(new Proxy(manager, {}), 'bad'); } catch (e) { error = e; }
      assert(error instanceof other.TypeError && (await manager.getState()).headerValue === 'from-window', 'author Proxy rejected before mutation');
    });
    await check('registration/worker-realm', async () => {
      const channel = new MessageChannel();
      const result = await new Promise((resolve, reject) => {
        const timeout = setTimeout(() => { channel.port1.close(); reject(Error('worker probe timeout')); }, 10000);
        channel.port1.onmessage = event => { clearTimeout(timeout); channel.port1.close(); resolve(event.data); };
        worker.postMessage('probe', [channel.port2]);
      });
      rows.push(...result);
      assert(result.every(row => row.pass), JSON.stringify(result.filter(row => !row.pass)));
      assert((await manager.getState()).headerValue === 'from-worker', 'Window and Worker share native state');
    });
  } catch (error) { rows.push({ name: 'registration/setup', pass: false, message: String(error) }); }
  finally { if (registration) await registration.unregister(); }
  globalThis.__nodeReplacementResults = { rows, failures: rows.filter(row => !row.pass), passed: rows.filter(row => row.pass).length, total: rows.length };
  return rows.every(row => row.pass);
})()
