globalThis.rangeChecks = [];
async function keyRangeProbe(name = 'key-range-' + Math.random()) {
  const checks = globalThis.rangeChecks;
  const check = (label, pass, actual = '') => checks.push({label, pass: !!pass, actual: JSON.stringify(actual)});
  const equal = (label, actual, expected) => check(label, JSON.stringify(actual) === JSON.stringify(expected), actual);
  const throws = (label, call, name, identity) => {
    try { call(); check(label, false, 'accepted'); }
    catch (error) { check(label, error.name === name && (!identity || error === identity), error.name); }
  };
  const request = r => new Promise((resolve, reject) => {
    r.onsuccess = () => resolve(r.result);
    r.onerror = () => reject(r.error);
  });
  const done = tx => new Promise((resolve, reject) => {
    tx.oncomplete = resolve;
    tx.onabort = () => reject(tx.error);
  });
  const includes = IDBKeyRange.prototype.includes;
  const names = ['lower', 'upper', 'lowerOpen', 'upperOpen'];
  const descriptors = Object.fromEntries(names.map(name => [name, Object.getOwnPropertyDescriptor(IDBKeyRange.prototype, name)]));
  for (const [method, length] of [['only',1], ['bound',2], ['lowerBound',1], ['upperBound',1]]) {
    equal(method + ' arity', IDBKeyRange[method].length, length);
    throws(method + ' missing key', () => IDBKeyRange[method](), 'TypeError');
  }
  throws('bound missing upper', () => IDBKeyRange.bound(1), 'TypeError');
  throws('illegal constructor', () => new IDBKeyRange(), 'TypeError');
  equal('static method has no receiver requirement', IDBKeyRange.only.call(null, 3).lower, 3);
  for (const [label, range, expected, membership] of [
    ['only', IDBKeyRange.only(2), [2,2,false,false], [false,false,true,false,false]],
    ['lower', IDBKeyRange.lowerBound(2), [2,undefined,false,true], [false,false,true,true,true]],
    ['lower open', IDBKeyRange.lowerBound(2,true), [2,undefined,true,true], [false,false,false,true,true]],
    ['upper', IDBKeyRange.upperBound(2), [undefined,2,true,false], [true,true,true,false,false]],
    ['upper open', IDBKeyRange.upperBound(2,true), [undefined,2,true,true], [true,true,false,false,false]],
    ['bound', IDBKeyRange.bound(1,3,true,false), [1,3,true,false], [false,false,true,true,false]]
  ]) {
    equal(label + ' bounds', names.map(name => range[name]), expected);
    equal(label + ' membership', [0,1,2,3,4].map(key => includes.call(range,key)), membership);
  }
  const real = IDBKeyRange.only(2);
  equal('no own range attributes or markers', Reflect.ownKeys(real), []);
  for (const name of names) {
    const descriptor = descriptors[name];
    check('readonly prototype accessor ' + name, descriptor && typeof descriptor.get === 'function' && descriptor.set === undefined && descriptor.enumerable && descriptor.configurable);
    check('ordinary write rejected ' + name, Reflect.set(real,name,123) === false);
    throws('strict write rejected ' + name, () => { 'use strict'; real[name] = 123; }, 'TypeError');
    if (descriptor && descriptor.get) equal('getter arity ' + name, descriptor.get.length, 0);
  }
  const revoked = Proxy.revocable(real,{}); revoked.revoke();
  for (const [label, receiver] of [
    ['plain',{}], ['prototype',IDBKeyRange.prototype],
    ['forged',Object.create(IDBKeyRange.prototype)], ['inheritor',Object.create(real)],
    ['proxy',new Proxy(real,{})], ['revoked',revoked.proxy], ['null',null]
  ]) {
    let reads = 0;
    const key = Object.defineProperty([],0,{get(){reads++;throw new Error('key read');}});
    throws('includes brand ' + label, () => includes.call(receiver,key), 'TypeError');
    equal('brand before key conversion ' + label, reads, 0);
    for (const name of names) {
      const getter = descriptors[name] && descriptors[name].get;
      if (getter) throws('getter brand ' + label + ' ' + name, () => getter.call(receiver), 'TypeError');
    }
  }
  const order = [], sentinel = new URIError('key conversion');
  const lower = Object.defineProperty([],0,{get(){order.push('lower');return 1;}});
  const upper = Object.defineProperty([],0,{get(){order.push('upper');return 3;}});
  const open = {valueOf(){throw new Error('boolean conversion called valueOf');}};
  const converted = IDBKeyRange.bound(lower,upper,open);
  equal('convert keys once in order', order, ['lower','upper']);
  equal('boolean conversion', [converted.lowerOpen,converted.upperOpen], [true,false]);
  const bad = Object.defineProperty([],0,{get(){throw sentinel;}});
  throws('includes preserves getter exception', () => includes.call(real,bad), 'URIError', sentinel);
  throws('bound preserves getter exception', () => IDBKeyRange.bound(bad,upper), 'URIError', sentinel);
  equal('failed lower skips upper conversion', order, ['lower','upper']);
  throws('includes missing key', () => includes.call(real), 'TypeError');
  for (const invalid of [undefined,null,NaN,new Date(NaN),[undefined],{},new Proxy(real,{})]) {
    throws('includes invalid key', () => includes.call(real,invalid), 'DataError');
  }
  for (const [label, make, mutate] of [
    ['array', () => [[1]], value => value[0][0] = 99],
    ['date', () => new Date(1), value => value.setTime(99)],
    ['binary', () => new Uint8Array([1]).buffer, value => new Uint8Array(value)[0] = 99]
  ]) {
    const input = make(), range = IDBKeyRange.only(input);
    mutate(input);
    check(label + ' captures input', includes.call(range,make()));
    check(label + ' rejects changed input', !includes.call(range,input));
    for (const name of ['lower','upper']) {
      const value = range[name]; mutate(value);
      check(label + ' ' + name + ' cannot change native range', includes.call(range,make()));
      check(label + ' ' + name + ' cannot add a key', !includes.call(range,value));
    }
  }
  const bare = IDBKeyRange.only(2); Object.setPrototypeOf(bare,null); Object.freeze(bare);
  check('brand survives prototype removal and freeze', includes.call(bare,2));
  for (const name of names) {
    const getter = descriptors[name] && descriptors[name].get;
    if (getter) equal('getter survives prototype removal ' + name, getter.call(bare), name.endsWith('Open') ? false : 2);
  }
  // Shadow every public attribute with a throwing getter. Queries must use
  // the native bounds even when no IDBKeyRange prototype remains reachable.
  let publicReads = 0;
  const poison = (range = IDBKeyRange.bound([1],[3],true,false)) => {
    const lower = range.lower, upper = range.upper;
    if (lower !== undefined) lower[0] = 90;
    if (upper !== undefined) upper[0] = 99;
    for (const name of names) Object.defineProperty(range,name,{get(){publicReads++;throw new Error('public range attribute read');}});
    for (const name of ['__moli_idb_key_range_lower','__moli_idb_key_range_upper','__moli_idb_key_range_lower_open','__moli_idb_key_range_upper_open','__moliIndexedDbKeyRangeMarker']) {
      Object.defineProperty(range,name,{value:null});
    }
    Object.setPrototypeOf(range,null); Object.freeze(range);
    return range;
  };
  const verifyQueries = (store, label) => {
    const pending = [];
    for (const [rangeName, range, ids] of [
      ['bounded',poison(),[2,3]],
      ['lower',poison(IDBKeyRange.lowerBound([2],true)),[3,4]],
      ['upper',poison(IDBKeyRange.upperBound([3],true)),[1,2]],
      ['empty',poison(IDBKeyRange.lowerBound([9])),[]]
    ]) for (const [sourceName, source] of [['store',store],['index',store.index('i')]]) {
      for (const method of ['get','getKey','getAll','getAllKeys','getAllRecords','count','openCursor','openKeyCursor']) {
        const test = label + ' ' + rangeName + ' ' + sourceName + '.' + method;
        try {
          const r = source[method](method === 'getAllRecords' ? {query:range} : range);
          pending.push(request(r).then(value => {
            let actual, expected;
            if (method === 'get') { actual = value && value.id; expected = ids[0]; }
            else if (method === 'getKey') { actual = value; expected = ids.length ? [ids[0]] : undefined; }
            else if (method === 'getAll') { actual = value.map(v => v.id); expected = ids; }
            else if (method === 'getAllKeys') { actual = value; expected = ids.map(id => [id]); }
            else if (method === 'getAllRecords') { actual = value.map(v => v.primaryKey); expected = ids.map(id => [id]); }
            else if (method === 'count') { actual = value; expected = ids.length; }
            else { actual = value && [value.key,value.primaryKey]; expected = ids.length ? [[ids[0]],[ids[0]]] : null; }
            equal(test,actual,expected);
          }));
        } catch (error) { check(test,false,error.name); }
      }
    }
    return Promise.all(pending);
  };
  const eager = [], opening = indexedDB.open(name,1);
  opening.onupgradeneeded = () => {
    const db = opening.result;
    for (const name of ['s','d']) {
      const store = db.createObjectStore(name); store.createIndex('i','group');
      for (let id=1;id<=4;id++) store.put({id,group:[id]},[id]);
    }
    eager.push(verifyQueries(opening.transaction.objectStore('s'),'upgrade'));
  };
  const db = await request(opening);
  try {
    await Promise.all(eager);
    for (const mode of ['pending','started']) {
      const writer = db.transaction('s','readwrite'), written = done(writer);
      writer.objectStore('s').get([0]);
      const tx = db.transaction('s'), complete = done(tx), store = tx.objectStore('s');
      if (mode === 'started') await request(store.get([0]));
      await verifyQueries(store,mode);
      await complete; await written;
      const blocker = db.transaction('d','readwrite'), unblocked = done(blocker);
      blocker.objectStore('d').get([0]);
      const deletion = db.transaction('d','readwrite'), deleted = done(deletion), deletes = deletion.objectStore('d');
      if (mode === 'started') await request(deletes.get([0]));
      // Reseed inside the transaction to cover both immediate and queued delete.
      for (let id=1;id<=4;id++) deletes.put({id,group:[id]},[id]);
      try { await request(deletes.delete(poison())); }
      catch (error) { check(mode + ' delete accepted',false,error.name); }
      equal(mode + ' delete respects private bounds', await request(deletes.getAllKeys()), [[1],[4]]);
      equal(mode + ' delete updates index', await request(deletes.index('i').getAllKeys()), [[1],[4]]);
      await deleted; await unblocked;
    }
    // Prototype pollution must not become query input either.
    const polluted = IDBKeyRange.only([2]);
    let result;
    try {
      for (const name of names) Object.defineProperty(IDBKeyRange.prototype,name,{configurable:true,get(){publicReads++;throw new Error('prototype range attribute read');}});
      try { check('includes ignores prototype attributes',includes.call(polluted,[2])); }
      catch (error) { check('includes ignores prototype attributes',false,error.name); }
      result = request(db.transaction('s').objectStore('s').getAllKeys(polluted));
    } finally {
      for (const name of names) {
        if (descriptors[name]) Object.defineProperty(IDBKeyRange.prototype,name,descriptors[name]);
        else delete IDBKeyRange.prototype[name];
      }
    }
    equal('query ignores prototype attributes',await result,[[2]]);
    equal('queries never invoke public attributes',publicReads,0);
    if (typeof document !== 'undefined') {
      const frame = document.createElement('iframe'); document.body.appendChild(frame);
      const child = frame.contentWindow;
      try {
        const foreign = child.IDBKeyRange.only([2]);
        check('includes accepts foreign range',includes.call(foreign,[2]));
        check('foreign includes accepts local range',child.IDBKeyRange.prototype.includes.call(real,2));
        equal('query accepts foreign range',await request(db.transaction('s').objectStore('s').getAllKeys(foreign)),[[2]]);
        for (const name of names) {
          const descriptor = Object.getOwnPropertyDescriptor(child.IDBKeyRange.prototype,name);
          if (!descriptor || !descriptor.get) { check('foreign getter ' + name,false); continue; }
          const value = descriptor.get.call(IDBKeyRange.only([2]));
          check('foreign getter accepts local range ' + name, name.endsWith('Open') ? value === false : value instanceof child.Array && value[0] === 2);
          try { descriptor.get.call(new Proxy(real,{})); check('getter TypeError callee realm ' + name,false); }
          catch (error) { check('getter TypeError callee realm ' + name,error instanceof child.TypeError && !(error instanceof TypeError)); }
        }
        try { child.IDBKeyRange.prototype.includes.call({},2); check('includes TypeError callee realm',false); }
        catch (error) { check('includes TypeError callee realm',error instanceof child.TypeError && !(error instanceof TypeError)); }
      } finally { frame.remove(); }
    }
  } finally { db.close(); }
  await request(indexedDB.deleteDatabase(name));
  return {state:checks.every(check => check.pass) ? 'pass' : 'fail',checks};
}
