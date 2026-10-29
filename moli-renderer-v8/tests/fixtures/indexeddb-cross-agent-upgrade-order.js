globalThis.upgradeOrderProbe = {state: 'pending', events: []};
(() => {
  const config = globalThis.upgradeOrderConfig;
  const events = upgradeOrderProbe.events;
  const name = `cross-agent-upgrade-order-${config.holder}-${config.requester}`;
  const controls = {};
  const workers = [];
  const urls = [];
  let finished = false;
  let requesterStarted = false;
  const seen = new Set();
  const deadline = setTimeout(() => finish('timeout'), 5000);

  function finish(state) {
    if (finished) return;
    finished = true;
    clearTimeout(deadline);
    if (controls.holder) controls.holder({action: 'close'});
    for (const worker of workers) worker.terminate();
    for (const url of urls) URL.revokeObjectURL(url);
    upgradeOrderProbe.state = state;
  }

  // This function is also serialized into each worker. The request pump keeps
  // the first upgrade alive until the other event loop has queued its open.
  function agentMain(options, send) {
    const {role, name} = options;
    let database;
    let released = false;
    let openSucceeded = false;
    let successMicrotaskRan = false;
    const fail = detail => send({type: 'error', detail});
    const request = indexedDB.open(name, role === 'holder' ? 1 : 2);
    request.onerror = () => fail(`open:${request.error.name}`);
    request.onblocked = () => send({type: 'blocked'});
    request.onupgradeneeded = event => {
      if (role !== 'holder') {
        if (event.oldVersion !== 1 || event.newVersion !== 2 ||
            !request.result.objectStoreNames.contains('records')) {
          fail('incorrect-upgrade-state');
        }
        send({type: 'upgrade'});
        return;
      }
      database = request.result;
      database.onversionchange = event => {
        if (!openSucceeded || !successMicrotaskRan) {
          fail('versionchange-before-open-success-microtask');
          return;
        }
        if (event.oldVersion !== 1 || event.newVersion !== 2) {
          fail('incorrect-versionchange');
          return;
        }
        send({type: 'versionchange'});
        queueMicrotask(() => {
          database.close();
          send({type: 'close-microtask'});
        });
      };
      const transaction = request.transaction;
      const store = database.createObjectStore('records');
      store.put('seed', 1);
      transaction.onabort = () => fail(`initial-abort:${transaction.error?.name}`);
      transaction.oncomplete = () => {
        send({type: 'complete'});
        if (request.transaction !== transaction) {
          fail('upgrade-transaction-cleared-before-complete');
          return;
        }
        // The open result must already have its queue position when this
        // listener schedules an ordinary transaction result.
        const readTransaction = database.transaction('records');
        const read = readTransaction.objectStore('records').get(1);
        read.onerror = () => fail(`read:${read.error.name}`);
        read.onsuccess = () => {
          if (!openSucceeded || !successMicrotaskRan || read.result !== 'seed') {
            fail('regular-read-before-open-success-microtask');
            return;
          }
          send({type: 'read-success'});
        };
        readTransaction.onabort = () => fail(`read-abort:${readTransaction.error?.name}`);
        readTransaction.oncomplete = () => send({type: 'read-complete'});
      };
      function keepUpgradeActive() {
        const pending = store.get(1);
        pending.onerror = () => fail(`pump:${pending.error.name}`);
        pending.onsuccess = () => {
          if (!released) keepUpgradeActive();
        };
      }
      keepUpgradeActive();
      send({type: 'upgrading'});
    };
    request.onsuccess = () => {
      if (request.transaction !== null) {
        fail('upgrade-transaction-not-cleared-before-success');
        return;
      }
      if (role === 'holder') {
        openSucceeded = true;
        send({type: 'open-success'});
        queueMicrotask(() => {
          successMicrotaskRan = true;
          send({type: 'success-microtask'});
        });
      } else {
        request.result.close();
        send({type: 'upgrade-success'});
      }
    };
    if (role === 'requester') send({type: 'queued'});
    return data => {
      if (data.action === 'release') released = true;
      if (data.action === 'close' && database) database.close();
    };
  }

  function record(role, data) {
    if (finished) return;
    events.push({role, ...data});
    if (data.type === 'error') {
      finish(`error:${role}:${data.detail}`);
      return;
    }
    seen.add(`${role}:${data.type}`);
    if (data.type === 'upgrading' && !requesterStarted) {
      requesterStarted = true;
      startAgent('requester', config.requester);
    }
    if (data.type === 'queued') controls.holder({action: 'release'});
    if (seen.has('holder:read-complete') && seen.has('holder:close-microtask') &&
        seen.has('requester:upgrade-success')) {
      const position = (role, type) => events.findIndex(e => e.role === role && e.type === type);
      const order = [
        ['holder', 'upgrading'], ['requester', 'queued'], ['holder', 'complete'],
        ['holder', 'open-success'], ['holder', 'success-microtask'], ['holder', 'versionchange']
      ].map(([role, type]) => position(role, type));
      const ordered = order.every((index, i) => index >= 0 && (i === 0 || order[i - 1] < index));
      finish(ordered && position('holder', 'success-microtask') < position('holder', 'read-success') &&
        events.filter(e => e.type === 'versionchange').length === 1 &&
        events.filter(e => e.type === 'upgrade').length === 1 ? 'pass' : 'unexpected-order');
    }
  }

  function startAgent(role, kind) {
    if (kind === 'window') {
      controls[role] = agentMain({role, name}, data => record(role, data));
      return;
    }
    const source = `const start = ${agentMain.toString()}; let control;
      onmessage = event => {
        if (control) control(event.data);
        else control = start(event.data, data => postMessage(data));
      };`;
    const url = URL.createObjectURL(new Blob([source], {type: 'text/javascript'}));
    urls.push(url);
    const worker = new Worker(url);
    workers.push(worker);
    controls[role] = data => worker.postMessage(data);
    worker.onmessage = event => record(role, event.data);
    worker.onerror = event => finish(`worker-error:${event.message}`);
    worker.postMessage({role, name});
  }
  startAgent('holder', config.holder);
})();
