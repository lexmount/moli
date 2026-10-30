globalThis.bitmapStorageProbe = async function(name) {
  const checks = globalThis.bitmapStorageChecks = [];
  const check = (label, pass) => checks.push({label, pass: !!pass});
  const request = r => new Promise((resolve, reject) => {r.onsuccess = () => resolve(r.result); r.onerror = () => reject(r.error);});
  const done = t => new Promise((resolve, reject) => {t.oncomplete = resolve; t.onabort = () => reject(t.error);});
  const opening = indexedDB.open(name, 1); opening.onupgradeneeded = () => opening.result.createObjectStore('values');
  let db = await request(opening);
  try {
    const canvas = new OffscreenCanvas(2,1);
    const context = canvas.getContext('2d');
    // Both translucent channels remain exact when multiplied by alpha 85/255.
    const expected = [11,22,33,255,51,102,0,85];
    context.putImageData(new ImageData(new Uint8ClampedArray(expected),2,1),0,0);
    const bitmap = canvas.transferToImageBitmap();
    bitmap.expando = true;
    for (const key of ['width','height']) Object.defineProperty(bitmap,key,{get() {throw new Error('author dimension getter');}});
    const graph = [bitmap, bitmap]; graph.push(graph);
    const tx = db.transaction('values','readwrite'), complete = done(tx);
    tx.objectStore('values').put(graph,'bitmap'); bitmap.close(); await complete;
    db.close(); db = await request(indexedDB.open(name));
    const copy = await request(db.transaction('values').objectStore('values').get('bitmap'));
    check('alias',copy[0]===copy[1]); check('cycle',copy[2]===copy);
    check('prototype',Object.getPrototypeOf(copy[0])===ImageBitmap.prototype);
    check('dimensions',copy[0].width===2&&copy[0].height===1);
    check('expando excluded',copy[0].expando===undefined);
    const destination=new OffscreenCanvas(2,1);
    destination.getContext('bitmaprenderer').transferFromImageBitmap(copy[0]);
    const output=new OffscreenCanvas(2,1),drawing=output.getContext('2d');
    drawing.drawImage(destination,0,0);
    check('native pixels',JSON.stringify(Array.from(drawing.getImageData(0,0,2,1).data))===JSON.stringify(expected));
    const bad = db.transaction('values','readwrite'); const badDone=done(bad);
    let error;try {bad.objectStore('values').put(bitmap,'closed');} catch(e) {error=e;}
    check('closed bitmap rejected synchronously',error?.name==='DataCloneError'); await badDone;
    return {state:checks.every(c=>c.pass)?'pass':'fail',checks};
  } finally {db.close();await request(indexedDB.deleteDatabase(name));}
};
