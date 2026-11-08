globalThis.compoundDepthChecks = [];
async function compoundDepthProbe(name = 'compound-depth-' + Math.random()) {
  const checks = compoundDepthChecks;
  const check = (label, pass, actual = '') => checks.push({label, pass: !!pass, actual: String(actual)});
  const equal = (label, actual, expected) => check(label, JSON.stringify(actual) === JSON.stringify(expected), JSON.stringify(actual));
  const request = r => new Promise((resolve, reject) => { r.onsuccess = () => resolve(r.result); r.onerror = e => { e.preventDefault(); reject(r.error); }; });
  const done = tx => new Promise(resolve => { tx.oncomplete = () => resolve('complete'); tx.onabort = () => resolve('abort'); });
  const nested = (depth, value = 1) => { for (let i = 0; i < depth; i++) value = [value]; return value; };
  const depthOf = value => { let depth = 0; while (Array.isArray(value)) { depth++; value = value[0]; } return depth; };
  const opening = indexedDB.open(name, 1);
  opening.onupgradeneeded = () => {
    opening.result.createObjectStore('primary', {keyPath: ['k']});
    const indexed = opening.result.createObjectStore('indexed');
    indexed.createIndex('compound', ['k']);
    indexed.createIndex('unique', ['k'], {unique: true});
  };
  let db = await request(opening);
  try {
    for (const method of ['add', 'put']) for (const depth of [255, 256]) {
      const label = method + ' component depth ' + depth;
      const tx = db.transaction(['primary', 'indexed'], 'readwrite'), complete = done(tx);
      const primary = tx.objectStore('primary'), indexed = tx.objectStore('indexed');
      primary.clear(); indexed.clear();
      const value = {k: nested(depth)};
      if (depth === 255) {
        const key = await request(primary[method](value));
        equal(label + ' assembled key at limit', depthOf(key), 256);
        const stored = await request(primary.get(key));
        equal(label + ' returned primary key queries record', depthOf(stored.k), 255);
      } else {
        let error;
        try { primary[method](value); } catch (caught) { error = caught; }
        equal(label + ' assembled key rejected synchronously', error?.name, 'DataError');
        equal(label + ' transaction accepts a valid replacement', await request(primary.put({k: 'recovery'})), ['recovery']);
      }
      const first = request(indexed[method](value, 1));
      const second = request(indexed[method](depth === 256 ? value : {k: nested(depth, 2)}, 2));
      equal(label + ' ordinary values still stored', await Promise.all([first, second]), [1, 2]);
      for (const index of ['compound', 'unique']) equal(label + ' index depth policy ' + index, await request(indexed.index(index).count()), depth === 255 ? 2 : 0);
      equal(label + ' rejected index keys do not reject records', await request(indexed.count()), 2);
      equal(label + ' transaction completes', await complete, 'complete');
      const later = db.transaction('indexed', 'readwrite'), laterDone = done(later);
      later.objectStore('indexed').put({k: 'later'}, 99);
      equal(label + ' later same-origin commit remains healthy', await laterDone, 'complete');
    }
    db.close(); db = await request(indexedDB.open(name));
    equal('reopen version remains healthy', db.version, 1);
    equal('reopen retains later committed record', (await request(db.transaction('indexed').objectStore('indexed').get(99))).k, 'later');
  } finally { db.close(); }
  await request(indexedDB.deleteDatabase(name));
  return {state: checks.every(c => c.pass) ? 'pass' : 'fail', checks};
}
