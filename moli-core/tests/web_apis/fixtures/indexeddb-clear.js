globalThis.clearChecks = [];
async function clearProbe(prefix = 'clear-probe') {
  const checks = clearChecks;
  const check = (label, pass, actual = '') => checks.push({label, pass, actual: String(actual)});
  const same = (label, actual, expected) => check(label, JSON.stringify(actual) === JSON.stringify(expected), JSON.stringify(actual));
  const complete = tx => new Promise(resolve => {
    tx.oncomplete = () => resolve('complete');
    tx.onabort = () => resolve('abort');
  });
  const result = r => new Promise((resolve, reject) => {
    r.onsuccess = () => resolve(r.result);
    r.onerror = () => reject(r.error);
  });
  const rejectClear = (label, method, receiver, name, Realm = globalThis) => {
    try {
      const request = method.call(receiver, { get unused() { throw new Error('unused argument'); } });
      check(label + ': synchronous name', false, 'returned a request');
      check(label + ': callee realm', false, 'did not throw');
      // Preserve the old implementation's asynchronous error as evidence,
      // while allowing the rest of the assertions to run.
      if (request) request.onerror = event => event.preventDefault();
    } catch (error) {
      check(label + ': synchronous name', error.name === name, error);
      const Constructor = name === 'TypeError' ? Realm.TypeError : Realm.DOMException;
      check(label + ': callee realm', error instanceof Constructor, error);
    }
  };
  const open = (factory, name) => new Promise((resolve, reject) => {
    const request = factory.open(name, 1);
    let deleted;
    request.onupgradeneeded = () => {
      const db = request.result;
      const store = db.createObjectStore('store', {autoIncrement: true});
      store.createIndex('tag', 'tag');
      store.put({tag: 'one'});
      store.put({tag: 'two'});
      deleted = db.createObjectStore('deleted');
      db.deleteObjectStore('deleted');
    };
    request.onsuccess = () => resolve({db: request.result, deleted});
    request.onerror = () => reject(request.error);
  });
  const shadow = (store, tx, mode) => {
    let calls = 0;
    for (const name of ['transaction', 'name', 'db']) Object.defineProperty(store, name, {
      configurable: true, get() { ++calls; throw new Error('author ' + name); }
    });
    Object.defineProperty(tx, 'mode', {value: mode, configurable: true});
    return () => calls;
  };
  const readonly = async (label, factory, method, Realm, queued) => {
    const {db, deleted} = await open(factory, prefix + '-' + label);
    const blocker = queued ? db.transaction('store', 'readwrite') : null;
    const blockerDone = blocker ? complete(blocker) : null;
    const tx = db.transaction('store');
    const done = complete(tx);
    const store = tx.objectStore('store');
    let errors = 0;
    tx.addEventListener('error', () => ++errors);
    const shadowCalls = shadow(store, tx, 'readwrite');
    rejectClear(label + ': active readonly', method, store, 'ReadOnlyError', Realm);
    rejectClear(label + ': repeated readonly', method, store, 'ReadOnlyError', Realm);
    rejectClear(label + ': deleted beats inactive', method, deleted, 'InvalidStateError', Realm);
    const revoked = Proxy.revocable(store, {}); revoked.revoke();
    for (const fake of [null, {}, Object.create(Object.getPrototypeOf(store)), Object.create(store), new Proxy(store, {}), revoked.proxy]) {
      rejectClear(label + ': invalid receiver', method, fake, 'TypeError', Realm);
    }
    if (blocker) { blocker.abort(); await blockerDone; }
    check(label + ': readonly still completes', await done === 'complete');
    check(label + ': no request error events', errors === 0, errors);
    check(label + ': no author property reads', shadowCalls() === 0, shadowCalls());
    rejectClear(label + ': finished beats readonly', method, store, 'TransactionInactiveError', Realm);
    const read = db.transaction('store');
    const readDone = complete(read);
    const readStore = read.objectStore('store');
    const count = result(readStore.count());
    const indexCount = result(readStore.index('tag').count());
    check(label + ': records preserved', await count === 2);
    check(label + ': index records preserved', await indexCount === 2);
    await readDone;
    db.close();
  };
  const writable = async (label, factory, method, Realm, RequestRealm, queued) => {
    const {db} = await open(factory, prefix + '-' + label);
    const blocker = queued ? db.transaction('store') : null;
    const blockerDone = blocker ? complete(blocker) : null;
    const tx = db.transaction('store', 'readwrite');
    const done = complete(tx);
    const store = tx.objectStore('store');
    const index = store.index('tag');
    const shadowCalls = shadow(store, tx, 'readonly');
    const order = [];
    const track = (name, request) => {
      request.addEventListener('success', () => order.push(name));
      return result(request);
    };
    const before = track('before', store.put({tag: 'before'}));
    const ignored = new Proxy({}, {get() { throw new Error('clear must ignore arguments'); }});
    const request = method.call(store, ignored, Symbol('unused'));
    check(label + ': request belongs to receiver', request instanceof RequestRealm.IDBRequest);
    check(label + ': native request source', request.source === store);
    check(label + ': native request transaction', request.transaction === tx);
    check(label + ': request initially pending', request.readyState === 'pending');
    const cleared = track('clear', request);
    const after = track('after', store.put({tag: 'after'}));
    const count = track('count', store.count());
    const keys = track('index', index.getAllKeys());
    if (blocker) { blocker.abort(); await blockerDone; }
    check(label + ': old generator value', await before === 3);
    check(label + ': clear result is undefined', await cleared === undefined);
    check(label + ': generator is preserved', await after === 4);
    check(label + ': one remaining record', await count === 1);
    same(label + ': index reflects ordered writes', await keys, [4]);
    check(label + ': write transaction completes', await done === 'complete');
    same(label + ': request event order', order, ['before', 'clear', 'after', 'count', 'index']);
    check(label + ': no author property reads', shadowCalls() === 0, shadowCalls());
    rejectClear(label + ': finished write', method, store, 'TransactionInactiveError', Realm);

    const committing = db.transaction('store', 'readwrite');
    const committingDone = complete(committing);
    const committingStore = committing.objectStore('store');
    committing.commit();
    rejectClear(label + ': committing write', method, committingStore, 'TransactionInactiveError', Realm);
    check(label + ': commit still completes', await committingDone === 'complete');
    const aborted = db.transaction('store', 'readwrite');
    const abortedDone = complete(aborted);
    const abortedStore = aborted.objectStore('store');
    aborted.abort();
    rejectClear(label + ': aborted write', method, abortedStore, 'TransactionInactiveError', Realm);
    check(label + ': abort completes', await abortedDone === 'abort');
    const verify = db.transaction('store');
    const verifyDone = complete(verify);
    const verifyStore = verify.objectStore('store');
    const records = result(verifyStore.getAll());
    const primaryKeys = result(verifyStore.getAllKeys());
    same(label + ': rejected clears preserve data', await records, [{tag: 'after'}]);
    same(label + ': rejected clears preserve keys', await primaryKeys, [4]);
    await verifyDone;
    db.close();
  };
  for (const queued of [false, true]) {
    await readonly('readonly-' + queued, indexedDB, IDBObjectStore.prototype.clear, globalThis, queued);
    await writable('writable-' + queued, indexedDB, IDBObjectStore.prototype.clear, globalThis, globalThis, queued);
  }
  if (typeof document !== 'undefined') {
    const frame = document.createElement('iframe');
    const loaded = new Promise(resolve => { frame.onload = resolve; });
    frame.srcdoc = '<!doctype html>'; document.documentElement.appendChild(frame); await loaded;
    const other = frame.contentWindow;
    for (const queued of [false, true]) {
      await readonly('foreign-method-' + queued, indexedDB, other.IDBObjectStore.prototype.clear, other, queued);
      await readonly('foreign-store-' + queued, other.indexedDB, IDBObjectStore.prototype.clear, globalThis, queued);
      await writable('foreign-write-method-' + queued, indexedDB, other.IDBObjectStore.prototype.clear, other, globalThis, queued);
      await writable('foreign-write-store-' + queued, other.indexedDB, IDBObjectStore.prototype.clear, globalThis, other, queued);
    }
    frame.remove();
  }
  return {state: checks.every(c => c.pass) ? 'pass' : 'fail', checks};
}
