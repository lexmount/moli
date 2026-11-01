(async () => {
  const urls = [], names = [];
  const timeout = (promise, label) => new Promise((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error(label)), 8000);
    promise.then(value => { clearTimeout(timer); resolve(value); }, error => { clearTimeout(timer); reject(error); });
  });
  const makeURL = source => { const url = URL.createObjectURL(new Blob([source], {type: 'text/javascript'})); urls.push(url); return url; };
  const makeTree = (depth, name, scriptType = 'classic') => {
    names.push(name);
    let url = makeURL(`const request = indexedDB.open(${JSON.stringify(name)}, 1);
      request.onupgradeneeded = () => request.result.createObjectStore('items');
      request.onsuccess = () => { globalThis.database = request.result; postMessage({kind:'held'}); };
      request.onerror = () => postMessage({kind:'error', message:request.error.name});`);
    for (let i = 0; i < depth; i++) url = makeURL(`const child = new Worker(${JSON.stringify(url)}, {type:${JSON.stringify(scriptType)}});
      child.onmessage = event => postMessage(event.data);
      child.onerror = event => postMessage({kind:'error', message:event.message});
      onmessage = event => { if (event.data === 'close') close(); if (event.data === 'terminate-child') { child.terminate(); postMessage({kind:'terminated'}); } };`);
    return new Worker(url, {type:scriptType});
  };
  const message = (worker, kind) => timeout(new Promise((resolve, reject) => {
    const cleanup = () => { worker.removeEventListener('message', onmessage); worker.removeEventListener('error', onerror); };
    const onmessage = event => {
      if (event.data.kind === 'error') { cleanup(); reject(new Error(event.data.message)); }
      else if (event.data.kind === kind) { cleanup(); resolve(); }
    };
    const onerror = event => { cleanup(); reject(new Error(event.message || 'worker failed')); };
    worker.addEventListener('message', onmessage); worker.addEventListener('error', onerror);
  }), 'missing worker ' + kind);
  const upgrade = name => {
    let blocked, opened, failed;
    const probe = {opened:false, wasBlocked:false};
    probe.blocked = new Promise(resolve => { blocked = resolve; });
    probe.complete = new Promise((resolve, reject) => { opened = resolve; failed = reject; });
    const request = indexedDB.open(name, 2);
    request.onblocked = () => { probe.wasBlocked = true; blocked(); };
    request.onsuccess = () => { probe.opened = true; request.result.close(); opened(); };
    request.onerror = () => failed(request.error);
    return probe;
  };
  const control = makeTree(0, 'retirement-unrelated-live-worker');
  await message(control, 'held');
  const controlProbe = upgrade('retirement-unrelated-live-worker');
  await timeout(controlProbe.blocked, 'control connection not held');
  const cases = [], active = new Set([control]);
  try {
    for (const scriptType of ['classic', 'module']) {
      for (const [depth, mode] of [[0,'terminate'], [1,'terminate'], [2,'terminate'], [1,'close'], [1,'terminate-child']]) {
        const name = `retirement-owned-worker-${scriptType}-${depth}-${mode}`;
        const worker = makeTree(depth, name, scriptType); active.add(worker);
        await message(worker, 'held');
        const probe = upgrade(name); await timeout(probe.blocked, 'worker connection not held: ' + name);
        if (mode === 'terminate') worker.terminate();
        else if (mode === 'close') worker.postMessage('close');
        else { const done = message(worker, 'terminated'); worker.postMessage('terminate-child'); await done; }
        await timeout(probe.complete, 'worker connection not released: ' + name);
        await new Promise(resolve => setTimeout(resolve, 0));
        cases.push({scriptType, depth, mode, heldBefore:probe.wasBlocked, released:probe.opened, controlRetained:!controlProbe.opened});
        worker.terminate(); active.delete(worker);
      }
    }
  } finally {
    for (const worker of active) worker.terminate();
    for (const url of urls) URL.revokeObjectURL(url);
  }
  await timeout(controlProbe.complete, 'control connection not released');
  for (const name of names) await timeout(new Promise((resolve, reject) => {
    const request = indexedDB.deleteDatabase(name);
    request.onsuccess = resolve; request.onerror = () => reject(request.error);
  }), 'database cleanup blocked: ' + name);
  const remainingNames = (await indexedDB.databases()).map(database => database.name).filter(name => names.includes(name)).sort();
  return {cases, controlReleased:controlProbe.opened, remainingNames};
})()
