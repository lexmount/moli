globalThis.navigationChecks = [];
async function cursorNavigationProbe(name = 'cursor-navigation-' + Math.random()) {
  const checks = globalThis.navigationChecks;
  const check = (label, pass, actual = '') => { checks.push({label, pass: !!pass, actual: String(actual)}); return !!pass; };
  const equal = (label, actual, expected) => check(label, actual === expected, actual);
  const throws = (label, call, expected, realm = globalThis) => {
    try { call(); return check(label, false, 'accepted'); }
    catch (error) { return check(label, error.name === expected && error instanceof (expected === 'TypeError' ? realm.TypeError : realm.DOMException), error.name); }
  };
  const request = r => new Promise((resolve, reject) => { r.onsuccess = () => resolve(r.result); r.onerror = event => { event.preventDefault(); reject(r.error); }; });
  const done = tx => new Promise(resolve => { tx.oncomplete = () => resolve('complete'); tx.onabort = () => resolve('abort'); });
  const methods = Object.fromEntries(['advance', 'continue', 'continuePrimaryKey', 'update', 'delete'].map(key => [key, IDBCursor.prototype[key]]));
  const sourceOf = (store, kind) => kind === 'store' ? store : store.index('i');
  const seed = db => {
    const store = db.createObjectStore('s'); store.createIndex('i', 'index');
    for (let n = 1; n <= 3; n++) store.put({id: n, index: [n]}, [n]);
    return store;
  };
  const opening = indexedDB.open(name, 1); opening.onupgradeneeded = () => seed(opening.result);
  const db = await request(opening);
  let argumentReads = 0;
  const key = Object.defineProperty([], 0, {get() { argumentReads++; return 2; }});
  const value = {get id() { argumentReads++; return 2; }};
  const inactive = (label, cursor, realm = globalThis) => {
    const p = realm.IDBCursor.prototype;
    argumentReads = 0;
    throws(label + ' zero before inactive', () => p.advance.call(cursor, 0), 'TypeError', realm);
    throws(label + ' required count before inactive', () => p.advance.call(cursor), 'TypeError', realm);
    throws(label + ' required tuple before inactive', () => p.continuePrimaryKey.call(cursor), 'TypeError', realm);
    throws(label + ' advance inactive', () => p.advance.call(cursor, 1), 'TransactionInactiveError', realm);
    throws(label + ' continue inactive', () => p.continue.call(cursor, key), 'TransactionInactiveError', realm);
    throws(label + ' tuple inactive', () => p.continuePrimaryKey.call(cursor, key, key), 'TransactionInactiveError', realm);
    throws(label + ' update inactive', () => p.update.call(cursor, value), 'TransactionInactiveError', realm);
    throws(label + ' delete inactive', () => p.delete.call(cursor), 'TransactionInactiveError', realm);
    equal(label + ' inactive arguments untouched', argumentReads, 0);
  };
  let completedCursor;
  try {
    for (const mode of ['readonly', 'readwrite']) for (const kind of ['store', 'index']) for (const direction of ['next', 'prev', 'nextunique', 'prevunique']) for (const open of ['openCursor', 'openKeyCursor']) {
      const tupleAllowed = kind === 'index' && !direction.endsWith('unique');
      for (const first of tupleAllowed ? ['advance', 'continue', 'continuePrimaryKey'] : ['advance', 'continue']) {
        const label = [mode, kind, direction, open, first].join(' ');
        const tx = db.transaction('s', mode), complete = done(tx), r = sourceOf(tx.objectStore('s'), kind)[open](null, direction);
        const cursor = await request(r), oldKey = cursor.key, oldValue = cursor.value;
        throws(label + ' rejected zero preserves state', () => methods.advance.call(cursor, 0), 'TypeError');
        throws(label + ' rejected key preserves state', () => methods.continue.call(cursor, cursor.key), 'DataError');
        const marker = {};
        const throwingKey = Object.defineProperty([], 0, {get() { throw marker; }});
        try { methods.continue.call(cursor, throwingKey); check(label + ' key getter exception', false); }
        catch (error) { check(label + ' key getter exception', error === marker); }
        if (!tupleAllowed) {
          argumentReads = 0;
          throws(label + ' source or direction before key', () => methods.continuePrimaryKey.call(cursor, key, key), 'InvalidAccessError');
          equal(label + ' invalid tuple arguments untouched', argumentReads, 0);
        }
        methods[first].apply(cursor, first === 'advance' ? [1] : first === 'continue' ? [] : [[2], [2]]);
        equal(label + ' pending request', r.readyState, 'pending');
        check(label + ' pending getters retain values', cursor.key === oldKey && cursor.value === oldValue);
        let valid = throws(label + ' zero before pending', () => methods.advance.call(cursor, 0), 'TypeError');
        argumentReads = 0;
        valid = throws(label + ' count conversion before pending', () => methods.advance.call(cursor, {valueOf() { argumentReads++; return 1; }}), 'InvalidStateError') && valid;
        equal(label + ' count converted', argumentReads, 1);
        argumentReads = 0;
        valid = throws(label + ' continue pending', () => methods.continue.call(cursor, key), 'InvalidStateError') && valid;
        valid = throws(label + ' tuple pending', () => methods.continuePrimaryKey.call(cursor, key, key), tupleAllowed ? 'InvalidStateError' : 'InvalidAccessError') && valid;
        valid = throws(label + ' update pending', () => methods.update.call(cursor, value), mode === 'readonly' ? 'ReadOnlyError' : 'InvalidStateError') && valid;
        valid = throws(label + ' delete pending', () => methods.delete.call(cursor), mode === 'readonly' ? 'ReadOnlyError' : 'InvalidStateError') && valid;
        equal(label + ' pending keys and value untouched', argumentReads, 0);
        // A broken baseline may have queued extra requests. Abort that variant
        // so it cannot mutate the database or corrupt later checks.
        if (!valid) { tx.abort(); await complete; continue; }
        check(label + ' next result reuses cursor', await request(r) === cursor);
        equal(label + ' next position', cursor.key[0], 2);
        equal(label + ' ready after success', r.readyState, 'done');
        methods.advance.call(cursor, 100);
        equal(label + ' exhausted result', await request(r), null);
        throws(label + ' zero before exhausted', () => methods.advance.call(cursor, 0), 'TypeError');
        throws(label + ' advance exhausted', () => methods.advance.call(cursor, 1), 'InvalidStateError');
        argumentReads = 0;
        throws(label + ' continue exhausted', () => methods.continue.call(cursor, key), 'InvalidStateError');
        throws(label + ' tuple exhausted', () => methods.continuePrimaryKey.call(cursor, key, key), tupleAllowed ? 'InvalidStateError' : 'InvalidAccessError');
        equal(label + ' exhausted keys untouched', argumentReads, 0);
        equal(label + ' completion', await complete, 'complete');
        inactive(label, cursor);
        completedCursor = cursor;
      }
    }
    // Count conversion can change the cursor's state. Validate only after it.
    for (const transition of ['continue', 'abort']) {
      const tx = db.transaction('s'), complete = done(tx), r = tx.objectStore('s').openCursor(), cursor = await request(r);
      throws('count conversion triggers ' + transition, () => cursor.advance({valueOf() { if (transition === 'abort') tx.abort(); else cursor.continue(); return 1; }}), transition === 'abort' ? 'TransactionInactiveError' : 'InvalidStateError');
      await complete;
      inactive('after ' + transition, cursor);
    }
    if (typeof document !== 'undefined' && completedCursor) {
      const frame = document.createElement('iframe'); document.body.appendChild(frame);
      try { inactive('foreign callee', completedCursor, frame.contentWindow); }
      finally { frame.remove(); }
    }
  } finally { db.close(); }
  await request(indexedDB.deleteDatabase(name));

  // Recreating a deleted source under the same name must not revive a cursor.
  for (const removed of ['store', 'index', 'index-store']) for (const direction of ['next', 'nextunique']) for (const phase of ['ready', 'exhausted']) {
    const label = ['deleted', removed, direction, phase].join(' '), databaseName = name + label;
    const r = indexedDB.open(databaseName, 1);
    let probe;
    r.onupgradeneeded = () => {
      const database = r.result, tx = r.transaction, complete = done(tx), store = seed(database);
      probe = (async () => {
        const source = removed === 'store' ? store : store.index('i'), cr = source.openCursor(null, direction), cursor = await request(cr);
        if (phase === 'exhausted') { cursor.advance(100); await request(cr); }
        if (removed === 'index') { store.deleteIndex('i'); store.createIndex('i', 'index'); }
        else { database.deleteObjectStore('s'); seed(database); }
        argumentReads = 0;
        let valid = throws(label + ' zero before deleted', () => cursor.advance(0), 'TypeError');
        valid = throws(label + ' advance deleted', () => cursor.advance(1), 'InvalidStateError') && valid;
        valid = throws(label + ' continue deleted', () => cursor.continue(key), 'InvalidStateError') && valid;
        valid = throws(label + ' tuple deleted', () => cursor.continuePrimaryKey(key, key), 'InvalidStateError') && valid;
        valid = throws(label + ' update deleted', () => cursor.update(value), 'InvalidStateError') && valid;
        valid = throws(label + ' delete deleted', () => cursor.delete(), 'InvalidStateError') && valid;
        equal(label + ' deleted arguments untouched', argumentReads, 0);
        if (!valid) tx.abort();
        await complete;
        inactive(label + ' inactive', cursor);
      })();
    };
    const opened = await request(r).catch(() => null);
    await probe;
    if (opened) opened.close();
    await request(indexedDB.deleteDatabase(databaseName));
  }
  return {state: checks.every(item => item.pass) ? 'pass' : 'fail', checks};
}
