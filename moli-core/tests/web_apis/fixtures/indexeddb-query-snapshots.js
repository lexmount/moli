globalThis.queryChecks = [];
async function querySnapshotProbe(name = 'query-snapshots-' + Math.random()) {
  const checks = globalThis.queryChecks;
  const check = (name, pass, detail) => checks.push({name, pass: !!pass, detail});
  const request = req => new Promise(resolve => {
    req.onsuccess = () => resolve({value: req.result});
    req.onerror = event => { event.preventDefault(); resolve({error: req.error}); };
  });
  const done = tx => new Promise(resolve => {
    tx.oncomplete = () => resolve(true);
    tx.onabort = () => resolve(false);
  });
  const open = indexedDB.open(name, 1);
  const seeds = [
    ['array', [1]], ['nested', [[1]]], ['date', new Date(1)]
  ];
  open.onupgradeneeded = () => {
    const store = open.result.createObjectStore('keys');
    store.createIndex('byKey', 'key');
    for (const [label, key] of seeds) store.put({label, key}, key);
    const deletes = open.result.createObjectStore('deletes');
    deletes.createIndex('byId', 'id');
  };
  const db = (await request(open)).value;
  const variants = [
    ['array', () => { const key = [1]; return {key, mutate: () => key[0] = 99}; }],
    ['nested', () => { const key = [[1]]; return {key, mutate: () => key[0][0] = 99}; }],
    ['date', () => { const key = new Date(1); return {key, mutate: () => key.setTime(99)}; }],
    ['array', () => {
      let reads = 0;
      const key = Object.defineProperty([], 0, {get() {
        if (++reads > 1) throw new Error('key converted again');
        return 1;
      }});
      return {key, mutate() {}, reads: () => reads};
    }]
  ];
  const block = storeName => {
    const tx = db.transaction(storeName,'readwrite');
    const completion = done(tx);
    tx.objectStore(storeName).get('barrier');
    return completion;
  };
  try {
    for (const mode of ['pending', 'started']) {
      for (const sourceName of ['store', 'index']) {
        for (const method of ['get', 'getKey', 'getAll', 'getAllKeys', 'count', 'openCursor', 'openKeyCursor']) {
          for (const [i, [label, create]] of variants.entries()) {
            const unblocked = mode === 'pending' ? block('keys') : Promise.resolve(true);
            const tx = db.transaction('keys', mode === 'pending' ? 'readwrite' : 'readonly');
            const completion = done(tx), store = tx.objectStore('keys');
            if (mode === 'started') await request(store.get('barrier'));
            const source = sourceName === 'store' ? store : store.index('byKey');
            const input = create();
            const result = request(source[method](input.key));
            input.mutate();
            const actual = await result;
            const expected = seeds.find(entry => entry[0] === label)[1];
            let matches = false;
            if (!actual.error) {
              const value = actual.value;
              if (method === 'get') matches = value && value.label === label;
              else if (method === 'getKey') matches = value !== undefined && indexedDB.cmp(value,expected) === 0;
              else if (method === 'getAll') matches = value.length === 1 && value[0].label === label;
              else if (method === 'getAllKeys') matches = value.length === 1 && indexedDB.cmp(value[0],expected) === 0;
              else if (method === 'count') matches = value === 1;
              else matches = value && indexedDB.cmp(value.key,expected) === 0 && indexedDB.cmp(value.primaryKey,expected) === 0;
            }
            check(`${mode} ${sourceName}.${method} input ${i}`, matches && (!input.reads || input.reads() === 1), String(actual.error || ''));
            check(`${mode} ${sourceName}.${method} input ${i} completes`, await completion && await unblocked);
          }
        }
      }
    }
    for (const sourceName of ['store','index']) {
      for (const method of ['getAll','getAllKeys']) {
        const unblocked = block('keys');
        const tx = db.transaction('keys','readwrite'), completion = done(tx), store = tx.objectStore('keys');
        const source = sourceName === 'store' ? store : store.index('byKey');
        const key = [1];
        const reads = {query:0,count:0,direction:0};
        const options = {
          get query() { reads.query++; return key; },
          get count() { reads.count++; return 1; },
          get direction() { reads.direction++; return 'prev'; }
        };
        const pending = request(source[method](options));
        key[0] = 99;
        const result = await pending;
        check(`${sourceName}.${method} options query snapshot`, !result.error && result.value.length === 1);
        check(`${sourceName}.${method} options read once`, Object.values(reads).every(count => count === 1));
        check(`${sourceName}.${method} options completes`, await completion && await unblocked);
      }
    }
    // Capture the query at call time, but read records after preceding writes.
    {
      const writer = db.transaction('keys','readwrite'), written = done(writer);
      writer.objectStore('keys').put({label:'new value',key:[1]},[1]);
      const reader = db.transaction('keys','readwrite'), read = done(reader), key = [1];
      const pending = request(reader.objectStore('keys').get(key));
      key[0] = 99;
      const result = await pending;
      check('waiting read sees preceding committed write using original key', !result.error && result.value && result.value.label === 'new value');
      check('waiting read and preceding writer complete', await written && await read);
    }
    const seedDeletes = async keys => {
      const tx = db.transaction('deletes','readwrite'), completion = done(tx), store = tx.objectStore('deletes');
      store.clear();
      keys.forEach((key,i) => store.put({id:i+1},key));
      if (!await completion) throw new Error('delete setup aborted');
    };
    for (const mode of ['pending','started']) {
      for (const [i, [label, create]] of variants.entries()) {
        const expected = seeds.find(entry => entry[0] === label)[1];
        await seedDeletes([expected]);
        const unblocked = mode === 'pending' ? block('deletes') : Promise.resolve(true);
        const tx = db.transaction('deletes','readwrite'), completion = done(tx), store = tx.objectStore('deletes');
        if (mode === 'started') await request(store.get('barrier'));
        const input = create(), pending = request(store.delete(input.key));
        input.mutate();
        const result = await pending;
        const count = await request(store.count());
        check(`${mode} delete input ${i}`, !result.error && result.value === undefined && count.value === 0 && (!input.reads || input.reads() === 1));
        check(`${mode} delete input ${i} completes`, await completion && await unblocked);
      }
    }
    const ranges = [
      [() => IDBKeyRange.bound(2,4), [1,5]],
      [() => IDBKeyRange.bound(2,4,true), [1,2,5]],
      [() => IDBKeyRange.bound(2,4,false,true), [1,4,5]],
      [() => IDBKeyRange.bound(2,4,true,true), [1,2,4,5]],
      [() => IDBKeyRange.lowerBound(3), [1,2]],
      [() => IDBKeyRange.upperBound(3,true), [3,4,5]],
      [() => IDBKeyRange.only(3), [1,2,4,5]],
      [() => IDBKeyRange.bound(10,20), [1,2,3,4,5]]
    ];
    for (const mode of ['pending','started']) {
      for (const [i,[range,expected]] of ranges.entries()) {
        await seedDeletes([1,2,3,4,5]);
        const unblocked = mode === 'pending' ? block('deletes') : Promise.resolve(true);
        const tx = db.transaction('deletes','readwrite'), completion = done(tx), store = tx.objectStore('deletes');
        if (mode === 'started') await request(store.get('barrier'));
        let result;
        try { result = await request(store.delete(range())); }
        catch (error) { result = {error}; }
        const keys = await request(store.getAllKeys());
        const indexKeys = await request(store.index('byId').getAllKeys());
        check(`${mode} delete range ${i}`, !result.error && result.value === undefined && JSON.stringify(keys.value) === JSON.stringify(expected));
        check(`${mode} delete range ${i} index agrees`, JSON.stringify(indexKeys.value) === JSON.stringify(expected));
        check(`${mode} delete range ${i} completes`, await completion && await unblocked);
      }
    }
    {
      const tx = db.transaction('deletes'), completion = done(tx), store = tx.objectStore('deletes');
      let reads = 0, error;
      const key = Object.defineProperty([],0,{get(){reads++;throw new Error('conversion');}});
      try { store.delete(key); } catch (caught) { error = caught; }
      check('readonly delete rejects before key conversion', error && error.name === 'ReadOnlyError' && reads === 0);
      check('readonly rejection leaves transaction usable', (await request(store.count())).value === 5);
      check('readonly transaction completes', await completion);
    }
    {
      await seedDeletes([1,2,3,4,5]);
      const tx = db.transaction('deletes','readwrite'), completion = done(tx), store = tx.objectStore('deletes');
      let deleted;
      try { deleted = request(store.delete(IDBKeyRange.bound(2,4))); }
      catch (error) { deleted = Promise.resolve({error}); }
      tx.abort();
      await deleted;
      check('range delete transaction aborts', !await completion);
      const reader = db.transaction('deletes'), read = done(reader);
      const result = await request(reader.objectStore('deletes').getAllKeys());
      check('range deletion rolls back', JSON.stringify(result.value) === '[1,2,3,4,5]');
      check('rollback verification completes', await read);
    }
  } finally { db.close(); }
  await request(indexedDB.deleteDatabase(name));
  return {state:checks.every(check => check.pass)?'pass':'fail',checks};
}
