globalThis.storeRenameChecks = [];
async function storeRenameProbe(name) {
  'use strict';
  name ??= 'store-rename-' + Math.random();
  const checks = globalThis.storeRenameChecks;
  const check = (label, pass, actual = '') => checks.push({label, pass: !!pass, actual: String(actual)});
  const equal = (label, actual, expected) => check(label, JSON.stringify(actual) === JSON.stringify(expected), JSON.stringify(actual));
  const caught = fn => { try { fn(); return null; } catch (error) { return error; } };
  const throws = (label, kind, fn) => { const error = caught(fn); equal(label, error?.name, kind); return error; };
  const request = r => new Promise((resolve, reject) => {
    r.onsuccess = () => resolve(r.result);
    r.onerror = event => { event.preventDefault(); reject(r.error); };
  });
  const done = tx => new Promise(resolve => {
    tx.oncomplete = () => resolve('complete'); tx.onabort = () => resolve('abort');
  });
  const open = (version, setup) => new Promise((resolve, reject) => {
    const r = indexedDB.open(name, version); let setupError;
    r.onupgradeneeded = () => {
      try { setup(r.result, r.transaction); }
      catch (error) { setupError = error; r.transaction.abort(); }
    };
    r.onsuccess = () => resolve(r.result);
    r.onerror = event => { event.preventDefault(); reject(setupError || r.error); };
  });
  const descriptor = Object.getOwnPropertyDescriptor(IDBObjectStore.prototype, 'name');
  check('name is a prototype accessor', descriptor?.enumerable && descriptor?.configurable && typeof descriptor.get === 'function' && typeof descriptor.set === 'function');
  const dbList = Object.getOwnPropertyDescriptor(IDBDatabase.prototype, 'objectStoreNames');
  const txList = Object.getOwnPropertyDescriptor(IDBTransaction.prototype, 'objectStoreNames');
  check('database names are a readonly prototype accessor', typeof dbList?.get === 'function' && dbList.set === undefined);
  check('transaction names are a readonly prototype accessor', typeof txList?.get === 'function' && txList.set === undefined);
  const get = descriptor?.get || function() { return this.name; };
  const set = descriptor?.set || function(value) { this.name = value; };
  const names = ['', '\0', '\uD800', '\uD801', '\uDC00', '\uDC01', '\uFFFD', '\uDC00\uD800', '\u{1F600}', '\uE000', '__proto__'];
  let frame, foreign, db, saved, initialReads;
  if (typeof document !== 'undefined') {
    if (document.readyState === 'loading') await new Promise(resolve => document.addEventListener('DOMContentLoaded', resolve, {once: true}));
    frame = document.createElement('iframe'); document.body.appendChild(frame); foreign = frame.contentWindow;
  }
  try {
    db = await open(1, (database, tx) => {
      const store = database.createObjectStore('a', {autoIncrement: true}); saved = store;
      const index = store.createIndex('index', 'key', {unique: true});
      database.createObjectStore('b');
      const databaseNamesBefore = database.objectStoreNames;
      const transactionNamesBefore = tx.objectStoreNames;
      throws('database name list is readonly', 'TypeError', () => { database.objectStoreNames = []; });
      throws('transaction name list is readonly', 'TypeError', () => { tx.objectStoreNames = []; });
      for (const [decl, value, prototype] of [[dbList, database, IDBDatabase.prototype], [txList, tx, IDBTransaction.prototype]]) {
        const read = decl?.get || function() { return this.objectStoreNames; };
        for (const receiver of [{}, Object.create(prototype), new Proxy(value, {})]) {
          throws('name list getter validates receiver', 'TypeError', () => read.call(receiver));
        }
      }
      check('no own name property', !Object.hasOwn(store, 'name'));
      check('initial handle identity', tx.objectStore('a') === store);
      let conversions = 0;
      const poison = {toString() { ++conversions; throw 'conversion'; }};
      const revoked = Proxy.revocable(store, {}); revoked.revoke();
      for (const receiver of [null, undefined, {}, Object.create(IDBObjectStore.prototype), Object.create(store), new Proxy(store, {}), revoked.proxy, index, tx]) {
        throws('getter rejects forged receiver', 'TypeError', () => get.call(receiver));
        throws('setter rejects forged receiver', 'TypeError', () => set.call(receiver, poison));
      }
      equal('brand check precedes conversion', conversions, 0);
      const sentinel = {};
      check('conversion preserves exception', caught(() => { store.name = {toString() { throw sentinel; }}; }) === sentinel);
      throws('Symbol rejected', 'TypeError', () => { store.name = Symbol(); });
      store.name = {[Symbol.toPrimitive](hint) { equal('conversion string hint', hint, 'string'); ++conversions; return 'renamed'; }};
      equal('conversion once', conversions, 1);
      equal('native name changes', get.call(store), 'renamed');
      equal('database name list snapshot stays unchanged', Array.from(databaseNamesBefore), ['a', 'b']);
      equal('transaction name list snapshot stays unchanged', Array.from(transactionNamesBefore), ['a', 'b']);
      check('renamed lookup identity', tx.objectStore('renamed') === store);
      check('index retains store', index.objectStore === store);
      equal('database names change', Array.from(database.objectStoreNames), ['b', 'renamed']);
      equal('upgrade scope changes', Array.from(tx.objectStoreNames), ['b', 'renamed']);
      throws('old name disappears', 'NotFoundError', () => tx.objectStore('a'));
      throws('duplicate fails', 'ConstraintError', () => { store.name = 'b'; });
      equal('failed rename atomic', store.name, 'renamed');
      store.name = 'renamed';
      Object.defineProperty(store, 'name', {value: 'forged', configurable: true});
      set.call(store, 'native');
      equal('native setter ignores own field', get.call(store), 'native');
      equal('author field stays author controlled', store.name, 'forged');
      delete store.name; store.name = 'renamed';
      if (foreign) {
        const fd = foreign.Object.getOwnPropertyDescriptor(foreign.IDBObjectStore.prototype, 'name');
        check('foreign getter accepts genuine store', fd.get.call(store) === 'renamed');
        const typeError = caught(() => fd.set.call({}, poison));
        check('foreign brand error realm', typeError instanceof foreign.TypeError && !(typeError instanceof TypeError));
        const conflict = caught(() => fd.set.call(store, 'b'));
        check('foreign DOMException realm', conflict instanceof foreign.DOMException && !(conflict instanceof DOMException));
        fd.set.call(store, 'foreign');
        check('foreign rename preserves wrapper realm', foreign.IDBTransaction.prototype.objectStore.call(tx, 'foreign') === store);
        fd.set.call(store, 'renamed');
      }
      const first = store.add({key: 10});
      const second = store.add({key: 20});
      const cursorRequest = index.openCursor();
      initialReads = Promise.all([request(first), request(second), request(cursorRequest)]).then(async ([one, two, cursor]) => {
        equal('key generator before rename', [one, two], [1, 2]);
        store.name = 'cursor-name';
        equal('index cursor follows store rename', cursor.source.objectStore.name, 'cursor-name');
        const updated = request(cursor.update({key: 11}));
        cursor.continue();
        const moved = await request(cursorRequest);
        equal('cursor sees updated key after rename', moved.key, 11);
        moved.continue();
        const next = await request(cursorRequest);
        equal('cursor continues across rename', next.key, 20);
        await updated;
        store.name = 'renamed';
        equal('key generator after rename', await request(store.add({key: 30})), 3);
      });
      for (const [number, n] of names.entries()) {
        const item = database.createObjectStore(n);
        equal('create lossless name', get.call(item), n);
        check('lookup lossless identity', tx.objectStore(n) === item);
        item.put(number, 1);
        item.name = 'temp-' + number;
        throws('old UTF16 name removed', 'NotFoundError', () => tx.objectStore(n));
        item.name = n;
        check('rename returns to same UTF16 identity', tx.objectStore(n) === item);
        check('list contains exact UTF16 name', database.objectStoreNames.contains(n));
      }
      equal('database names UTF16 order', Array.from(database.objectStoreNames), ['b', 'renamed', ...names].sort());
      equal('upgrade names UTF16 order', Array.from(tx.objectStoreNames), ['b', 'renamed', ...names].sort());
      const deleted = database.createObjectStore('deleted');
      deleted.name = 'gone'; database.deleteObjectStore('gone');
      throws('deleted no-op rejected', 'InvalidStateError', () => { deleted.name = 'gone'; });
      const reentrant = database.createObjectStore('reentrant');
      throws('conversion deletion observed', 'InvalidStateError', () => { reentrant.name = {toString() { database.deleteObjectStore('reentrant'); return 'reentrant'; }}; });
    });
    await initialReads;
    throws('finished upgrade no-op rejected', 'TransactionInactiveError', () => { saved.name = 'renamed'; });
    let tx = db.transaction([...names].reverse().concat(names), 'readonly');
    let completed = done(tx);
    equal('regular scope sorted and deduplicated', Array.from(tx.objectStoreNames), [...names].sort());
    throws('out of scope rejected', 'NotFoundError', () => tx.objectStore('renamed'));
    const reads = names.map((n, number) => {
      const store = tx.objectStore(n);
      equal('reopened name preserved', get.call(store), n);
      throws('regular transaction no-op rejected', 'InvalidStateError', () => { store.name = n; });
      return request(store.get(1)).then(value => equal('UTF16 store keeps distinct record', value, number));
    });
    const same = tx.objectStore(names[0]);
    tx.commit();
    // The specification rejects finished transactions here, not committing ones.
    const committingError = caught(() => check('committing lookup identity', tx.objectStore(names[0]) === same));
    check('committing transaction still resolves store', committingError === null, committingError);
    await Promise.all(reads); await completed;
    throws('finished transaction lookup rejected', 'InvalidStateError', () => tx.objectStore(names[0]));
    db.close();

    let original, oldIndex, replacement, newIndex, removed;
    const abort = await open(2, (database, upgrade) => {
      original = upgrade.objectStore('renamed'); oldIndex = original.index('index');
      original.name = 'temp'; oldIndex.name = 'changed-index';
      original.name = 'twice'; database.deleteObjectStore('twice');
      replacement = database.createObjectStore('renamed'); replacement.name = 'replacement';
      newIndex = replacement.createIndex('new', 'key'); newIndex.name = 'new-last';
      removed = upgrade.objectStore('b'); database.deleteObjectStore('b');
      const deletedReplacement = database.createObjectStore('b'); deletedReplacement.name = 'new-b';
      upgrade.abort();
      equal('existing name restored synchronously', original.name, 'renamed');
      equal('existing index restored with store', oldIndex.name, 'index');
      equal('new store retains final name', replacement.name, 'replacement');
      equal('new index retains final name', newIndex.name, 'new-last');
      equal('new store has no indexes after abort', Array.from(replacement.indexNames), []);
      equal('deleted old store restored', removed.name, 'b');
      equal('new replacement keeps own identity', deletedReplacement.name, 'new-b');
      equal('database rollback names', Array.from(database.objectStoreNames), ['b', 'renamed', ...names].sort());
      equal('scope rollback names', Array.from(upgrade.objectStoreNames), ['b', 'renamed', ...names].sort());
      throws('restored existing store inactive', 'TransactionInactiveError', () => { original.name = 'renamed'; });
      throws('new aborted store deleted before inactive', 'InvalidStateError', () => { replacement.name = 'replacement'; });
    }).then(() => null, error => error);
    equal('upgrade aborted', abort?.name, 'AbortError');
    db = await open(1);
    tx = db.transaction('renamed'); completed = done(tx);
    const restored = tx.objectStore('renamed');
    equal('rollback preserves records including cursor update', await request(restored.getAll()), [{key: 11}, {key: 20}, {key: 30}]);
    await completed; db.close();
    db = await open(2, (database, upgrade) => {
      const store = upgrade.objectStore('renamed'); store.name = 'committed';
      equal('finished handle name stays frozen', saved.name, 'renamed');
      equal('aborted handle name stays frozen', original.name, 'renamed');
      Object.defineProperty(upgrade, 'db', {value: {}, configurable: true});
      let listReads = 0, listWrites = 0;
      const shadowNames = {
        configurable: true,
        get() { ++listReads; return ['author']; },
        set() { ++listWrites; throw 'author setter'; },
      };
      Object.defineProperty(upgrade, 'objectStoreNames', shadowNames);
      check('lookup uses native scope and database', upgrade.objectStore('committed') === store);
      Object.defineProperty(database, 'objectStoreNames', shadowNames);
      check('rename does not throw through an author name-list setter', caught(() => { store.name = 'committed-again'; }) === null);
      equal('rename never reads author name lists', listReads, 0);
      equal('rename never writes author name lists', listWrites, 0);
      equal('own transaction getter remains author controlled', Array.from(upgrade.objectStoreNames), ['author']);
      equal('own database getter remains author controlled', Array.from(database.objectStoreNames), ['author']);
      delete upgrade.objectStoreNames; delete database.objectStoreNames;
      equal('prototype lists reflect native rename after removing shadow', Array.from(upgrade.objectStoreNames), ['b', 'committed-again', ...names].sort());
      check('rename ignores author name list', upgrade.objectStore('committed-again') === store);
      for (const n of names) equal('author list cannot change native scope', upgrade.objectStore(n).name, n);
      database.deleteObjectStore('\uD800');
      const item = database.createObjectStore('\uD800'); item.put('replacement', 1);
    });
    tx = db.transaction(['committed-again', '\uD800', '\uD801'], 'readwrite'); completed = done(tx);
    const values = await Promise.all([
      request(tx.objectStore('committed-again').add({key: 40})),
      request(tx.objectStore('\uD800').get(1)), request(tx.objectStore('\uD801').get(1)),
    ]);
    equal('committed rename preserves generator and isolated replacement', values, [4, 'replacement', 3]);
    await completed;
    return {state: checks.every(check => check.pass) ? 'pass' : 'fail', checks};
  } finally {
    db?.close(); frame?.remove(); await request(indexedDB.deleteDatabase(name));
  }
}
