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

  const ids = [];
  for (const creator of ['dedicated', 'shared']) for (let i = 0; i < 8; i++) {
    const id = creator + '-' + i;
    ids.push(id);
    const url = '/controlled/' + creator + '.js?instance=' + i;
    const parent = creator === 'dedicated' ? new Worker(url) : new SharedWorker(url);
    abortParents.push(parent);
    const port = creator === 'shared' ? parent.port : parent;
    if (port.start) port.start();
    port.postMessage({kind: 'string', id});
  }
  async function states() { return await Promise.all(ids.map(async id => ({id, ...(await (await fetch('/state?case=' + id)).json())}))); }
  while (!(await states()).every(row => row.started)) await new Promise(resolve => setTimeout(resolve, 20));
  const seen = new Set();
  for (const target of ids) {
    await fetch('/release?case=' + target);
    let reports;
    do { reports = (await states()).filter(row => row.report && !seen.has(row.id)); if (!reports.length) await new Promise(resolve => setTimeout(resolve, 20)); } while (!reports.length);
    for (const row of reports) seen.add(row.id);
    const matched = reports.length === 1 && reports[0].id === target;
    abortRows.push({target, matched, reports});
    if (!matched) break;
  }
  globalThis.abortReady = true;
})().catch(error => abortError = String(error));
