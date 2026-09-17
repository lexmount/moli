globalThis.schedulingChecks = [];

async function schedulingProbe(prefix = 'transaction-scheduling-' + Math.random()) {
  const checks = globalThis.schedulingChecks;
  const check = (name, pass, detail) => checks.push({name, pass: !!pass, detail});
  const request = req => new Promise(resolve => {
    req.onsuccess = () => resolve({value: req.result});
    req.onerror = event => { event.preventDefault(); resolve({error: req.error.name}); };
  });
  const done = (tx, log, label) => new Promise(resolve => {
    tx.addEventListener('complete', () => { if (log) log.push(label); resolve(true); });
    tx.addEventListener('abort', () => { if (log) log.push(label + ' abort'); resolve(false); });
  });
  const open = name => {
    const req = indexedDB.open(name, 1);
    req.onupgradeneeded = () => {
      for (const name of ['a', 'b', 'c']) req.result.createObjectStore(name).put('old', 'key');
    };
    return request(req).then(result => {
      if (result.error) throw new Error(result.error);
      return result.value;
    });
  };
  const run = async (label, body) => {
    const name = prefix + '-' + label, connections = [];
    try {
      const db = await open(name);
      connections.push(db);
      const other = await open(name);
      connections.push(other);
      await body(db, other, name);
    } catch (error) {
      check(label + ' unexpected exception', false, String(error));
    } finally {
      for (const db of connections) db.close();
    }
  };
  const read = async (db, stores) => {
    const tx = db.transaction(stores), completion = done(tx);
    const values = await Promise.all(stores.map(name => request(tx.objectStore(name).get('key'))));
    await completion;
    return values.map(result => result.value);
  };

  for (const crossConnection of [false, true]) {
    for (const firstMode of ['readonly', 'readwrite']) {
      for (const secondMode of ['readonly', 'readwrite']) {
        for (const [firstScope, secondScope] of [
          [['a'], ['a']], [['a', 'b'], ['b', 'c']], [['a'], ['b']]
        ]) {
          const label = [crossConnection, firstMode, secondMode, firstScope.join(''), secondScope.join('')].join('-');
          await run(label, async (db, other) => {
            const log = [];
            const first = db.transaction(firstScope, firstMode), firstDone = done(first, log, 'first');
            const second = (crossConnection ? other : db).transaction(secondScope, secondMode);
            const secondDone = done(second, log, 'second');
            if (firstMode === 'readwrite') {
              for (const name of firstScope) first.objectStore(name).put('first', 'key');
            }
            // Keep the older transaction active through several request events.
            // An overlapping younger writer must not execute during these events.
            const firstValues = [];
            const pump = left => {
              const req = first.objectStore(firstScope[0]).get('key');
              req.onsuccess = () => {
                firstValues.push(req.result);
                if (left) pump(left - 1);
              };
            };
            pump(5);
            const observations = secondScope.map(name => {
              const req = second.objectStore(name).get('key');
              return request(req).then(result => { log.push('read ' + name); return result; });
            });
            if (secondMode === 'readwrite') {
              for (const name of secondScope) second.objectStore(name).put('second', 'key');
            }
            const results = await Promise.all(observations);
            check(label + ' completes', await firstDone && await secondDone);
            const overlap = firstScope.some(name => secondScope.includes(name));
            if (overlap && (firstMode === 'readwrite' || secondMode === 'readwrite')) {
              check(label + ' waits for predecessor', log.indexOf('first') < log.indexOf('read ' + secondScope[0]), log);
            }
            check(label + ' stable older snapshot', firstValues.length === 6 &&
              firstValues.every(value => value === (firstMode === 'readwrite' ? 'first' : 'old')), firstValues);
            for (const [index, name] of secondScope.entries()) {
              const expected = firstMode === 'readwrite' && firstScope.includes(name) ? 'first' : 'old';
              check(label + ' observes ' + name, !results[index].error && results[index].value === expected, results[index]);
            }
            const final = await read(db, ['a', 'b', 'c']);
            const expected = ['a', 'b', 'c'].map(name =>
              secondMode === 'readwrite' && secondScope.includes(name) ? 'second' :
              firstMode === 'readwrite' && firstScope.includes(name) ? 'first' : 'old');
            check(label + ' preserves every committed store', JSON.stringify(final) === JSON.stringify(expected), final);
          });
        }
      }
    }
  }

  await run('pending-writer-fairness', async db => {
    const log = [];
    const first = db.transaction('a'), firstDone = done(first, log, 'first');
    const writer = db.transaction(['a', 'b'], 'readwrite'), writerDone = done(writer, log, 'writer');
    const last = db.transaction('b'), lastDone = done(last, log, 'last');
    const pump = left => {
      first.objectStore('a').get('key').onsuccess = () => { if (left) pump(left - 1); };
    };
    pump(8);
    writer.objectStore('b').put('writer', 'key');
    const result = await request(last.objectStore('b').get('key'));
    check('pending writer blocks a later reader of its other store', result.value === 'writer', result);
    check('pending fairness completes', await firstDone && await writerDone && await lastDone);
    check('pending fairness order', log.join(',') === 'first,writer,last', log);
  });

  for (const pending of [false, true]) {
    await run('abort-' + pending, async db => {
      const blocker = pending ? db.transaction('a') : null;
      const blockerDone = blocker ? done(blocker) : Promise.resolve(true);
      const writer = db.transaction(['a', 'b'], 'readwrite'), writerDone = done(writer);
      const write = request(writer.objectStore('b').put('aborted', 'key'));
      const later = db.transaction('b'), laterDone = done(later);
      const value = request(later.objectStore('b').get('key'));
      // Pending work has no backend handle; both paths must release admission.
      writer.abort();
      check('abort ' + pending + ' rolls back and unblocks reader', (await value).value === 'old');
      check('abort ' + pending + ' dispatches request error', (await write).error === 'AbortError');
      check('abort ' + pending + ' finishes', !(await writerDone) && await blockerDone && await laterDone);
    });
  }

  await run('immutable-start-scope', async db => {
    const writer = db.transaction('a', 'readwrite'), writerDone = done(writer);
    const reader = db.transaction('a'), readerDone = done(reader);
    const value = request(reader.objectStore('a').get('key'));
    writer.objectStore('a').put('new', 'key');
    for (const name of ['db', 'mode', 'objectStoreNames']) {
      Object.defineProperty(reader, name, {get() { throw new Error('scheduler read ' + name); }});
    }
    check('deferred start uses immutable native scope', (await value).value === 'new');
    check('deferred readonly completes', await writerDone && await readerDone);
  });

  await run('close-with-queued-reader', async (db, other) => {
    const writer = db.transaction('a', 'readwrite'), writerDone = done(writer);
    const reader = other.transaction('a'), readerDone = done(reader);
    const value = request(reader.objectStore('a').get('key'));
    writer.objectStore('a').put('new', 'key');
    other.close();
    let error;
    try { other.transaction('a'); } catch (caught) { error = caught.name; }
    check('close rejects new transactions', error === 'InvalidStateError', error);
    check('close lets accepted reader observe earlier write', (await value).value === 'new');
    check('close drains accepted transactions', await writerDone && await readerDone);
  });

  await run('empty-readers', async db => {
    const log = [], empty = [];
    const writer = db.transaction('a', 'readwrite'), writerDone = done(writer, log, 'writer');
    const store = writer.objectStore('a');
    store.put('one', 'key').onsuccess = () => {
      log.push('one');
      empty.push(done(db.transaction('a'), log, 'reader1'), done(db.transaction('a'), log, 'reader2'));
      store.put('two', 'key').onsuccess = () => log.push('two');
    };
    await writerDone;
    check('empty readers complete', (await Promise.all(empty)).every(Boolean));
    check('empty readers wait for writer', log.join(',') === 'one,two,writer,reader1,reader2', log);
  });

  for (const [mode, laterMode, action] of [
    ['readwrite', 'readonly', 'commit'],
    ['readwrite', 'readwrite', 'abort'],
    ['readonly', 'readwrite', 'commit'],
    ['readwrite', 'readonly', 'terminate']
  ]) {
    const label = [mode, laterMode, action].join('-');
    await run('worker-' + label, async (db, other, name) => {
      const source = '(' + schedulingWorker.toString() + ')()';
      const url = URL.createObjectURL(new Blob([source], {type: 'text/javascript'}));
      const worker = new Worker(url), messages = [], waiters = [];
      worker.onmessage = event => {
        if (waiters.length) waiters.shift()(event.data); else messages.push(event.data);
      };
      const next = () => messages.length ? Promise.resolve(messages.shift()) : new Promise(resolve => waiters.push(resolve));
      try {
        worker.postMessage({name, mode});
        check(label + ' worker holds transaction', (await next()).state === 'holding');
        const tx = db.transaction('a', laterMode), completion = done(tx);
        const req = tx.objectStore('a').get('key'), value = request(req);
        if (laterMode === 'readwrite') tx.objectStore('a').put('parent', 'key');
        worker.postMessage({checkpoint: true});
        check(label + ' worker checkpoint', (await next()).state === 'checkpoint');
        check(label + ' waits across agents', req.readyState === 'pending', req.readyState);
        if (action === 'terminate') worker.terminate();
        else worker.postMessage({release: action});
        const expected = mode === 'readwrite' && action === 'commit' ? 'worker' : 'old';
        check(label + ' observes committed snapshot', (await value).value === expected);
        check(label + ' parent completes', await completion);
        if (action !== 'terminate') {
          check(label + ' worker finishes', (await next()).state === (action === 'abort' ? 'aborted' : 'complete'));
        }
        check(label + ' final value', (await read(other, ['a']))[0] === (laterMode === 'readwrite' ? 'parent' : expected));
      } finally {
        worker.terminate();
        URL.revokeObjectURL(url);
      }
    });
  }
  await run('terminate-pending-worker', async (db, other, name) => {
    const url = URL.createObjectURL(new Blob(['(' + schedulingWorker.toString() + ')()'], {type: 'text/javascript'}));
    const worker = new Worker(url);
    let keepAlive = true;
    const writer = db.transaction('a', 'readwrite'), writerDone = done(writer);
    writer.objectStore('a').put('parent', 'key');
    const pump = () => {
      writer.objectStore('a').get('key').onsuccess = () => { if (keepAlive) pump(); };
    };
    pump();
    try {
      const queued = new Promise(resolve => { worker.onmessage = event => resolve(event.data); });
      worker.postMessage({name, mode: 'readwrite', notifyQueued: true});
      check('pending worker accepts transaction', (await queued).state === 'queued');
      const reader = other.transaction('a'), readerDone = done(reader);
      const value = request(reader.objectStore('a').get('key'));
      worker.terminate();
      keepAlive = false;
      check('terminating pending worker unblocks its successor', (await value).value === 'parent');
      check('pending worker teardown preserves live transactions', await writerDone && await readerDone);
    } finally {
      keepAlive = false;
      worker.terminate();
      URL.revokeObjectURL(url);
    }
  });
  return {state: checks.every(entry => entry.pass) ? 'pass' : 'fail', checks};
}

// Executed in its own agent. Request chaining keeps the transaction live until
// an explicit handshake releases it; no timeout is used to infer completion.
function schedulingWorker() {
  let release, checkpoint = false;
  onmessage = event => {
    const message = event.data;
    if (message.release) { release = message.release; return; }
    if (message.checkpoint) { checkpoint = true; return; }
    const req = indexedDB.open(message.name, 1);
    req.onerror = () => postMessage({state: 'error', error: req.error.name});
    req.onsuccess = () => {
      const db = req.result;
      let tx;
      try { tx = db.transaction('a', message.mode); }
      catch (error) { db.close(); postMessage({state: 'error', error: error.name}); return; }
      const store = tx.objectStore('a');
      if (message.notifyQueued) postMessage({state: 'queued'});
      tx.oncomplete = () => { db.close(); postMessage({state: 'complete'}); };
      tx.onabort = () => { db.close(); postMessage({state: 'aborted'}); };
      const first = message.mode === 'readwrite' ? store.put('worker', 'key') : store.get('key');
      const pump = () => {
        store.get('key').onsuccess = () => {
          if (release === 'abort') tx.abort();
          else if (!release) {
            pump();
            if (checkpoint) { checkpoint = false; postMessage({state: 'checkpoint'}); }
          }
        };
      };
      first.onsuccess = () => { pump(); postMessage({state: 'holding'}); };
    };
  };
}
