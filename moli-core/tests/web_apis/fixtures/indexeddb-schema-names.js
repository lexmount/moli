globalThis.schemaNameChecks = [];
async function schemaNameProbe(databaseName) {
  const checks = globalThis.schemaNameChecks;
  const check = (label, pass, actual = '') => checks.push({label, pass: !!pass, actual: String(actual)});
  const equal = (label, actual, expected) => check(label, JSON.stringify(actual) === JSON.stringify(expected), JSON.stringify(actual));
  const request = r => new Promise((resolve, reject) => {
    r.onsuccess = () => resolve(r.result);
    r.onerror = event => { event.preventDefault(); reject(r.error); };
  });
  const done = tx => new Promise((resolve, reject) => {
    tx.oncomplete = resolve;
    tx.onabort = () => reject(tx.error || new Error('unexpected abort'));
  });
  const names = ['', String.fromCharCode(0), String.fromCharCode(0xd800), String.fromCharCode(0xd801),
    String.fromCharCode(0xdc00), String.fromCharCode(0xfffd), String.fromCharCode(0xdc00, 0xd800),
    String.fromCodePoint(0x1f600), String.fromCharCode(0xe000), '__proto__'];
  const open = (version, setup) => new Promise((resolve, reject) => {
    const r = indexedDB.open(databaseName, version);
    let setupError;
    r.onupgradeneeded = () => { try { setup?.(r.result, r.transaction); } catch (error) { setupError = error; r.transaction.abort(); } };
    r.onsuccess = () => resolve(r.result);
    r.onerror = event => { event.preventDefault(); reject(setupError || r.error); };
  });
  let db;
  try {
    db = await open(1, (database, tx) => {
      for (const [ordinal, raw] of names.entries()) {
        const store = database.createObjectStore(raw);
        equal('created exact store name', store.name, raw);
        for (const indexName of names) {
          const index = store.createIndex(indexName, 'label');
          equal('created exact index name', index.name, indexName);
        }
        equal('index list uses code-unit ordering', Array.from(store.indexNames), [...names].sort());
        store.add({label: ordinal, value: ordinal}, 1);
      }
      equal('database list uses code-unit ordering', Array.from(database.objectStoreNames), [...names].sort());
      equal('upgrade scope uses code-unit ordering', Array.from(tx.objectStoreNames), [...names].sort());
    });
    let tx = db.transaction([...names].reverse().concat(names));
    let complete = done(tx);
    equal('regular scope sorts and deduplicates exact names', Array.from(tx.objectStoreNames), [...names].sort());
    const reads = [];
    for (const [ordinal, raw] of names.entries()) {
      const store = tx.objectStore(raw);
      equal('lookup exact store name', store.name, raw);
      check('store list contains exact name', db.objectStoreNames.contains(raw));
      for (const indexName of names) {
        const index = store.index(indexName);
        equal('lookup exact index name', index.name, indexName);
        check('index list contains exact name', store.indexNames.contains(indexName));
        reads.push(request(index.get(ordinal)).then(value => equal('index keeps independent record', value, {label: ordinal, value: ordinal})));
      }
      reads.push(request(store.get(1)).then(value => equal('store keeps independent record', value, {label: ordinal, value: ordinal})));
    }
    await Promise.all(reads); await complete;
    db.close();
    const abort = await open(2, (database, upgrade) => {
      const raw = names[2];
      const store = upgrade.objectStore(names[3]);
      store.deleteIndex(raw);
      check('delete index removes only exact surrogate', !store.indexNames.contains(raw) && store.indexNames.contains(names[3]) && store.indexNames.contains(names[5]));
      database.deleteObjectStore(raw);
      check('delete store removes only exact surrogate', !database.objectStoreNames.contains(raw) && database.objectStoreNames.contains(names[3]) && database.objectStoreNames.contains(names[5]));
      upgrade.abort();
      equal('abort restores exact store labels', Array.from(database.objectStoreNames), [...names].sort());
      equal('abort restores exact index labels', Array.from(store.indexNames), [...names].sort());
    }).then(() => null, error => error);
    equal('upgrade abort preserves prior names and version', abort?.name, 'AbortError');
    db = await open(1);
    tx = db.transaction(names);
    complete = done(tx);
    equal('reopen keeps exact schema labels', Array.from(db.objectStoreNames), [...names].sort());
    await Promise.all(names.map((raw, ordinal) => request(tx.objectStore(raw).get(1))
      .then(value => equal('reopen retains independent records', value, {label: ordinal, value: ordinal}))));
    await complete;
    return {state: checks.every(c => c.pass) ? 'pass' : 'fail', checks};
  } finally { db?.close(); await request(indexedDB.deleteDatabase(databaseName)); }
}
