async function indexCursorRenameProbe(name) {
  const request = r => new Promise((resolve, reject) => {
    r.onsuccess = () => resolve(r.result);
    r.onerror = e => { e.preventDefault(); reject(r.error); };
  });
  const seen = [], reads = [];
  let failure, saved;
  const open = (version, setup) => new Promise((resolve, reject) => {
    const r = indexedDB.open(name, version);
    r.onupgradeneeded = () => { try { setup(r.result, r.transaction); } catch (error) { failure = error; r.transaction.abort(); } };
    r.onsuccess = () => resolve(r.result);
    r.onerror = e => { e.preventDefault(); reject(failure || r.error); };
  });
  let db = await open(1, database => {
    const store = database.createObjectStore('s');
    store.createIndex('a', 'key');
    for (const value of [1, 2, 3]) store.put({key: value}, value);
  });
  db.close();
  try {
    db = await open(2, (database, tx) => {
      const index = tx.objectStore('s').index('a'); saved = index;
      const cursorRequest = index.openCursor();
      cursorRequest.onsuccess = () => {
        try {
          const cursor = cursorRequest.result;
          if (!cursor) return;
          seen.push([cursor.key, index.name]);
          if (cursor.key === 1) {
            reads.push(request(index.getAllKeys())); // Accepted under a.
            index.name = 'b';
            reads.push(request(index.getAllKeys())); // Accepted under b.
            cursor.continue();                      // Accepted under b.
            index.name = 'c';
            reads.push(request(index.getAllKeys())); // Accepted under c.
          } else if (cursor.key === 2) {
            cursor.continue(); // Accepted under c, before the following rename.
            index.name = 'd';
          } else cursor.continue();
        } catch (error) { failure = error; tx.abort(); }
      };
    });
    const values = await Promise.all(reads);
    const keys = await request(db.transaction('s').objectStore('s').index('d').getAllKeys());
    return {state: JSON.stringify(seen) === '[[1,"a"],[2,"c"],[3,"d"]]'
      && values.every(value => JSON.stringify(value) === '[1,2,3]')
      && JSON.stringify(keys) === '[1,2,3]' && saved.name === 'd' ? 'pass' : 'fail', seen, values, keys};
  } finally { db?.close(); await request(indexedDB.deleteDatabase(name)); }
}
