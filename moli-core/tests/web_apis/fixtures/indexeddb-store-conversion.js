globalThis.storeConversionChecks = [];
async function storeConversionProbe(prefix) {
  const checks = storeConversionChecks;
  const check = (label, pass, actual = '') => checks.push({label, pass: !!pass, actual: String(actual)});
  const equal = (label, actual, expected) => check(label, JSON.stringify(actual) === JSON.stringify(expected), JSON.stringify(actual));
  const watch = (request, convert = value => value) => new Promise(resolve => {
    request.onsuccess = () => resolve({value: convert(request.result)});
    request.onerror = () => resolve({error: request.error.name});
  });
  const seedValue = {key: [1], payload: 'seed'};
  for (const kind of ['store', 'index']) {
    const methods = ['get', 'getKey', 'count', 'getAll', 'getAllKeys', 'openCursor', 'openKeyCursor'];
    if (kind === 'store') methods.push('delete', 'put');
    for (const method of methods) for (const conversion of method.includes('Cursor') ? ['query', 'direction'] : ['query']) {
      const label = [kind, method, conversion].join('/'), name = prefix + '-' + label;
      const seed = indexedDB.open(name, 1);
      seed.onupgradeneeded = () => {
        const store = seed.result.createObjectStore('before');
        store.createIndex('index', 'key'); store.put(seedValue, [1]);
      };
      const seeded = await watch(seed);
      if (seeded.error) throw Error(label + ': seed ' + seeded.error);
      seeded.value.close();
      let reads, conversions = 0, setupError;
      const opening = indexedDB.open(name, 2);
      opening.onupgradeneeded = () => {
        const transaction = opening.transaction, store = transaction.objectStore('before');
        const source = kind === 'store' ? store : store.index('index');
        try {
          const first = watch(store.get([1]));
          const rename = () => { ++conversions; store.name = 'converted'; };
          const query = [1];
          Object.defineProperty(query, 0, {get() { rename(); return 1; }});
          const changed = {key: [1], payload: 'changed'};
          const request = method === 'put' ? store.put(changed, query)
            : conversion === 'query' ? source[method](query)
            : source[method](undefined, {toString() { rename(); return 'next'; }});
          check(label + ' request keeps source and transaction', request.source === source && request.transaction === transaction);
          const current = watch(request, value => method.includes('Cursor') && value
            ? {key: value.key, primaryKey: value.primaryKey, source: value.source === source,
              ...(method === 'openCursor' ? {value: value.value} : {})} : value);
          // Commands already submitted must retain the name at admission.
          store.name = 'later';
          check(label + ' latest lookup keeps the handle', transaction.objectStore('later') === store);
          reads = Promise.all([first, current, watch(store.get([1]))]);
        } catch (error) { setupError = error.name; transaction.abort(); }
      };
      const opened = await watch(opening);
      equal(label + ' upgrade commits', opened.error || setupError || 'complete', 'complete');
      equal(label + ' conversion happens once', conversions, 1);
      if (reads) {
        const [first, current, last] = await reads;
        equal(label + ' earlier read survives rename', first, {value: seedValue});
        let expected = seedValue;
        if (method === 'getKey' || method === 'put') expected = [1];
        else if (method === 'count') expected = 1;
        else if (method === 'delete') expected = undefined;
        else if (method === 'getAll') expected = [seedValue];
        else if (method === 'getAllKeys') expected = [[1]];
        else if (method.includes('Cursor')) expected = {key: [1], primaryKey: [1], source: true,
          ...(method === 'openCursor' ? {value: seedValue} : {})};
        equal(label + ' request uses converted name', current, {value: expected});
        equal(label + ' later read sees ordered changes', last, {value: method === 'delete' ? undefined
          : method === 'put' ? {key: [1], payload: 'changed'} : seedValue});
      }
      if (opened.value) {
        equal(label + ' committed name', Array.from(opened.value.objectStoreNames), ['later']);
        opened.value.close();
      }
      await watch(indexedDB.deleteDatabase(name));
    }
  }
  // Structured clone intentionally runs with an inactive transaction. A
  // getter cannot rename the store; catching that error may still allow update.
  for (const catchRename of [false, true]) {
    const name = prefix + '-clone-' + catchRename, label = 'clone/' + catchRename;
    let update, renameError, updateError;
    const opening = indexedDB.open(name, 1);
    opening.onupgradeneeded = () => {
      const store = opening.result.createObjectStore('before'); store.put(seedValue, [1]);
      update = new Promise(resolve => {
        const cursor = store.openCursor();
        cursor.onsuccess = () => {
          try {
            const value = {key: [1], get payload() {
              try { store.name = 'forbidden'; }
              catch (error) { renameError = error.name; if (!catchRename) throw error; }
              return 'changed';
            }};
            watch(cursor.result.update(value)).then(resolve);
          } catch (error) { updateError = error.name; resolve({error: updateError}); }
        };
        cursor.onerror = () => resolve({error: cursor.error.name});
      });
    };
    const opened = await watch(opening);
    equal(label + ' upgrade commits', opened.error || 'complete', 'complete');
    equal(label + ' clone cannot rename', renameError, 'TransactionInactiveError');
    if (update) equal(label + ' update outcome', await update, catchRename ? {value: [1]} : {error: 'TransactionInactiveError'});
    if (opened.value) {
      equal(label + ' name is unchanged', Array.from(opened.value.objectStoreNames), ['before']);
      equal(label + ' persisted value', await watch(opened.value.transaction('before').objectStore('before').get([1])),
        {value: catchRename ? {key: [1], payload: 'changed'} : seedValue});
      opened.value.close();
    }
    await watch(indexedDB.deleteDatabase(name));
  }
  return {state: checks.every(check => check.pass) ? 'pass' : 'fail', checks};
}
