globalThis.schemaRenameChecks = [];
async function schemaRenameProbe(name, scenario) {
  const checks = schemaRenameChecks;
  const check = (label, pass, actual = '') => checks.push({label, pass: !!pass, actual: String(actual)});
  const equal = (label, actual, expected) => check(label, JSON.stringify(actual) === JSON.stringify(expected), JSON.stringify(actual));
  const errorName = operation => { try { operation(); return 'accepted'; } catch (error) { return error.name; } };
  const request = r => new Promise((resolve, reject) => {
    r.onsuccess = () => resolve(r.result);
    r.onerror = event => { event.preventDefault(); reject(r.error); };
  });
  const open = (databaseName, version, upgrade) => {
    const r = indexedDB.open(databaseName, version);
    r.onupgradeneeded = () => {
      try {
        const pending = upgrade(r.result, r.transaction);
        if (pending) pending.catch(error => {
          check('upgrade callback completed', false, error.stack || error);
          try { r.transaction.abort(); } catch (_) {}
        });
      } catch (error) {
        check('upgrade callback completed', false, error.stack || error);
        r.transaction.abort();
      }
    };
    return request(r);
  };
  const remove = databaseName => request(indexedDB.deleteDatabase(databaseName));

  if (scenario === 'validation') {
    let retainedStore, retainedIndex, deletedStore, deletedIndex;
    const db = await open(name, 1, (db, tx) => {
      const store = retainedStore = db.createObjectStore('s');
      const index = retainedIndex = store.createIndex('i', 'tag');
      db.createObjectStore('occupied');
      store.createIndex('occupied', 'other');
      for (const [prototype, object] of [[IDBObjectStore.prototype, store], [IDBIndex.prototype, index]]) {
        const descriptor = Object.getOwnPropertyDescriptor(prototype, 'name');
        check('name is a native prototype accessor', typeof descriptor?.get === 'function' && typeof descriptor?.set === 'function');
        check('name is enumerable and configurable', descriptor?.enumerable && descriptor?.configurable);
        check('name is not an own data property', !Object.hasOwn(object, 'name'));
        if (!descriptor?.set) continue;
        equal('setter arity', descriptor.set.length, 1);
        equal('invalid getter receiver', errorName(() => descriptor.get.call({})), 'TypeError');
        let converted = 0;
        const poison = {toString() { converted++; throw new Error('conversion'); }};
        equal('invalid setter receiver', errorName(() => descriptor.set.call({}, poison)), 'TypeError');
        equal('receiver validation precedes conversion', converted, 0);
        equal('valid setter converts before mutation', errorName(() => descriptor.set.call(object, poison)), 'Error');
        equal('conversion occurs once', converted, 1);
        equal('Symbol is rejected', errorName(() => descriptor.set.call(object, Symbol())), 'TypeError');
      }
      equal('store duplicate rejected', errorName(() => { store.name = 'occupied'; }), 'ConstraintError');
      equal('index duplicate rejected', errorName(() => { index.name = 'occupied'; }), 'ConstraintError');
      equal('store unchanged after duplicate', store.name, 's');
      equal('index unchanged after duplicate', index.name, 'i');
      store.name = store.name; index.name = index.name;
      store.name = 'renamed\0\ud800'; index.name = 'by\0\udc00';
      equal('store preserves UTF16 name', store.name, 'renamed\0\ud800');
      equal('index preserves UTF16 name', index.name, 'by\0\udc00');
      equal('database names update synchronously', Array.from(db.objectStoreNames), ['occupied', 'renamed\0\ud800']);
      equal('transaction names update synchronously', Array.from(tx.objectStoreNames), ['occupied', 'renamed\0\ud800']);
      equal('index names update synchronously', Array.from(store.indexNames), ['by\0\udc00', 'occupied']);
      equal('new store lookup', tx.objectStore(store.name).name, store.name);
      equal('old store lookup rejected', errorName(() => tx.objectStore('s')), 'NotFoundError');
      equal('new index lookup', store.index(index.name).name, index.name);
      equal('old index lookup rejected', errorName(() => store.index('i')), 'NotFoundError');
      store.name = ''; index.name = '';
      equal('empty store name', tx.objectStore('').name, '');
      equal('empty index name', store.index('').name, '');
      store.name = null; index.name = undefined;
      equal('null DOMString conversion', store.name, 'null');
      equal('undefined DOMString conversion', index.name, 'undefined');
      store.name = 's'; index.name = 'i';
      deletedStore = db.createObjectStore('gone');
      deletedIndex = store.createIndex('gone', 'tag');
      store.deleteIndex('gone'); db.deleteObjectStore('gone');
      equal('deleted store rejects same-name assignment', errorName(() => { deletedStore.name = deletedStore.name; }), 'InvalidStateError');
      equal('deleted index rejects same-name assignment', errorName(() => { deletedIndex.name = deletedIndex.name; }), 'InvalidStateError');
      let probing = true;
      const keepAlive = () => {
        if (probing) store.count().onsuccess = keepAlive;
      };
      keepAlive();
      setTimeout(() => {
        try {
          equal('live inactive store rejects rename', errorName(() => { store.name = 'late'; }), 'TransactionInactiveError');
          equal('live inactive index rejects rename', errorName(() => { index.name = 'late'; }), 'TransactionInactiveError');
          equal('live deleted store precedes inactive', errorName(() => { deletedStore.name = 'late'; }), 'InvalidStateError');
          equal('live inactive index precedes deletion', errorName(() => { deletedIndex.name = 'late'; }), 'TransactionInactiveError');
          equal('inactive store name unchanged', store.name, 's');
          equal('inactive index name unchanged', index.name, 'i');
        } finally { probing = false; }
      }, 0);
    });
    equal('finished upgrade store rejects even no-op', errorName(() => { retainedStore.name = retainedStore.name; }), 'TransactionInactiveError');
    equal('finished upgrade index rejects even no-op', errorName(() => { retainedIndex.name = retainedIndex.name; }), 'TransactionInactiveError');
    equal('deleted store precedes inactive check', errorName(() => { deletedStore.name = 'x'; }), 'InvalidStateError');
    equal('inactive index precedes deleted check', errorName(() => { deletedIndex.name = 'x'; }), 'TransactionInactiveError');
    for (const mode of ['readonly', 'readwrite']) {
      const store = db.transaction('s', mode).objectStore('s'), index = store.index('i');
      equal(mode + ' store rename rejected', errorName(() => { store.name = 's'; }), 'InvalidStateError');
      equal(mode + ' index rename rejected', errorName(() => { index.name = 'i'; }), 'InvalidStateError');
    }
    db.close(); await remove(name);
  } else if (scenario.startsWith('cursor-')) {
    const kind = scenario.slice('cursor-'.length);
    for (const direction of ['next', 'prev']) for (const keyOnly of [false, true]) {
      const databaseName = name + '-' + direction + '-' + keyOnly;
      const label = kind + '/' + direction + '/' + keyOnly;
      const seed = await open(databaseName, 1, db => {
        const store = db.createObjectStore('s', {autoIncrement: true});
        store.createIndex('i', 'tag');
        for (let key = 1; key <= 3; key++) store.put({tag: 'k' + key, other: 'wrong' + key, marker: 'original-' + key}, key);
      });
      seed.close();
      const db = await open(databaseName, 2, async (db, tx) => {
        const store = tx.objectStore('s'), index = store.index('i');
        const source = kind === 'store' ? store : index;
        const admittedStoreRead = request(store.get(1));
        const admittedIndexRead = request(index.get('k1'));
        const admittedWrite = request(store.put({tag:'k3', other:'wrong3', marker:'accepted-before'}, 3));
        const range = kind === 'store' ? IDBKeyRange.bound(1, 3) : IDBKeyRange.bound('k1', 'k3');
        const r = source[keyOnly ? 'openKeyCursor' : 'openCursor'](range, direction);
        const firstResult = request(r);
        store.name = 'kept'; index.name = 'kept-index';
        const replacement = db.createObjectStore('s', {autoIncrement: true});
        replacement.createIndex('i', 'tag');
        replacement.put({tag: 'k2', marker: 'replacement'}, 2);
        store.createIndex('i', 'other');
        const cursor = await firstResult;
        check(label + ' accepted open retains source', cursor?.source === source);
        equal(label + ' admitted store read retains identity', (await admittedStoreRead).marker, 'original-1');
        equal(label + ' admitted index read retains identity', (await admittedIndexRead).marker, 'original-1');
        equal(label + ' admitted write retains identity', await admittedWrite, 3);
        equal(label + ' first key survives old-name reuse', cursor?.primaryKey, direction === 'next' ? 1 : 3);
        equal(label + ' source name is live', source.name, kind === 'store' ? 'kept' : 'kept-index');
        const next = request(r); cursor.continue();
        // The iteration is already admitted before these further renames.
        store.name = 'final'; index.name = 'final-index';
        const secondReplacement = db.createObjectStore('kept');
        secondReplacement.put({marker: 'replacement-again'}, 2);
        store.createIndex('kept-index', 'other');
        const middle = await next;
        check(label + ' cursor identity survives rename', middle === cursor);
        equal(label + ' admitted iteration sees original second row', middle?.primaryKey, 2);
        if (!keyOnly) {
          const value = middle.value;
          equal(label + ' middle value is original', value.marker, 'original-2');
          await request(middle.update({tag: 'updated', marker: 'updated'}));
          check(label + ' update retains pending value', middle.value === value);
          await request(middle.delete());
        }
        const last = request(r); middle.continue();
        const finalCursor = await last;
        check(label + ' later iteration retains cursor identity', finalCursor === cursor);
        equal(label + ' later iteration uses current source', finalCursor?.primaryKey, direction === 'next' ? 3 : 1);
        const end = request(r); finalCursor.continue();
        equal(label + ' iteration ends', await end, null);
        equal(label + ' original generator survives rename', await request(store.put({tag: 'fresh', marker: 'fresh'})), 4);
      });
      equal(label + ' persisted final name', Array.from(db.objectStoreNames), ['final', 'kept', 's']);
      equal(label + ' old-name store remains separate', (await request(db.transaction('s').objectStore('s').get(2))).marker, 'replacement');
      equal(label + ' repeatedly reused store remains separate', (await request(db.transaction('kept').objectStore('kept').get(2))).marker, 'replacement-again');
      equal(label + ' original second row', await request(db.transaction('final').objectStore('final').get(2)), keyOnly ? {tag:'k2', other:'wrong2', marker:'original-2'} : undefined);
      equal(label + ' renamed index remains writable', (await request(db.transaction('final').objectStore('final').index('final-index').get('fresh'))).marker, 'fresh');
      equal(label + ' admitted write persists on original store', (await request(db.transaction('final').objectStore('final').get(3))).marker, 'accepted-before');
      db.close(); await remove(databaseName);
    }
  } else if (scenario.startsWith('rollback-')) {
    let frame, peer = globalThis;
    if (typeof document !== 'undefined') {
      frame = document.createElement('iframe'); document.documentElement.appendChild(frame); peer = frame.contentWindow;
    }
    try {
      const seed = await open(name, 1, db => {
        const store = db.createObjectStore('s', {autoIncrement:true});
        store.createIndex('i', 'tag');
        store.put({tag:'duplicate', marker:'original-1'}, 1); store.put({tag:'duplicate', marker:'original-2'}, 2);
      });
      seed.close();
      let db, tx, stores, indexes, createdStore, createdIndex, replacement, replacementIndex, discarded;
      const opening = open(name, 2, (database, transaction) => {
        db = database; tx = transaction;
        const store = tx.objectStore('s'), index = store.index('i');
        stores = [store, peer.IDBTransaction.prototype.objectStore.call(tx, 's')];
        indexes = [index, peer.IDBObjectStore.prototype.index.call(stores[1], 'i')];
        if (scenario === 'rollback-constraint') store.createIndex('conflict', 'tag', {unique:true});
        discarded = store.createIndex('temporary', 'tag'); store.deleteIndex('temporary');
        const setStoreName = Object.getOwnPropertyDescriptor(peer.IDBObjectStore.prototype, 'name').set;
        const setIndexName = Object.getOwnPropertyDescriptor(peer.IDBIndex.prototype, 'name').set;
        setIndexName.call(index, 'temporary');
        indexes.push(stores[1].index('temporary'));
        setStoreName.call(store, 'renamed'); setIndexName.call(index, 'renamed-index');
        stores.push(peer.IDBTransaction.prototype.objectStore.call(tx, 'renamed'));
        indexes.push(peer.IDBObjectStore.prototype.index.call(stores[2], 'renamed-index'));
        for (const alias of stores) equal('all store aliases rename immediately', alias.name, 'renamed');
        for (const alias of indexes) equal('all index aliases rename immediately', alias.name, 'renamed-index');
        createdIndex = store.createIndex('new', 'tag'); createdIndex.name = 'newer';
        createdStore = db.createObjectStore('fresh'); createdStore.name = 'fresher';
        store.deleteIndex('renamed-index'); db.deleteObjectStore('renamed');
        replacement = db.createObjectStore('s'); replacementIndex = replacement.createIndex('i', 'wrong');
        replacement.put({wrong:'replacement'}, 1);
        if (scenario === 'rollback-explicit') tx.abort();
      });
      try { await opening; check('upgrade abort rejects open', false); }
      catch (error) { equal('upgrade abort rejects open', error.name, 'AbortError'); }
      equal('transaction abort reason', tx.error?.name || null, scenario === 'rollback-explicit' ? null : 'ConstraintError');
      equal('abort restores version', db.version, 1);
      equal('abort restores database names', Array.from(db.objectStoreNames), ['s']);
      equal('abort restores transaction names', Array.from(tx.objectStoreNames), ['s']);
      for (const alias of stores) {
        equal('abort restores original store identity', alias.name, 's');
        equal('abort restores original index set', Array.from(alias.indexNames), ['i']);
      }
      for (const alias of indexes) equal('abort restores original index identity', alias.name, 'i');
      equal('new store retains its last name', createdStore.name, 'fresher');
      equal('new index retains its last name', createdIndex.name, 'newer');
      for (const alias of [createdStore, replacement]) equal('created store remains deleted', errorName(() => alias.get(1)), 'InvalidStateError');
      for (const alias of [createdIndex, replacementIndex, discarded]) equal('created index remains deleted', errorName(() => alias.get(1)), 'InvalidStateError');
      const reopened = await open(name, 1, () => { check('reopen needs no upgrade', false); });
      equal('SQL rollback restores old records', (await request(reopened.transaction('s').objectStore('s').get(1))).marker, 'original-1');
      equal('SQL rollback restores old index', await request(reopened.transaction('s').objectStore('s').index('i').count('duplicate')), 2);
      equal('SQL rollback restores generator', await request(reopened.transaction('s','readwrite').objectStore('s').put({tag:'next'})), 3);
      reopened.close(); await remove(name);
    } finally { if (frame) frame.remove(); }
  } else {
    throw new Error('unknown rename scenario: ' + scenario);
  }
  return {state: checks.every(item => item.pass) ? 'pass' : 'fail', checks};
}
