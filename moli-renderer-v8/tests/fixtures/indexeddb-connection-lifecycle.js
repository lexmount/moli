(async () => {
  const requestResult = request => new Promise((resolve, reject) => {
    request.onsuccess = () => resolve(request.result);
    request.onerror = event => { event.preventDefault(); reject(request.error); };
  });
  const exceptionName = action => {
    try { action(); return 'unexpected-success'; }
    catch (error) { return error.name; }
  };
  const openDatabase = (name, version, upgrade) => {
    const request = version === undefined ? indexedDB.open(name) : indexedDB.open(name, version);
    if (upgrade) request.onupgradeneeded = event => upgrade(request, event);
    return requestResult(request);
  };

  const terminal = {};
  for (const abort of [false, true]) {
    const label = abort ? 'abort' : 'complete';
    const request = indexedDB.open('connection-terminal-' + label, 1);
    const trace = [];
    request.onupgradeneeded = () => {
      const db = request.result;
      const transaction = request.transaction;
      db.createObjectStore('original');
      transaction['on' + label] = () => {
        trace.push(label);
        terminal[label] = {
          transactionAttachedDuringEvent: request.transaction === transaction,
          create: exceptionName(() => db.createObjectStore('late')),
          remove: exceptionName(() => db.deleteObjectStore('original')),
          trace,
        };
        queueMicrotask(() => trace.push(label + '-microtask'));
      };
      if (abort) transaction.abort();
    };
    try {
      const db = await requestResult(request);
      trace.push('open-success');
      db.close();
    } catch (error) {
      trace.push('open-error:' + error.name);
    }
    terminal[label].transactionClearedAfterResult = request.transaction === null;
  }

  const db = await openDatabase('connection-delete-order', 1);
  const deletionOrder = [];
  const deletion = label => requestResult(indexedDB.deleteDatabase(db.name))
    .then(() => deletionOrder.push(label));
  const firstDelete = deletion('delete1');
  db.close();
  const secondDelete = deletion('delete2');
  await Promise.all([firstDelete, secondDelete]);

  // Both calls are accepted before any event runs. The second must see the
  // first upgrade's committed schema, not start another initial upgrade.
  let initialUpgrades = 0;
  const initial = openDatabase('connection-concurrent-open', 1, request => {
    initialUpgrades++;
    request.result.createObjectStore('records').put('retained', 1);
  });
  const joined = openDatabase('connection-concurrent-open', 1, () => initialUpgrades++);
  const [first, second] = await Promise.all([initial, joined]);
  const joinedValue = await requestResult(second.transaction('records').objectStore('records').get(1));
  first.close();
  second.close();

  // Resolve versions at the queue head, after an earlier upgrade/delete, not
  // at API admission while the database still has its old committed version.
  (await openDatabase('connection-version-order', 1)).close();
  const upgrades = [];
  const upgraded = openDatabase('connection-version-order', 2, (_, event) => {
    upgrades.push([event.oldVersion, event.newVersion]);
  });
  const implicit = openDatabase('connection-version-order');
  const obsolete = openDatabase('connection-version-order', 1)
    .then(db => { db.close(); return 'unexpected-success'; }, error => error.name);
  const [upgradedDb, implicitDb, obsoleteResult] = await Promise.all([upgraded, implicit, obsolete]);
  const implicitVersion = implicitDb.version;
  upgradedDb.close();
  implicitDb.close();

  // A versionchange callback can close in a microtask. The blocked check must
  // happen after that checkpoint, not immediately after the callback returns.
  const blocker = await openDatabase('connection-microtask-close', 1);
  const notifications = [];
  blocker.onversionchange = event => {
    notifications.push('versionchange:' + event.oldVersion + ':' + event.newVersion);
    queueMicrotask(() => { notifications.push('close-microtask'); blocker.close(); });
  };
  const upgrade = indexedDB.open(blocker.name, 2);
  upgrade.onblocked = () => notifications.push('unexpected-blocked');
  upgrade.onupgradeneeded = () => notifications.push('upgrade');
  (await requestResult(upgrade)).close();
  notifications.push('success');

  // A blocked head must not block a different database, but later requests
  // for its own key (including opens at the old version) cannot pass it.
  const held = await openDatabase('connection-blocked-head', 1);
  const blockedOrder = [];
  const head = indexedDB.open(held.name, 2);
  head.onblocked = () => {
    blockedOrder.push('blocked');
    openDatabase('connection-independent', 1).then(other => {
      blockedOrder.push('independent');
      other.close();
      held.close();
    });
  };
  const headResult = requestResult(head).then(db => {
    blockedOrder.push('head:2'); db.close();
  });
  const oldVersion = openDatabase(held.name, 1).then(
    db => { db.close(); blockedOrder.push('unexpected-old-success'); },
    error => blockedOrder.push('old:' + error.name),
  );
  const tail = openDatabase(held.name).then(db => {
    blockedOrder.push('tail:' + db.version); db.close();
  });
  await Promise.all([headResult, oldVersion, tail]);

  const failedHeads = {};
  for (const failure of ['abort', 'close']) {
    const name = 'connection-failed-head-' + failure;
    (await openDatabase(name, 1)).close();
    const events = [];
    const failing = indexedDB.open(name, 2);
    failing.onupgradeneeded = () => {
      failing.result.createObjectStore('upgrade');
      if (failure === 'abort') failing.transaction.abort();
      else failing.result.close();
    };
    const failed = requestResult(failing).then(
      db => { db.close(); events.push('unexpected-success'); },
      error => events.push(error.name),
    );
    const recovered = openDatabase(name).then(db => {
      events.push('reopened:' + db.version);
      events.push('stores:' + Array.from(db.objectStoreNames).join(','));
      db.close();
    });
    await Promise.all([failed, recovered]);
    failedHeads[failure] = events;
  }

  // The upstream open-request-queue WPT's open/delete interleaving, with a
  // no-version open at the end to witness version resolution after deletion.
  const mixedOrder = [];
  const mixedOpen = (version, label) => {
    const name = 'connection-mixed-order';
    const request = version === undefined ? indexedDB.open(name) : indexedDB.open(name, version);
    request.onupgradeneeded = event => mixedOrder.push(label + ':upgrade:' + event.oldVersion + ':' + event.newVersion);
    return requestResult(request).then(db => {
      mixedOrder.push(label + ':success:' + db.version);
      db.onversionchange = () => { mixedOrder.push(label + ':close'); db.close(); };
      return db;
    });
  };
  const mixedDelete = label => requestResult(indexedDB.deleteDatabase('connection-mixed-order'))
    .then(() => mixedOrder.push(label));
  const mixed = await Promise.all([
    mixedOpen(2, 'first'), mixedDelete('delete1'),
    mixedOpen(3, 'second'), mixedDelete('delete2'), mixedOpen(undefined, 'last'),
  ]);
  mixed[4].close();

  return {terminal, deletionOrder, initialUpgrades, joinedValue, upgrades,
          implicitVersion, obsoleteResult, notifications, blockedOrder, failedHeads, mixedOrder};
})()
