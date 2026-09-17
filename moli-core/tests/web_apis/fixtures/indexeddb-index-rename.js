globalThis.renameChecks = [];
async function indexRenameProbe(name) {
  'use strict';
  name ??= 'index-rename-' + Math.random();
  const checks = globalThis.renameChecks;
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
  const descriptor = Object.getOwnPropertyDescriptor(IDBIndex.prototype, 'name');
  check('name is an enumerable configurable prototype accessor', descriptor?.enumerable && descriptor?.configurable && typeof descriptor.get === 'function' && typeof descriptor.set === 'function');
  const get = descriptor?.get || function() { return this.name; };
  const set = descriptor?.set || function(value) { this.name = value; };
  const names = ['', '\0', '\uD800', '\uD801', '\uDC00', '\uDC01', '\uFFFD', '\uDC00\uD800', '\u{1F600}', '\uE000', '__proto__'];
  let original, savedStore, initialReads, foreign, frame;
  if (typeof document !== 'undefined') {
    if (document.readyState === 'loading') {
      await new Promise(resolve => document.addEventListener('DOMContentLoaded', resolve, {once: true}));
    }
    frame = document.createElement('iframe'); document.body.appendChild(frame);
    foreign = frame.contentWindow;
  }
  let db;
  try {
    db = await open(1, (database, tx) => {
      const store = database.createObjectStore('s'); savedStore = store;
      original = store.createIndex('a', 'key', {unique: true});
      store.createIndex('b', 'tags', {multiEntry: true});
      check('store handle identity', tx.objectStore('s') === store);
      check('index handle identity', store.index('a') === original);
      check('no own name data property', !Object.hasOwn(original, 'name'));
      const revoked = Proxy.revocable(original, {}); revoked.revoke();
      let conversions = 0;
      const poison = {toString() { ++conversions; throw 'conversion'; }};
      for (const receiver of [null, undefined, {}, Object.create(IDBIndex.prototype), Object.create(original), new Proxy(original, {}), revoked.proxy, store, tx]) {
        throws('getter rejects invalid receiver', 'TypeError', () => get.call(receiver));
        throws('setter rejects invalid receiver', 'TypeError', () => set.call(receiver, poison));
      }
      equal('receiver check precedes conversion', conversions, 0);
      const sentinel = {};
      check('setter preserves toString exception', caught(() => { original.name = {toString() { throw sentinel; }}; }) === sentinel);
      throws('setter rejects Symbol', 'TypeError', () => { original.name = Symbol(); });
      original.name = { [Symbol.toPrimitive](hint) { equal('rename string hint', hint, 'string'); ++conversions; return 'renamed'; } };
      equal('name conversion runs once', conversions, 1);
      equal('native name after rename', get.call(original), 'renamed');
      equal('indexNames reflect rename', Array.from(store.indexNames), ['b', 'renamed']);
      check('lookup preserves index identity after rename', caught(() => check('lookup is original index', store.index('renamed') === original)) === null);
      throws('old name no longer resolves', 'NotFoundError', () => store.index('a'));
      throws('duplicate name rejected', 'ConstraintError', () => { original.name = 'b'; });
      equal('failed rename is atomic', get.call(original), 'renamed');
      original.name = 'renamed';
      equal('same-name no-op keeps names', Array.from(store.indexNames), ['b', 'renamed']);
      Object.defineProperty(original, 'name', {value: 'forged', configurable: true});
      equal('own field cannot replace native name', get.call(original), 'renamed');
      set.call(original, 'native');
      equal('native setter ignores own field', get.call(original), 'native');
      equal('own field stays author controlled', original.name, 'forged');
      delete original.name;
      set.call(original, 'renamed');
      store.put({key: 10, tags: [2, 3]}, 1);
      store.put({key: 20, tags: [3]}, 2);
      const read = original.getAllKeys();
      const cursorRequest = original.openCursor();
      initialReads = Promise.all([request(read), request(cursorRequest)]).then(([keys, cursor]) => {
        equal('renamed index reads existing records', keys, [1, 2]);
        check('cursor keeps renamed index as source', cursor.source === original);
        equal('cursor reads correct index key', cursor.key, 10);
        original.name = 'cursor-name';
        equal('cursor source name follows rename', get.call(cursor.source), 'cursor-name');
        original.name = 'renamed';
        cursor.continue();
        return request(cursorRequest).then(next => equal('cursor continues across rename', next.key, 20));
      });
      const utf16 = database.createObjectStore('utf16');
      for (let number = 0; number < names.length; ++number) {
        const name = names[number], index = utf16.createIndex('source-' + number, 'key');
        index.name = name;
        equal('UTF-16 getter ' + number, get.call(index), name);
        check('UTF-16 contains ' + number, utf16.indexNames.contains(name));
        check('UTF-16 lookup ' + number, caught(() => check('UTF-16 identity ' + number, utf16.index(name) === index)) === null);
      }
      equal('UTF-16 ordering', Array.from(utf16.indexNames), names.slice().sort());
      const direct = database.createObjectStore('direct');
      for (let number = 0; number < names.length; ++number) {
        const raw = names[number];
        check('create lossless name ' + number, caught(() => equal('created name ' + number, get.call(direct.createIndex(raw, 'key')), raw)) === null);
      }
      equal('direct names preserve distinct surrogates', Array.from(direct.indexNames), names.slice().sort());
      check('contains does not alias missing surrogate', !direct.indexNames.contains('\uD802'));
      for (let number = 0; number < names.length; ++number) equal('DOMStringList item ' + number, direct.indexNames.item(number), names.slice().sort()[number]);
      const stringify = direct.createIndex('stringify', 'key');
      for (const value of [undefined, null, 17, true]) {
        stringify.name = value;
        equal('DOMString conversion ' + String(value), get.call(stringify), String(value));
      }
      direct.deleteIndex(get.call(stringify));
      if (foreign) {
        const d = Object.getOwnPropertyDescriptor(foreign.IDBIndex.prototype, 'name');
        check('foreign realm accessor exists', !!d?.set && !!d?.get);
        if (d?.set && d?.get) {
          equal('foreign getter accepts local index', d.get.call(original), 'renamed');
          d.set.call(original, 'foreign-name');
          equal('foreign setter mutates local index', get.call(original), 'foreign-name');
          d.set.call(original, 'renamed');
          for (const [label, fn, Constructor] of [
            ['receiver', () => d.set.call(new Proxy(original, {}), poison), foreign.TypeError],
            ['Symbol', () => d.set.call(original, Symbol()), foreign.TypeError],
            ['duplicate', () => d.set.call(original, 'b'), foreign.DOMException],
            ['lookup Symbol', () => foreign.IDBObjectStore.prototype.index.call(store, Symbol()), foreign.TypeError],
          ]) check('callee error realm ' + label, caught(fn) instanceof Constructor);
          check('foreign lookup keeps local identity', foreign.IDBObjectStore.prototype.index.call(store, 'renamed') === original);
        }
      }
    });
    await initialReads;
    equal('name retained after initial commit', get.call(original), 'renamed');
    throws('finished upgrade rejects even same name', 'TransactionInactiveError', () => set.call(original, get.call(original)));
    for (const mode of ['readonly', 'readwrite']) {
      const tx = db.transaction('s', mode), complete = done(tx), index = tx.objectStore('s').index('renamed');
      throws(mode + ' mode check precedes same-name no-op', 'InvalidStateError', () => { index.name = index.name; });
      const sentinel = {};
      check(mode + ' conversion precedes mode check', caught(() => { index.name = {toString() { throw sentinel; }}; }) === sentinel);
      await complete;
      throws(mode + ' mode check precedes finished state', 'InvalidStateError', () => { index.name = 'changed'; });
    }
    db.close(); db = null;

    let old, other, created, replacement, removedStore;
    const abort = await open(2, (database, tx) => {
      const store = tx.objectStore('s'); removedStore = store;
      old = store.index('renamed'); other = store.index('b');
      old.name = 'temporary'; other.name = 'renamed'; old.name = 'b';
      equal('swap through temporary name', [get.call(old), get.call(other)], ['b', 'renamed']);
      check('swapped handles remain unique', store.index('b') === old && tx.objectStore('s').index('renamed') === other);
      created = store.createIndex('created', 'key'); created.name = 'created-last';
      store.deleteIndex('b');
      throws('deleted index rejects same-name assignment', 'InvalidStateError', () => { old.name = 'b'; });
      const recreated = store.createIndex('b', 'other');
      check('recreated name has distinct identity', recreated !== old);
      database.deleteObjectStore('s');
      throws('deleted store rejects rename', 'InvalidStateError', () => { other.name = 'new'; });
      replacement = database.createObjectStore('s').createIndex('new-store-index', 'key');
      replacement.name = 'new-store-last';
      tx.abort();
      equal('existing renamed/deleted handles roll back immediately', [get.call(old), get.call(other)], ['renamed', 'b']);
      equal('new index retains last name after abort', get.call(created), 'created-last');
      equal('new store index retains last name after abort', get.call(replacement), 'new-store-last');
      equal('existing store indexNames restored', Array.from(store.indexNames), ['b', 'renamed']);
      for (const index of [old, other, created, replacement]) throws('inactive upgrade precedes deletion', 'TransactionInactiveError', () => set.call(index, get.call(index)));
    }).then(() => null, error => error);
    equal('upgrade abort delivered', abort?.name, 'AbortError');
    equal('abort event retains original index names', [get.call(old), get.call(other)], ['renamed', 'b']);

    // Conversion may synchronously rename, create a conflicting index, delete
    // the receiver's index, or abort. State must be read after ToString.
    const sideEffectAbort = await open(2, (database, tx) => {
      const store = tx.objectStore('s'), index = store.index('renamed');
      index.name = {toString() { index.name = 'nested'; return 'outer'; }};
      equal('reentrant rename observes latest native name', get.call(index), 'outer');
      throws('conversion-created duplicate', 'ConstraintError', () => {
        index.name = {toString() { store.createIndex('taken', 'key'); return 'taken'; }};
      });
      throws('conversion-deleted index', 'InvalidStateError', () => {
        index.name = {toString() { store.deleteIndex('outer'); return 'after-delete'; }};
      });
      const last = store.index('b');
      throws('conversion-aborted transaction', 'TransactionInactiveError', () => {
        last.name = {toString() { tx.abort(); return 'after-abort'; }};
      });
      equal('conversion abort restores name', get.call(index), 'renamed');
    }).then(() => null, error => error);
    equal('conversion abort delivered', sideEffectAbort?.name, 'AbortError');

    db = await open(2, (database, tx) => {
      const store = tx.objectStore('s'); store.index('renamed').name = 'committed';
      const utf16 = tx.objectStore('utf16');
      for (const raw of names) equal('reopened UTF-16 name', get.call(utf16.index(raw)), raw);
      for (const raw of ['\uD800', '\uDC00', '\uFFFD']) utf16.deleteIndex(raw);
      check('lossless delete preserves neighboring surrogate', utf16.indexNames.contains('\uD801') && !utf16.indexNames.contains('\uD800'));
    });
    equal('earlier transaction handle name stays frozen', get.call(original), 'renamed');
    equal('earlier store index names stay frozen', Array.from(savedStore.indexNames), ['b', 'renamed']);
    equal('aborted handles unaffected by later commit', [get.call(old), get.call(other)], ['renamed', 'b']);
    equal('aborted store names unaffected by later commit', Array.from(removedStore.indexNames), ['b', 'renamed']);
    const tx = db.transaction('s'), complete = done(tx), store = tx.objectStore('s'), index = store.index('committed');
    check('fresh transaction index identity', store.index('committed') === index && index !== original);
    equal('committed metadata', Array.from(store.indexNames), ['b', 'committed']);
    equal('committed index content', await request(index.getAllKeys()), [1, 2]);
    await complete;
  } finally { db?.close(); frame?.remove(); }
  await request(indexedDB.deleteDatabase(name));
  return {state: checks.every(item => item.pass) ? 'pass' : 'fail', checks};
}
