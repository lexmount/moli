globalThis.transactionAttributeChecks = [];
async function transactionAttributesProbe(prefix = 'transaction-attributes') {
  const checks = transactionAttributeChecks;
  const check = (label, pass, actual = '') => checks.push({label, pass, actual: String(actual)});
  const requestResult = request => new Promise((resolve, reject) => {
    request.onsuccess = () => resolve(request.result);
    request.onerror = () => reject(request.error);
  });
  const settled = tx => new Promise(resolve => {
    tx.addEventListener('complete', () => resolve('complete'), {once: true});
    tx.addEventListener('abort', () => resolve('abort'), {once: true});
  });
  const exercise = async (label, Realm, GetterRealm) => {
    const checkHere = (name, pass, actual) => check(label + ': ' + name, pass, actual);
    const attributes = ['db', 'mode', 'error'];
    const descriptors = Object.fromEntries(attributes.map(name => [name,
      Object.getOwnPropertyDescriptor(GetterRealm.IDBTransaction.prototype, name)]));
    const get = (name, receiver) => typeof descriptors[name]?.get === 'function'
      ? descriptors[name].get.call(receiver) : undefined;
    const values = (name, tx, db, mode, error = null) => {
      checkHere(name + ' database identity', get('db', tx) === db);
      checkHere(name + ' mode', get('mode', tx) === mode, get('mode', tx));
      checkHere(name + ' error identity', get('error', tx) === error, get('error', tx)?.name);
    };
    for (const name of attributes) {
      const descriptor = descriptors[name];
      checkHere(name + ' readonly prototype descriptor', typeof descriptor?.get === 'function'
        && descriptor.set === undefined && descriptor.enumerable && descriptor.configurable);
      checkHere(name + ' getter metadata', descriptor?.get?.name === 'get ' + name && descriptor.get.length === 0);
    }
    const name = prefix + '-' + label;
    const opening = Realm.indexedDB.open(name, 1);
    let upgrade;
    opening.onupgradeneeded = () => {
      upgrade = opening.transaction;
      opening.result.createObjectStore('records').add('seed', 'existing');
      values('active upgrade', upgrade, opening.result, 'versionchange');
    };
    const db = await requestResult(opening);
    values('completed upgrade', upgrade, db, 'versionchange');
    const retained = [{tx: upgrade, mode: 'versionchange', error: null}];
    for (const mode of ['readonly', 'readwrite']) {
      const tx = db.transaction('records', mode);
      const done = settled(tx);
      values('created ' + mode, tx, db, mode);
      for (const name of attributes) {
        checkHere(mode + ' inherited ' + name, !Object.hasOwn(tx, name));
        checkHere(mode + ' readonly ' + name, Reflect.set(tx, name, 'author') === false);
      }
      const request = tx.objectStore('records').get('existing');
      request.onsuccess = () => values('request callback ' + mode, tx, db, mode);
      checkHere(mode + ' completes', await done === 'complete');
      values('completed ' + mode, tx, db, mode);
      retained.push({tx, mode, error: null});
    }
    const tx = retained[1].tx;
    const revoked = Proxy.revocable(tx, {}); revoked.revoke();
    let traps = 0;
    const proxy = new Proxy(tx, {get() { ++traps; }, getPrototypeOf() { ++traps; }});
    const invalid = [null, undefined, 1, 'transaction', true, Symbol(), 1n, {},
      Realm.IDBTransaction.prototype, Object.create(Realm.IDBTransaction.prototype), Object.create(tx),
      new Proxy(tx, {}), revoked.proxy, proxy, db];
    for (const name of attributes) {
      for (let i = 0; i < invalid.length; ++i) {
        let error;
        try { get(name, invalid[i]); } catch (caught) { error = caught; }
        checkHere(name + ' rejects invalid receiver ' + i, error !== undefined
          && Object.getPrototypeOf(error) === GetterRealm.TypeError.prototype && error.name === 'TypeError', error?.name);
      }
    }
    checkHere('getter does not invoke Proxy traps', traps === 0, traps);
    Object.setPrototypeOf(tx, null);
    values('prototype removed', tx, db, 'readonly');
    Object.setPrototypeOf(tx, Realm.IDBTransaction.prototype);

    const first = db.transaction('records', 'readwrite');
    const firstDone = settled(first);
    first.objectStore('records').put('first', 'queue');
    const queued = db.transaction('records', 'readwrite');
    const queuedDone = settled(queued);
    values('queued', queued, db, 'readwrite');
    let shadowReads = 0;
    for (const name of attributes) {
      Object.defineProperty(queued, name, {configurable: true, get() { ++shadowReads; throw new Error('author shadow'); }});
    }
    queued.objectStore('records').put('second', 'queue');
    checkHere('first transaction completes', await firstDone === 'complete');
    checkHere('shadowed transaction completes', await queuedDone === 'complete');
    values('shadowed completed', queued, db, 'readwrite');
    checkHere('internal lifecycle does not read author attributes', shadowReads === 0, shadowReads);
    for (const name of attributes) delete queued[name];
    checkHere('deleting shadows restores inherited attributes', queued.db === db && queued.mode === 'readwrite' && queued.error === null);
    retained.push({tx: queued, mode: 'readwrite', error: null});

    const committing = db.transaction('records', 'readwrite');
    const committingDone = settled(committing);
    committing.objectStore('records').put('commit', 'committed');
    committing.commit();
    values('committing', committing, db, 'readwrite');
    checkHere('explicit commit completes', await committingDone === 'complete');
    values('explicit commit completed', committing, db, 'readwrite');
    retained.push({tx: committing, mode: 'readwrite', error: null});

    const aborted = db.transaction('records', 'readwrite');
    const abortedDone = settled(aborted);
    aborted.abort();
    values('explicit abort before event', aborted, db, 'readwrite');
    checkHere('explicit abort dispatches', await abortedDone === 'abort');
    values('explicit abort after event', aborted, db, 'readwrite');
    retained.push({tx: aborted, mode: 'readwrite', error: null});

    let retainedError;
    for (const protection of ['none', 'setter', 'throwing-setter', 'frozen']) {
      const failed = db.transaction('records', 'readwrite');
      const failedDone = settled(failed);
      let setterCalls = 0;
      if (protection.endsWith('setter')) {
        Object.defineProperty(failed, 'error', {configurable: true,
          get() { throw new Error('author error getter'); },
          set() { ++setterCalls; if (protection === 'throwing-setter') throw new Error('author error setter'); },
        });
      } else if (protection === 'frozen') {
        Object.freeze(failed);
      }
      const duplicate = failed.objectStore('records').add('duplicate', 'existing');
      duplicate.onerror = () => {
        checkHere(protection + ' request error identity before abort', get('error', failed) === null);
      };
      checkHere(protection + ' request failure aborts', await failedDone === 'abort');
      checkHere(protection + ' abort bypasses author setter', setterCalls === 0, setterCalls);
      checkHere(protection + ' request reason', duplicate.error.name === 'ConstraintError', duplicate.error.name);
      values(protection + ' failed transaction', failed, db, 'readwrite', duplicate.error);
      if (protection.endsWith('setter')) delete failed.error;
      checkHere(protection + ' public error identity', failed.error === duplicate.error);
      retained.push({tx: failed, mode: 'readwrite', error: duplicate.error});
      retainedError = duplicate.error;
    }

    const prevented = db.transaction('records', 'readwrite');
    const preventedDone = settled(prevented);
    const handled = prevented.objectStore('records').add('duplicate', 'existing');
    handled.onerror = event => { event.preventDefault(); values('prevented request error', prevented, db, 'readwrite'); };
    checkHere('prevented request error commits', await preventedDone === 'complete');
    values('completed prevented error', prevented, db, 'readwrite');
    retained.push({tx: prevented, mode: 'readwrite', error: null});

    const exception = db.transaction('records', 'readwrite');
    const exceptionDone = settled(exception);
    const suppress = event => event.preventDefault();
    globalThis.addEventListener('error', suppress);
    try {
      exception.objectStore('records').get('existing').onsuccess = () => { throw new Error('transaction listener exception'); };
      checkHere('listener exception aborts', await exceptionDone === 'abort');
    } finally { globalThis.removeEventListener('error', suppress); }
    checkHere('listener exception reason', get('error', exception)?.name === 'AbortError', get('error', exception)?.name);
    values('listener exception', exception, db, 'readwrite', exception.error);
    retained.push({tx: exception, mode: 'readwrite', error: exception.error});

    db.close();
    await requestResult(Realm.indexedDB.deleteDatabase(name));
    for (let i = 0; i < retained.length; ++i) {
      const {tx, mode, error} = retained[i];
      values('retained after close and delete ' + i, tx, db, mode, error);
    }
    checkHere('retained error realm', Object.getPrototypeOf(retainedError) === Realm.DOMException.prototype);
    return () => {
      for (let i = 0; i < retained.length; ++i) {
        const {tx, mode, error} = retained[i];
        values('retained after realm removal ' + i, tx, db, mode, error);
      }
      checkHere('error realm retained after removal', Object.getPrototypeOf(retainedError) === Realm.DOMException.prototype);
    };
  };
  await exercise('local', globalThis, globalThis);
  if (typeof document !== 'undefined') {
    const frame = document.createElement('iframe');
    const loaded = new Promise(resolve => frame.onload = resolve);
    frame.srcdoc = '<!doctype html>'; document.documentElement.appendChild(frame); await loaded;
    let localRetained, foreignRetained;
    try {
      localRetained = await exercise('foreign-getter', globalThis, frame.contentWindow);
      foreignRetained = await exercise('foreign-receiver', frame.contentWindow, globalThis);
    } finally { frame.remove(); }
    localRetained(); foreignRetained();
  }
  return {state: checks.every(check => check.pass) ? 'pass' : 'fail', checks};
}
