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
    if (scenario === 'execution-indexes') {
      open.result.createObjectStore('nonunique').createIndex('by-label', 'label');
      open.result.createObjectStore('unique').createIndex('by-label', 'label', {unique: true});
      const inline = open.transaction.objectStore('inline');
      inline.createIndex('by-label', 'label', {unique: true});
      inline.createIndex('by-id', 'id', {unique: true});
    }
    if (scenario === 'host-keys') {
      const store = open.result.createObjectStore('host');
      for (const field of ['size', 'type', 'name', 'lastModified'])
        store.createIndex(field, `payload.${field}`, {unique: field === 'name'});
      open.result.createObjectStore('host-inline', {keyPath: 'payload.name'});
    }
    if (scenario === 'multientry') {
      const multi = open.result.createObjectStore('multi');
      multi.createIndex('by-tag', 'tags', {multiEntry: true});
      multi.createIndex('unique-tags', 'tags', {multiEntry: true, unique: true});
      open.result.createObjectStore('nonunique').createIndex('by-tag', 'tags', {multiEntry: true});
    }
  };
  const db = await result(open);
  try {
    if (scenario === 'host-keys') {
      const blob = new realm.Blob(['hello'], {type: 'text/plain'});
      const file = new realm.File(['world'], 'native.txt', {type: 'text/plain', lastModified: 42});
      const attributes = [[realm.Blob.prototype, 'size'], [realm.Blob.prototype, 'type'],
        [realm.File.prototype, 'name'], [realm.File.prototype, 'lastModified']];
      const descriptors = attributes.map(([object, name]) => Object.getOwnPropertyDescriptor(object, name));
      let reads = 0;
      try {
        for (const [object, name] of attributes) Object.defineProperty(object, name, {
          configurable: true, get() {reads++; throw new Error(`author getter ${name}`);}
        });
        const tx = db.transaction(['host', 'host-inline'], 'readwrite'), done = complete(tx);
        const store = tx.objectStore('host');
        const writes = [result(store.put({payload: blob}, 1)), result(store.put({payload: file}, 2)),
          result(tx.objectStore('host-inline').put({payload: file}))];
        assert(JSON.stringify(await Promise.all(writes)) === '[1,2,"native.txt"]', 'host primary keys changed');
        await done;
        const read = db.transaction('host'), readDone = complete(read), saved = read.objectStore('host');
        const queries = [result(saved.index('size').getAllKeys(5)), result(saved.index('type').getAllKeys('text/plain')),
          result(saved.index('name').getAllKeys('native.txt')), result(saved.index('lastModified').getAllKeys(42))];
        assert(JSON.stringify(await Promise.all(queries)) === '[[1,2],[1,2],[2],[2]]', 'host index keys used author attributes');
        await readDone;
        assert(reads === 0, `host key extraction invoked ${reads} author getters`);
      } finally {
        attributes.forEach(([object, name], index) => Object.defineProperty(object, name, descriptors[index]));
      }
    } else if (scenario === 'queued') {
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
    } else if (scenario === 'key-errors') {
      for (const queued of [false, true]) {
        const stores = ['plain', 'inline', 'generated'];
        let firstDone;
        if (queued) {
          const first = db.transaction(stores, 'readwrite');
          firstDone = complete(first);
          first.objectStore('plain').put('blocker', 'hold');
        }
        const tx = db.transaction(stores, 'readwrite'), done = complete(tx);
        const accepted = [];
        for (const name of stores) {
          const store = tx.objectStore(name);
          const before = requestStateEntries?.(store);
          for (const method of ['put', 'add']) {
            for (const badKey of [{}, null, NaN, [undefined], [null], new realm.Date(NaN)]) {
              throws(() => store[method](() => {}, badKey), 'DataCloneError');
              let reads = 0;
              throws(() => store[method]({get payload() {reads++; return 'snapshot';}}, badKey), 'DataError');
              assert(reads === 1, `${name}/${method}: invalid key skipped value serialization`);
              for (const marker of [new realm.Error('original value'), {}, null, undefined]) {
                throws(() => store[method]({get payload() {throw marker;}}, badKey), marker);
              }
            }
            const order = [];
            const key = Object.defineProperty([1], '0', {get() {order.push('key'); return undefined;}});
            throws(() => store[method]({get payload() {order.push('value'); return 'snapshot';}}, key), 'DataError');
            assert(order.join() === 'key,value', `${name}/${method}: wrong conversion order: ${order}`);
            for (const marker of [new realm.Error('original key'), {}, null, undefined]) {
              let valueReads = 0;
              const key = Object.defineProperty([1], '0', {get() {throw marker;}});
              throws(() => store[method]({get payload() {valueReads++; return 'unused';}}, key), marker);
              assert(valueReads === 0, 'key getter exception did not stop serialization');
            }
          }
          if (requestStateEntries) assert(requestStateEntries(store) === before, 'invalid key allocated a request');
          accepted.push(result(name === 'plain' ? store.put('accepted', 'valid') : store.put({payload: 'accepted'})));
        }
        // These failures also follow Chromium's serialization-first behavior.
        for (const method of ['put', 'add']) {
          throws(() => tx.objectStore('inline')[method](() => {}, 123), 'DataCloneError');
          throws(() => tx.objectStore('plain')[method](() => {}), 'DataCloneError');
        }
        const keys = await Promise.all(accepted);
        assert(JSON.stringify(keys) === JSON.stringify(['valid', queued ? 2 : 1, queued ? 2 : 1]),
          `invalid writes consumed generated keys: ${keys}`);
        await done;
        if (firstDone) await firstDone;
      }
    } else if (scenario === 'execution-indexes') {
      for (const queued of [false, true]) {
        const names = ['plain', 'nonunique', 'unique', 'inline', 'generated'];
        const label = `snapshot-${queued}`;
        let firstDone;
        if (queued) {
          const first = db.transaction(names, 'readwrite');
          firstDone = complete(first);
          first.objectStore('plain').put('blocker', 'hold');
        }
        const tx = db.transaction(names, 'readwrite'), done = complete(tx);
        const writes = names.map(name => {
          let reads = 0;
          const value = {label, date: new realm.Date(1234), get payload() {
            reads++;
            return new realm.Map([['text', 'snapshot']]);
          }};
          const request = name === 'inline' || name === 'generated'
            ? tx.objectStore(name).put(value)
            : tx.objectStore(name).put(value, 'one');
          assert(reads === 1, `${name}: value was not snapshotted at the call`);
          assert(!Object.hasOwn(value, 'id'), `${name}: generated key mutated the original value`);
          value.label = 'changed';
          return result(request);
        });
        const second = result(tx.objectStore('nonunique').add({label}, `two-${queued}`));
        const failures = ['unique', 'inline'].map(name => {
          const store = tx.objectStore(name);
          const request = name === 'inline' ? store.add({label}) : store.add({label}, 'two');
          return new Promise((resolve, reject) => {
            request.onsuccess = () => reject(new Error(`${name}: duplicate unique key was accepted`));
            request.onerror = event => {
              event.preventDefault();
              resolve(request.error.name);
            };
          });
        });
        const keys = await Promise.all(writes);
        await second;
        assert((await Promise.all(failures)).every(name => name === 'ConstraintError'),
          'unique index did not reject duplicate values');
        await done;
        if (firstDone) await firstDone;
        const read = db.transaction(names), readDone = complete(read);
        const values = names.map((name, index) => result(read.objectStore(name).get(keys[index])));
        const nonunique = result(read.objectStore('nonunique').index('by-label').getAll(label));
        const unique = result(read.objectStore('unique').index('by-label').get(label));
        const inline = result(read.objectStore('inline').index('by-id').get(keys[3]));
        for (const value of await Promise.all(values)) {
          assert(value.label === label && value.payload.get('text') === 'snapshot'
            && value.date.getTime() === 1234, 'execution changed the prepared value');
        }
        assert((await nonunique).length === 2, 'nonunique index lost a value');
        assert((await unique).label === label, 'unique index lost the accepted value');
        assert((await inline).id === keys[3], 'generated inline key was not available to the unique index');
        await readDone;
      }
    } else if (scenario === 'multientry') {
      const tx = db.transaction(['multi', 'nonunique'], 'readwrite'), done = complete(tx);
      const multi = tx.objectStore('multi'), nonunique = tx.objectStore('nonunique');
      const tags = [
        -0, 0, 'x', 'x', new realm.Date(0), new realm.Date(0),
        new realm.Uint8Array([1, 2]), new realm.Uint8Array([1, 2]),
        [1, 'a'], [1, 'a'], [[1, 'a']], [[1, 'a']],
        null, {}, undefined, NaN, [undefined]
      ];
      const first = result(multi.put({tags}, 1));
      const nonuniqueWrites = [
        result(nonunique.put({tags: ['shared', 'shared']}, 1)),
        result(nonunique.put({tags: ['shared', 'shared']}, 2))
      ];
      const duplicate = multi.put({tags: ['x', 'x']}, 2);
      const conflict = new Promise((resolve, reject) => {
        duplicate.onsuccess = () => reject(new Error('unique index accepted another record'));
        duplicate.onerror = event => {
          event.preventDefault();
          resolve(duplicate.error.name);
        };
      });
      const last = result(multi.put({tags: ['later', 'later']}, 3));
      assert(await first === 1, 'duplicates in one record rejected its write');
      await Promise.all(nonuniqueWrites);
      assert(await conflict === 'ConstraintError', 'different records did not conflict');
      assert(await last === 3, 'preventDefault did not let the transaction continue');
      await done;
      const read = db.transaction(['multi', 'nonunique']), readDone = complete(read);
      const store = read.objectStore('multi');
      const counts = [];
      for (const name of ['by-tag', 'unique-tags']) {
        const index = store.index(name);
        counts.push(result(index.count()).then(count => assert(count === 7, `${name}: duplicate entries counted`)));
        for (const key of [0, 'x', new realm.Date(0), new realm.Uint8Array([1, 2]), [1, 'a'], [[1, 'a']]]) {
          counts.push(result(index.count(key)).then(count => assert(count === 1, `${name}: equal keys were not deduplicated`)));
        }
        counts.push(result(index.getAllKeys()).then(keys => {
          assert(keys.filter(key => key === 1).length === 6 && keys.filter(key => key === 3).length === 1,
            `${name}: wrong primary-key collection ${keys}`);
        }));
      }
      counts.push(result(read.objectStore('nonunique').index('by-tag').getAllKeys('shared')).then(keys => {
        assert(JSON.stringify(keys) === '[1,2]', 'nonunique entries were deduplicated across records');
      }));
      await Promise.all(counts);
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
