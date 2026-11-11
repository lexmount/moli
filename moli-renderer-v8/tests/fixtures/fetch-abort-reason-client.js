globalThis.abortRows = [];
globalThis.abortError = null;
globalThis.abortParents = [];
(async () => {
  const registration = await navigator.serviceWorker.register('/sw.js', {scope: '/controlled/'});
  const worker = registration.installing || registration.waiting || registration.active;
  if (worker.state !== 'activated') await new Promise(resolve => worker.addEventListener('statechange', () => {
    if (worker.state === 'activated') resolve();
  }));
  if (!navigator.serviceWorker.controller) await new Promise(resolve => navigator.serviceWorker.addEventListener('controllerchange', resolve, {once: true}));
  for (const creator of ['window', 'dedicated', 'shared']) {
    let port;
    if (creator !== 'window') {
      const parent = creator === 'dedicated' ? new Worker('/controlled/dedicated.js') : new SharedWorker('/controlled/shared.js');
      abortParents.push(parent);
      port = creator === 'shared' ? parent.port : parent;
      if (port.start) port.start();
    }
    for (const kind of ['default', 'null', 'string', 'object', 'error', 'typeerror', 'custom', 'domexception', 'function', 'getter', 'throwing']) {
      const id = creator + '-' + kind;
      const row = creator === 'window' ? await runAbort(kind, id) : await new Promise((resolve, reject) => {
        port.onmessage = event => event.data.error ? reject(new Error(event.data.error)) : resolve(event.data);
        port.postMessage({kind, id});
      });
      abortRows.push({...row, creator});
    }
  }
  globalThis.abortReady = true;
})().catch(error => abortError = String(error));
