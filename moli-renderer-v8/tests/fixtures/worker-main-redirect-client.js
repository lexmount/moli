globalThis.redirectResult = null;
globalThis.redirectError = null;
globalThis.redirectParents = [];
(async () => {
  const registrations = [];
  for (const scope of ['/child/', '/other/']) {
    const registration = await navigator.serviceWorker.register('/sw.js', {scope});
    const worker = registration.installing || registration.waiting || registration.active;
    if (worker.state !== 'activated') await new Promise(resolve => worker.addEventListener('statechange', () => {
      if (worker.state === 'activated') resolve();
    }));
    registrations.push(registration);
  }
  const rows = [];
  const routes = MODE === 'network' ? ['direct', 'one', 'two', 'policy', 'body'] : ['exit', 'scope'];
  for (const route of routes) for (const parentKind of ['window-dedicated', 'window-shared', 'dedicated', 'shared']) for (const type of ['classic', 'module']) {
    const id = route + '-' + parentKind + '-' + type;
    const shared = parentKind === 'window-shared';
    const path = route === 'direct' ? '/child/child.js' : ['exit', 'scope'].includes(route) ? '/child/redirect.js' : '/start/' + route + '.js';
    const target = path + '?case=' + id + '&route=' + route + '&shared=' + (shared ? '1' : '0');
    const options = {type, credentials: type === 'module' ? 'include' : 'omit'};
    const windowCreator = parentKind.startsWith('window-');
    const parent = windowCreator ? (shared ? new SharedWorker(target, options) : new Worker(target, options)) : (parentKind === 'shared' ? new SharedWorker('/outer/shared.js?case=' + id) : new Worker('/outer/dedicated.js?case=' + id));
    redirectParents.push(parent);
    const port = shared || parentKind === 'shared' ? parent.port : parent;
    const events = [];
    const completed = new Promise((resolve, reject) => {
      parent.onerror = event => reject(new Error(event.message || 'Worker failed'));
      port.onmessage = event => {
        events.push(event.data);
        if (event.data.kind === 'child-error') reject(new Error('Nested Worker failed: ' + id));
        if (events.some(event => event.kind === 'child') && events.some(event => event.kind === 'echo')) resolve();
      };
    });
    if (port.start) port.start();
    port.postMessage(windowCreator ? 'queued-before-load' : {url: target, type});
    await completed;
    rows.push({id, route, parentKind, type, events});
  }
  const lists = [];
  for (const registration of registrations) {
    const channel = new MessageChannel();
    const listed = new Promise(resolve => channel.port1.onmessage = event => resolve(event.data));
    registration.active.postMessage('list', [channel.port2]);
    lists.push(await listed);
  }
  redirectResult = {rows, clients: lists[0].clients, requests: lists.flatMap(list => list.requests), windowControlled: !!navigator.serviceWorker.controller};
})().catch(error => redirectError = String(error));
