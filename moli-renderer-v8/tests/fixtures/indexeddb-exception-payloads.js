async function indexedDBExceptionPayloadProbe(global) {
  const rows = [], failures = [];
  const check = (name, pass) => {
    rows.push({name, pass: !!pass});
    if (!pass) failures.push(name);
  };
  const request = value => new Promise((resolve, reject) => {
    value.onsuccess = () => resolve(value.result);
    value.onerror = () => reject(value.error);
  });
  const completion = tx => new Promise((resolve, reject) => {
    tx.oncomplete = resolve;
    tx.onabort = () => reject(tx.error || new Error('transaction aborted'));
  });
  const DOM = global.DOMException, Quota = global.QuotaExceededError;
  const domDescriptor = Object.getOwnPropertyDescriptor(global, 'DOMException');
  const quotaDescriptor = Object.getOwnPropertyDescriptor(global, 'QuotaExceededError');
  const message = 'stored\u0000\ud83d\ude00';
  class Derived extends DOM {}
  let native;
  try { global.structuredClone(() => {}); } catch (error) { native = error; }
  const cases = [
    {label:'abort', error:new DOM(message, 'AbortError'), ctor:DOM, name:'AbortError', message, code:20},
    {label:'default', error:new DOM(), ctor:DOM, name:'Error', message:'', code:0},
    {label:'custom', error:new DOM(message, 'CustomError'), ctor:DOM, name:'CustomError', message, code:0},
    {label:'base-quota-name', error:new DOM(message, 'QuotaExceededError'), ctor:DOM, name:'QuotaExceededError', message, code:22},
    {label:'subclass', error:new Derived(message, 'NotFoundError'), ctor:DOM, name:'NotFoundError', message, code:8},
    {label:'native', error:native, ctor:DOM, name:'DataCloneError', message:native.message, code:25},
    ...[
      ['empty', {}], ['quota', {quota:10}], ['requested', {requested:12}],
      ['zero', {quota:0, requested:0}], ['fraction', {quota:10.25, requested:12.5}]
    ].map(([label, options]) => ({label:'quota-'+label, error:new Quota(message, options),
      ctor:Quota, name:'QuotaExceededError', message, code:22,
      quota:options.quota ?? null, requested:options.requested ?? null}))
  ];
  check('native API exception retains its diagnostic stack', typeof native.stack === 'string');
  const ordinary = new TypeError('ordinary', {cause:cases[0].error});
  const ordinaryStack = ordinary.stack;
  let authorReads = 0;
  const poison = () => { ++authorReads; throw new Error('author getter must not run'); };
  for (const entry of cases) {
    Object.setPrototypeOf(entry.error, null);
    for (const name of ['message', 'name', 'code', 'quota', 'requested', 'stack', 'expando']) {
      Object.defineProperty(entry.error, name, {enumerable:true, get:poison});
    }
  }
  const value = {kind:'original', errors:cases.map(entry => entry.error), ordinary};
  value.alias = value.errors[0]; value.self = value;
  value.map = new Map([[value.errors[0], value.errors[1]]]);
  value.set = new Set([value.errors[0]]);
  const inspect = (label, copy) => {
    check(label+': fresh graph with cycles and aliases', copy !== value && copy.self === copy && copy.alias === copy.errors[0]);
    check(label+': Map and Set preserve native-object identity', copy.map.get(copy.errors[0]) === copy.errors[1] && copy.set.has(copy.errors[0]));
    check(label+': ordinary Error keeps prototype, cause and stack', Object.getPrototypeOf(copy.ordinary) === TypeError.prototype && copy.ordinary.cause === copy.errors[0] && copy.ordinary.stack === ordinaryStack);
    check(label+': exception count', copy.errors.length === cases.length);
    for (let i=0; i<cases.length; ++i) {
      const entry=cases[i], error=copy.errors[i], name=label+': '+entry.label;
      check(name+': native prototype and independent identity', Object.getPrototypeOf(error) === entry.ctor.prototype && error !== entry.error && error instanceof DOM);
      check(name+': data fields', error.name === entry.name && error.message === entry.message && error.code === entry.code);
      if (entry.ctor === Quota) check(name+': nullable quota data', Object.is(error.quota, entry.quota) && Object.is(error.requested, entry.requested));
      check(name+': no authored properties or invented stack', Object.keys(error).length === 0 && !Object.hasOwn(error,'stack') && error.expando === undefined);
    }
  };
  const dbName = 'exception-payloads-'+Date.now()+'-'+Math.random();
  let db;
  try {
    const open = global.indexedDB.open(dbName, 1);
    open.onupgradeneeded = () => open.result.createObjectStore('values').createIndex('kind','kind');
    db = await request(open);
    const tx = db.transaction('values','readwrite'), done = completion(tx), store = tx.objectStore('values');
    for (const entry of cases) {
      let error;
      try { store.put(new Proxy(entry.error, {}), 'proxy-'+entry.label); } catch (caught) { error = caught; }
      check(entry.label+': author Proxy rejected', error instanceof DOM && error.name === 'DataCloneError');
    }
    let serialized = true;
    try { store.add(value,'add'); store.put(value,'put'); } catch (error) {
      serialized = false;
      check('native exception data can be stored: '+error.name, false);
    }
    await done;
    check('native exception records accepted', serialized);
    if (serialized) {
      Object.defineProperty(global,'DOMException',{configurable:true,get:poison});
      Object.defineProperty(global,'QuotaExceededError',{configurable:true,get:poison});
      const read = db.transaction('values');
      const readDone = completion(read), values = read.objectStore('values');
      const one = request(values.get('add'));
      const many = request(values.getAll());
      const indexed = request(values.index('kind').get('original'));
      const [copy, copies, indexCopy] = await Promise.all([one,many,indexed]);
      await readDone;
      inspect('get',copy);
      check('getAll returns both records',copies.length === 2);
      for (let i=0;i<copies.length;++i) inspect('getAll['+i+']',copies[i]);
      inspect('index.get',indexCopy);
      value.kind='updated';
      const update=db.transaction('values','readwrite'), updateDone=completion(update);
      const cursor=update.objectStore('values').openCursor();
      const changed = new Promise((resolve,reject) => {
        cursor.onerror=() => reject(cursor.error);
        cursor.onsuccess=() => {
          const change=cursor.result.update(value);
          change.onsuccess=resolve; change.onerror=() => reject(change.error);
        };
      });
      await changed; await updateDone;
      db.close(); db=await request(global.indexedDB.open(dbName));
      const reopened=db.transaction('values'), reopenedDone=completion(reopened);
      const saved=await request(reopened.objectStore('values').index('kind').get('updated'));
      await reopenedDone;
      inspect('cursor.update after reopening',saved);
    }
    check('native data serialization never reads author getters',authorReads === 0);
  } catch(error) {
    check('probe completes without '+error.name+': '+error.message,false);
  } finally {
    Object.defineProperty(global,'DOMException',domDescriptor);
    Object.defineProperty(global,'QuotaExceededError',quotaDescriptor);
    if (db) db.close();
    await request(global.indexedDB.deleteDatabase(dbName));
  }
  return {rows,failures};
}
