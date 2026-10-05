(async () => {
  const checks = [];
  const check = (name, passed) => checks.push({name, passed: !!passed});
  const progress = globalThis.__uiEventResults = {complete: false, checks, stage: 'initialization'};
  const request = req => new Promise((resolve, reject) => {
    req.onsuccess = () => resolve(req.result); req.onerror = () => reject(req.error);
  });
  const realms = [globalThis];
  if (typeof document !== 'undefined') realms.push(document.getElementById('child').contentWindow);
  try {
    for (const [index, realm] of realms.entries()) {
      const color = new realm.VideoColorSpace({matrix: 'rgb', fullRange: true});
      let reads = 0;
      Object.defineProperty(color, 'author', {enumerable: true, get() {reads++; throw Error('author getter');}});
      const ports = new realm.MessageChannel();
      try {
        let error;
        try {ports.port1.postMessage({color});} catch (caught) {error = caught;}
        check(`${index}: MessagePort rejects native color metadata without author getters`, error instanceof realm.DOMException && error.name === 'DataCloneError' && reads === 0);
        const delivered = new Promise(resolve => {ports.port2.onmessage = event => resolve(event.data);});
        progress.stage = `${index}: MessagePort delivery`;
        ports.port1.postMessage('safe');
        check(`${index}: rejected serialization leaves MessagePort usable`, await delivered === 'safe');
      } finally {ports.port1.close(); ports.port2.close();}
      progress.stage = `${index}: IndexedDB open`;
      const name = `video-color-space-storage-${index}`, open = realm.indexedDB.open(name, 1);
      open.onupgradeneeded = () => open.result.createObjectStore('metadata');
      const db = await request(open);
      try {
        const tx = db.transaction('metadata', 'readwrite'), store = tx.objectStore('metadata');
        const done = new Promise((resolve, reject) => {tx.oncomplete = resolve; tx.onabort = () => reject(tx.error);});
        let error;
        try {store.put({color}, 'rejected');} catch (caught) {error = caught;}
        check(`${index}: IndexedDB rejects native metadata synchronously in callee realm`, error instanceof realm.DOMException && error.name === 'DataCloneError' && reads === 0);
        progress.stage = `${index}: IndexedDB commit`;
        store.put('safe', 'accepted'); await done;
        const read = db.transaction('metadata');
        const rejected = request(read.objectStore('metadata').get('rejected'));
        const accepted = request(read.objectStore('metadata').get('accepted'));
        progress.stage = `${index}: IndexedDB read`;
        check(`${index}: clone rejection leaves transaction usable and inserts no record`, await rejected === undefined && await accepted === 'safe');
      } finally {progress.stage = `${index}: IndexedDB delete`; db.close(); await request(realm.indexedDB.deleteDatabase(name));}
    }
  } catch (error) {checks.push({name: 'async completion', passed: false, error: String(error), stack: error.stack});}
  globalThis.__uiEventResults = {complete: true, passed: checks.filter(row => row.passed).length, total: checks.length, checks};
  return true;
})()
