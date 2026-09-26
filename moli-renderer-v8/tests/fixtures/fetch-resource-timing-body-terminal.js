(async () => {
  const rows = [], bodySize = 68;
  let sequence = 0;
  const assert = (condition, message) => { if (!condition) throw new Error(message); };
  const test = async (name, run) => {
    try { await run(); rows.push({name, passed: true}); }
    catch (error) { rows.push({name, passed: false, error: String(error)}); }
  };
  const turn = () => new Promise(resolve => setTimeout(resolve, 0));
  const urlFor = (origin = 0, query = '') => `${__fetchOrigins[origin]}/resource?key=case-${++sequence}&${query}`;
  const entryFor = (realm, url) => new Promise((resolve, reject) => {
    const previous = realm.performance.getEntriesByName(url, 'resource');
    if (previous.length) { resolve(previous[previous.length - 1]); return; }
    const observer = new realm.PerformanceObserver(list => {
      const entry = list.getEntriesByName(url, 'resource')[0];
      if (entry) { clearTimeout(timer); observer.disconnect(); resolve(entry); }
    });
    observer.observe({type: 'resource'});
    const timer = setTimeout(() => { observer.disconnect(); reject(new Error('resource entry was not reported')); }, 500);
  });
  const checkEntry = (realm, entry, url) => {
    assert(entry instanceof realm.PerformanceResourceTiming, 'native resource interface');
    assert(entry.name === url && entry.entryType === 'resource' && entry.initiatorType === 'fetch', 'entry identity');
    assert(entry.startTime >= 0 && entry.responseEnd >= entry.startTime && entry.duration >= 0, 'start and body-terminal timestamps');
    assert(Math.abs(entry.duration - (entry.responseEnd - entry.startTime)) < 0.00001, 'duration uses body terminal');
    const json = entry.toJSON();
    for (const key of ['name', 'startTime', 'responseEnd', 'duration', 'encodedBodySize', 'decodedBodySize', 'responseStatus'])
      assert(json[key] === entry[key], `native JSON ${key}`);
    assert(realm.performance.getEntriesByName(url, 'resource').length === 1, 'exactly one entry per fetch');
  };
  for (const status of [200, 204, 404, 503]) await test(`same-origin status ${status}`, async () => {
    const url = urlFor(0, `status=${status}`), response = await fetch(url), bytes = await response.arrayBuffer();
    const entry = await entryFor(window, url); checkEntry(window, entry, url);
    assert(entry.responseStatus === status, 'HTTP status is not a network error');
    assert(entry.encodedBodySize === bytes.byteLength && entry.decodedBodySize === bytes.byteLength, 'completed response body size');
    assert(entry.transferSize === bytes.byteLength + 300, 'uncached transfer overhead');
  });
  for (const tao of [false, true]) for (const mode of ['cors', 'no-cors']) await test(`${mode} cross-origin TAO=${tao}`, async () => {
    const url = urlFor(1, `${mode === 'cors' ? 'cors=*&' : ''}${tao ? 'tao=*' : ''}`);
    const response = await fetch(url, {mode}); await response.arrayBuffer();
    const entry = await entryFor(window, url); checkEntry(window, entry, url);
    const readable = mode === 'cors';
    assert(entry.responseStatus === (readable ? 200 : 0), 'filtered response status');
    assert(entry.encodedBodySize === (readable ? bodySize : 0) && entry.decodedBodySize === (readable ? bodySize : 0), 'body sizes follow CORS, independently of TAO');
    assert(entry.transferSize === (tao ? entry.encodedBodySize + 300 : 0), 'transfer size follows TAO');
    if (!tao) assert(entry.responseStart === 0 && entry.nextHopProtocol === '', 'TAO hides connection details');
  });
  for (const failure of ['network', 'cors']) await test(`${failure} failure still completes timing`, async () => {
    const url = failure === 'network' ? __fetchOrigins[0] + '/fail?key=network-failure' : urlFor(1);
    let error; try { await fetch(url); } catch (caught) { error = caught; }
    assert(error instanceof TypeError, 'fetch rejects');
    const entry = await entryFor(window, url); checkEntry(window, entry, url);
    for (const key of ['responseStatus', 'transferSize', 'encodedBodySize', 'decodedBodySize', 'responseStart']) assert(entry[key] === 0, `failed ${key} is opaque`);
  });
  await test('streaming timing waits for EOF and ignores Response clones', async () => {
    const key = `stream-${++sequence}`, url = `${__fetchOrigins[0]}/resource?gate=body&key=${key}`;
    const response = await fetch(url), copy = response.clone();
    await turn(); assert(performance.getEntriesByName(url).length === 0, 'headers are not completion');
    const first = response.arrayBuffer(), second = copy.arrayBuffer();
    await fetch(`${__fetchOrigins[0]}/release?key=${key}`);
    assert((await first).byteLength === bodySize && (await second).byteLength === bodySize, 'both branches receive the body');
    const entry = await entryFor(window, url); checkEntry(window, entry, url);
    await turn(); assert(performance.getEntriesByName(url).length === 1, 'cloning does not duplicate completion');
  });
  for (const gate of ['before', 'body']) await test(`AbortSignal during ${gate}`, async () => {
    const key = `abort-${gate}-${++sequence}`, url = `${__fetchOrigins[0]}/resource?gate=${gate}&key=${key}`;
    const controller = new AbortController(); let reason;
    try {
      if (gate === 'body') {
        const response = await fetch(url, {signal: controller.signal}), body = response.arrayBuffer();
        controller.abort(); try { await body; } catch (error) { reason = error; }
      } else {
        const response = fetch(url, {signal: controller.signal});
        while (!await fetch(`${__fetchOrigins[0]}/seen?target=${key}`).then(r => r.json())) await turn();
        controller.abort(); try { await response; } catch (error) { reason = error; }
      }
      assert(reason?.name === 'AbortError', 'abort reason is preserved');
      const entry = await entryFor(window, url); checkEntry(window, entry, url);
      assert(entry.responseStatus === 0 && entry.encodedBodySize === 0 && entry.decodedBodySize === 0, 'aborted response exposes no partial body metadata');
    } finally { await fetch(`${__fetchOrigins[0]}/release?key=${key}`); }
    await turn(); assert(performance.getEntriesByName(url).length === 1, 'late terminal after abort is ignored');
  });
  await test('pre-aborted fetch has no timing entry', async () => {
    const url = urlFor(), controller = new AbortController(); controller.abort();
    try { await fetch(url, {signal: controller.signal}); } catch (error) { assert(error.name === 'AbortError', 'pre-abort reason'); }
    await turn(); assert(performance.getEntriesByName(url).length === 0, 'no fetch was started');
  });
  await test('child fetch reports in its client realm', async () => {
    const frame = document.body.appendChild(document.createElement('iframe')), realm = frame.contentWindow;
    try {
      const url = urlFor(), response = await realm.fetch(url); await response.arrayBuffer();
      const entry = await entryFor(realm, url); checkEntry(realm, entry, url);
      assert(!(entry instanceof PerformanceResourceTiming) && performance.getEntriesByName(url).length === 0, 'parent timeline is separate');
    } finally { frame.remove(); }
  });
  await test('resource observer receives entries with a zero buffer size', async () => {
    const url = urlFor(); performance.clearResourceTimings(); performance.setResourceTimingBufferSize(0);
    const observed = entryFor(window, url);
    try {
      const response = await fetch(url); await response.arrayBuffer();
      const entry = await observed;
      assert(entry.name === url && entry.initiatorType === 'fetch', 'observer delivery');
      assert(performance.getEntriesByName(url).length === 0, 'timeline stays empty');
    } finally { performance.setResourceTimingBufferSize(250); }
  });
  const redirectTo = (origin, target, tao = '') => `${__fetchOrigins[origin]}/redirect?key=redirect-${++sequence}&to=${encodeURIComponent(target)}&cors=*&tao=${encodeURIComponent(tao)}`;
  await test('redirect records the initial URL once', async () => {
    const target = urlFor(), url = redirectTo(0, target);
    const response = await fetch(url); await response.arrayBuffer();
    checkEntry(window, await entryFor(window, url), url);
    assert(performance.getEntriesByName(target).length === 0, 'no separate redirect-hop entry');
  });
  for (const [middleTAO, finalTAO, allowed] of [
    ['', '*', false], ['*', '*', true],
    [__fetchOrigins[0], 'null', true], [__fetchOrigins[0], __fetchOrigins[0], false]
  ]) await test(`cross-origin redirect TAO ${middleTAO}/${finalTAO}`, async () => {
    const target = urlFor(0, `cors=*&tao=${encodeURIComponent(finalTAO)}`);
    const url = redirectTo(0, redirectTo(1, target, middleTAO));
    const response = await fetch(url); await response.arrayBuffer();
    const entry = await entryFor(window, url); checkEntry(window, entry, url);
    assert(entry.responseStatus === 200 && entry.encodedBodySize === bodySize, 'CORS-visible redirected response');
    assert(entry.transferSize === (allowed ? bodySize + 300 : 0), 'TAO is sticky and uses the serialized request origin');
    if (!allowed) assert(entry.responseStart === 0 && entry.nextHopProtocol === '', 'redirect TAO failure hides timing');
  });
  await test('manual redirect keeps filtered metadata', async () => {
    const url = redirectTo(0, urlFor());
    const response = await fetch(url, {redirect: 'manual'});
    assert(response.type === 'opaqueredirect', 'filtered redirect');
    const entry = await entryFor(window, url); checkEntry(window, entry, url);
    assert(entry.responseStatus === 0 && entry.encodedBodySize === 0 && entry.decodedBodySize === 0, 'opaque redirect metadata');
  });
  await test('redirect error completes timing', async () => {
    const url = redirectTo(0, urlFor());
    let reason; try { await fetch(url, {redirect: 'error'}); } catch (error) { reason = error; }
    assert(reason instanceof TypeError, 'redirect mode rejects');
    const entry = await entryFor(window, url); checkEntry(window, entry, url);
    assert(entry.responseStatus === 0 && entry.transferSize === 0, 'failed redirect metadata');
  });
  await test('body cancellation reports one terminal entry', async () => {
    const key = `cancel-${++sequence}`, url = `${__fetchOrigins[0]}/resource?gate=body&key=${key}`;
    try {
      const response = await fetch(url); await response.body.cancel();
      checkEntry(window, await entryFor(window, url), url);
    } finally { await fetch(`${__fetchOrigins[0]}/release?key=${key}`); }
    await turn(); assert(performance.getEntriesByName(url).length === 1, 'late EOF does not report twice');
  });
  await test('unconsumed response completes timing', async () => {
    const url = urlFor(), response = await fetch(url);
    checkEntry(window, await entryFor(window, url), url);
    assert((await response.arrayBuffer()).byteLength === bodySize, 'reading later preserves the body');
    await turn(); assert(performance.getEntriesByName(url).length === 1, 'reading later does not report twice');
  });
  await test('parent AbortSignal reports only in child fetch realm', async () => {
    const frame = document.body.appendChild(document.createElement('iframe')), realm = frame.contentWindow;
    const key = `child-abort-${++sequence}`, url = `${__fetchOrigins[0]}/resource?gate=body&key=${key}`;
    try {
      const controller = new AbortController(), response = await realm.fetch(url, {signal: controller.signal});
      const body = response.arrayBuffer(); controller.abort();
      try { await body; } catch (error) { assert(error.name === 'AbortError', 'child body abort'); }
      checkEntry(realm, await entryFor(realm, url), url);
      assert(performance.getEntriesByName(url).length === 0, 'parent timeline remains separate');
    } finally { await fetch(`${__fetchOrigins[0]}/release?key=${key}`); frame.remove(); }
  });
  for (const [tao, tao2, allowed] of [
    ['"*"', '', false], ['"ignored,*,value"', '', false],
    [`https://unused.test, ${__fetchOrigins[0]}`, '', true],
    ['"ignored,', '*,value"', false], ['https://unused.test', '*', true]
  ]) await test(`TAO list ${tao}/${tao2}`, async () => {
    const url = urlFor(1, `cors=*&tao=${encodeURIComponent(tao)}&tao2=${encodeURIComponent(tao2)}`);
    await fetch(url).then(response => response.arrayBuffer());
    const entry = await entryFor(window, url); checkEntry(window, entry, url);
    assert(entry.transferSize === (allowed ? bodySize + 300 : 0), 'quoted tokens and repeated fields follow Fetch list parsing');
  });
  for (const valid of [true, false]) await test(`integrity validation ${valid}`, async () => {
    const url = urlFor(), integrity = 'sha256-' + (valid ? '0vzSmlPFahjQJNdFMsA6oL5kgbIzB+klmeVz0zlid/M=' : 'AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=');
    let response, reason; try { response = await fetch(url, {integrity}); } catch (error) { reason = error; }
    if (valid) assert((await response.arrayBuffer()).byteLength === bodySize, 'valid integrity body');
    else assert(reason instanceof TypeError, 'integrity mismatch rejects');
    const entry = await entryFor(window, url); checkEntry(window, entry, url);
    assert(entry.responseStatus === (valid ? 200 : 0) && entry.encodedBodySize === (valid ? bodySize : 0), 'integrity response metadata');
  });
  await test('data and blob fetches do not create HTTP timing entries', async () => {
    const blob = URL.createObjectURL(new Blob(['local']));
    try {
      for (const url of ['data:text/plain,local', blob]) {
        assert(await fetch(url).then(response => response.text()) === 'local', 'local fetch succeeds');
        await turn(); assert(performance.getEntriesByName(url).length === 0, 'non-HTTP fetch excluded');
      }
    } finally { URL.revokeObjectURL(blob); }
  });
  await test('canceling one clone leaves the fetch running', async () => {
    const key = `clone-cancel-${++sequence}`, url = `${__fetchOrigins[0]}/resource?gate=body&key=${key}`;
    const response = await fetch(url), copy = response.clone(), canceled = response.body.cancel();
    assert(performance.getEntriesByName(url).length === 0, 'one branch is not fetch completion');
    const body = copy.arrayBuffer(); await fetch(`${__fetchOrigins[0]}/release?key=${key}`);
    assert((await body).byteLength === bodySize, 'other branch retains the complete response');
    await canceled;
    const entry = await entryFor(window, url); checkEntry(window, entry, url);
    assert(entry.responseStatus === 200 && entry.encodedBodySize === bodySize, 'successful terminal remains successful');
  });
  await test('truncated response reports a terminal entry', async () => {
    const url = urlFor(0, 'truncate=1');
    let reason; try { await fetch(url).then(response => response.arrayBuffer()); } catch (error) { reason = error; }
    assert(reason instanceof TypeError, 'truncated response rejects');
    checkEntry(window, await entryFor(window, url), url);
  });
  return JSON.stringify({total: rows.length, failures: rows.filter(row => !row.passed)});
})();
