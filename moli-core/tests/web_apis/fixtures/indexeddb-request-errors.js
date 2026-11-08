globalThis.requestErrorChecks = [];
async function requestErrorProbe(prefix) {
  const checks = requestErrorChecks;
  const check = (label, pass, actual = '') => checks.push({label, pass: !!pass, actual: String(actual)});
  const equal = (label, actual, expected) => check(label, JSON.stringify(actual) === JSON.stringify(expected), JSON.stringify(actual));
  const request = r => new Promise((resolve, reject) => {
    r.onsuccess = () => resolve(r.result);
    r.onerror = event => { event.preventDefault(); reject(r.error); };
  });
  const modes = ['unhandled', 'request', 'transaction', 'database', 'return-false', 'capture', 'stop-only', 'stop-cancel', 'throw', 'intrinsic', 'passive'];
  const OriginalEvent = Event;
  const suppressThrownListener = event => { if (String(event.message).includes('idb-listener-marker')) event.preventDefault(); };
  globalThis.addEventListener('error', suppressThrownListener);
  let completed = 0;
  try {
    for (const existing of [false, true]) for (const mode of modes) {
      const label = (existing ? 'upgrade ' : 'initial ') + mode, name = prefix + '-' + completed;
      if (existing) {
        const seed = indexedDB.open(name, 1);
        seed.onupgradeneeded = () => seed.result.createObjectStore('s').put('stable', 0);
        const db = await request(seed); db.close();
      }
      let db, tx, duplicate, eventObject, transactionOutcome, abortError, pendingError;
      const order = [], trace = [];
      const canceled = ['request', 'transaction', 'database', 'return-false', 'capture', 'stop-cancel'].includes(mode);
      const opening = indexedDB.open(name, existing ? 2 : 1);
      const opened = new Promise(resolve => {
        opening.onsuccess = () => { order.push('open-success'); resolve('success'); };
        opening.onerror = event => { event.preventDefault(); order.push('open-error'); equal(label + ' open error', opening.error.name, 'AbortError'); resolve('error'); };
      });
      opening.onupgradeneeded = () => {
        db = opening.result; tx = opening.transaction;
        const store = existing ? tx.objectStore('s') : db.createObjectStore('s');
        if (existing) db.createObjectStore('during-upgrade');
        store.add('first', 1);
        duplicate = store.add('duplicate', 1);
        const pending = store.add('tail', 2);
        pending.onerror = event => { event.preventDefault(); pendingError = pending.error.name; };
        const record = (name, target, phase, capture = false) => function(event) {
          if (event.target !== duplicate) return;
          trace.push(name);
          check(label + ' error target ' + name, event.target === duplicate && event.currentTarget === target && this === target);
          equal(label + ' event phase ' + name, event.eventPhase, phase);
          check(label + ' event flags ' + name, event.bubbles && event.cancelable && event.isTrusted && event instanceof OriginalEvent);
          check(label + ' same error event ' + name, !eventObject || event === eventObject);
          eventObject = event;
          equal(label + ' composed path ' + name, event.composedPath().map(value => value === duplicate ? 'request' : value === tx ? 'transaction' : value === db ? 'database' : '?'), ['request', 'transaction', 'database']);
          if ((mode === name && !capture) || (mode === 'capture' && name === 'database-capture')) event.preventDefault();
          if (name === 'request' && (mode === 'stop-only' || mode === 'stop-cancel')) event.stopPropagation();
          if (name === 'request' && mode === 'stop-cancel') event.preventDefault();
          if (name === 'request' && mode === 'throw') { event.preventDefault(); throw new Error('idb-listener-marker'); }
        };
        db.addEventListener('error', record('database-capture', db, 1, true), true);
        tx.addEventListener('error', record('transaction-capture', tx, 1, true), true);
        duplicate.addEventListener('error', record('request-capture', duplicate, 2, true), {capture: true, once: true});
        duplicate.addEventListener('error', record('request', duplicate, 2));
        tx.addEventListener('error', record('transaction', tx, 3));
        db.addEventListener('error', record('database', db, 3));
        if (mode === 'return-false') duplicate.onerror = () => false;
        if (mode === 'passive') duplicate.addEventListener('error', event => event.preventDefault(), {passive: true});
        if (mode === 'intrinsic') globalThis.Event = function() { throw new Error('author Event constructor must not run'); };
        tx.oncomplete = () => { transactionOutcome = 'complete'; order.push('complete'); };
        tx.onabort = () => {
          transactionOutcome = 'abort'; abortError = tx.error.name; order.push('abort');
          check(label + ' open retains transaction through abort', opening.transaction === tx);
          equal(label + ' rollback version', db.version, existing ? 1 : 0);
          equal(label + ' rollback schema', Array.from(db.objectStoreNames), existing ? ['s'] : []);
          try { db.transaction('s'); check(label + ' failed connection closed before abort', false); }
          catch (error) { equal(label + ' failed connection closed before abort', error.name, 'InvalidStateError'); }
        };
      };
      const outcome = await opened;
      globalThis.Event = OriginalEvent;
      equal(label + ' open outcome', outcome, canceled ? 'success' : 'error');
      equal(label + ' transaction outcome', transactionOutcome, canceled ? 'complete' : 'abort');
      equal(label + ' completion precedes open', order, canceled ? ['complete', 'open-success'] : ['abort', 'open-error']);
      equal(label + ' request error preserved', duplicate.error.name, 'ConstraintError');
      equal(label + ' pending request', pendingError, canceled ? undefined : 'AbortError');
      if (!canceled) equal(label + ' transaction cause', abortError, mode === 'throw' ? 'AbortError' : 'ConstraintError');
      equal(label + ' event path order', trace, mode.startsWith('stop-') ? ['database-capture', 'transaction-capture', 'request-capture', 'request'] : ['database-capture', 'transaction-capture', 'request-capture', 'request', 'transaction', 'database']);
      check(label + ' canceled flag', eventObject.defaultPrevented === (canceled || mode === 'throw'));
      check(label + ' dispatch cleanup', eventObject.currentTarget === null && eventObject.eventPhase === 0 && eventObject.composedPath().length === 0);
      check(label + ' open clears transaction', opening.transaction === null);
      if (outcome === 'success') db.close();
      const reopening = indexedDB.open(name);
      let oldVersion;
      reopening.onupgradeneeded = event => { oldVersion = event.oldVersion; };
      const reopened = await request(reopening);
      equal(label + ' durable version', reopened.version, canceled ? (existing ? 2 : 1) : 1);
      equal(label + ' durable schema', Array.from(reopened.objectStoreNames), canceled ? (existing ? ['during-upgrade', 's'] : ['s']) : existing ? ['s'] : []);
      if (!canceled && !existing) equal(label + ' new database upgrade restored to zero', oldVersion, 0);
      if (reopened.objectStoreNames.contains('s')) {
        const read = reopened.transaction('s').objectStore('s').getAll();
        equal(label + ' durable rows', await request(read), canceled ? (existing ? ['stable', 'first', 'tail'] : ['first', 'tail']) : ['stable']);
      }
      reopened.close();
      await request(indexedDB.deleteDatabase(name));
      completed++;
    }
    equal('all upgrade variants completed', completed, modes.length * 2);
  } finally {
    globalThis.Event = OriginalEvent;
    globalThis.removeEventListener('error', suppressThrownListener);
  }
  return {state: checks.every(check => check.pass) ? 'pass' : 'fail', checks};
}
