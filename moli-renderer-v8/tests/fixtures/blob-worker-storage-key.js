(async () => {
  const rows = [];
  const outer = document.createElement('iframe');
  const foreignWindow = new Promise(resolve => {
    const ready = event => {
      if (event.data !== 'blob-worker-child-ready') return;
      removeEventListener('message', ready);
      resolve(event.source);
    };
    addEventListener('message', ready);
  });
  outer.src = `http://cross-site.test:${location.port}/outer`;
  document.body.appendChild(outer);
  const foreign = await foreignWindow;
  const urls = [];
  function scriptURL(realm, source) {
    const url = realm.URL.createObjectURL(new realm.Blob([source], {type: 'text/javascript'}));
    urls.push([realm, url]);
    return url;
  }
  function start(realm, kind, type, url) {
    return new Promise(resolve => {
      let worker;
      try {
        worker = new realm[kind](url, {type, name: type});
      } catch (error) {
        resolve(`throw:${error.name}`);
        return;
      }
      const finish = value => {
        if (kind === 'SharedWorker') worker.port.close();
        else worker.terminate();
        resolve(value);
      };
      worker.onerror = event => {
        event.preventDefault();
        finish('error');
      };
      const target = kind === 'SharedWorker' ? worker.port : worker;
      target.onmessage = event => finish(event.data);
    });
  }
  try {
    for (const kind of ['Worker', 'SharedWorker']) {
      const source = kind === 'Worker'
        ? `postMessage('loaded')`
        : `onconnect = event => event.ports[0].postMessage('loaded')`;
      const url = scriptURL(window, source);
      for (const type of ['classic', 'module']) {
        for (const [name, realm, expected] of [
          ['first-party', window, 'loaded'],
          ['cross-site-ancestor', foreign, 'error']
        ]) {
          rows.push({kind, type, name, expected, observed: await start(realm, kind, type, url)});
        }
      }
    }
    for (const type of ['classic', 'module']) {
      const deniedURL = scriptURL(window, `postMessage('loaded')`);
      const source = `
        const type = ${JSON.stringify(type)};
        async function start(url) {
          return new Promise(resolve => {
            let worker;
            try { worker = new Worker(url, {type}); }
            catch (error) { resolve('throw:' + error.name); return; }
            worker.onerror = event => {
              event.preventDefault(); worker.terminate(); resolve('error');
            };
            worker.onmessage = event => { worker.terminate(); resolve(event.data); };
          });
        }
        const own = URL.createObjectURL(new Blob(["postMessage('loaded')"], {type: 'text/javascript'}));
        (async () => {
          const allowed = await start(own);
          const denied = await start(${JSON.stringify(deniedURL)});
          URL.revokeObjectURL(own);
          postMessage([allowed, denied]);
        })();`;
      const parentURL = scriptURL(foreign, source);
      const result = await start(foreign, 'Worker', type, parentURL);
      rows.push({kind: 'nested Worker', type, name: 'own', expected: 'loaded', observed: result[0]});
      rows.push({kind: 'nested Worker', type, name: 'foreign partition', expected: 'error', observed: result[1]});
    }
    return rows;
  } finally {
    for (const [realm, url] of urls) realm.URL.revokeObjectURL(url);
    outer.remove();
  }
})()
