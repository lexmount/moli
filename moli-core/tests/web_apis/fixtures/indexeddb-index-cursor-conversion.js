globalThis.cursorConversionChecks = [];
async function indexCursorConversionProbe(prefix) {
  const checks = cursorConversionChecks;
  const check = (label, pass, actual = '') => checks.push({label, pass: !!pass, actual: String(actual)});
  const equal = (label, actual, expected) => check(label, JSON.stringify(actual) === JSON.stringify(expected), JSON.stringify(actual));
  const watch = (request, convert = value => value) => new Promise(resolve => {
    request.onsuccess = () => resolve({value: convert(request.result)});
    request.onerror = () => resolve({error: request.error.name});
  });
  for (const method of ['openCursor', 'openKeyCursor']) for (const conversion of ['query', 'direction']) {
    const label = method + '/' + conversion, name = prefix + '-' + label;
    const seed = indexedDB.open(name, 1);
    seed.onupgradeneeded = () => {
      const store = seed.result.createObjectStore('s');
      store.createIndex('before', 'key');
      store.put({key: [1], payload: 'seed'}, [1]);
    };
    const seeded = await watch(seed);
    if (seeded.error) throw Error(label + ': seed ' + seeded.error);
    seeded.value.close();
    let reads, conversions = 0, setupError;
    const opening = indexedDB.open(name, 2);
    opening.onupgradeneeded = () => {
      const transaction = opening.transaction, store = transaction.objectStore('s'), index = store.index('before');
      try {
        const first = watch(index.get([1]));
        const rename = () => { ++conversions; index.name = 'converted'; };
        const query = [1];
        Object.defineProperty(query, 0, {get() { rename(); return 1; }});
        const request = conversion === 'query' ? index[method](query)
          : index[method](undefined, {toString() { rename(); return 'next'; }});
        const cursor = watch(request, value => value && ({
          key: value.key, primaryKey: value.primaryKey,
          source: value.source === index, transaction: request.transaction === transaction,
          value: method === 'openCursor' ? value.value.payload : undefined,
        }));
        // The submitted cursor must keep its admission label even if a later
        // synchronous rename is already visible when its SQL command runs.
        index.name = 'later';
        check(label + ' latest lookup keeps the handle', store.index('later') === index);
        const last = watch(index.get([1]));
        reads = Promise.all([first, cursor, last]);
      } catch (error) { setupError = error.name; transaction.abort(); }
    };
    const opened = await watch(opening);
    equal(label + ' upgrade commits', opened.error || setupError || 'complete', 'complete');
    equal(label + ' conversion happens once', conversions, 1);
    if (reads) {
      const [first, cursor, last] = await reads;
      equal(label + ' read admitted before rename', first, {value: {key: [1], payload: 'seed'}});
      const expected = {key: [1], primaryKey: [1], source: true, transaction: true};
      if (method === 'openCursor') expected.value = 'seed';
      equal(label + ' cursor admitted after conversion', cursor, {value: expected});
      equal(label + ' read admitted after later rename', last, first);
    }
    opened.value?.close();
    await watch(indexedDB.deleteDatabase(name));
  }
  return {state: checks.every(check => check.pass) ? 'pass' : 'fail', checks};
}
