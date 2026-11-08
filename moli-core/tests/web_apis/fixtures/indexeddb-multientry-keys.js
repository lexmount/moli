const multiEntryChecks = [];

async function multiEntryProbe(name, scenario) {
  let context = scenario;
  const check = (condition, label) => {
    if (!condition) throw new Error(`${context}: ${label}`);
    multiEntryChecks.push(`${context}: ${label}`);
  };
  const result = request => new Promise((resolve, reject) => {
    request.onsuccess = () => resolve(request.result);
    request.onerror = () => reject(request.error);
  });
  const terminal = tx => new Promise(resolve => {
    tx.oncomplete = () => resolve('complete');
    tx.onabort = () => resolve(tx.error?.name || 'AbortError');
  });
  const cursorKeys = index => new Promise((resolve, reject) => {
    const keys = [], request = index.openCursor();
    request.onerror = () => reject(request.error);
    request.onsuccess = () => {
      const cursor = request.result;
      if (!cursor) { resolve(keys); return; }
      keys.push(cursor.key);
      cursor.continue();
    };
  });
  const methods = ['add', 'put', 'object-store-cursor', 'index-cursor'];
  const keyCases = [
    {name: 'string', values: () => ['new', 'new', 'new'], keys: () => ['new']},
    {name: 'zero', values: () => [-0, 0, -0], keys: () => [0]},
    {name: 'date', values: () => [new Date(10), new Date(10)], keys: () => [new Date(10)]},
    {name: 'binary', values: () => [new Uint8Array([1, 2]), new Uint8Array([0, 1, 2]).subarray(1)],
      keys: () => [new Uint8Array([1, 2])]},
    {name: 'array', values: () => [['x', [1]], ['x', [1]]], keys: () => [['x', [1]]]},
    {name: 'mixed', values: () => ['1', 1, new Date(1), 1, new Date(1), '1', null, undefined, NaN, {}],
      keys: () => [1, new Date(1), '1']},
  ];
  const open = indexedDB.open(name, 1);
  open.onupgradeneeded = () => {
    for (const unique of [false, true]) {
      const store = open.result.createObjectStore(unique ? 'unique' : 'nonunique', {keyPath: 'id'});
      store.createIndex('byTag', 'tags', {multiEntry: true, unique});
    }
    const ordinary = open.result.createObjectStore('ordinary', {keyPath: 'id'});
    ordinary.createIndex('byTag', 'tags');
    ordinary.add({id: 1, tags: ['new', 'new']});
  };
  const db = await result(open);
  let cases = 0;
  try {
    if (scenario === 'duplicates') {
      for (const storeName of ['unique', 'nonunique']) {
        for (const method of methods) {
          for (const keyCase of keyCases) {
            context = `${storeName}/${method}/${keyCase.name}`;
            const tags = keyCase.values();
            const tx = db.transaction(storeName, 'readwrite'), done = terminal(tx);
            const store = tx.objectStore(storeName);
            store.clear();
            let request;
            if (method === 'add' || method === 'put') {
              if (method === 'put') store.add({id: 1, tags: ['old']});
              request = store[method]({id: 1, tags});
            } else {
              store.add({id: 1, tags: ['old']});
              const source = method === 'index-cursor' ? store.index('byTag') : store;
              const cursor = await result(source.openCursor());
              const snapshot = cursor.value;
              request = cursor.update({id: 1, tags});
              check(cursor.value === snapshot && snapshot.tags[0] === 'old', 'cursor value snapshot survives update');
            }
            check(await result(request) === 1, 'write succeeds with the original primary key');
            check(await done === 'complete', 'transaction commits');

            const read = db.transaction(storeName), readDone = terminal(read);
            const readStore = read.objectStore(storeName), index = readStore.index('byTag');
            const [saved, count, primaryKeys, keys] = await Promise.all([
              result(readStore.get(1)), result(index.count()), result(index.getAllKeys()), cursorKeys(index),
            ]);
            check(await readDone === 'complete', 'readback completes');
            const expectedKeys = keyCase.keys();
            check(saved.tags.length === tags.length, 'index deduplication preserves the stored value');
            check(count === expectedKeys.length, 'count includes each key once per record');
            check(primaryKeys.length === expectedKeys.length && primaryKeys.every(key => key === 1),
              'getAllKeys includes each key once per record');
            check(keys.length === expectedKeys.length && keys.every((key, i) => indexedDB.cmp(key, expectedKeys[i]) === 0),
              'cursor keys use IndexedDB equality and ordering without flattening nested arrays');
            cases++;
          }
        }
      }
      context = 'ordinary';
      const tx = db.transaction('ordinary'), done = terminal(tx), index = tx.objectStore('ordinary').index('byTag');
      const [arrayMatches, scalarMatches] = await Promise.all([
        result(index.getAllKeys(['new', 'new'])), result(index.getAllKeys('new')),
      ]);
      check(arrayMatches.length === 1 && scalarMatches.length === 0, 'non-multiEntry array keys remain intact');
      check(await done === 'complete', 'ordinary index read completes');
    } else if (scenario === 'conflicts') {
      for (const method of methods) {
        for (const keyCase of keyCases) {
          for (const cancel of [false, true]) {
            context = `${method}/${keyCase.name}/cancel=${cancel}`;
            const occupied = keyCase.keys()[0];
            const seed = db.transaction('unique', 'readwrite'), seedDone = terminal(seed);
            const seedStore = seed.objectStore('unique');
            seedStore.clear();
            seedStore.add({id: 1, tags: ['old']});
            seedStore.add({id: 2, tags: [occupied]});
            check(await seedDone === 'complete', 'seed commits');

            const tx = db.transaction('unique', 'readwrite'), done = terminal(tx), store = tx.objectStore('unique');
            store.add({id: 4, tags: ['side-effect']});
            let request;
            if (method === 'add' || method === 'put') {
              request = store[method]({id: method === 'add' ? 3 : 1, tags: keyCase.values()});
            } else {
              const source = method === 'index-cursor' ? store.index('byTag') : store;
              const cursor = await result(source.openCursor(method === 'index-cursor' ? 'old' : 1));
              request = cursor.update({id: 1, tags: keyCase.values()});
            }
            const error = await new Promise(resolve => {
              request.onsuccess = () => resolve('unexpected-success');
              request.onerror = event => {
                if (cancel) event.preventDefault();
                resolve(request.error.name);
              };
            });
            check(error === 'ConstraintError', 'another primary key still causes a unique constraint error');
            check(await done === (cancel ? 'complete' : 'ConstraintError'), 'error cancellation controls transaction abort');

            const read = db.transaction('unique'), readDone = terminal(read), readStore = read.objectStore('unique');
            const [original, owner, rejected, sideEffect, occupiedKeys] = await Promise.all([
              result(readStore.get(1)), result(readStore.get(2)), result(readStore.get(3)), result(readStore.get(4)),
              result(readStore.index('byTag').getAllKeys(occupied)),
            ]);
            check(await readDone === 'complete', 'conflict readback completes');
            check(original.tags.length === 1 && original.tags[0] === 'old' && rejected === undefined,
              'rejected write preserves the original record');
            check(indexedDB.cmp(owner.tags[0], occupied) === 0 && occupiedKeys.length === 1 && occupiedKeys[0] === 2,
              'conflicting record retains sole ownership of its key');
            check((sideEffect !== undefined) === cancel, 'uncanceled error rolls back other writes');
            cases++;
          }
        }
      }
    } else {
      throw new Error(`unknown scenario: ${scenario}`);
    }
    return {state: 'pass', cases, checks: multiEntryChecks};
  } finally {
    db.close();
  }
}
