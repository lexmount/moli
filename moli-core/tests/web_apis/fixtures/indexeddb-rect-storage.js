globalThis.rectStorageProbe = async function(name) {
  const checks = globalThis.rectStorageChecks = [];
  const check = (label, pass) => checks.push({label, pass: !!pass});
  const request = r => new Promise((resolve, reject) => {
    r.onsuccess = () => resolve(r.result); r.onerror = () => reject(r.error);
  });
  const done = tx => new Promise((resolve, reject) => {
    tx.oncomplete = resolve; tx.onabort = () => reject(tx.error);
  });
  const opening = indexedDB.open(name, 1);
  opening.onupgradeneeded = () => opening.result.createObjectStore('values');
  let db = await request(opening);
  const prototypes = [DOMRectReadOnly.prototype, DOMRect.prototype];
  const getters = ['x', 'y', 'width', 'height'].map(key => Object.getOwnPropertyDescriptor(DOMRectReadOnly.prototype, key).get);
  const values = [[1, -0, Infinity, NaN], [10, 20, -30, -40]];
  const rectangles = [new DOMRectReadOnly(...values[0]), new DOMRect(...values[1])];
  const saved = ['DOMRect', 'DOMRectReadOnly'].map(name => Object.getOwnPropertyDescriptor(globalThis, name));
  try {
    const graph = {rectangles, repeated: rectangles[0]}; graph.self = graph;
    for (const rect of rectangles) {
      rect.expando = 'not serialized';
      for (const key of ['x', 'y', 'width', 'height']) Object.defineProperty(rect, key, {configurable: true, get() {throw new Error('author getter');}});
    }
    const tx = db.transaction('values', 'readwrite'), complete = done(tx);
    tx.objectStore('values').put(graph, 'snapshot');
    Object.getOwnPropertyDescriptor(DOMRect.prototype, 'x').set.call(rectangles[1], 999);
    await complete;
    db.close(); db = await request(indexedDB.open(name));
    for (const key of ['DOMRect', 'DOMRectReadOnly']) Object.defineProperty(globalThis, key, {configurable: true, get() {throw new Error('author constructor');}});
    const copy = await request(db.transaction('values').objectStore('values').get('snapshot'));
    check('graph cycle', copy.self === copy);
    check('graph alias', copy.rectangles[0] === copy.repeated);
    check('snapshot independent', copy.rectangles[0] !== rectangles[0] && copy.rectangles[1] !== rectangles[1]);
    for (let n = 0; n < rectangles.length; n++) {
      const rect = copy.rectangles[n];
      check('native prototype ' + n, Object.getPrototypeOf(rect) === prototypes[n]);
      check('expando excluded ' + n, rect.expando === undefined);
      for (let i = 0; i < getters.length; i++) check('native value ' + n + '/' + i, Object.is(getters[i].call(rect), values[n][i]));
    }
    return {state: checks.every(c => c.pass) ? 'pass' : 'fail', checks};
  } finally {
    ['DOMRect', 'DOMRectReadOnly'].forEach((key, i) => Object.defineProperty(globalThis, key, saved[i]));
    db.close(); await request(indexedDB.deleteDatabase(name));
  }
};
