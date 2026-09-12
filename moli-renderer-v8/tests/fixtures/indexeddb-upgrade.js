(async () => {
  const requestResult = request => new Promise((resolve, reject) => {
    request.onsuccess = () => resolve(request.result);
    request.onerror = () => reject(request.error);
  });
  const trace = [];
  const open = indexedDB.open('upgrade-request-drain', 1);
  open.onupgradeneeded = () => {
    trace.push(`upgrade:${open.readyState}`);
    const db = open.result;
    const tx = open.transaction;
    tx.oncomplete = () => {
      trace.push('complete');
      queueMicrotask(() => trace.push('complete-microtask'));
    };
    const store = db.createObjectStore('source', {keyPath:'id'});
    store.put({id:1, text:'original'}).onsuccess = () => trace.push('seed');
    store.getAll().onsuccess = event => {
      trace.push(`read:${event.target.result[0].text}`);
      const destination = db.createObjectStore('destination', {keyPath:'id'});
      destination.createIndex('id', 'id', {unique:true});
      destination.put(event.target.result[0]).onsuccess = () => trace.push('migration-write');
      db.deleteObjectStore('source');
      Promise.resolve().then(() => Promise.resolve()).then(() => {
        trace.push('request-microtask');
        destination.put({id:2, text:'microtask write'}).onsuccess = () => trace.push('microtask-write');
      });
    };
  };
  const db = await requestResult(open);
  trace.push('open-success');
  const transactionCleared = open.transaction === null;
  const records = await requestResult(db.transaction('destination').objectStore('destination').getAll());
  db.close();

  // No initial request: microtasks from upgradeneeded can still add schema
  // and requests before end-of-checkpoint deactivation starts auto-commit.
  const micro = indexedDB.open('upgrade-microtask-only', 1);
  micro.onupgradeneeded = () => {
    Promise.resolve().then(() => Promise.resolve()).then(() => {
      micro.result.createObjectStore('later').put('value', 'key');
    });
  };
  const microDb = await requestResult(micro);
  const microtaskValue = await requestResult(microDb.transaction('later').objectStore('later').get('key'));
  microDb.close();

  // Aborting from a request callback must prevent the open-success event,
  // close the provisional connection, and roll back its version and schema.
  const abortTrace = [];
  const aborted = indexedDB.open('upgrade-request-abort', 2);
  aborted.onupgradeneeded = () => {
    const tx = aborted.transaction;
    tx.onabort = () => abortTrace.push('abort');
    tx.oncomplete = () => abortTrace.push('unexpected-complete');
    aborted.result.createObjectStore('transient').put('discarded', 1).onsuccess = () => {
      abortTrace.push('request-success');
      tx.abort();
    };
  };
  try { (await requestResult(aborted)).close(); abortTrace.push('unexpected-open-success'); }
  catch (error) { abortTrace.push(`open-error:${error.name}`); }
  const reopened = indexedDB.open('upgrade-request-abort', 1);
  let rollback;
  reopened.onupgradeneeded = event => {
    rollback = {oldVersion:event.oldVersion, stores:Array.from(reopened.result.objectStoreNames)};
    reopened.result.createObjectStore('retained');
  };
  (await requestResult(reopened)).close();
  const closed = indexedDB.open('upgrade-close-before-success', 1);
  closed.onupgradeneeded = () => {
    const connection = closed.result;
    closed.transaction.oncomplete = () => queueMicrotask(() => connection.close());
  };
  let closedResult;
  try { (await requestResult(closed)).close(); closedResult = 'unexpected-success'; }
  catch (error) { closedResult = error.name; }
  return {trace, transactionCleared, records, microtaskValue, abortTrace, rollback, closedResult};
})()
