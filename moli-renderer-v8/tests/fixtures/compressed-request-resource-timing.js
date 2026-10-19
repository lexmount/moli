(async () => {
  const probe = async (origins, label, filter) => {
    const rows = [];
    globalThis.__compressedTimingProgress = rows;
    const checks = async (name, run) => {
      try { await run(); rows.push({name: label + ': ' + name, passed: true}); }
      catch (error) { rows.push({name: label + ': ' + name, passed: false, error: String(error)}); }
    };
    const assert = (condition, message) => { if (!condition) throw Error(message); };
    const entryFor = url => new Promise((resolve, reject) => {
      const existing = performance.getEntriesByName(url, 'resource');
      if (existing.length) { resolve(existing[0]); return; }
      const observer = new PerformanceObserver(list => {
        const entry = list.getEntries().find(value => value.name === url);
        if (entry) { clearTimeout(timer); observer.disconnect(); resolve(entry); }
      });
      observer.observe({type: 'resource', buffered: true});
      const timer = setTimeout(() => { observer.disconnect(); reject(Error('Missing entry: ' + url)); }, 2000);
    });
    const request = async (api, url, method, opaque, validate) => {
      if (api === 'fetch') return (await fetch(url, {method, mode: opaque ? 'no-cors' : 'cors', cache: validate ? 'no-cache' : 'default'})).text();
      const xhr = new XMLHttpRequest();
      xhr.open(method, url, api !== 'sync-xhr');
      if (validate) xhr.setRequestHeader('Cache-Control', 'max-age=0');
      if (api === 'sync-xhr') { xhr.send(); return xhr.responseText; }
      return new Promise((resolve, reject) => {
        xhr.onload = () => resolve(xhr.responseText);
        xhr.onerror = () => reject(Error('XHR network error'));
        xhr.send();
      });
    };
    const apis = typeof document === 'undefined' ? ['fetch', 'xhr', 'sync-xhr'] : ['fetch', 'xhr'];
    const specs = [
      {kind: 'gzip', encoded: 23, decoded: 3},
      {kind: 'gzip', chunked: true, encoded: 23, decoded: 3},
      {kind: 'identity', encoded: 3, decoded: 3},
      {kind: 'empty', encoded: 20, decoded: 0},
      {kind: 'empty', chunked: true, encoded: 20, decoded: 0},
      {kind: 'bad', encoded: 3, decoded: 0, fails: true},
      {kind: 'bad', chunked: true, encoded: 3, decoded: 0, fails: true},
      {kind: 'gzip', method: 'HEAD', encoded: 0, decoded: 0},
      {kind: 'zstd', encoded: 12, decoded: 3},
      {kind: 'zstd', chunked: true, encoded: 12, decoded: 3},
      {kind: 'zstd-empty', encoded: 9, decoded: 0},
      {kind: 'zstd-empty', chunked: true, encoded: 9, decoded: 0},
      {kind: 'zstd-bad', encoded: 3, decoded: 0, fails: true},
      {kind: 'zstd-bad', chunked: true, encoded: 3, decoded: 0, fails: true},
      {kind: 'zstd', method: 'HEAD', encoded: 0, decoded: 0}
    ];
    let next = 0;
    for (const api of apis) for (const mode of ['same', 'cors', 'cors-tao', 'denied', ...(api === 'fetch' ? ['opaque', 'opaque-tao'] : [])]) for (const spec of specs) {
      if (filter && (filter.api !== api || filter.mode !== mode)) continue;
      await checks(`${api}/${mode}/${spec.kind}/${spec.chunked ? 'chunked' : 'length'}/${spec.method || 'GET'}`, async () => {
        const url = new URL('/compressed', origins[mode === 'same' ? 0 : 1]);
        url.searchParams.set('key', `${label}-${++next}`);
        url.searchParams.set('kind', spec.kind);
        if (spec.chunked) url.searchParams.set('chunked', '1');
        if (mode.startsWith('opaque')) url.searchParams.set('opaque', '1');
        if (mode.startsWith('cors')) url.searchParams.set('cors', '*');
        if (mode.endsWith('tao')) url.searchParams.set('tao', '*');
        const name = url.href;
        const opaque = mode.startsWith('opaque');
        let failed = false;
        try { await request(api, name, spec.method || 'GET', opaque, false); } catch { failed = true; }
        const entry = await entryFor(name);
        const hidden = opaque || mode === 'denied';
        assert(failed === (mode === 'denied' || !!spec.fails && !opaque), 'Unexpected body result');
        assert(entry.encodedBodySize === (hidden ? 0 : spec.encoded), `encoded=${entry.encodedBodySize}`);
        assert(entry.decodedBodySize === (hidden ? 0 : spec.decoded), `decoded=${entry.decodedBodySize}`);
        const encoding = hidden || !spec.encoded || spec.kind === 'identity' ? '' : spec.kind.startsWith('zstd') ? 'zstd' : 'gzip';
        assert(entry.contentEncoding === encoding, `encoding=${entry.contentEncoding}`);
        assert(entry.responseStatus === (hidden ? 0 : 200), `status=${entry.responseStatus}`);
        const allowed = mode === 'same' || mode.endsWith('tao');
        assert(entry.transferSize === (allowed ? (hidden ? 0 : spec.encoded) + 300 : 0), `transfer=${entry.transferSize}`);
        assert(performance.getEntriesByName(name, 'resource').length === 1, 'Duplicate timing');
      });
    }
    for (const api of apis) for (const {kind, encoded} of [{kind: 'gzip', encoded: 23}, {kind: 'zstd', encoded: 12}]) {
      if (filter && (filter.api !== api || filter.mode !== 'cache')) continue;
      const base = `${origins[0]}/compressed?kind=${kind}&cache=1&key=${label}-${api}-cache`;
      for (let phase = 0; phase < 4; ++phase) await checks(`${api}/${kind}/cache/${phase}`, async () => {
        const url = base + '#phase-' + phase;
        assert(await request(api, url, 'GET', false, phase === 2) === 'abc', 'Wrong cached representation');
        const entry = await entryFor(url);
        assert(entry.encodedBodySize === encoded && entry.decodedBodySize === 3, `cache sizes=${entry.encodedBodySize}/${entry.decodedBodySize}`);
        assert(entry.contentEncoding === kind, `cache encoding=${entry.contentEncoding}`);
        assert(entry.transferSize === [encoded + 300, 0, 300, 0][phase], `cache transfer=${entry.transferSize}`);
        assert(performance.getEntriesByName(url, 'resource').length === 1, 'Duplicate cached timing');
      });
    }
    return {total: rows.length, failures: rows.filter(row => !row.passed), names: rows.map(row => row.name)};
  };
  const scenario = globalThis.__compressedTimingScenario;
  const wants = realm => !scenario || scenario.realm === realm;
  const filter = scenario ? {api: scenario.api, mode: scenario.mode} : null;
  const results = [];
  if (wants('window')) results.push(await probe(__fetchOrigins, 'window', filter));
  if (wants('child')) {
    const iframe = document.createElement('iframe');
    const child = new Promise((resolve, reject) => {
      const timer = setTimeout(() => reject(Error('Child timing probe did not finish')), 30000);
      const receive = event => {
        if (event.source !== iframe.contentWindow || event.data?.kind !== 'compressed-timing') return;
        clearTimeout(timer); removeEventListener('message', receive); resolve(event.data.result);
      };
      addEventListener('message', receive);
    });
    iframe.srcdoc = '<!doctype html><script>const probe = ' + probe.toString() + '; probe(' + JSON.stringify(__fetchOrigins) + ', "child", ' + JSON.stringify(filter) + ').then(result => parent.postMessage({kind:"compressed-timing",result}, "*"));</scr' + 'ipt>';
    document.body.appendChild(iframe);
    try { results.push(await child); } finally { iframe.remove(); }
  }
  if (wants('worker')) {
    const worker = new Worker(__fetchOrigins[0] + '/compressed-worker.js');
    try {
      results.push(await new Promise((resolve, reject) => {
        worker.onmessage = event => resolve(event.data);
        worker.onerror = event => reject(Error(event.message));
        worker.postMessage({source: probe.toString(), origins: __fetchOrigins, label: 'worker', filter});
      }));
    } finally { worker.terminate(); }
  }
  return JSON.stringify({total: results.reduce((sum, result) => sum + result.total, 0), failures: results.flatMap(result => result.failures), names: results.flatMap(result => result.names)});
})()
