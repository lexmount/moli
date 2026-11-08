async function transactionConversionProbe(prefix) {
  const checks = [];
  const check = (label, condition) => { if (!condition) throw Error(label); checks.push(label); };
  const open = indexedDB.open(prefix, 1);
  open.onupgradeneeded = () => { open.result.createObjectStore('store'); };
  const db = await new Promise((resolve,reject) => {
    open.onsuccess = () => resolve(open.result);
    open.onerror = () => reject(open.error);
  });
  const throws = (label, callback, name) => {
    let caught; try { callback(); } catch (error) { caught = error; }
    check(label, caught && caught.name === name);
  };
  const events = [];
  const stores = {[Symbol.iterator]() { events.push('iterator'); return ['store'][Symbol.iterator](); }};
  const mode = {toString() { events.push('mode'); return 'readonly'; }};
  const tx = db.transaction(stores, mode);
  const completed = new Promise((resolve,reject) => { tx.oncomplete = resolve; tx.onabort = () => reject(tx.error); });
  check('names converted before mode', events.join(',') === 'iterator,mode');
  let reads = 0, modes = 0;
  const invalid = {get [Symbol.iterator]() { reads++; return 0; }};
  throws('invalid iterator is TypeError', () => db.transaction(invalid,{toString() {modes++;return 'readonly';}}),'TypeError');
  check('iterator read once before mode', reads === 1 && modes === 0);
  throws('empty scope before valid versionchange mode', () => db.transaction([], 'versionchange'),'InvalidAccessError');
  throws('missing scope before valid versionchange mode', () => db.transaction('missing', 'versionchange'),'NotFoundError');
  throws('unrecognized enum converts first', () => db.transaction([], 'unsupported'),'TypeError');
  const sentinel = {};
  let caught;
  try { db.transaction({get [Symbol.iterator]() {throw sentinel;}}, Symbol('mode')); } catch(error) {caught=error;}
  check('original names getter exception', caught === sentinel);
  await completed;
  db.close();
  throws('closed connection before scope validation', () => db.transaction([]),'InvalidStateError');
  throws('closed connection after enum conversion', () => db.transaction('store','unsupported'),'TypeError');
  return {state:'pass',checks};
}
