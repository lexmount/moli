globalThis.liveChecks = [];
async function cursorLiveProbe(name = 'cursor-live-' + Math.random()) {
  const checks = globalThis.liveChecks;
  const check = (label, pass, actual = '') => checks.push({label, pass: !!pass, actual: String(actual)});
  const equal = (label, actual, expected) => check(label, JSON.stringify(actual) === JSON.stringify(expected), JSON.stringify(actual));
  const request = r => new Promise((resolve, reject) => {
    r.onsuccess = () => resolve(r.result);
    r.onerror = event => { event.preventDefault(); reject(r.error); };
  });
  const done = tx => new Promise(resolve => {
    tx.oncomplete = () => resolve('complete'); tx.onabort = () => resolve('abort');
  });
  const opening = indexedDB.open(name, 1);
  opening.onupgradeneeded = () => {
    const store = opening.result.createObjectStore('s');
    store.createIndex('i', 'index');
    store.createIndex('m', 'tags', {multiEntry: true});
  };
  const db = await request(opening);
  const row = (pk, index = pk, label = 'seed') => ({id: pk, index: [index], tags: [[index]], label});
  const setup = records => {
    const tx = db.transaction('s', 'readwrite'), complete = done(tx), store = tx.objectStore('s');
    store.clear();
    for (const [pk, value] of records) store.put(value, [pk]);
    return {tx, complete, store};
  };
  const drain = async (r, cursor) => {
    const values = [];
    while (cursor && values.length < 30) {
      values.push([cursor.key[0], cursor.primaryKey[0], cursor.value?.label]);
      cursor.continue(); cursor = await request(r);
    }
    check('iteration terminates', cursor === null);
    return values;
  };
  try {
    for (const kind of ['store', 'index']) for (const direction of ['next', 'prev', 'nextunique', 'prevunique']) for (const keyOnly of [false, true]) for (const cached of [false, true]) for (const method of ['continue', 'advance', 'seek']) {
      const label = [kind, direction, keyOnly, cached, method].join(' '), reverse = direction.startsWith('prev');
      const {complete, store} = setup([10, 30, 50, 70, 90].map(pk => [pk, row(pk)]));
      const range = IDBKeyRange.bound([10], [90]);
      const source = kind === 'store' ? store : store.index('i');
      const r = source[keyOnly ? 'openKeyCursor' : 'openCursor'](range, direction), cursor = await request(r);
      const first = reverse ? 90 : 10;
      const oldKey = cached ? cursor.key : null, oldValue = cached ? cursor.value : null;
      range.lower[0] = 1000; range.upper[0] = -1;
      store.delete([first]);
      store.delete([reverse ? 70 : 30]);
      store.put(row(reverse ? 80 : 20, reverse ? 80 : 20, 'insert'), [reverse ? 80 : 20]);
      store.put(row(50, 50, 'update'), [50]);
      store.put(row(reverse ? 100 : 0), [reverse ? 100 : 0]);
      if (method === 'advance') cursor.advance(2);
      else if (method === 'seek') cursor.continue([reverse ? 40 : 60]);
      else cursor.continue();
      equal(label + ' pending key stays at old record', cursor.key, [first]);
      equal(label + ' pending value stays at old record', cursor.value?.id, keyOnly ? undefined : first);
      if (cached) check(label + ' pending cached identity', cursor.key === oldKey && cursor.value === oldValue);
      const expected = reverse ? [80, 50, 30, 10] : [20, 50, 70, 90];
      if (method === 'advance') expected.shift();
      if (method === 'seek') expected.splice(0, 2);
      const actual = await drain(r, await request(r));
      equal(label + ' writes visible in original range', actual, expected.map(pk => [pk, pk, keyOnly ? undefined : pk === 50 ? 'update' : pk === 20 || pk === 80 ? 'insert' : 'seed']));
      equal(label + ' complete', await complete, 'complete');
    }

    // Duplicate index keys require both the index key and primary key as the
    // saved boundary; unique directions must skip the whole previous group.
    for (const direction of ['next', 'prev', 'nextunique', 'prevunique']) for (const keyOnly of [false, true]) for (const method of direction.endsWith('unique') ? ['continue', 'advance'] : ['continue', 'advance', 'tuple']) {
      const label = ['duplicates', direction, keyOnly, method].join(' '), reverse = direction.startsWith('prev');
      const {complete, store} = setup([[10, 1], [20, 1], [30, 2], [40, 2], [50, 3], [60, 3]].map(([pk, index]) => [pk, row(pk, index)]));
      const r = store.index('i')[keyOnly ? 'openKeyCursor' : 'openCursor'](null, direction), cursor = await request(r);
      const first = cursor.primaryKey[0];
      store.delete([first]); store.delete([reverse ? 50 : 20]);
      for (const [pk, index] of reverse ? [[55, 3], [65, 3], [45, 2]] : [[15, 1], [5, 1], [25, 2]]) store.put(row(pk, index), [pk]);
      if (method === 'advance') cursor.advance(2);
      else if (method === 'tuple') cursor.continuePrimaryKey([2], [reverse ? 45 : 25]);
      else cursor.continue();
      let expected = reverse ? [[3, 55], [2, 45], [2, 40], [2, 30], [1, 20], [1, 10]] : [[1, 15], [2, 25], [2, 30], [2, 40], [3, 50], [3, 60]];
      if (direction === 'nextunique') expected = [[2, 25], [3, 50]];
      if (direction === 'prevunique') expected = [[2, 30], [1, 10]];
      if (method === 'advance' || method === 'tuple') expected.shift();
      equal(label + ' native tuple boundary', (await drain(r, await request(r))).map(v => v.slice(0, 2)), expected);
      equal(label + ' complete', await complete, 'complete');
    }

    for (const direction of ['next', 'prev', 'nextunique', 'prevunique']) for (const action of ['update', 'delete', 'clear']) {
      const label = ['cursor mutation', direction, action].join(' '), reverse = direction.startsWith('prev');
      const {complete, store} = setup([[10, row(10, 1)], [20, row(20, 2)], [30, row(30, 3)]]);
      const r = store.index('i').openCursor(null, direction), cursor = await request(r), first = reverse ? 30 : 10;
      if (action === 'update') cursor.update(row(first, reverse ? 0 : 4, 'moved'));
      else if (action === 'delete') cursor.delete();
      else { store.clear(); store.put(row(25, 2, 'after-clear'), [25]); }
      cursor.continue();
      equal(label + ' pending value precedes write', cursor.value.index, [reverse ? 3 : 1]);
      let expected = reverse ? [[2, 20, 'seed'], [1, 10, 'seed']] : [[2, 20, 'seed'], [3, 30, 'seed']];
      if (action === 'update') expected.push([reverse ? 0 : 4, first, 'moved']);
      if (action === 'clear') expected = [[2, 25, 'after-clear']];
      equal(label + ' subsequent records', await drain(r, await request(r)), expected);
      equal(label + ' complete', await complete, 'complete');
    }

    for (const direction of ['next', 'prev', 'nextunique', 'prevunique']) {
      const label = 'multiEntry ' + direction, reverse = direction.startsWith('prev');
      const {complete, store} = setup([[10, {...row(10), tags: [[1], [3]]}], [20, {...row(20), tags: [[2]]}], [30, {...row(30), tags: [[4]]}]]);
      const r = store.index('m').openCursor(null, direction), cursor = await request(r);
      cursor.update({...row(reverse ? 30 : 10), tags: reverse ? [[0], [2]] : [[2], [5]]});
      store.put({...row(25), tags: [[2], [3]]}, [25]);
      cursor.continue();
      let expected = reverse ? [[3, 25], [3, 10], [2, 30], [2, 25], [2, 20], [1, 10], [0, 30]] : [[2, 10], [2, 20], [2, 25], [3, 25], [4, 30], [5, 10]];
      if (direction === 'nextunique') expected = [[2, 10], [3, 25], [4, 30], [5, 10]];
      if (direction === 'prevunique') expected = [[3, 10], [2, 20], [1, 10], [0, 30]];
      equal(label + ' removed and new entries', (await drain(r, await request(r))).map(v => v.slice(0, 2)), expected);
      equal(label + ' complete', await complete, 'complete');
    }

    for (const kind of ['store', 'index']) {
      const {complete, store} = setup([10, 30, 50, 70].map(pk => [pk, row(pk)]));
      const source = kind === 'store' ? store : store.index('i');
      const a = source.openCursor(), b = source.openCursor();
      const [left, right] = await Promise.all([request(a), request(b)]);
      store.delete([30]); left.continue();
      store.put(row(30, 30, 'new'), [30]); right.continue();
      store.delete([50]);
      equal(kind + ' first pending cursor retains old unread value', left.value.id, 10);
      equal(kind + ' second pending cursor retains old unread value', right.value.id, 10);
      const [nextLeft, nextRight] = await Promise.all([request(a), request(b)]);
      equal(kind + ' earlier iteration keeps selected record despite later delete', [nextLeft.key[0], nextLeft.value.id], [50, 50]);
      equal(kind + ' later iteration observes preceding insert', [nextRight.key[0], nextRight.value.label], [30, 'new']);
      nextLeft.continue(); nextRight.continue();
      const [lastLeft, lastRight] = await Promise.all([request(a), request(b)]);
      equal(kind + ' next scans see deleted record', [lastLeft.key[0], lastRight.key[0]], [70, 70]);
      equal(kind + ' complete', await complete, 'complete');
    }
  } finally { db.close(); }
  await request(indexedDB.deleteDatabase(name));
  return {state: checks.every(item => item.pass) ? 'pass' : 'fail', checks};
}
