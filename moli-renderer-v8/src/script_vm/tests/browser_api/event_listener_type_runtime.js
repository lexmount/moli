(async () => {
  const rows = [];
  const source = `const probe = ${__probeEventListenerTypes.toString()};
    const channel = new MessageChannel();
    try {
      postMessage(probe([['WorkerGlobalScope', self], ['worker EventTarget', new EventTarget()],
        ['worker AbortSignal', new AbortController().signal], ['worker MessagePort', channel.port1]], Event));
    } finally { channel.port1.close(); channel.port2.close(); }`;
  const url = URL.createObjectURL(new Blob([source], {type:'text/javascript'}));
  const worker = new Worker(url);
  try {
    rows.push(...await new Promise((resolve, reject) => {
      const timer = setTimeout(() => reject(new Error('worker listener identity timeout')), 5000);
      worker.onmessage = event => { clearTimeout(timer); resolve(event.data); };
      worker.onerror = event => { clearTimeout(timer); event.preventDefault(); reject(new Error(event.message)); };
    }));
  } finally { worker.terminate(); URL.revokeObjectURL(url); }
  const name = 'event-listener-type-' + crypto.randomUUID();
  let database;
  try {
    await new Promise((resolve, reject) => {
      const request = indexedDB.open(name, 1);
      request.onupgradeneeded = () => {
        database = request.result;
        const transaction = request.transaction;
        const store = database.createObjectStore('store');
        rows.push(...__probeEventListenerTypes([
          ['IDBDatabase', database], ['IDBOpenDBRequest', request],
          ['IDBTransaction', transaction], ['IDBRequest', store.get('key')]], Event));
      };
      request.onsuccess = () => { database = request.result; resolve(); };
      request.onerror = () => reject(request.error);
    });
  } finally {
    database?.close();
    await new Promise((resolve, reject) => {
      const request = indexedDB.deleteDatabase(name);
      request.onsuccess = () => resolve();
      request.onerror = () => reject(request.error);
      request.onblocked = () => reject(new Error('listener identity database remained open'));
    });
  }
  const channel = new MessageChannel();
  try {
    for (const target of [new EventTarget(), channel.port1]) {
      const raw = '\ud800', replacement = '\ufffd', calls = [];
      const controller = new AbortController();
      target.when(raw).subscribe(() => calls.push('raw'), {signal:controller.signal});
      target.when(replacement).subscribe(() => calls.push('replacement'), {signal:controller.signal});
      target.dispatchEvent(new Event(raw));
      rows.push({name:'Observable exact event type', passed:calls.join(',') === 'raw'});
      controller.abort();
      target.dispatchEvent(new Event(raw));
      rows.push({name:'Observable abort removes exact type', passed:calls.join(',') === 'raw'});
    }
  } finally { channel.port1.close(); channel.port2.close(); }
  globalThis.__eventListenerRuntimeResults = {rows, passed:rows.filter(row => row.passed).length, total:rows.length};
  const combined = [...__eventListenerTypeResults.rows, ...rows];
  globalThis.__eventListenerTypeResults = {rows:combined, passed:combined.filter(row => row.passed).length, total:combined.length};
  return combined.every(row => row.passed) || JSON.stringify(combined.filter(row => !row.passed));
})()
