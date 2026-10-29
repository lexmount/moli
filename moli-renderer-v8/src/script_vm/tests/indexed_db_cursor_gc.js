async function prepareIndexedDbCursorGc() {
  const requestResult = request => new Promise((resolve, reject) => {
    request.onsuccess = () => resolve(request.result);
    request.onerror = () => reject(request.error);
  });
  const completed = transaction => new Promise((resolve, reject) => {
    transaction.oncomplete = resolve;
    transaction.onabort = () => reject(transaction.error);
  });
  const opening = indexedDB.open('cursor-gc', 1);
  opening.onupgradeneeded = () => {
    const store = opening.result.createObjectStore('s');
    store.createIndex('byGroup', 'group');
    store.put({group: 7, bytes: new Uint8Array(65536), marker: 'kept'}, 42);
  };
  const db = await requestResult(opening);
  globalThis.cursorGcDiscarded = [];
  globalThis.cursorGcRetained = null;
  for (let i = 0; i < 33; i++) {
    const transaction = db.transaction('s');
    const done = completed(transaction);
    const store = transaction.objectStore('s');
    const source = i % 2 ? store.index('byGroup') : store;
    const request = source.openCursor();
    await new Promise((resolve, reject) => {
      request.onerror = () => reject(request.error);
      request.onsuccess = () => {
        const cursor = request.result;
        // Author cycles exercise indirect ObjectStore/Index native roots too.
        transaction.authorRequest = request;
        source.authorCursor = cursor;
        cursor.authorRequest = request;
        const refs = [request, cursor, transaction, store, source, cursor.value].map(value => new WeakRef(value));
        if (i === 32) {
          globalThis.cursorGcHeld = cursor;
          globalThis.cursorGcRetained = refs;
        } else {
          cursorGcDiscarded.push(refs);
        }
        resolve();
      };
    });
    await done;
  }
  db.close();
  return 'prepared';
}

function checkIndexedDbCursorGc() {
  const check = (value, label) => { if (!value) throw new Error(label); };
  check(cursorGcDiscarded.length === 32, 'all requests delivered after collection');
  check(cursorGcDiscarded.every(group => group.every(ref => ref.deref() === undefined)), 'discarded request/cursor/source cycles collected');
  check(cursorGcRetained.every(ref => ref.deref() !== undefined), 'retained cursor graph alive');
  const cursor = cursorGcHeld;
  check(cursor.key === 42 && cursor.primaryKey === 42 && cursor.direction === 'next', 'completed cursor key state');
  check(cursor.value.marker === 'kept' && cursor.value.bytes.byteLength === 65536, 'completed cursor value');
  check(cursor.request === cursorGcRetained[0].deref() && cursor.request.result === cursor, 'request identity after completion and GC');
  check(cursor.source === cursorGcRetained[4].deref(), 'source identity after completion and GC');
  let error;
  try { cursor.continue(); } catch (caught) { error = caught; }
  check(error && error.name === 'TransactionInactiveError', 'completed cursor mutation still validates transaction');
  return true;
}

function checkReleasedIndexedDbCursorGc() {
  if (!cursorGcRetained.every(ref => ref.deref() === undefined)) throw new Error('released cursor graph was retained');
  return true;
}
