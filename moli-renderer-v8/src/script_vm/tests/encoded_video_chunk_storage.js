(async () => {
  const checks = [];
  const check = (name, passed) => checks.push({name, passed: !!passed});
  const request = req => new Promise((resolve, reject) => {
    req.onsuccess = () => resolve(req.result); req.onerror = () => reject(req.error);
  });
  const realms = [globalThis];
  if (typeof document !== 'undefined') realms.push(document.getElementById('child').contentWindow);
  try {
    for (const [index, realm] of realms.entries()) {
      const C = realm.EncodedVideoChunk;
      const chunk = new C({type: 'delta', timestamp: -3, data: new Uint8Array([7, 8, 9])});
      const ports = new realm.MessageChannel();
      try {
        const received = new Promise(resolve => {ports.port2.onmessage = event => resolve(event.data);});
        ports.port1.postMessage([chunk, chunk]);
        const graph = await received, dest = new Uint8Array(3);
        graph[0].copyTo(dest);
        check(`${index}: MessagePort native identity and destination prototype`, graph[0] !== chunk && graph[0] === graph[1] && Object.getPrototypeOf(graph[0]) === C.prototype);
        check(`${index}: MessagePort preserves nullable duration and negative timestamp`, graph[0].duration === null && graph[0].timestamp === -3 && graph[0].type === 'delta');
        check(`${index}: MessagePort preserves bytes and sender`, dest.join(',') === '7,8,9' && chunk.byteLength === 3);
      } finally {ports.port1.close(); ports.port2.close();}
      const name = `encoded-video-chunk-storage-${index}`, open = realm.indexedDB.open(name, 1);
      open.onupgradeneeded = () => open.result.createObjectStore('chunks');
      const db = await request(open);
      try {
        const tx = db.transaction('chunks', 'readwrite'), store = tx.objectStore('chunks');
        const done = new Promise((resolve, reject) => {tx.oncomplete = resolve; tx.onabort = () => reject(tx.error);});
        let error;
        try {store.put({chunk}, 'rejected');} catch (caught) {error = caught;}
        check(`${index}: IndexedDB rejects nested chunk synchronously in callee realm`, error instanceof realm.DOMException && error.name === 'DataCloneError');
        store.put('safe', 'accepted'); await done;
        const read = db.transaction('chunks');
        const rejected = request(read.objectStore('chunks').get('rejected'));
        const accepted = request(read.objectStore('chunks').get('accepted'));
        check(`${index}: clone rejection leaves transaction usable and inserts no record`, await rejected === undefined && await accepted === 'safe');
      } finally {db.close(); await request(realm.indexedDB.deleteDatabase(name));}
    }
  } catch (error) {checks.push({name: 'async completion', passed: false, error: String(error), stack: error.stack});}
  globalThis.__uiEventResults = {complete: true, passed: checks.filter(row => row.passed).length, total: checks.length, checks};
  return true;
})()
