globalThis.databaseNameChecks = [];
async function databaseNamesProbe(prefix = 'database-names') {
  const checks = databaseNameChecks;
  const check = (label, pass, actual = '') => checks.push({label, pass, actual: String(actual)});
  const units = name => Array.from({length: name.length}, (_, i) => name.charCodeAt(i)).join(',');
  const result = request => new Promise((resolve, reject) => {
    request.onsuccess = () => resolve(request.result);
    request.onerror = () => reject(request.error);
  });
  const complete = tx => new Promise((resolve, reject) => {
    tx.oncomplete = resolve;
    tx.onabort = () => reject(tx.error || new Error('unexpected abort'));
  });
  const exercise = async (label, Realm, MethodRealm) => {
    const here = (name, pass, actual) => check(label + ': ' + name, pass, actual);
    const factory = Realm.indexedDB;
    const open = (...args) => MethodRealm.IDBFactory.prototype.open.apply(factory, args);
    const remove = (...args) => MethodRealm.IDBFactory.prototype.deleteDatabase.apply(factory, args);
    const list = () => MethodRealm.IDBFactory.prototype.databases.call(factory);
    const getName = Object.getOwnPropertyDescriptor(MethodRealm.IDBDatabase.prototype, 'name').get;
    const stem = prefix + '-' + label + '-';
    const names = ['', '\0', '\ud800', '\udfff', '\ufffd', '\ud801',
      '\udfff\ud800', '\ufffd\ufffd', '💾', 'd800', '\ud800\0'].map(s => stem + s);
    const retained = [];
    for (let i = 0; i < names.length; ++i) {
      const conversion = [];
      const name = {[Symbol.toPrimitive](hint) { conversion.push(hint); return names[i]; }};
      const version = {valueOf() { conversion.push('version'); return 1; }};
      const request = open(name, version);
      here('conversion order ' + i, conversion.join() === 'string,version', conversion);
      let oldVersion, upgradeName;
      request.onupgradeneeded = event => {
        oldVersion = event.oldVersion;
        upgradeName = getName.call(request.result);
        request.result.createObjectStore('records');
      };
      const db = await result(request);
      retained.push(db);
      here('upgrade name ' + i, upgradeName === names[i], upgradeName === undefined ? '' : units(upgradeName));
      here('independent creation ' + i, oldVersion === 0, oldVersion);
      here('open name ' + i, db.name === names[i], units(db.name));
      const tx = db.transaction('records', 'readwrite');
      const done = complete(tx);
      tx.objectStore('records').put(i, 'same-key');
      await done;
      db.close();
      here('closed name ' + i, getName.call(db) === names[i], units(getName.call(db)));
    }
    let listed = (await list()).filter(db => db.name.startsWith(stem));
    here('listing keeps every database', listed.length === names.length, listed.length);
    for (let i = 0; i < names.length; ++i) {
      here('listing units ' + i, listed.some(db => db.name === names[i] && db.version === 1));
      const db = await result(open(new Realm.String(names[i])));
      const tx = db.transaction('records');
      const done = complete(tx);
      const read = tx.objectStore('records').get('same-key');
      await done;
      here('independent records ' + i, read.result === i, read.result);
      db.close();
    }

    const order = [];
    const marker = new Error('name conversion');
    let caught;
    try {
      open({toString() { order.push('name'); throw marker; }}, {valueOf() { order.push('version'); return 1; }});
    } catch (error) { caught = error; }
    here('name exception preserves identity and prevents version conversion', caught === marker && order.join() === 'name', order);
    for (const [method, callback] of [['open', open], ['deleteDatabase', remove]]) {
      let error;
      try { callback(Symbol('name')); } catch (caught) { error = caught; }
      here(method + ' rejects Symbol in callee realm', error !== undefined && Object.getPrototypeOf(error) === MethodRealm.TypeError.prototype, error?.name);
    }

    const upgrading = open(names[2], 2);
    let upgradeOld;
    upgrading.onupgradeneeded = event => {
      upgradeOld = event.oldVersion;
      upgrading.transaction.objectStore('records').put('upgraded', 'same-key');
    };
    const upgraded = await result(upgrading);
    here('upgrade uses exact database', upgradeOld === 1 && upgraded.name === names[2], upgradeOld);
    upgraded.close();
    const aborted = open(names[3], 3);
    aborted.onupgradeneeded = () => {
      aborted.transaction.objectStore('records').put('aborted', 'same-key');
      aborted.transaction.abort();
    };
    let abortError;
    try { (await result(aborted)).close(); } catch (error) { abortError = error; }
    here('aborted upgrade reports AbortError', abortError?.name === 'AbortError', abortError?.name);
    listed = (await list()).filter(db => db.name.startsWith(stem));
    for (let i = 0; i < names.length; ++i) {
      here('versions remain independent ' + i, listed.some(db => db.name === names[i] && db.version === (i === 2 ? 2 : 1)));
    }
    const rollback = await result(open(names[3]));
    const readTx = rollback.transaction('records');
    const readDone = complete(readTx);
    const rolledBack = readTx.objectStore('records').get('same-key');
    await readDone;
    here('aborted data remains independent', rolledBack.result === 3, rolledBack.result);
    rollback.close();

    // Keep a different surrogate name open. A mistaken identity match still
    // completes via onblocked so every engine reports the same assertions.
    const held = await result(open(names[3]));
    let notifications = 0;
    held.onversionchange = () => { ++notifications; };
    const deleting = remove({toString() { return names[2]; }});
    let blocked = false;
    deleting.onblocked = () => { blocked = true; held.close(); };
    await result(deleting);
    held.close();
    here('delete does not notify another database', notifications === 0, notifications);
    here('delete is not blocked by another database', !blocked, blocked);
    listed = (await list()).filter(db => db.name.startsWith(stem));
    here('delete removes only the exact database', !listed.some(db => db.name === names[2])
      && listed.length === names.length - 1, listed.length);
    here('retained connection name survives deletion', getName.call(retained[2]) === names[2], units(getName.call(retained[2])));
    for (const name of names) await result(remove(name));
    here('cleanup removes all test databases', !(await list()).some(db => db.name.startsWith(stem)));
  };

  await exercise('own', globalThis, globalThis);
  if (typeof document !== 'undefined') {
    const frame = document.createElement('iframe');
    const loaded = new Promise(resolve => { frame.onload = resolve; });
    frame.src = 'about:blank';
    document.body.appendChild(frame);
    await loaded;
    try {
      await exercise('foreign-method', globalThis, frame.contentWindow);
      await exercise('foreign-factory', frame.contentWindow, globalThis);
    } finally { frame.remove(); }

    const firstName = prefix + '-shared-\ud800';
    const secondName = prefix + '-shared-\udfff';
    for (const name of [firstName, secondName]) {
      const request = indexedDB.open(name, 1);
      request.onupgradeneeded = () => request.result.createObjectStore('records');
      (await result(request)).close();
    }
    const held = await result(indexedDB.open(firstName));
    let notices = 0;
    held.onversionchange = () => { ++notices; held.close(); };
    const source = `onmessage = async event => {
      const request = indexedDB.open(event.data, 2);
      let oldVersion;
      request.onupgradeneeded = event => { oldVersion = event.oldVersion; };
      request.onerror = () => postMessage({error: request.error.name});
      request.onsuccess = () => {
        const db = request.result;
        const name = db.name;
        db.close();
        postMessage({name, oldVersion});
      };
    };`;
    const url = URL.createObjectURL(new Blob([source], {type: 'text/javascript'}));
    const worker = new Worker(url);
    try {
      const reply = new Promise((resolve, reject) => {
        worker.onmessage = event => resolve(event.data);
        worker.onerror = event => reject(new Error(event.message));
      });
      worker.postMessage(secondName);
      const observed = await reply;
      check('worker upgrade preserves database name', observed.name === secondName && observed.oldVersion === 1, JSON.stringify(observed));
      check('worker upgrade does not notify a different Window connection', notices === 0, notices);
      held.close();
      const listed = await indexedDB.databases();
      check('Window sees independent versions after worker upgrade', listed.some(db => db.name === firstName && db.version === 1)
        && listed.some(db => db.name === secondName && db.version === 2));
    } finally {
      held.close();
      worker.terminate();
      URL.revokeObjectURL(url);
      await result(indexedDB.deleteDatabase(firstName));
      await result(indexedDB.deleteDatabase(secondName));
    }
  }
  return {state: checks.every(check => check.pass) ? 'pass' : 'fail', checks};
}
