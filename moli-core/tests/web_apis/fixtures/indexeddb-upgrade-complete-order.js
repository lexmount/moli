globalThis.upgradeCompleteChecks = [];
async function upgradeCompleteOrderProbe(prefix) {
  const checks = upgradeCompleteChecks;
  const check = (label, pass, actual) => checks.push({label, pass, actual: String(actual)});
  const result = request => new Promise((resolve, reject) => {
    request.onsuccess = () => resolve(request.result);
    request.onerror = event => { event.preventDefault(); reject(request.error); };
  });
  const scriptTrace = [];
  const target = new EventTarget();
  target.addEventListener('probe', () => {
    scriptTrace.push('first'); queueMicrotask(() => scriptTrace.push('microtask'));
  });
  target.addEventListener('probe', () => scriptTrace.push('second'));
  target.dispatchEvent(new Event('probe'));
  check('script dispatch keeps its outer boundary', scriptTrace.join() === 'first,second', scriptTrace);
  await Promise.resolve();
  check('script dispatch completes deferred microtasks', scriptTrace.join() === 'first,second,microtask', scriptTrace);
  for (const closing of ['none', 'sync', 'microtask']) {
    const name = prefix + '-' + closing;
    const events = [];
    const callbackTrace = [];
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
            check(closing + ': transaction retained through complete callback microtask', opening.transaction === upgrade);
          });
        };
        let callbackTransaction;
        upgrade.addEventListener('complete', () => {
          callbackTransaction = db.transaction('s');
          queueMicrotask(() => {
            try {
              callbackTransaction.objectStore('s').get(1);
              check(closing + ': callback-created transaction active in microtask', true);
            } catch (error) {
              check(closing + ': callback-created transaction active in microtask', false, error.name);
            }
          });
        }, {capture: true});
        for (const [name, capture] of [['capture', true], ['ordinary', false]]) {
          upgrade.addEventListener('complete', () => {
            if (!capture) {
              let actual = 'accepted';
              try { callbackTransaction.objectStore('s').get(1); }
              catch (error) { actual = error.name; }
              check(closing + ': callback-created transaction inactive before next listener',
                actual === 'TransactionInactiveError', actual);
            }
            callbackTrace.push(name);
            queueMicrotask(() => callbackTrace.push(name + ':microtask'));
          }, {capture});
        }
        upgrade.onabort = () => reject(upgrade.error || new Error('unexpected upgrade abort'));
      };
    });
    const outcome = await finished;
    check(closing + ': callback microtask order',
      callbackTrace.join() === 'capture,capture:microtask,ordinary,ordinary:microtask', callbackTrace);

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
