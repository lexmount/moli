globalThis.nestedResult = null;
globalThis.nestedError = null;
globalThis.nestedParents = [];
(async () => {
  let registration;
  if (['intercept', 'redirect', 'rewritten'].includes(MODE)) {
    registration = await navigator.serviceWorker.register('/sw.js', {scope: '/child/'});
    const worker = registration.installing || registration.waiting || registration.active;
    if (worker.state !== 'activated') {
      await new Promise(resolve => worker.addEventListener('statechange', () => {
        if (worker.state === 'activated') resolve();
      }));
    }
  }
  const rows = [];
  const routes = MODE === 'lifecycle' ? ['status', 'mime', 'syntax', 'redirect', 'terminate'] : [MODE];
  for (const route of routes) for (const parentKind of ['dedicated', 'shared']) for (const type of ['classic', 'module']) {
    const id = route + '-' + parentKind + '-' + type;
    const parentPath = '/outer/' + parentKind + '.js?case=' + id;
    const parent = parentKind === 'shared' ? new SharedWorker(parentPath) : new Worker(parentPath);
    nestedParents.push(parent);
    const port = parentKind === 'shared' ? parent.port : parent;
    const events = [];
    let releaseReason = null, releasePromise = null;
    function release(reason) {
      if (releaseReason !== null) return;
      releaseReason = reason;
      releasePromise = fetch('/release?case=' + id);
    }
    const completed = new Promise((resolve, reject) => {
      parent.onerror = event => reject(new Error(event.message || 'Parent Worker failed'));
      port.onmessage = event => {
        events.push(event.data);
        if (route === 'nonblocking' && event.data.kind === 'before-new') port.postMessage('ping');
        if (route === 'nonblocking' && event.data.kind === 'pong') release('pong');
        if (event.data.kind === 'terminated') { release('terminated'); resolve(); }
        if (event.data.kind === 'child-error' || (events.some(e => e.kind === 'child') && events.some(e => e.kind === 'echo') && (route !== 'rewritten' || events.some(e => e.kind === 'probe')))) resolve();
      };
    });
    if (port.start) port.start();
    const directory = MODE === 'lifecycle' && route !== 'terminate' ? 'failure' : ['intercept', 'rewritten'].includes(route) ? 'child' : route === 'redirect' ? 'redirect' : 'slow';
    port.postMessage({url: '/' + directory + '/script.js?case=' + id + '&route=' + route, type});
    if (route === 'terminate') {
      while ((await (await fetch('/started?case=' + id)).text()) !== 'yes') await new Promise(resolve => setTimeout(resolve, 0));
      port.postMessage('terminate');
    }
    await completed;
    if (releasePromise) await releasePromise;
    rows.push({id, route, parentKind, type, events, releaseReason});
  }
  let clients = [], mainRequests = [];
  if (registration) {
    const channel = new MessageChannel();
    const listed = new Promise(resolve => channel.port1.onmessage = event => resolve(event.data));
    registration.active.postMessage('list', [channel.port2]);
    const result = await listed;
    clients = result.clients;
    mainRequests = result.requests;
  }
  // Give any incorrectly surviving child an actual networking turn after its
  // held HTTP response has been released.
  await fetch('/barrier');
  nestedResult = {rows, clients, mainRequests, windowControlled: !!navigator.serviceWorker.controller};
})().catch(error => nestedError = String(error));
