globalThis.databaseAttributeChecks = [];
async function databaseAttributesProbe(prefix = 'database-attributes') {
  const checks = databaseAttributeChecks;
  const check = (label, pass, actual = '') => checks.push({label, pass, actual: String(actual)});
  const requestResult = request => new Promise((resolve, reject) => {
    request.onsuccess = () => resolve(request.result);
    request.onerror = () => reject(request.error);
  });
  const completed = tx => new Promise((resolve, reject) => {
    tx.addEventListener('complete', resolve, {once: true});
    tx.addEventListener('abort', () => reject(tx.error || new Error('unexpected abort')), {once: true});
  });
  const exercise = async (label, Realm, GetterRealm) => {
    const checkHere = (name, pass, actual) => check(label + ': ' + name, pass, actual);
    const attributes = ['name', 'version'];
    const descriptors = Object.fromEntries(attributes.map(name => [name,
      Object.getOwnPropertyDescriptor(GetterRealm.IDBDatabase.prototype, name)]));
    const get = (name, receiver) => typeof descriptors[name]?.get === 'function'
      ? descriptors[name].get.call(receiver) : undefined;
    const values = (stage, db, name, version) => {
      checkHere(stage + ' native name', get('name', db) === name, get('name', db));
      checkHere(stage + ' native version', get('version', db) === version, get('version', db));
    };
    for (const name of attributes) {
      const descriptor = descriptors[name];
      checkHere(name + ' readonly prototype descriptor', typeof descriptor?.get === 'function'
        && descriptor.set === undefined && descriptor.enumerable && descriptor.configurable);
      checkHere(name + ' getter metadata', descriptor?.get?.name === 'get ' + name && descriptor.get.length === 0);
    }

    const name = prefix + '-' + label + '-\0-数据库-💾';
    const opening = Realm.indexedDB.open(name, 1);
    let upgrading;
    opening.onupgradeneeded = () => {
      upgrading = opening.result;
      values('initial upgrade', upgrading, name, 1);
      upgrading.createObjectStore('records').add('seed', 'existing');
      opening.transaction.oncomplete = () => values('upgrade complete callback', upgrading, name, 1);
    };
    const db = await requestResult(opening);
    checkHere('open preserves connection identity', db === upgrading);
    values('open success', db, name, 1);
    const retained = [{db, name, version: 1}];
    for (const attribute of attributes) {
      checkHere(attribute + ' inherited', !Object.hasOwn(db, attribute));
      checkHere(attribute + ' readonly', Reflect.set(db, attribute, 'author') === false);
    }
    values('after attempted assignment', db, name, 1);
    const peer = await requestResult(Realm.indexedDB.open(name));
    checkHere('second open creates a separate connection', peer !== db);
    values('second connection', peer, name, 1);
    retained.push({db: peer, name, version: 1});

    const revoked = Proxy.revocable(db, {}); revoked.revoke();
    let traps = 0;
    const proxy = new Proxy(db, {get() { ++traps; }, getPrototypeOf() { ++traps; }});
    const invalid = [null, undefined, 1, 'database', true, Symbol(), 1n, {},
      Realm.IDBDatabase.prototype, Object.create(Realm.IDBDatabase.prototype), Object.create(db),
      new Proxy(db, {}), revoked.proxy, proxy, opening];
    for (const attribute of attributes) {
      for (let i = 0; i < invalid.length; ++i) {
        let error;
        try { get(attribute, invalid[i]); } catch (caught) { error = caught; }
        checkHere(attribute + ' invalid receiver ' + i, error !== undefined
          && Object.getPrototypeOf(error) === GetterRealm.TypeError.prototype && error.name === 'TypeError', error?.name);
      }
    }
    checkHere('getter skips Proxy traps', traps === 0, traps);
    Object.setPrototypeOf(db, null);
    values('prototype removed', db, name, 1);
    Object.setPrototypeOf(db, Realm.IDBDatabase.prototype);

    let authorReads = 0;
    const authorGetter = () => { ++authorReads; throw new Error('author database attribute'); };
    const authorPrototype = Object.create(Realm.IDBDatabase.prototype);
    Object.setPrototypeOf(db, authorPrototype);
    for (const attribute of attributes) {
      Object.defineProperty(db, attribute, {configurable: true, enumerable: false, get: authorGetter});
    }
    const write = db.transaction('records', 'readwrite');
    const writeDone = completed(write);
    write.objectStore('records').put('value', 'written');
    await writeDone;
    values('shadowed completed transaction', db, name, 1);
    checkHere('transaction does not read author attributes', authorReads === 0, authorReads);
    for (const attribute of attributes) {
      const descriptor = Object.getOwnPropertyDescriptor(db, attribute);
      checkHere('commit preserves author ' + attribute, descriptor?.get === authorGetter
        && descriptor.configurable && !descriptor.enumerable);
    }
    checkHere('commit preserves author prototype', Object.getPrototypeOf(db) === authorPrototype);
    Object.setPrototypeOf(db, Realm.IDBDatabase.prototype);
    for (const attribute of attributes) delete db[attribute];
    checkHere('deleting shadows restores attributes', db.name === name && db.version === 1);
    const read = db.transaction('records');
    const readDone = completed(read);
    const stored = read.objectStore('records').get('written');
    stored.onsuccess = () => values('request on closing connection', db, name, 1);
    db.close();
    values('close pending', db, name, 1);
    await readDone;
    checkHere('shadowed transaction committed to original database', stored.result === 'value', stored.result);
    values('closed', db, name, 1);
    peer.close();

    for (const fresh of [false, true]) {
      for (const protection of ['none', 'accessor', 'locked', 'sealed', 'frozen']) {
        const caseName = (fresh ? 'new ' : 'existing ') + protection;
        const abortName = fresh ? name + '-' + protection : name;
        const oldVersion = fresh ? 0 : 1;
        const requestedVersion = fresh ? 7 : 2;
        const failedOpen = Realm.indexedDB.open(abortName, requestedVersion);
        let failedDb, verifyProtection;
        const failed = new Promise((resolve, reject) => {
          failedOpen.onsuccess = () => { failedOpen.result.close(); reject(new Error('upgrade unexpectedly committed')); };
          failedOpen.onerror = event => { event.preventDefault(); resolve(failedOpen.error); };
        });
        failedOpen.onupgradeneeded = () => {
          failedDb = failedOpen.result;
          const tx = failedOpen.transaction;
          values(caseName + ' upgrade', failedDb, abortName, requestedVersion);
          let reads = 0, writes = 0;
          const getter = () => { ++reads; throw new Error('author version getter'); };
          const setter = () => { ++writes; throw new Error('author version setter'); };
          if (protection === 'accessor') {
            Object.defineProperty(failedDb, 'version', {configurable: true, enumerable: false, get: getter, set: setter});
          } else if (protection === 'locked') {
            Object.defineProperty(failedDb, 'version', {value: 'author version', writable: false, configurable: false, enumerable: false});
          } else if (protection === 'sealed') {
            Object.seal(failedDb);
          } else if (protection === 'frozen') {
            Object.freeze(failedDb);
          }
          verifyProtection = () => {
            const descriptor = Object.getOwnPropertyDescriptor(failedDb, 'version');
            if (protection === 'accessor') {
              checkHere(caseName + ' author descriptor retained', descriptor?.get === getter && descriptor?.set === setter
                && descriptor.configurable && !descriptor.enumerable);
              checkHere(caseName + ' author accessors not invoked', reads === 0 && writes === 0, reads + '/' + writes);
              delete failedDb.version;
            } else if (protection === 'locked') {
              checkHere(caseName + ' locked author value retained', descriptor?.value === 'author version'
                && !descriptor.writable && !descriptor.configurable && !descriptor.enumerable);
            } else {
              checkHere(caseName + ' public rollback version', failedDb.version === oldVersion, failedDb.version);
              if (protection === 'sealed') checkHere(caseName + ' remains sealed', Object.isSealed(failedDb));
              if (protection === 'frozen') checkHere(caseName + ' remains frozen', Object.isFrozen(failedDb));
            }
          };
          tx.onabort = () => values(caseName + ' abort callback', failedDb, abortName, oldVersion);
          const store = failedDb.createObjectStore('temporary');
          if (protection === 'sealed' || protection === 'frozen') {
            store.add('first', 0);
            store.add('duplicate', 0);
          } else {
            tx.abort();
            values(caseName + ' synchronous abort', failedDb, abortName, oldVersion);
          }
        };
        const error = await failed;
        checkHere(caseName + ' open error', error.name === 'AbortError', error.name);
        values(caseName + ' open error callback finished', failedDb, abortName, oldVersion);
        verifyProtection();
        checkHere(caseName + ' schema rolled back', failedDb.objectStoreNames.length === (fresh ? 0 : 1));
        retained.push({db: failedDb, name: abortName, version: oldVersion});
        if (fresh) {
          const infos = await Realm.indexedDB.databases();
          checkHere(caseName + ' database was not committed', !infos.some(info => info.name === abortName));
          await requestResult(Realm.indexedDB.deleteDatabase(abortName));
        } else {
          const reopened = await requestResult(Realm.indexedDB.open(name));
          values(caseName + ' backend version restored', reopened, name, 1);
          reopened.close();
        }
      }
    }

    const old = await requestResult(Realm.indexedDB.open(name));
    retained.push({db: old, name, version: 1});
    old.onversionchange = event => {
      checkHere('versionchange event versions', event.oldVersion === 1 && event.newVersion === 4);
      values('versionchange before close', old, name, 1);
      old.close();
      values('versionchange after close', old, name, 1);
    };
    const upgradingOpen = Realm.indexedDB.open(name, 4);
    upgradingOpen.onupgradeneeded = () => {
      values('later upgrade', upgradingOpen.result, name, 4);
      upgradingOpen.result.createObjectStore('committed');
      Object.freeze(upgradingOpen.result);
    };
    const newer = await requestResult(upgradingOpen);
    values('committed later upgrade', newer, name, 4);
    checkHere('successful upgrade keeps database frozen', Object.isFrozen(newer));
    retained.push({db: newer, name, version: 4});
    newer.close();
    await requestResult(Realm.indexedDB.deleteDatabase(name));
    for (let i = 0; i < retained.length; ++i) {
      const {db, name, version} = retained[i];
      values('retained after later upgrade and deletion ' + i, db, name, version);
    }

    for (const [suffix, version] of [['empty', 1], ['large', 2 ** 32 + 1], ['maximum-safe', Number.MAX_SAFE_INTEGER]]) {
      const specialName = suffix === 'empty' ? '' : name + '-' + suffix;
      const request = Realm.indexedDB.open(specialName, version);
      const special = await requestResult(request);
      values(suffix + ' name and version', special, specialName, version);
      special.close();
      await requestResult(Realm.indexedDB.deleteDatabase(specialName));
      retained.push({db: special, name: specialName, version});
    }
    return () => {
      for (let i = 0; i < retained.length; ++i) {
        const {db, name, version} = retained[i];
        values('retained after realm removal ' + i, db, name, version);
      }
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
