(async () => {
  const rows = [];
  const fields = ['domainLookupStart', 'domainLookupEnd', 'connectStart', 'connectEnd', 'secureConnectionStart'];
  const assert = (condition, message) => { if (!condition) throw Error(message); };
  const test = async (name, run) => {
    try { await run(); rows.push({name, passed: true}); }
    catch (error) { rows.push({name, passed: false, error: String(error)}); }
  };
  let sequence = 0;
  for (const owner of ['main', 'child']) {
    let frame;
    const realm = owner === 'main' ? window :
      (frame = document.body.appendChild(document.createElement('iframe'))).contentWindow;
    try {
      for (const api of ['fetch', 'xhr', 'sync-xhr']) for (const mode of ['same', 'cors-tao', 'cors-hidden', 'redirect-same', 'redirect-cors-tao', 'redirect-cors-hidden']) {
        await test(`${owner} ${api} ${mode}`, async () => {
          const cross = mode.includes('cors'), allowed = !mode.endsWith('hidden'), redirected = mode.startsWith('redirect-');
          const target = `${__fetchOrigins[cross ? 1 : 0]}/resource?key=connection-${++sequence}&cors=*&${allowed ? 'tao=*' : ''}`;
          const url = redirected ? `${__fetchOrigins[0]}/redirect?to=${encodeURIComponent(target)}&tao=*` : target;
          // Do not access performance before the first response: that response
          // must keep all connection fields through the lazy-surface queue.
          if (api === 'fetch') await (await realm.fetch(url)).arrayBuffer();
          else if (api === 'xhr') await new Promise((resolve, reject) => {
            const xhr = new realm.XMLHttpRequest();
            xhr.onload = resolve; xhr.onerror = () => reject(Error('XHR failed'));
            xhr.open('GET', url); xhr.send();
          });
          else {
            const xhr = new realm.XMLHttpRequest();
            xhr.open('GET', url, false); xhr.send();
            assert(xhr.status === 200, 'synchronous XHR succeeds');
          }
          const entries = realm.performance.getEntriesByName(url, 'resource');
          assert(entries.length === 1, 'exactly one entry at the body terminal');
          const entry = entries[0];
          assert(entry instanceof realm.PerformanceResourceTiming, 'requesting realm brand');
          assert(entry.fetchStart >= entry.startTime && entry.responseEnd >= entry.fetchStart, 'fetch bounds');
          if (allowed) {
            const ordered = ['fetchStart', ...fields.slice(0, 4), 'requestStart', 'responseStart', 'responseEnd'];
            for (let i = 1; i < ordered.length; ++i)
              assert(entry[ordered[i]] >= entry[ordered[i - 1]], `${ordered[i - 1]} <= ${ordered[i]}`);
            assert(entry.secureConnectionStart === 0, 'HTTP has no origin TLS phase');
            assert(entry.nextHopProtocol === 'http/1.1', 'native negotiated protocol');
            if (redirected) assert(entry.redirectStart === entry.startTime && entry.redirectEnd >= entry.redirectStart && entry.redirectEnd <= entry.fetchStart, 'redirect and final-hop boundaries');
          } else {
            for (const field of [...fields, 'redirectStart', 'redirectEnd', 'requestStart', 'responseStart']) assert(entry[field] === 0, `TAO hides ${field}`);
            assert(entry.nextHopProtocol === '', 'TAO hides protocol');
          }
          const expected = Object.fromEntries(['fetchStart', 'redirectStart', 'redirectEnd', ...fields].map(field => [field, entry[field]]));
          let reads = 0;
          Object.defineProperty(entry, 'connectStart', {configurable: true, get() {reads++; throw Error('author getter');}});
          const json = realm.PerformanceResourceTiming.prototype.toJSON.call(entry);
          assert(Object.getPrototypeOf(json) === realm.Object.prototype, 'ordinary snapshot in method realm');
          for (const [field, value] of Object.entries(expected)) {
            const d = Object.getOwnPropertyDescriptor(json, field);
            assert(d && d.value === value && d.enumerable && d.writable && d.configurable && !d.get && !d.set, `native data property ${field}`);
          }
          assert(reads === 0, 'JSON never re-evaluates author shadowing properties');
          delete entry.connectStart;
        });
      }
      await test(`${owner} redirected hop includes its preflight`, async () => {
        const key = `preflight-clock-${++sequence}`;
        const target = `${__fetchOrigins[1]}/resource?key=${key}&cors=*&tao=*&preflight-gate=1`;
        const url = `${__fetchOrigins[0]}/redirect?to=${encodeURIComponent(target)}&tao=*`;
        const pending = realm.fetch(url, {headers: {'x-timing': '1'}}).then(response => response.arrayBuffer());
        while (!(await (await realm.fetch(`${__fetchOrigins[1]}/seen?target=${encodeURIComponent('OPTIONS:' + key)}&cors=*`)).json())) {}
        const blockedAt = realm.performance.now();
        await realm.fetch(`${__fetchOrigins[1]}/release?key=${key}&cors=*`);
        await pending;
        const entry = realm.performance.getEntriesByName(url, 'resource')[0];
        assert(entry && entry.redirectEnd >= entry.redirectStart && entry.fetchStart >= entry.redirectEnd, 'redirect provenance');
        assert(entry.fetchStart <= blockedAt + 0.1, 'fetchStart precedes completion of the held preflight');
        assert(entry.requestStart >= blockedAt, 'the actual request follows preflight release');
      });
    } finally { if (frame) frame.remove(); }
  }
  return JSON.stringify({total: rows.length, failures: rows.filter(row => !row.passed)});
})();
