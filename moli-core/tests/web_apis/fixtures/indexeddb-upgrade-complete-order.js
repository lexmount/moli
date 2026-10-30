globalThis.upgradeCompleteChecks = [];
async function upgradeCompleteOrderProbe(prefix) {
  const checks = upgradeCompleteChecks;
  const check = (label, pass, actual) => checks.push({label, pass, actual: String(actual)});
  const result = request => new Promise((resolve, reject) => {
    request.onsuccess = () => resolve(request.result);
    request.onerror = event => { event.preventDefault(); reject(request.error); };
  });
  for (const closing of ['none', 'sync', 'microtask']) {
    const name = prefix + '-' + closing;
    const events = [];
    let db, upgrade, read, readDone;
    const opening = indexedDB.open(name, 1);
    const finished = new Promise((resolve, reject) => {
      opening.onerror = event => {
        event.preventDefault(); events.push('open:error');
        check(closing + ': open error', opening.error?.name === 'AbortError', opening.error?.name);
        resolve('error');
      };
      opening.onsuccess = () => { events.push('open:success'); resolve('success'); };
      opening.onupgradeneeded = () => {
        db = opening.result; upgrade = opening.transaction; events.push('upgrade');
        const store = db.createObjectStore('s');
        store.put('committed', 1).onsuccess = () => events.push('put:success');
        upgrade.oncomplete = () => {
          events.push('complete');
          check(closing + ': transaction retained during complete', opening.transaction === upgrade);
          if (closing === 'sync') db.close();
          else if (closing === 'microtask') queueMicrotask(() => db.close());
          else {
            const tx = db.transaction('s');
            readDone = new Promise((resolve, reject) => {
              tx.oncomplete = resolve; tx.onabort = () => reject(tx.error);
            });
            read = result(tx.objectStore('s').get(1)).then(value => {
              events.push('read:success');
              check('none: stored value', value === 'committed', value);
            });
          }
          queueMicrotask(() => {
            check(closing + ': transaction retained through complete microtasks', opening.transaction === upgrade);
          });
        };
        upgrade.onabort = () => reject(upgrade.error || new Error('unexpected upgrade abort'));
      };
    });
    const outcome = await finished;
    check(closing + ': outcome', outcome === (closing === 'none' ? 'success' : 'error'), outcome);
    check(closing + ': transaction cleared for result', opening.transaction === null);
    if (closing === 'none') {
      await read; await readDone;
      check('none: open before regular request', events.indexOf('open:success') < events.indexOf('read:success'), events);
    } else {
      check(closing + ': no success after close', !events.includes('open:success'), events);
    }
    check(closing + ': complete before result', events[0] === 'upgrade' && events[1] === 'put:success' && events[2] === 'complete', events);
    db.close();
    const reopened = await result(indexedDB.open(name, 1));
    const tx = reopened.transaction('s');
    const done = new Promise((resolve, reject) => { tx.oncomplete = resolve; tx.onabort = () => reject(tx.error); });
    check(closing + ': committed data survives', await result(tx.objectStore('s').get(1)) === 'committed');
    await done; reopened.close(); await result(indexedDB.deleteDatabase(name));
  }
  return {state: checks.every(row => row.pass) ? 'pass' : 'fail', checks};
}
