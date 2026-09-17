async function mediaSourceUrlProbe() {
  const checks = [];
  const urls = [];
  const check = (label, actual, wanted) => checks.push({label, actual, wanted, pass: JSON.stringify(actual) === JSON.stringify(wanted)});
  const create = (source, api = URL) => {
    try { const url = api.createObjectURL(source); urls.push(url); return url; }
    catch (error) { return error.name; }
  };
  const isUrl = value => typeof value === 'string' && value.startsWith('blob:');
  const fetchResult = async input => {
    let promise;
    try { promise = fetch(input); } catch (error) { return 'sync:' + error.name; }
    try { await promise; return 'resolved'; } catch (error) { return 'async:' + error.name; }
  };
  try {
    const source = new MediaSource();
    const url = create(source);
    const other = create(source);
    check('create', isUrl(url), true);
    check('unique-urls', isUrl(url) && isUrl(other) && url !== other, true);
    check('origin', isUrl(url) ? new URL(url).origin : url, location.origin);
    check('uuid', isUrl(url) && /\/[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/.test(url), true);
    const inherited = new (class extends MediaSource {})();
    check('subclass', isUrl(create(inherited)), true);
    const changedPrototype = new MediaSource();
    Object.setPrototypeOf(changedPrototype, null);
    check('changed-prototype', isUrl(create(changedPrototype)), true);
    let publicReads = 0;
    for (const key of ['constructor', 'readyState', 'toString', Symbol.toStringTag, Symbol.toPrimitive]) {
      Object.defineProperty(source, key, {get() { publicReads++; throw new Error('public member'); }});
    }
    check('private-identity', isUrl(create(source)), true);
    check('no-public-reads', publicReads, 0);
    const descriptor = Object.getOwnPropertyDescriptor(globalThis, 'MediaSource');
    try {
      globalThis.MediaSource = function() { throw new Error('replaced constructor'); };
      check('replaced-global-constructor', isUrl(create(source)), true);
    } finally { Object.defineProperty(globalThis, 'MediaSource', descriptor); }

    let proxyReads = 0;
    const revoked = Proxy.revocable(source, {}); revoked.revoke();
    for (const [name, value] of [
      ['undefined', undefined], ['null', null], ['record', {}], ['prototype', MediaSource.prototype],
      ['forged', Object.create(MediaSource.prototype)], ['inherited', Object.create(source)],
      ['proxy', new Proxy(source, {get() { proxyReads++; throw new Error('proxy get'); }})],
      ['revoked', revoked.proxy], ['forged-blob', Object.create(Blob.prototype)],
    ]) check('invalid/' + name, create(value), 'TypeError');
    check('proxy-not-observed', proxyReads, 0);

    check('fetch', isUrl(url) ? await fetchResult(url) : 'no-url', 'async:TypeError');
    check('fetch-fragment', isUrl(url) ? await fetchResult(url + '#fragment') : 'no-url', 'async:TypeError');
    const request = isUrl(url) ? new Request(url) : null;
    check('fetch-request', request ? await fetchResult(request.clone()) : 'no-url', 'async:TypeError');
    check('fetch-no-cors', request ? await fetchResult(new Request(request, {mode: 'no-cors'})) : 'no-url', 'async:TypeError');
    if (isUrl(url)) { URL.revokeObjectURL(url); URL.revokeObjectURL(url); }
    check('captured-after-revoke', request ? await fetchResult(request.clone()) : 'no-url', 'async:TypeError');
    check('revoked-url', isUrl(url) ? await fetchResult(url) : 'no-url', 'async:TypeError');
    check('second-url', isUrl(other) ? await fetchResult(other) : 'no-url', 'async:TypeError');

    const blobUrl = create(new Blob(['payload'], {type: 'text/plain'}));
    const blobRequest = new Request(blobUrl);
    URL.revokeObjectURL(blobUrl);
    check('blob-captured-control', await (await fetch(blobRequest)).text(), 'payload');
    check('blob-revoked-control', await fetchResult(blobUrl), 'async:TypeError');
    const fileUrl = create(new File(['file'], 'test.txt'));
    check('file-control', await (await fetch(fileUrl)).text(), 'file');

    for (const shared of [false, true]) {
      const kind = shared ? 'shared' : 'dedicated';
      let result = 'no-url';
      if (isUrl(other)) {
        const code = `const probe = async () => {
          try { await fetch(${JSON.stringify(other)}); return 'resolved'; }
          catch (error) { return error.name; }
        };` + (shared ? 'onconnect = event => probe().then(value => event.ports[0].postMessage(value));' : 'probe().then(postMessage);');
        const workerUrl = create(new Blob([code], {type: 'text/javascript'}));
        result = await new Promise(resolve => {
          const worker = shared ? new SharedWorker(workerUrl) : new Worker(workerUrl);
          const port = shared ? worker.port : worker;
          port.onmessage = event => {
            shared ? port.close() : worker.terminate();
            URL.revokeObjectURL(workerUrl);
            resolve(event.data);
          };
          worker.onerror = event => { event.preventDefault(); resolve('worker-error:' + event.message); };
        });
      }
      check(kind + '/fetch-media-source', result, 'TypeError');
    }

    const frame = document.createElement('iframe');
    (document.body || document.documentElement).appendChild(frame);
    try {
      const child = frame.contentWindow;
      const childSource = new child.MediaSource();
      check('cross-realm-source', isUrl(create(childSource)), true);
      check('cross-realm-method', isUrl(create(source, child.URL)), true);
      let caught;
      try { child.URL.createObjectURL(new Proxy(source, {})); } catch (error) { caught = error; }
      check('callee-error-realm', [caught instanceof child.TypeError, caught instanceof TypeError], [true, false]);
    } finally { frame.remove(); }
  } finally {
    for (const url of urls) URL.revokeObjectURL(url);
  }
  return {state: checks.every(check => check.pass) ? 'pass' : 'fail', checks};
}
