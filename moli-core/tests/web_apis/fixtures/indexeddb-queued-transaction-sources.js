globalThis.foreignQueueChecks = [];
async function foreignQueueProbe(prefix) {
  const checks = foreignQueueChecks;
  const check = (label, pass, actual = '') => checks.push({label, pass: !!pass, actual: String(actual)});
  const equal = (label, actual, expected) => check(label, JSON.stringify(actual) === JSON.stringify(expected), JSON.stringify(actual));
  const request = r => new Promise((resolve, reject) => { r.onsuccess = () => resolve(r.result); r.onerror = event => { event.preventDefault(); reject(r.error); }; });
  const done = tx => new Promise(resolve => { tx.oncomplete = () => resolve('complete'); tx.onabort = () => resolve('abort'); });
  let frame, peer = globalThis;
  if (typeof document !== 'undefined') {
    frame = document.createElement('iframe'); document.documentElement.appendChild(frame); peer = frame.contentWindow;
  }
  let sequence = 0;
  try {
    for (const ownerRealm of frame ? [globalThis, peer] : [globalThis]) {
      const foreignRealm = ownerRealm === globalThis ? peer : globalThis;
      const name = prefix + '-' + sequence;
      const opening = ownerRealm.indexedDB.open(name, 1);
      opening.onupgradeneeded = () => opening.result.createObjectStore('s').createIndex('i', 'group');
      const db = await request(opening);
      for (const kind of ['store', 'index']) for (const method of ['getAll', 'getAllKeys']) for (const callerRealm of frame ? [globalThis, peer] : [globalThis]) {
        const id = ++sequence, label = [id, kind, method].join(' ');
        let blockerFinished = false;
        const blocker = db.transaction('s', 'readwrite');
        const blockerDone = new Promise(resolve => { blocker.oncomplete = () => { blockerFinished = true; resolve('complete'); }; blocker.onabort = () => resolve('abort'); });
        blocker.objectStore('s').put({group: [id], stamp: label}, [id]);
        const waiting = db.transaction('s', 'readwrite'), waitingDone = done(waiting);
        const store = foreignRealm.IDBTransaction.prototype.objectStore.call(waiting, 's');
        const source = kind === 'store' ? store : foreignRealm.IDBObjectStore.prototype.index.call(store, 'i');
        check(label + ' borrowed lookup keeps owner realm and identity',
          Object.getPrototypeOf(source) === ownerRealm[kind === 'store' ? 'IDBObjectStore' : 'IDBIndex'].prototype &&
          source === (kind === 'store' ? waiting.objectStore('s') : waiting.objectStore('s').index('i')));
        const reads = [], query = [id];
        Object.defineProperty(query, 0, {get() { reads.push('query'); return id; }, configurable: true});
        const r = callerRealm[kind === 'store' ? 'IDBObjectStore' : 'IDBIndex'].prototype[method].call(source, query, 1);
        check(label + ' queued request source identity', r.source === source);
        check(label + ' request transaction identity', r.transaction === waiting);
        check(label + ' request relevant realm', Object.getPrototypeOf(r) === ownerRealm.IDBRequest.prototype);
        equal(label + ' key converted once before queueing', reads, ['query']);
        Object.defineProperty(query, 0, {value: -1});
        let eventOwner = false;
        const result = await new Promise((resolve, reject) => {
          r.onsuccess = event => {
            check(label + ' starts after blocker completes', blockerFinished);
            check(label + ' event request identity', event.target === r && event.currentTarget === r);
            eventOwner = event instanceof ownerRealm.Event;
            resolve(r.result);
          };
          r.onerror = event => { event.preventDefault(); reject(r.error); };
        });
        check(label + ' event relevant realm', eventOwner);
        check(label + ' result array relevant realm', Object.getPrototypeOf(result) === ownerRealm.Array.prototype);
        equal(label + ' key is not reread by queued execution', reads, ['query']);
        equal(label + ' result count', result.length, 1);
        if (method === 'getAllKeys') equal(label + ' sees preceding committed key and frozen query', result, [[id]]);
        else {
          const row = result[0];
          equal(label + ' sees preceding committed value', row.stamp, label);
          check(label + ' value relevant realm', Object.getPrototypeOf(row) === ownerRealm.Object.prototype);
        }
        equal(label + ' blocker commit', await blockerDone, 'complete');
        equal(label + ' waiting commit', await waitingDone, 'complete');
      }
      const blocker = db.transaction('s', 'readwrite'), blockerDone = done(blocker);
      blocker.objectStore('s').put({group: [999]}, [999]);
      const canceled = db.transaction('s', 'readwrite'), canceledDone = done(canceled);
      const foreign = foreignRealm.IDBTransaction.prototype.objectStore.call(canceled, 's');
      const r = foreign.getAll();
      const canceledRequest = new Promise(resolve => {
        r.onsuccess = () => resolve('unexpected-success');
        r.onerror = event => {
          event.preventDefault();
          check('canceled foreign source identity', r.source === foreign);
          check('canceled request event owner', event instanceof ownerRealm.Event);
          check('canceled request error realm', r.error instanceof ownerRealm.DOMException);
          resolve(r.error.name);
        };
      });
      canceled.abort();
      equal('queued cancellation preserves abort semantics', await canceledRequest, 'AbortError');
      equal('queued transaction abort', await canceledDone, 'abort');
      equal('blocker survives queued cancellation', await blockerDone, 'complete');
      db.close(); await request(ownerRealm.indexedDB.deleteDatabase(name));
    }
    equal('all queued variants executed', sequence, frame ? 16 : 4);
  } finally { if (frame) frame.remove(); }
  return {state: checks.every(check => check.pass) ? 'pass' : 'fail', checks};
}
