globalThis.connectionProbe = {state: 'pending', events: []};
(() => {
  const config = globalThis.connectionConfig;
  const name = ['cross-agent', config.holder, config.requester, config.operation, config.closeMode].join('-');
  const events = connectionProbe.events;
  const workers = [];
  const urls = [];
  let database;
  let holderWorker;
  let finished = false;
  const waiters = {};
  function waitFor(type) {
    return new Promise(resolve => { waiters[type] = resolve; });
  }
  const ready = waitFor('ready');
  const notified = waitFor('notification');
  const closed = waitFor('closed');
  const succeeded = waitFor('success');
  function finish(state) {
    if (finished) return;
    finished = true;
    clearTimeout(deadline);
    if (database) database.close();
    for (const worker of workers) worker.terminate();
    for (const url of urls) URL.revokeObjectURL(url);
    connectionProbe.state = state;
  }
  const deadline = setTimeout(() => finish('timeout'), 3000);
  function closeHolder() {
    if (holderWorker) holderWorker.postMessage({action: 'close'});
    else {
      database.close();
      record('holder', {type: 'closed'});
    }
  }
  function record(role, data) {
    if (finished) return;
    events.push({role, ...data});
    if (data.type === 'error') {
      finish(`error:${data.name}`);
      return;
    }
    if (data.type === 'blocked' && config.closeMode === 'blocked') closeHolder();
    if (waiters[data.type]) waiters[data.type](data);
  }
  function workerMain() {
    let database;
    onmessage = event => {
      const data = event.data;
      if (data.action === 'close') {
        database.close();
        postMessage({type: 'closed'});
        return;
      }
      const {role, name, operation, closeMode} = data;
      const request = role === 'holder' ? indexedDB.open(name, 1) :
        operation === 'upgrade' ? indexedDB.open(name, 2) : indexedDB.deleteDatabase(name);
      request.onupgradeneeded = e => {
        if (role === 'holder') request.result.createObjectStore('records');
        else postMessage({type: 'upgrade', oldVersion: e.oldVersion, newVersion: e.newVersion});
      };
      request.onblocked = () => postMessage({type: 'blocked'});
      request.onerror = () => postMessage({type: 'error', name: request.error.name});
      request.onsuccess = () => {
        if (role === 'holder') {
          database = request.result;
          database.onversionchange = e => {
            postMessage({type: 'notification', oldVersion: e.oldVersion, newVersion: e.newVersion});
            const closeDatabase = () => { database.close(); postMessage({type: 'closed'}); };
            if (closeMode === 'microtask') queueMicrotask(closeDatabase);
            if (closeMode === 'timer') setTimeout(closeDatabase, 0);
          };
          postMessage({type: 'ready'});
        } else {
          if (operation === 'upgrade') request.result.close();
          postMessage({type: 'success'});
        }
      };
    };
  }
  function startWorker(role) {
    const url = URL.createObjectURL(new Blob([`(${workerMain.toString()})()`], {type: 'text/javascript'}));
    urls.push(url);
    const worker = new Worker(url);
    workers.push(worker);
    worker.onmessage = event => record(role, event.data);
    worker.onerror = event => finish(`worker-error:${event.message}`);
    worker.postMessage({role, name, ...config});
    return worker;
  }
  if (config.holder === 'worker') holderWorker = startWorker('holder');
  else {
    const initial = indexedDB.open(name, 1);
    initial.onupgradeneeded = () => initial.result.createObjectStore('records');
    initial.onerror = () => finish(`initial-error:${initial.error.name}`);
    initial.onsuccess = () => {
      database = initial.result;
      database.onversionchange = e => {
        record('holder', {type: 'notification', oldVersion: e.oldVersion, newVersion: e.newVersion});
        if (config.closeMode === 'microtask') queueMicrotask(closeHolder);
        if (config.closeMode === 'timer') setTimeout(closeHolder, 0);
      };
      record('holder', {type: 'ready'});
    };
  }
  ready.then(() => {
    if (config.requester === 'worker') startWorker('requester');
    else {
      const request = config.operation === 'upgrade' ? indexedDB.open(name, 2) : indexedDB.deleteDatabase(name);
      request.onblocked = () => record('requester', {type: 'blocked'});
      request.onerror = () => finish(`operation-error:${request.error.name}`);
      request.onupgradeneeded = e => record('requester', {type: 'upgrade', oldVersion: e.oldVersion, newVersion: e.newVersion});
      request.onsuccess = () => {
        if (config.operation === 'upgrade') request.result.close();
        record('requester', {type: 'success'});
      };
    }
  });
  Promise.all([notified, closed, succeeded]).then(([notification]) => {
    const count = type => events.filter(event => event.type === type).length;
    const expectedBlocked = config.closeMode === 'microtask' ? 0 : 1;
    const expectedVersion = config.operation === 'upgrade' ? 2 : null;
    finish(notification.oldVersion === 1 && notification.newVersion === expectedVersion &&
      count('notification') === 1 && count('blocked') === expectedBlocked && count('success') === 1 &&
      count('upgrade') === (config.operation === 'upgrade' ? 1 : 0) ? 'pass' : 'unexpected-events');
  });
})();
