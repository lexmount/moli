globalThis.schemaAttributeChecks = [];
async function schemaAttributesProbe(prefix = 'schema-attributes') {
  const checks = schemaAttributeChecks;
  const check = (label, pass, actual = '') => checks.push({label, pass, actual: String(actual)});
  const equal = (label, actual, expected) => check(label, JSON.stringify(actual) === JSON.stringify(expected), JSON.stringify(actual));
  const throwsType = (label, fn, Type = TypeError) => {
    try { fn(); check(label, false, 'did not throw'); }
    catch (error) { check(label, error instanceof Type, error); }
  };
  const attributes = [
    [IDBObjectStore.prototype, ['keyPath', 'autoIncrement', 'indexNames', 'transaction']],
    [IDBIndex.prototype, ['keyPath', 'multiEntry', 'unique', 'objectStore']]
  ];
  const getters = attributes.map(([prototype, names]) => Object.fromEntries(names.map(name => {
    const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
    check(`${prototype.constructor.name}.${name}: getter`, typeof descriptor?.get === 'function');
    check(`${prototype.constructor.name}.${name}: readonly descriptor`, !!descriptor && descriptor.set === undefined && descriptor.enumerable && descriptor.configurable);
    return [name, descriptor?.get];
  })));
  if (checks.some(c => !c.pass)) return {state: 'fail', checks};
  const storeGet = (store, name) => getters[0][name].call(store);
  const indexGet = (index, name) => getters[1][name].call(index);
  const request = r => new Promise((resolve, reject) => {
    r.onsuccess = () => resolve(r.result);
    r.onerror = () => reject(r.error);
  });
  const transaction = tx => new Promise(resolve => {
    tx.oncomplete = () => resolve('complete');
    tx.onabort = () => resolve('abort');
  });
  const open = (factory, name, version, upgrade) => new Promise((resolve, reject) => {
    const r = factory.open(name, version);
    r.onupgradeneeded = () => {
      try { upgrade(r.result, r.transaction); }
      catch (error) { reject(error); r.transaction.abort(); }
    };
    r.onsuccess = () => resolve(r.result);
    r.onerror = () => reject(r.error);
  });
  const inspect = (object, names, kind) => {
    for (const name of names) {
      const get = getters[kind][name];
      check(`${kind}.${name}: inherited`, !Object.hasOwn(object, name));
      check(`${kind}.${name}: Reflect.set fails`, !Reflect.set(object, name, null));
      throwsType(`${kind}.${name}: strict assignment`, () => { 'use strict'; object[name] = null; });
      const revoked = Proxy.revocable(object, {}); revoked.revoke();
      for (const fake of [null, {}, Object.create(Object.getPrototypeOf(object)), Object.create(object), new Proxy(object, {}), revoked.proxy]) {
        throwsType(`${kind}.${name}: receiver`, () => get.call(fake));
      }
    }
  };
  let retainedStore, retainedIndex, storePath, indexPath, upgradeTransaction;
  let traps = 0;
  const db = await open(indexedDB, prefix, 1, (db, tx) => {
    upgradeTransaction = tx;
    const store = retainedStore = db.createObjectStore('store', {keyPath: ['id']});
    storePath = store.keyPath;
    const index = retainedIndex = store.createIndex('index', ['tag'], {unique: true});
    indexPath = index.keyPath;
    inspect(store, attributes[0][1], 0);
    inspect(index, attributes[1][1], 1);
    check('keyPath identity after createIndex', store.keyPath === storePath);
    check('transaction identity', store.transaction === tx && store.transaction === store.transaction);
    check('objectStore identity', index.objectStore === store && index.objectStore === index.objectStore);
    check('store has no nonstandard db property', !('db' in store));
    equal('native flags', [store.autoIncrement, index.unique, index.multiEntry], [false, true, false]);
    const generated = db.createObjectStore('generated', {autoIncrement: true});
    check('null keyPath and autoIncrement', generated.keyPath === null && generated.autoIncrement);
    const multi = generated.createIndex('multi', 'tags', {multiEntry: true});
    equal('string keyPath and multiEntry', [multi.keyPath, multi.multiEntry, multi.unique], ['tags', true, false]);
    const before = store.indexNames;
    check('fresh name list', before !== store.indexNames);
    storePath[0] = 'authorStorePath';
    indexPath[0] = 'authorIndexPath';
    for (const [object, names] of [[store, attributes[0][1]], [index, attributes[1][1]]]) {
      for (const name of names) Object.defineProperty(object, name, {
        configurable: false,
        get() { ++traps; throw new Error('author getter: ' + name); },
        set() { ++traps; throw new Error('author setter: ' + name); }
      });
    }
    index.name = 'renamed';
    store.createIndex('temporary', 'other');
    store.deleteIndex('temporary');
    store.name = 'renamedStore';
    equal('native name list after schema edits', Array.from(storeGet(store, 'indexNames')), ['renamed']);
    equal('old name list is a snapshot', Array.from(before), ['index']);
    check('schema edits preserve keyPath identities', storeGet(store, 'keyPath') === storePath && indexGet(index, 'keyPath') === indexPath);
    equal('exposed array mutations survive schema edits', [storePath[0], indexPath[0]], ['authorStorePath', 'authorIndexPath']);
    check('native parent links ignore shadows', storeGet(store, 'transaction') === tx && indexGet(index, 'objectStore') === store);
    equal('native flags ignore shadows', [storeGet(store, 'autoIncrement'), indexGet(index, 'unique'), indexGet(index, 'multiEntry')], [false, true, false]);
    store.put({id: 1, tag: 'value'});
  });
  check('schema edits did not invoke author properties', traps === 0, traps);
  check('finished store retains transaction', storeGet(retainedStore, 'transaction') === upgradeTransaction);
  check('finished index retains store', indexGet(retainedIndex, 'objectStore') === retainedStore);
  for (const [object, names] of [[retainedStore, attributes[0][1]], [retainedIndex, attributes[1][1]]]) {
    for (const name of names) check(`shadow ${name} survives schema edits`, Object.getOwnPropertyDescriptor(object, name).configurable === false);
  }
  const tx = db.transaction('renamedStore');
  const done = transaction(tx);
  const store = tx.objectStore('renamedStore');
  const index = store.index('renamed');
  equal('new handles expose original native key paths', [store.keyPath, index.keyPath], [['id'], ['tag']]);
  check('new handles use distinct arrays', store.keyPath !== storePath && index.keyPath !== indexPath);
  const get = request(store.get([1]));
  const getKey = request(index.getKey(['value']));
  equal('writes use native store key path', await get, {id: 1, tag: 'value'});
  equal('index uses native key path', await getKey, [1]);
  check('read transaction completes', await done === 'complete');
  check('finished arrays retain identity', storeGet(retainedStore, 'keyPath') === storePath && indexGet(retainedIndex, 'keyPath') === indexPath);
  db.close();

  let oldStore, oldIndex, oldPath, oldIndexPath, newStore, newPath, newIndex, newIndexPath;
  const abort = indexedDB.open(prefix, 2);
  const aborted = new Promise((resolve, reject) => {
    abort.onsuccess = () => reject(new Error('aborted upgrade succeeded'));
    abort.onerror = event => { event.preventDefault(); resolve(abort.error.name); };
    abort.onupgradeneeded = () => {
      try {
        const tx = abort.transaction;
        oldStore = tx.objectStore('renamedStore'); oldPath = oldStore.keyPath;
        oldIndex = oldStore.index('renamed'); oldIndexPath = oldIndex.keyPath;
        newStore = abort.result.createObjectStore('temporary', {keyPath: ['newKey']}); newPath = newStore.keyPath;
        newIndex = newStore.createIndex('newIndex', ['newTag']); newIndexPath = newIndex.keyPath;
        oldStore.deleteIndex('renamed');
        oldStore.createIndex('replacement', 'other');
        abort.result.deleteObjectStore('renamedStore');
        check('deleted store keeps keyPath identity', oldStore.keyPath === oldPath);
        check('deleted index keeps keyPath identity', oldIndex.keyPath === oldIndexPath);
        tx.abort();
        equal('abort restores native name list', Array.from(oldStore.indexNames), ['renamed']);
        equal('abort clears new store name list', Array.from(newStore.indexNames), []);
      } catch (error) { reject(error); }
    };
  });
  check('upgrade abort error', await aborted === 'AbortError');
  check('rollback preserves all cached arrays', oldStore.keyPath === oldPath && oldIndex.keyPath === oldIndexPath && newStore.keyPath === newPath && newIndex.keyPath === newIndexPath);
  equal('deleted index retains flags', [oldIndex.unique, oldIndex.multiEntry, newIndex.unique], [true, false, false]);
  check('deleted handles retain native parent links', newIndex.objectStore === newStore && oldIndex.objectStore === oldStore && newStore.transaction === oldStore.transaction);

  if (typeof document !== 'undefined') {
    const frame = document.createElement('iframe');
    const loaded = new Promise(resolve => { frame.onload = resolve; });
    frame.srcdoc = '<!doctype html>'; document.documentElement.appendChild(frame); await loaded;
    const other = frame.contentWindow;
    let foreignStore, foreignIndex;
    const foreignDb = await open(other.indexedDB, prefix + '-foreign', 1, db => {
      foreignStore = db.createObjectStore('store', {keyPath: ['id']});
      foreignIndex = foreignStore.createIndex('index', ['tag']);
    });
    for (const [kind, object, foreign, prototype] of [
      [0, store, foreignStore, other.IDBObjectStore.prototype],
      [1, index, foreignIndex, other.IDBIndex.prototype]
    ]) for (const name of attributes[kind][1]) {
      const otherGet = Object.getOwnPropertyDescriptor(prototype, name).get;
      const localValue = getters[kind][name].call(object);
      const borrowedValue = otherGet.call(object);
      check(`borrowed ${kind}.${name}: genuine receiver`, name === 'indexNames' ? Array.from(borrowedValue).join() === Array.from(localValue).join() : borrowedValue === localValue);
      throwsType(`borrowed ${kind}.${name}: callee realm error`, () => otherGet.call(new Proxy(object, {})), other.TypeError);
      const foreignValue = getters[kind][name].call(foreign);
      check(`foreign ${kind}.${name}: native receiver`, name !== 'keyPath' || Object.getPrototypeOf(foreignValue) === other.Array.prototype);
    }
    foreignDb.close(); frame.remove();
  }
  return {state: checks.every(c => c.pass) ? 'pass' : 'fail', checks};
}
