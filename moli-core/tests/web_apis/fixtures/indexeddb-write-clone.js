'use strict';
globalThis.writeChecks = [];
function writeCheck(name, pass, detail = '') {
  writeChecks.push({name, pass: !!pass, detail: String(detail)});
}
function writeThrows(name, callback, expected) {
  try { callback(); writeCheck(name, false, 'no exception'); }
  catch (error) { writeCheck(name, typeof expected === 'string' ? error.name === expected : error === expected, error); }
}
function writeRequest(request) {
  return new Promise(resolve => {
    request.onsuccess = () => resolve(request.result);
    request.onerror = event => { event.preventDefault(); resolve({requestError: request.error.name}); };
  });
}
function writeTransaction(transaction) {
  return new Promise(resolve => {
    transaction.oncomplete = () => resolve('complete');
    transaction.onabort = () => resolve('abort');
    transaction.onerror = event => event.preventDefault();
  });
}
async function writeDatabase(name) {
  const request = indexedDB.open(name, 1);
  request.onupgradeneeded = () => {
    const db = request.result;
    const store = db.createObjectStore('inline', {keyPath: 'id', autoIncrement: true});
    store.createIndex('byTag', 'tag', {unique: true});
    db.createObjectStore('nested', {keyPath: 'a.b.id', autoIncrement: true});
    db.createObjectStore('out', {autoIncrement: true});
    db.createObjectStore('length', {keyPath: 'text.length'});
    db.createObjectStore('special', {keyPath: 'blob.type'});
  };
  return writeRequest(request);
}
async function writeCloneProbe(prefix = 'write-clone-probe') {
  for (const method of ['add', 'put']) {
    for (const queued of [false, true]) {
      const label = `${method}-${queued ? 'queued' : 'started'}`;
      const db = await writeDatabase(`${prefix}-${label}`);
      const stores = Array.from(db.objectStoreNames);
      const blocker = queued ? db.transaction(stores, 'readwrite') : null;
      const blockerDone = blocker && writeTransaction(blocker);
      if (blocker) blocker.objectStore('out').get(0);
      const tx = db.transaction(stores, 'readwrite');
      const done = writeTransaction(tx);
      const store = tx.objectStore('inline');
      const out = tx.objectStore('out');
      const nested = tx.objectStore('nested');
      let idReads = 0, tagReads = 0;
      const record = {
        get id() { idReads++; return 10; },
        get tag() {
          tagReads++;
          writeThrows(`${label}: inactive during clone`, () => out.get(0), 'TransactionInactiveError');
          writeThrows(`${label}: commit during clone`, () => tx.commit(), 'InvalidStateError');
          return 'original';
        },
        data: {value: 'before'}
      };
      const request = store[method](record);
      const saved = writeRequest(request);
      writeCheck(`${label}: getters synchronous and once`, idReads === 1 && tagReads === 1, `${idReads}/${tagReads}`);
      writeCheck(`${label}: accepted request pending`, request.readyState === 'pending');
      record.data.value = 'after';
      const sentinel = {sentinel: label};
      writeThrows(`${label}: exact getter exception`, () => out[method]({get value() { throw sentinel; }}, 'throws'), sentinel);
      writeThrows(`${label}: uncloneable value`, () => out[method](() => {}, 'function'), 'DataCloneError');
      writeThrows(`${label}: invalid inline key`, () => store[method]({id: null, tag: 'invalid'}), 'DataError');
      writeThrows(`${label}: invalid explicit key`, () => out[method]({}, null), 'DataError');
      writeThrows(`${label}: inline explicit key before clone`, () => store[method]({get id() { throw sentinel; }}, 1), 'DataError');
      writeThrows(`${label}: injection through undefined`, () => nested[method]({a: undefined}), 'DataError');
      writeThrows(`${label}: injection through primitive`, () => nested[method]({a: {b: 'text'}}), 'DataError');
      const throwingKey = [];
      Object.defineProperty(throwingKey, '0', {get() { throw sentinel; }, configurable: true});
      writeThrows(`${label}: exact explicit array getter`, () => out[method]({}, throwingKey), sentinel);
      let prototypeReads = 0;
      Object.defineProperty(Object.prototype, 'a', {configurable: true, get() { prototypeReads++; throw sentinel; }});
      let injected;
      try { injected = writeRequest(nested[method]({})); }
      finally { delete Object.prototype.a; }
      writeCheck(`${label}: injection ignores prototype`, prototypeReads === 0, prototypeReads);
      const inherited = Object.create({id: 999});
      inherited.tag = 'generated';
      const generated = writeRequest(store[method](inherited));
      const conflict = writeRequest(store[method]({tag: 'original'}));
      const afterConflict = writeRequest(store[method]({tag: 'after-conflict'}));
      const sparse = Array(1);
      Object.setPrototypeOf(sparse, Object.create(Array.prototype, {0: {get() { throw sentinel; }}}));
      writeThrows(`${label}: array holes ignore inherited getters`, () => out[method]({}, sparse), 'DataError');
      for (const thrown of [null, undefined, 17, 'sentinel']) {
        try { out[method]({get value() { throw thrown; }}, 'primitive-throw'); writeCheck(`${label}: primitive exception ${String(thrown)}`, false); }
        catch (error) { writeCheck(`${label}: primitive exception ${String(thrown)}`, error === thrown, String(error)); }
      }
      const arrayKey = ['before', [2]];
      const arrayWrite = writeRequest(out[method]({data: 'array'}, arrayKey));
      arrayKey[0] = 'after'; arrayKey[1][0] = 9;
      const utf16 = writeRequest(tx.objectStore('length')[method]({text: 'A\u{1F600}'}));
      let blobReads = 0;
      const oldType = Object.getOwnPropertyDescriptor(Blob.prototype, 'type');
      Object.defineProperty(Blob.prototype, 'type', {configurable: true, get() { blobReads++; throw sentinel; }});
      let special;
      try { special = writeRequest(tx.objectStore('special')[method]({blob: new Blob(['x'], {type: 'text/plain'})})); }
      finally { Object.defineProperty(Blob.prototype, 'type', oldType); }
      writeCheck(`${label}: Blob key uses native data`, blobReads === 0, blobReads);
      Object.defineProperties(store, {
        keyPath: {configurable: true, get() { throw sentinel; }},
        autoIncrement: {configurable: true, get() { throw sentinel; }}
      });
      let native;
      try { native = writeRequest(store[method]({id: 20, tag: 'metadata'})); }
      catch (error) { writeCheck(`${label}: native metadata write`, false, error); }
      finally { delete store.keyPath; delete store.autoIncrement; }
      const status = await done;
      if (blockerDone) await blockerDone;
      writeCheck(`${label}: transaction completes after caught exceptions`, status === 'complete', status);
      writeCheck(`${label}: stored primary key`, await saved === 10);
      writeCheck(`${label}: no delayed getters`, idReads === 1 && tagReads === 1, `${idReads}/${tagReads}`);
      writeCheck(`${label}: failed injection preserves generator`, await injected === 1);
      writeCheck(`${label}: missing inherited key generates`, await generated === 11);
      writeCheck(`${label}: unique constraint rejects generated write`, (await conflict)?.requestError === 'ConstraintError');
      writeCheck(`${label}: constraint failure preserves generator`, await afterConflict === 12);
      writeCheck(`${label}: array key snapshot`, JSON.stringify(await arrayWrite) === '["before",[2]]');
      writeCheck(`${label}: String length counts UTF-16`, await utf16 === 3);
      writeCheck(`${label}: Blob type extracted`, await special === 'text/plain');
      if (native) writeCheck(`${label}: native metadata write`, await native === 20);
      const read = db.transaction(stores);
      const readDone = writeTransaction(read);
      const readValue = writeRequest(read.objectStore('inline').get(10));
      const readNested = writeRequest(read.objectStore('nested').get(1));
      const readIndex = writeRequest(read.objectStore('inline').index('byTag').getKey('original'));
      writeCheck(`${label}: value snapshot`, (await readValue)?.data?.value === 'before');
      writeCheck(`${label}: injected nested key`, (await readNested)?.a?.b?.id === 1);
      writeCheck(`${label}: index uses same snapshot`, await readIndex === 10);
      await readDone;
      db.close();
    }
  }
  const db = await writeDatabase(`${prefix}-cursor`);
  let tx = db.transaction('inline', 'readwrite');
  let done = writeTransaction(tx);
  tx.objectStore('inline').put({id: 1, tag: 'one'});
  tx.objectStore('inline').put({id: 2, tag: 'two'});
  await done;
  tx = db.transaction('inline', 'readwrite'); done = writeTransaction(tx);
  let store = tx.objectStore('inline');
  const cursor = await writeRequest(store.openCursor(1));
  const oldValue = cursor.value;
  let reads = 0;
  const newValue = {get id() { reads++; return 1; }, tag: 'updated'};
  const update = writeRequest(cursor.update(newValue));
  writeCheck('cursor: clone getter once', reads === 1, reads);
  writeCheck('cursor: cached value unchanged', cursor.value === oldValue && oldValue.tag === 'one');
  newValue.tag = 'mutated';
  writeThrows('cursor: reject changed inline key', () => cursor.update({id: 3}), 'DataError');
  const exact = {cursor: true};
  writeThrows('cursor: exact clone exception', () => cursor.update({get id() { throw exact; }}), exact);
  const constrained = writeRequest(cursor.update({id: 1, tag: 'two'}));
  const fresh = writeRequest(store.get(1));
  writeCheck('cursor: update result', await update === 1);
  writeCheck('cursor: unique index constraint', (await constrained)?.requestError === 'ConstraintError');
  writeCheck('cursor: stored snapshot after failed unique write', (await fresh)?.tag === 'updated');
  await done;
  tx = db.transaction('inline'); done = writeTransaction(tx); store = tx.objectStore('inline');
  let cloneCalls = 0;
  writeThrows('readonly: reject before cloning', () => store.put({get id() { cloneCalls++; return 3; }}), 'ReadOnlyError');
  writeCheck('readonly: no getters', cloneCalls === 0, cloneCalls);
  const keyCursor = await writeRequest(store.openKeyCursor());
  writeThrows('cursor: readonly before key-only', () => keyCursor.update({}), 'ReadOnlyError');
  writeThrows('cursor: readonly delete', () => keyCursor.delete(), 'ReadOnlyError');
  await done;
  tx = db.transaction('inline', 'readwrite'); done = writeTransaction(tx);
  const writeKeyCursor = await writeRequest(tx.objectStore('inline').openKeyCursor(1));
  writeThrows('cursor: readwrite key-only delete', () => writeKeyCursor.delete(), 'InvalidStateError');
  await done;
  for (const method of ['add', 'put']) {
    for (const queued of [false, true]) {
      const label = `abort-${method}-${queued ? 'queued' : 'started'}`;
      const blocker = queued ? db.transaction('out', 'readwrite') : null;
      const blockerDone = blocker && writeTransaction(blocker);
      if (blocker) blocker.objectStore('out').get(99);
      const aborted = db.transaction('out', 'readwrite');
      const abortedDone = writeTransaction(aborted);
      const target = aborted.objectStore('out');
      let request;
      try {
        request = target[method]({get value() { aborted.abort(); return 'cancelled'; }}, 99);
        writeCheck(`${label}: returns request`, request instanceof IDBRequest && request.readyState === 'pending');
      } catch (error) { writeCheck(`${label}: returns request`, false, error); }
      writeCheck(`${label}: transaction aborts`, await abortedDone === 'abort');
      writeThrows(`${label}: remains inactive`, () => target.get(99), 'TransactionInactiveError');
      if (blockerDone) await blockerDone;
      const verification = db.transaction('out');
      const verificationDone = writeTransaction(verification);
      writeCheck(`${label}: cancelled value absent`, await writeRequest(verification.objectStore('out').get(99)) === undefined);
      await verificationDone;
    }
  }
  tx = db.transaction('inline', 'readwrite'); done = writeTransaction(tx);
  store = tx.objectStore('inline');
  const abortingCursor = await writeRequest(store.openCursor(1));
  try {
    const request = abortingCursor.update({get id() { tx.abort(); return 1; }, tag: 'cancelled'});
    writeCheck('abort-cursor: returns request', request instanceof IDBRequest && request.readyState === 'pending');
  } catch (error) { writeCheck('abort-cursor: returns request', false, error); }
  writeCheck('abort-cursor: transaction aborts', await done === 'abort');
  writeThrows('abort-cursor: remains inactive', () => store.get(1), 'TransactionInactiveError');
  tx = db.transaction(['inline', 'out'], 'readwrite'); done = writeTransaction(tx);
  const preserved = writeRequest(tx.objectStore('inline').get(1));
  const generated = writeRequest(tx.objectStore('out').put({afterAbort: true}));
  writeCheck('abort-cursor: stored value preserved', (await preserved)?.tag === 'updated');
  writeCheck('abort: generator not consumed', await generated === 1);
  await done;
  db.close();
  return {state: writeChecks.every(check => check.pass) ? 'pass' : 'fail', checks: writeChecks};
}
