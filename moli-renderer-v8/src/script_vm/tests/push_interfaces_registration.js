(async () => {
  const rows = [], assert = (ok, message) => { if (!ok) throw Error(message); };
  const check = async (name, run) => { try { await run(); rows.push({ name, pass: true }); } catch (error) { rows.push({ name, pass: false, message: String(error) }); } };
  let registration;
  try {
    const C = PushManager, descriptor = Object.getOwnPropertyDescriptor(globalThis, 'PushManager');
    let reads = 0;
    Object.defineProperty(globalThis, 'PushManager', { configurable: true, get() { reads++; throw Error('author constructor'); } });
    try { registration = await navigator.serviceWorker.register('/app/worker.js', { scope: '/app/' }); }
    finally { Object.defineProperty(globalThis, 'PushManager', descriptor); }
    const manager = registration.pushManager;
    await check('manager/native-factory', () => assert(reads === 0 && Object.getPrototypeOf(manager) === C.prototype && manager === registration.pushManager && !Object.hasOwn(manager, 'subscribe'), 'native shared prototype and SameObject'));
    for (const method of ['subscribe', 'permissionState']) {
      await check('manager/' + method + '/conversion-order', async () => {
        const order = [], sentinel = {};
        let error;
        const result = manager[method]({ get applicationServerKey() { order.push('key'); return null; }, get userVisibleOnly() { order.push('visible'); throw sentinel; } });
        assert(result instanceof Promise, 'dictionary failures reject');
        try { await result; } catch (e) { error = e; }
        assert(error === sentinel && order.join() === 'key,visible', 'dictionary order and exception identity');
        error = undefined; try { await manager[method](7); } catch (e) { error = e; }
        assert(error instanceof TypeError, 'primitive dictionary rejected');
      });
    }
    await check('manager/cross-realm', async () => {
      const other = document.getElementById('child').contentWindow;
      const promise = other.PushManager.prototype.getSubscription.call(manager);
      assert(promise instanceof Promise && !(promise instanceof other.Promise), 'valid result belongs to receiver realm');
      assert(await promise === null, 'native empty subscription store');
      let error, conversions = 0;
      const bad = other.PushManager.prototype.subscribe.call(new Proxy(manager, {}), { get userVisibleOnly() { conversions++; } });
      assert(bad instanceof other.Promise, 'invalid result belongs to callee realm');
      try { await bad; } catch (e) { error = e; }
      assert(error instanceof other.TypeError && conversions === 0, 'author Proxy rejected before conversion');
    });
    const worker = registration.installing || registration.waiting || registration.active;
    if (worker.state !== 'activated') await new Promise((resolve, reject) => {
      const timeout = setTimeout(() => reject(Error('activation timeout')), 10000);
      worker.onstatechange = () => { if (worker.state === 'activated') { clearTimeout(timeout); worker.onstatechange = null; resolve(); } };
    });
    await check('manager/worker', async () => {
      const channel = new MessageChannel();
      const result = await new Promise((resolve, reject) => {
        const timeout = setTimeout(() => { channel.port1.close(); reject(Error('worker timeout')); }, 10000);
        channel.port1.onmessage = event => { clearTimeout(timeout); channel.port1.close(); resolve(event.data); };
        worker.postMessage('probe', [channel.port2]);
      });
      rows.push(...result);
      assert(result.every(row => row.pass), JSON.stringify(result));
    });
  } catch (error) { rows.push({ name: 'registration/setup', pass: false, message: String(error) }); }
  finally { if (registration) await registration.unregister(); }
  globalThis.__nodeReplacementResults = { rows, failures: rows.filter(row => !row.pass), passed: rows.filter(row => row.pass).length, total: rows.length };
  return rows.every(row => row.pass);
})()
