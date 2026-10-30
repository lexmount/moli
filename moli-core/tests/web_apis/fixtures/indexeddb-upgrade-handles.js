globalThis.upgradeHandleChecks = [];
async function upgradeHandleProbe(name = 'upgrade-handles-' + Math.random()) {
  const checks = upgradeHandleChecks;
  const check = (label, pass, actual = '') => checks.push({label, pass: !!pass, actual: String(actual)});
  const equal = (label, actual, expected) => check(label, JSON.stringify(actual) === JSON.stringify(expected), JSON.stringify(actual));
  const errorName = operation => { try { operation(); return 'accepted'; } catch (error) { return error.name; } };
  const request = r => new Promise((resolve, reject) => { r.onsuccess = () => resolve(r.result); r.onerror = e => { e.preventDefault(); reject(r.error); }; });
  let frame, peer = globalThis;
  if (typeof document !== 'undefined') {
    frame = document.createElement('iframe'); document.documentElement.appendChild(frame);
    peer = frame.contentWindow;
  }
  try {
    const seed = indexedDB.open(name, 1);
    seed.onupgradeneeded = () => {
      const store = seed.result.createObjectStore('s');
      store.createIndex('old', 'value'); store.put({value: 'original'}, 1);
    };
    const initial = await request(seed); initial.close();
    const opening = indexedDB.open(name, 2);
    let db, tx, store, alias, replacement, replacementIndex, newIndex, indexes;
    opening.onupgradeneeded = () => {
      db = opening.result; tx = opening.transaction;
      store = tx.objectStore('s');
      alias = peer.IDBTransaction.prototype.objectStore.call(tx, 's');
      if (frame) {
        check('borrowed method preserves wrapper creation realm', Object.getPrototypeOf(alias) === peer.IDBObjectStore.prototype);
        check('foreign wrapper stays distinct from local wrapper', alias !== store);
        check('foreign wrapper identity is cached in its creation realm', peer.IDBTransaction.prototype.objectStore.call(tx, 's') === alias);
      }
      for (const source of [store, alias]) equal('initial index names', Array.from(source.indexNames), ['old']);
      indexes = [store.index('old'), alias.index('old'),
        peer.IDBObjectStore.prototype.index.call(store, 'old'),
        IDBObjectStore.prototype.index.call(alias, 'old')];
      for (const index of indexes) {
        try { check('cross-table index remains usable', index.get(1).source === index); }
        catch (error) { check('cross-table index remains usable', false, error.name); }
      }
      alias.createIndex('transient', 'value');
      for (const source of [store, alias]) {
        equal('createIndex updates all aliases immediately', Array.from(source.indexNames), ['old', 'transient']);
        check('new index is immediately found', source.index('transient').name === 'transient');
      }
      check('local metadata keeps receiver realm', Object.getPrototypeOf(store.indexNames) === DOMStringList.prototype);
      check('foreign metadata keeps receiver realm', Object.getPrototypeOf(alias.indexNames) === peer.DOMStringList.prototype);
      store.deleteIndex('old');
      for (const index of indexes) equal('deleteIndex invalidates all index realms', errorName(() => index.get(1)), 'InvalidStateError');
      for (const source of [store, alias]) equal('deleted index lookup', errorName(() => source.index('old')), 'NotFoundError');
      newIndex = alias.createIndex('old', 'other');
      for (const source of [store, alias]) equal('recreated index updates aliases', Array.from(source.indexNames), ['old', 'transient']);
      db.deleteObjectStore('s');
      for (const source of [store, alias]) {
        equal('deleted store clears index names', Array.from(source.indexNames), []);
        equal('deleted store stays deleted', errorName(() => source.get(1)), 'InvalidStateError');
      }
      for (const index of indexes) equal('deleted store invalidates foreign indexes', errorName(() => index.get(1)), 'InvalidStateError');
      replacement = db.createObjectStore('s');
      replacementIndex = peer.IDBObjectStore.prototype.createIndex.call(replacement, 'replacement', 'value');
      equal('replacement has independent indexes', Array.from(replacement.indexNames), ['replacement']);
      tx.onabort = () => {
        check('abort request retains transaction', opening.transaction === tx);
        equal('abort connection already closed', errorName(() => db.transaction('s')), 'InvalidStateError');
      };
      tx.abort();
      equal('version restored synchronously', db.version, 1);
      for (const source of [store, alias]) equal('rollback restores every alias', Array.from(source.indexNames), ['old']);
      for (const index of indexes) equal('old indexes restored but transaction inactive', errorName(() => index.get(1)), 'TransactionInactiveError');
      equal('new index not revived', errorName(() => newIndex.get(1)), 'InvalidStateError');
      equal('replacement store not revived', errorName(() => replacement.get(1)), 'InvalidStateError');
      equal('replacement foreign index not revived', errorName(() => replacementIndex.get(1)), 'InvalidStateError');
    };
    try { await request(opening); check('upgrade abort rejects open', false); }
    catch (error) { equal('upgrade abort rejects open', error.name, 'AbortError'); }
    const reopened = await request(indexedDB.open(name));
    equal('reopen version unchanged', reopened.version, 1);
    equal('reopen data unchanged', await request(reopened.transaction('s').objectStore('s').get(1)), {value:'original'});
    const verifyTx = reopened.transaction('s');
    const localStore = verifyTx.objectStore('s');
    const foreignStore = peer.IDBTransaction.prototype.objectStore.call(verifyTx, 's');
    const reads = [peer.IDBObjectStore.prototype.index.call(localStore, 'old'),
      IDBObjectStore.prototype.index.call(foreignStore, 'old')].map(index => request(index.get('original')));
    for (const value of await Promise.all(reads)) equal('foreign index request reads restored data', value, {value:'original'});
    reopened.close(); await request(indexedDB.deleteDatabase(name));
    return {state: checks.every(c => c.pass) ? 'pass' : 'fail', checks};
  } finally { if (frame) frame.remove(); }
}
