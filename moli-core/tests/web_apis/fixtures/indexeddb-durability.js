globalThis.durabilityChecks = [];
async function durabilityProbe(prefix = 'durability') {
  const checks = durabilityChecks;
  const check = (label, pass, actual = '') => checks.push({label, pass, actual: String(actual)});
  const same = (label, actual, expected) => check(label, JSON.stringify(actual) === JSON.stringify(expected), JSON.stringify(actual));
  const completed = tx => new Promise((resolve, reject) => {
    tx.addEventListener('complete', resolve);
    tx.addEventListener('abort', () => reject(tx.error || new Error('unexpected abort')));
  });
  const requestResult = request => new Promise((resolve, reject) => {
    request.onsuccess = () => resolve(request.result);
    request.onerror = () => reject(request.error);
  });
  const exercise = async (label, Realm, MethodRealm) => {
    const checkHere = (name, pass, actual) => check(label + ': ' + name, pass, actual);
    const sameHere = (name, actual, expected) => same(label + ': ' + name, actual, expected);
    const method = MethodRealm.IDBDatabase.prototype.transaction;
    const descriptor = Object.getOwnPropertyDescriptor(MethodRealm.IDBTransaction.prototype, 'durability');
    const get = receiver => typeof descriptor?.get === 'function' ? descriptor.get.call(receiver) : undefined;
    const throws = (name, action, prototype = MethodRealm.TypeError.prototype, exceptionName = 'TypeError') => {
      let caught, unexpected;
      try { unexpected = action(); } catch (error) { caught = error; }
      if (unexpected?.abort) { try { unexpected.abort(); } catch {} }
      checkHere(name, caught !== undefined && Object.getPrototypeOf(caught) === prototype && caught.name === exceptionName, caught?.name);
    };
    checkHere('readonly prototype descriptor', typeof descriptor?.get === 'function' && descriptor.set === undefined && descriptor.enumerable && descriptor.configurable);
    checkHere('getter metadata', descriptor?.get?.name === 'get durability' && descriptor.get.length === 0);
    checkHere('transaction arity', method.length === 1, method.length);
    const name = prefix + '-' + label;
    const opening = Realm.indexedDB.open(name, 1);
    let upgrade;
    opening.onupgradeneeded = () => {
      upgrade = opening.transaction;
      opening.result.createObjectStore('records');
      checkHere('upgrade default hint', get(upgrade) === 'default', get(upgrade));
      const marker = {};
      let caught;
      try { method.call(opening.result, 'records', 'readonly', {get durability() { throw marker; }}); } catch (error) { caught = error; }
      checkHere('options conversion precedes upgrade validation', caught === marker);
    };
    const db = await requestResult(opening);
    checkHere('completed upgrade retains hint', get(upgrade) === 'default', get(upgrade));
    const cases = [
      [undefined, 'default'], [null, 'default'], [{}, 'default'], [{durability: undefined}, 'default'],
      [{durability: 'default'}, 'default'], [{durability: 'relaxed'}, 'relaxed'], [{durability: 'strict'}, 'strict'],
      [Object.create({durability: 'strict'}), 'strict'], [{durability: {toString() { return 'relaxed'; }}}, 'relaxed'],
    ];
    for (let i = 0; i < cases.length; ++i) {
      const [options, expected] = cases[i];
      const tx = method.call(db, 'records', 'readwrite', options);
      checkHere('created transaction realm ' + i, Object.getPrototypeOf(tx) === Realm.IDBTransaction.prototype);
      checkHere('hint ' + i, tx.durability === expected, tx.durability);
      checkHere('inherited hint ' + i, !Object.hasOwn(tx, 'durability'));
      checkHere('readonly hint ' + i, Reflect.set(tx, 'durability', 'invalid') === false);
      const done = completed(tx);
      tx.objectStore('records').put(i, i);
      await done;
      checkHere('completed hint ' + i, get(tx) === expected, get(tx));
    }
    for (const value of [1, true, 'strict', Symbol(), 1n]) {
      throws('reject primitive options ' + String(value), () => method.call(db, 'records', 'readwrite', value));
    }
    for (const value of [null, '', 'STRICT', 'invalid', 1, true, Symbol(), 1n]) {
      throws('reject invalid hint ' + String(value), () => method.call(db, 'records', 'readonly', {durability: value}));
    }
    const order = [];
    const tx = method.call(db, {
      [Symbol.iterator]() { order.push('stores'); return ['records'][Symbol.iterator](); }
    }, {toString() { order.push('mode'); return 'readonly'; }}, new Proxy({}, {
      get(_target, key) { order.push(key); return {toString() { order.push('hint'); return 'strict'; }}; }
    }));
    const convertedDone = completed(tx);
    sameHere('conversion order', order, ['stores', 'mode', 'durability', 'hint']);
    checkHere('converted hint', get(tx) === 'strict', get(tx));
    await convertedDone;
    const marker = {};
    let reads = 0;
    const throwingOptions = {get durability() { ++reads; throw marker; }};
    for (const [stores, mode] of [['records', 'readonly'], [[], 'readonly'], ['missing', 'readonly'], ['records', 'versionchange']]) {
      let caught;
      try { method.call(db, stores, mode, throwingOptions); } catch (error) { caught = error; }
      checkHere('options exception precedes semantic validation ' + stores + '/' + mode, caught === marker);
    }
    checkHere('options read exactly once per conversion', reads === 4, reads);
    reads = 0;
    throws('invalid mode precedes options', () => method.call(db, 'records', 'invalid', throwingOptions));
    checkHere('invalid mode skips options', reads === 0, reads);
    const typeMarker = {};
    let caught;
    try { method.call(db, 'records', 'readonly', {durability: {toString() { throw typeMarker; }}}); } catch (error) { caught = error; }
    checkHere('hint conversion exception identity', caught === typeMarker);
    const revoked = Proxy.revocable(tx, {}); revoked.revoke();
    let traps = 0;
    const proxy = new Proxy(tx, {get() { ++traps; }, getPrototypeOf() { ++traps; }});
    for (const value of [null, undefined, 1, {}, Realm.IDBTransaction.prototype, Object.create(tx), new Proxy(tx, {}), revoked.proxy, proxy, db]) {
      throws('getter receiver', () => get(value));
    }
    checkHere('getter skips Proxy traps', traps === 0, traps);
    for (const value of [{}, Object.create(db), new Proxy(db, {})]) {
      reads = 0;
      throws('method receiver precedes options', () => method.call(value, 'records', 'readonly', throwingOptions));
      checkHere('invalid receiver skips options', reads === 0, reads);
    }
    Object.setPrototypeOf(tx, null);
    checkHere('native identity survives prototype removal', get(tx) === 'strict', get(tx));
    Object.setPrototypeOf(tx, Realm.IDBTransaction.prototype);

    const first = method.call(db, 'records', 'readwrite', {durability: 'strict'});
    const firstDone = completed(first);
    first.objectStore('records').put('first', 'queued');
    const queued = method.call(db, 'records', 'readwrite', {durability: 'relaxed'});
    checkHere('queued hint', get(queued) === 'relaxed', get(queued));
    Object.defineProperty(queued, 'durability', {configurable: true, get() { throw new Error('author shadow'); }});
    const queuedDone = completed(queued);
    queued.objectStore('records').put('second', 'queued');
    await Promise.all([firstDone, queuedDone]);
    checkHere('author shadow cannot change commit hint', get(queued) === 'relaxed', get(queued));
    delete queued.durability;
    checkHere('restored completed hint', queued.durability === 'relaxed', queued.durability);
    const read = method.call(db, 'records', 'readonly', {durability: 'strict'});
    const readDone = completed(read);
    const result = read.objectStore('records').get('queued');
    await readDone;
    checkHere('queued writes commit in order', result.result === 'second', result.result);
    checkHere('readonly hint retained', get(read) === 'strict', get(read));
    const outer = method.call(db, 'records', 'readwrite', {durability: 'strict'});
    const reentered = new Promise((resolve, reject) => {
      outer.onabort = () => reject(outer.error);
      outer.oncomplete = () => {
        try {
          const inner = method.call(db, 'records', 'readwrite', {durability: 'relaxed'});
          const done = completed(inner);
          inner.objectStore('records').put('callback', 'reentry');
          done.then(() => resolve(get(inner)), reject);
        } catch (error) { reject(error); }
      };
    });
    outer.objectStore('records').put('outer', 'reentry');
    checkHere('complete callback can commit another write', await reentered === 'relaxed');
    const aborted = method.call(db, 'records', 'readwrite', {durability: 'strict'});
    const abortedDone = new Promise(resolve => aborted.onabort = resolve);
    aborted.abort(); await abortedDone;
    checkHere('abort retains hint', get(aborted) === 'strict', get(aborted));
    const failed = method.call(db, 'records', 'readwrite', {durability: 'relaxed'});
    const failedDone = new Promise(resolve => failed.onabort = resolve);
    failed.objectStore('records').add('duplicate', 'queued'); await failedDone;
    checkHere('failed request retains hint', get(failed) === 'relaxed', get(failed));
    checkHere('failed request abort reason', failed.error?.name === 'ConstraintError', failed.error?.name);
    db.close();
    caught = undefined;
    try { method.call(db, 'records', 'readonly', throwingOptions); } catch (error) { caught = error; }
    checkHere('options conversion precedes closed validation', caught === marker);
    throws('valid hint on closed database', () => method.call(db, 'records', 'readonly', {durability: 'strict'}), MethodRealm.DOMException.prototype, 'InvalidStateError');
    await requestResult(Realm.indexedDB.deleteDatabase(name));
  };
  await exercise('local', globalThis, globalThis);
  if (typeof document !== 'undefined') {
    const frame = document.createElement('iframe');
    const loaded = new Promise(resolve => frame.onload = resolve);
    frame.srcdoc = '<!doctype html>'; document.documentElement.appendChild(frame); await loaded;
    try {
      await exercise('foreign-method', globalThis, frame.contentWindow);
      await exercise('foreign-receiver', frame.contentWindow, globalThis);
    } finally { frame.remove(); }
  }
  return {state: checks.every(check => check.pass) ? 'pass' : 'fail', checks};
}
