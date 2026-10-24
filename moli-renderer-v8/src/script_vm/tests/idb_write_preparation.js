async function(realm, scenario, realmIndex, requestStateEntries) {
  const assert = (value, message) => {
    if (!value) throw new Error(`${scenario}/${realmIndex}: ${message}`);
  };
  const throws = (callback, expected) => {
    let caught = false;
    try { callback(); } catch (error) {
      caught = true;
      assert(typeof expected === 'string' ? error.name === expected : error === expected,
        `wrong exception: ${String(error)}`);
    }
    assert(caught, 'expected synchronous exception');
  };
  const result = request => new Promise((resolve, reject) => {
    request.onsuccess = () => resolve(request.result);
    request.onerror = () => reject(request.error);
  });
  const complete = tx => new Promise((resolve, reject) => {
    tx.oncomplete = resolve;
    tx.onabort = () => reject(tx.error);
  });
  const open = realm.indexedDB.open(`write-preparation-${scenario}-${realmIndex}`, 1);
  open.onupgradeneeded = () => {
    open.result.createObjectStore('plain');
    open.result.createObjectStore('inline', {keyPath: 'id', autoIncrement: true});
    open.result.createObjectStore('nested', {keyPath: 'meta.id', autoIncrement: true});
    open.result.createObjectStore('generated', {autoIncrement: true});
  };
  const db = await result(open);
  try {
    if (scenario === 'queued') {
      const first = db.transaction('plain', 'readwrite');
      const firstDone = complete(first);
      first.objectStore('plain').put('first', 'first');
      const tx = db.transaction('plain', 'readwrite');
      const done = complete(tx), store = tx.objectStore('plain');
      for (const method of ['put', 'add']) {
        throws(() => store[method](() => {}, 'bad'), 'DataCloneError');
        const marker = new realm.Error(`${method} getter`);
        throws(() => store[method]({get payload() {throw marker;}}, 'bad-getter'), marker);
        throws(() => store[method]('bad-key', {}), 'DataError');
      }
      const value = {payload: {text: 'snapshot'}};
      let keyReads = 0;
      const key = [Object.defineProperty([], '0', {enumerable: true, configurable: true,
        get() {keyReads++; return 'snapshot-key';}})];
      const write = result(store.put(value, key));
      value.payload.text = 'changed';
      Object.defineProperty(key[0], '0', {value: 'changed-key'});
      await firstDone;
      assert(JSON.stringify(await write) === '[["snapshot-key"]]', 'queued key was not snapshotted');
      assert(keyReads === 1, `queued key converted ${keyReads} times`);
      await done;
      const read = db.transaction('plain'), readDone = complete(read);
      const saved = await result(read.objectStore('plain').get([['snapshot-key']]));
      assert(saved.payload.text === 'snapshot', 'queued value was not snapshotted');
      await readDone;
    } else if (scenario === 'generated-keys') {
      const tx = db.transaction(['inline', 'generated', 'nested'], 'readwrite');
      const done = complete(tx), requests = [];
      for (const name of ['inline', 'generated', 'nested']) {
        const store = tx.objectStore(name);
        for (const method of ['put', 'add']) {
          throws(() => store[method]({payload: () => {}}), 'DataCloneError');
          if (name !== 'generated') {
            throws(() => store[method](0), 'DataError');
          }
          if (name === 'nested') {
            throws(() => store[method]({meta: 0}), 'DataError');
          }
          const value = {payload: `${name}-${method}`};
          requests.push(result(store[method](value)));
          assert(!Object.hasOwn(value, 'id') && !Object.hasOwn(value, 'meta'), 'mutated original value');
        }
      }
      const keys = await Promise.all(requests);
      assert(JSON.stringify(keys) === '[1,2,1,2,1,2]', `failed writes consumed generated keys: ${keys}`);
      await done;
      const read = db.transaction(['inline', 'nested']), readDone = complete(read);
      const inline = result(read.objectStore('inline').get(1));
      const nested = result(read.objectStore('nested').get(2));
      assert((await inline).id === 1 && (await nested).meta.id === 2, 'generated key was not stored inline');
      await readDone;
    } else if (scenario === 'getters') {
      const tx = db.transaction('inline', 'readwrite'), store = tx.objectStore('inline');
      const done = complete(tx);
      for (const marker of [new realm.Error('original'), {}, null, undefined]) {
        throws(() => store.put({get payload() {throw marker;}}), marker);
      }
      let inheritedReads = 0, inheritedWrite;
      const prototype = realm.Object.prototype;
      const originalId = Object.getOwnPropertyDescriptor(prototype, 'id');
      try {
        Object.defineProperty(prototype, 'id', {configurable: true, get() {
          inheritedReads++;
          throw new Error('inherited key getter');
        }});
        inheritedWrite = result(store.put({payload: 'own-only'}));
      } finally {
        if (originalId) Object.defineProperty(prototype, 'id', originalId);
        else delete prototype.id;
      }
      assert(inheritedReads === 0, 'key extraction invoked a prototype getter');
      let keyReads = 0, payloadReads = 0;
      const value = {
        get id() {keyReads++; return 7;},
        get payload() {
          payloadReads++;
          throws(() => store.get(7), 'TransactionInactiveError');
          throws(() => store.add({id: 99}), 'TransactionInactiveError');
          return 'snapshot';
        }
      };
      const key = await result(store.put(value));
      assert(await inheritedWrite === 1, 'missing own key did not use the key generator');
      assert(keyReads === 1 && payloadReads === 1, `getters ran ${keyReads}/${payloadReads} times`);
      assert(key === 7, 'key came from the wrong snapshot');
      await done;
      const read = db.transaction('inline'), readDone = complete(read);
      const saved = await result(read.objectStore('inline').get(7));
      assert(saved.id === 7 && saved.payload === 'snapshot', 'stored value and primary key disagree');
      await readDone;
    } else if (scenario === 'native-state') {
      const first = db.transaction('plain', 'readwrite'), firstDone = complete(first);
      const queued = db.transaction('plain', 'readwrite'), queuedDone = complete(queued);
      for (const tx of [first, queued]) {
        const store = tx.objectStore('plain');
        const before = requestStateEntries(store);
        for (let index = 0; index < 100; index++) {
          throws(() => store.put(() => {}, 'bad'), 'DataCloneError');
          throws(() => store.add(() => {}, 'bad'), 'DataCloneError');
        }
        assert(requestStateEntries(store) === before, 'rejected write retained native request state');
        store.put('accepted', 'valid');
      }
      await firstDone;
      await queuedDone;
    } else if (scenario === 'cursor-rejections' || scenario === 'cursor-readonly') {
      const seed = db.transaction(['plain', 'inline'], 'readwrite'), seedDone = complete(seed);
      seed.objectStore('plain').put({payload: 'before'}, 'one');
      seed.objectStore('inline').put({id: 1, payload: 'before'});
      await seedDone;
      const tx = db.transaction(['plain', 'inline'], scenario === 'cursor-readonly' ? 'readonly' : 'readwrite');
      const done = complete(tx);
      const updates = ['plain', 'inline'].map(name => new Promise((resolve, reject) => {
        const store = tx.objectStore(name), openCursor = store.openCursor();
        openCursor.onerror = () => reject(openCursor.error);
        openCursor.onsuccess = () => {
          try {
            const cursor = openCursor.result;
            if (!cursor) {resolve(); return;}
            const before = requestStateEntries?.(store);
            if (scenario === 'cursor-readonly') {
              let reads = 0;
              throws(() => cursor.update({get payload() {reads++; return 'after';}}), 'ReadOnlyError');
              throws(() => cursor.delete(), 'ReadOnlyError');
              assert(reads === 0, 'readonly cursor update invoked a getter');
              if (requestStateEntries) assert(requestStateEntries(store) === before, 'readonly update allocated native state');
              cursor.continue();
              return;
            }
            const marker = new realm.Error('cursor getter');
            for (const value of [() => {}, new realm.Event('no-clone'), {get payload() {throw marker;}}]) {
              throws(() => cursor.update(value), typeof value === 'object' && !(value instanceof realm.Event) ? marker : 'DataCloneError');
            }
            if (name === 'inline') throws(() => cursor.update({id: 2}), 'DataError');
            if (requestStateEntries) assert(requestStateEntries(store) === before, 'rejected cursor update retained native state');
            let keyReads = 0, valueReads = 0;
            const original = cursor.value;
            const value = {get payload() {
              valueReads++;
              throws(() => store.get('nested'), 'TransactionInactiveError');
              return 'after';
            }};
            if (name === 'inline') Object.defineProperty(value, 'id', {enumerable: true, get() {keyReads++; return 1;}});
            const update = cursor.update(value);
            assert(update.source === cursor && update.transaction === tx, 'cursor request lost source or transaction');
            assert(valueReads === 1 && keyReads === (name === 'inline' ? 1 : 0), `cursor getters ran ${keyReads}/${valueReads} times`);
            assert(cursor.value === original && cursor.value.payload === 'before', 'update replaced the cursor value snapshot');
            result(update).then(resolve, reject);
          } catch (error) {reject(error);}
        };
      }));
      await Promise.all(updates);
      await done;
      const read = db.transaction(['plain', 'inline']), readDone = complete(read);
      const saved = await Promise.all([
        result(read.objectStore('plain').get('one')),
        result(read.objectStore('inline').get(1))
      ]);
      const expected = scenario === 'cursor-readonly' ? 'before' : 'after';
      assert(saved.every(value => value.payload === expected), 'cursor update did not persist the expected snapshot');
      await readDone;
    } else {
      throw new Error(`unknown scenario: ${scenario}`);
    }
  } finally {
    db.close();
  }
}
