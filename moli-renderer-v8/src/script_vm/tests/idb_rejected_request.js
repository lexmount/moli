globalThis.__idbRejectedRequestRun = (async () => {
  const checks = [];
  const check = (name, passed) => checks.push({name, passed: !!passed});
  const progress = globalThis.__idbRejectedRequest = {complete:false, checks};
  const request = value => new Promise((resolve, reject) => {
    value.onsuccess = () => resolve(value.result);
    value.onerror = () => reject(value.error);
  });
  const finished = tx => new Promise((resolve, reject) => {
    tx.oncomplete = resolve;
    tx.onabort = () => reject(tx.error);
  });
  try {
    for (const [i, realm] of [window, document.getElementById('child').contentWindow].entries()) {
      const name = `rejected-sync-write-${i}`, open = realm.indexedDB.open(name, 1);
      open.onupgradeneeded = () => {
        const store = open.result.createObjectStore('records');
        let error;
        try {store.put(new realm.Event('not-cloneable'), 'upgrade-rejected');} catch (e) {error=e;}
        check(`${i}: rejected upgrade write`, error instanceof realm.DOMException && error.name === 'DataCloneError');
      };
      const db = await request(open);
      try {
        const tx = db.transaction('records','readwrite'), store = tx.objectStore('records');
        let completions = 0, requestErrors = 0, getterReads = 0;
        tx.addEventListener('complete', () => completions++);
        tx.addEventListener('error', () => requestErrors++);
        const done = finished(tx);
        for (const method of ['put','add']) {
          for (const [j, value] of [function(){}, new realm.Event('not-cloneable'),
            {get payload(){getterReads++; throw new realm.Error('clone getter');}}].entries()) {
            let error;
            try {store[method](value, `${method}-rejected-${j}`);} catch (e) {error=e;}
            check(`${i}: ${method} rejection ${j}`, error instanceof realm.DOMException && error.name === 'DataCloneError');
          }
        }
        store.put('accepted', 'safe'); await done;
        check(`${i}: rejected writes dispatch no request errors`, completions === 1 && requestErrors === 0 && getterReads === 2);
        const read = db.transaction('records'), readStore = read.objectStore('records'), readDone = finished(read);
        let readOnlyConversions = 0, readErrors = 0;
        read.addEventListener('error', () => readErrors++);
        for (const method of ['put', 'add']) {
          let readOnlyError;
          try {readStore[method]({get payload(){readOnlyConversions++; return 'rejected';}}, 'readonly');} catch(e) {readOnlyError=e;}
          check(`${i}: rejected readonly ${method}`, readOnlyError instanceof realm.DOMException && readOnlyError.name === 'ReadOnlyError');
        }
        const keys = await request(readStore.getAllKeys());
        const value = await request(readStore.get('safe'));
        await readDone;
        check(`${i}: readonly rejection precedes cloning and dispatches no error`, readOnlyConversions === 0 && readErrors === 0);
        check(`${i}: accepted write persists and rejected writes do not`, JSON.stringify(keys) === '["safe"]' && value === 'accepted');
      } finally {db.close(); await request(realm.indexedDB.deleteDatabase(name));}
      check(`${i}: closed database deletes`, true);
    }
  } catch(error) {checks.push({name:'async completion', passed:false, error:String(error), stack:error.stack});}
  globalThis.__idbRejectedRequest = {complete:true, checks, total:checks.length};
})()
