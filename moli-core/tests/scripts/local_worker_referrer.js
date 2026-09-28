(async () => {
  const script = await (await fetch('/local-referrer-worker.js')).text();
  const source = location.origin + '/private-worker-referrer?probe=1';
  const cross = globalThis.localWorkerCrossOrigin;
  const checks = globalThis.localWorkerReferrerChecks = [];
  const failures = globalThis.localWorkerReferrerFailures = [];
  let sequence = 0;
  function policy(owner, value) {
    const meta = owner.document.createElement('meta');
    meta.name = 'referrer';
    meta.content = value;
    owner.document.head.append(meta);
    meta.remove();
  }
  function blob(owner) {
    return owner.URL.createObjectURL(new owner.Blob([script], {type: 'text/javascript'}));
  }
  function start(owner, url, type, shared, name = 'referrer-' + ++sequence) {
    const worker = shared ? new owner.SharedWorker(url, {type, name}) : new owner.Worker(url, {type});
    const channel = shared ? worker.port : worker;
    const pending = new Map();
    channel.onmessage = ({data}) => {
      const operation = pending.get(data.id);
      if (!operation) return;
      pending.delete(data.id);
      clearTimeout(operation.timer);
      if (data.error) operation.reject(new Error(data.error));
      else operation.resolve(data.value);
    };
    if (shared) channel.start();
    return {
      name,
      send(data) {
        return new Promise((resolve, reject) => {
          const id = ++sequence;
          const timer = setTimeout(() => reject(new Error('Worker reply timeout: ' + globalThis.localWorkerReferrerProgress)), 8000);
          pending.set(id, {resolve, reject, timer});
          channel.postMessage({...data, id});
        });
      },
      close() {
        if (shared) {channel.postMessage({kind: 'close'}); channel.close();}
        else worker.terminate();
      }
    };
  }
  async function check(worker, expectedPolicy, label) {
    for (const target of [location.origin, cross]) {
      for (const explicit of [false, true]) {
        const command = {kind: 'fetch', url: target + '/meta-policy-fetch?worker=' + ++sequence, referrer: source};
        if (explicit) command.policy = 'unsafe-url';
        const expected = explicit || expectedPolicy === 'unsafe-url' ? source :
          expectedPolicy === 'no-referrer' ? null :
          expectedPolicy === 'default' && target === location.origin ? source : location.origin + '/';
        globalThis.localWorkerReferrerProgress = label;
        const actual = await worker.send(command);
        checks.push({label, target, explicit, actual, expected});
        if (actual !== expected) failures.push(label + ': ' + JSON.stringify(actual) + ' != ' + JSON.stringify(expected));
      }
    }
  }
  for (const shared of [false, true]) {
    for (const type of ['classic', 'module']) {
      const label = (shared ? 'shared ' : 'dedicated ') + type;
      const active = [];
      const urls = [];
      const frame = document.createElement('iframe');
      frame.src = '/local-referrer-page.html?policy=no-referrer';
      await new Promise(resolve => {frame.onload = resolve; document.body.append(frame);});
      const child = frame.contentWindow;
      const launch = (owner, url) => {
        const worker = start(owner, url, type, shared);
        active.push(worker);
        return worker;
      };
      try {
        policy(window, 'unsafe-url');
        const responseUrl = blob(child); urls.push(responseUrl);
        const responseWorker = launch(window, responseUrl);
        await check(responseWorker, 'no-referrer', label + ' Blob creator response policy');
        policy(window, 'origin');
        const url = blob(window); urls.push(url);
        policy(window, 'no-referrer');
        const first = launch(window, url);
        await check(first, 'no-referrer', label + ' Blob creator policy at launch');
        policy(window, 'origin');
        await check(launch(window, url), 'origin', label + ' later worker');
        await check(first, 'no-referrer', label + ' existing worker snapshot');
        if (shared) {
          const reused = start(window, url, type, true, first.name); active.push(reused);
          await check(reused, 'no-referrer', label + ' reused worker snapshot');
        }

        policy(child, 'origin');
        await check(responseWorker, 'no-referrer', label + ' response policy snapshot after meta delivery');
        policy(window, 'no-referrer');
        const foreignUrl = blob(child); urls.push(foreignUrl);
        const foreign = launch(window, foreignUrl);
        await check(foreign, 'origin', label + ' Blob URL creator differs from constructor');
        policy(child, 'no-referrer');
        await check(launch(window, foreignUrl), 'no-referrer', label + ' creator meta update');
        await check(foreign, 'origin', label + ' prior foreign worker snapshot');

        policy(window, 'unsafe-url');
        const parentUrl = blob(window); urls.push(parentUrl);
        await check(launch(child, parentUrl), 'unsafe-url', label + ' child constructor uses Blob creator');
        const workerUrl = await first.send({kind: 'blob', source: script}); urls.push(workerUrl);
        await check(launch(window, workerUrl), 'no-referrer', label + ' worker-created Blob');

        for (const nestedType of shared ? [] : ['classic', 'module']) {
          const nested = {send: command => first.send({kind: 'nested', source: script, type: nestedType, command})};
          await check(nested, 'no-referrer', label + ' nested Blob ' + nestedType);
        }
        policy(window, 'no-referrer');
        await check(launch(window, '/local-referrer-worker.js?policy=origin&id=' + ++sequence), 'origin', label + ' network response policy');
        await check(launch(window, '/local-referrer-worker.js?id=' + ++sequence), 'default', label + ' network default policy');
        if (!shared) {
          const networkParent = launch(window, '/local-referrer-worker.js?policy=origin&id=' + ++sequence);
          const nestedUrl = location.origin + '/local-referrer-worker.js?policy=no-referrer&id=' + ++sequence;
          const nested = {send: command => networkParent.send({kind: 'nested', url: nestedUrl, type, command})};
          const entryReferrer = await nested.send({kind: 'entry-referrer'});
          checks.push({label: label + ' nested script request', actual: entryReferrer, expected: location.origin + '/'});
          if (entryReferrer !== location.origin + '/') failures.push(label + ': nested entry referrer ' + entryReferrer);
          await check(nested, 'no-referrer', label + ' nested network response policy');
        }
      } finally {
        for (const worker of active) worker.close();
        for (const url of urls) URL.revokeObjectURL(url);
        frame.remove();
      }
    }
  }
  if (failures.length) throw new Error(failures.join('\n'));
  return true;
})();
