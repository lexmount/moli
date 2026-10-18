(async () => {
  const rows = [];
  let sequence = 0;
  const assert = (condition, message) => { if (!condition) throw new Error(message); };
  const test = async (name, run) => {
    try { await run(); rows.push({name, passed: true}); }
    catch (error) { rows.push({name, passed: false, error: String(error)}); }
  };
  const urlFor = (origin = 0, query = '', path = '/resource') =>
    `${__fetchOrigins[origin]}${path}?key=xhr-timing-${++sequence}&${query}`;
  const entries = (realm, url) => realm.performance.getEntriesByName(url, 'resource');
  const entryFor = (realm, url) => new Promise((resolve, reject) => {
    const previous = entries(realm, url);
    if (previous.length) { resolve(previous[0]); return; }
    const observer = new realm.PerformanceObserver(list => {
      const entry = list.getEntriesByName(url, 'resource')[0];
      if (entry) { clearTimeout(timer); observer.disconnect(); resolve(entry); }
    });
    observer.observe({type: 'resource'});
    const timer = setTimeout(() => {
      observer.disconnect(); reject(new Error('XHR resource entry was not reported'));
    }, 500);
  });
  const request = (realm, url, method = 'GET') => new Promise(resolve => {
    const xhr = new realm.XMLHttpRequest(), events = [];
    let entryCountAtTerminal = -1;
    xhr.open(method, url);
    for (const type of ['load', 'error']) xhr.addEventListener(type, () => {
      events.push(type);
      entryCountAtTerminal = entries(realm, url).length;
    });
    xhr.onloadend = () => resolve({xhr, events, entryCountAtTerminal});
    xhr.send();
  });
  const checkEntry = (realm, entry, url, initiator = 'xmlhttprequest') => {
    assert(entry instanceof realm.PerformanceResourceTiming, 'entry belongs to the requesting realm');
    assert(entry.name === url && entry.entryType === 'resource' && entry.initiatorType === initiator, 'entry identity');
    assert(entry.startTime >= 0 && entry.responseEnd >= entry.startTime && entry.duration >= 0, 'request and terminal timestamps');
    assert(Math.abs(entry.duration - (entry.responseEnd - entry.startTime)) < 0.00001, 'duration uses body completion');
    const json = entry.toJSON();
    for (const key of ['name', 'initiatorType', 'startTime', 'responseEnd', 'duration', 'responseStatus', 'encodedBodySize', 'decodedBodySize'])
      assert(json[key] === entry[key], `native JSON ${key}`);
    assert(entries(realm, url).length === 1, 'one entry per request');
    if (realm !== window) assert(entries(window, url).length === 0, 'child entries do not enter the parent timeline');
  };
  const checkFailure = (result, entry) => {
    assert(result.events.join() === 'error' && result.xhr.status === 0, 'XHR remains a network error');
    assert(result.entryCountAtTerminal === 1, 'entry is available before the error callback');
    for (const key of ['responseStatus', 'transferSize', 'encodedBodySize', 'decodedBodySize', 'requestStart', 'responseStart', 'finalResponseHeadersStart'])
      assert(entry[key] === 0, `network error hides ${key}`);
    for (const key of ['contentType', 'contentEncoding', 'nextHopProtocol'])
      assert(entry[key] === '', `network error hides ${key}`);
  };
  const checkResponse = (result, entry, status, size, timingAllowed) => {
    assert(result.events.join() === 'load' && result.xhr.status === status, 'HTTP response completes XHR');
    assert(result.entryCountAtTerminal === 1, 'entry is available before the load callback');
    assert(entry.responseStatus === status, 'HTTP error status is retained');
    assert(entry.encodedBodySize === size && entry.decodedBodySize === size, 'readable response sizes');
    assert(entry.contentType === 'image/png', 'readable response type');
    assert(entry.transferSize === (timingAllowed ? size + 300 : 0), 'TAO filters transfer size');
    if (timingAllowed) {
      assert(entry.requestStart >= entry.startTime && entry.requestStart <= entry.responseStart, 'native request and header order');
      assert(entry.responseStart > 0 && entry.responseStart <= entry.responseEnd, 'native header and body order');
      assert(entry.nextHopProtocol === 'http/1.1', 'negotiated protocol');
    } else {
      assert(entry.requestStart === 0 && entry.responseStart === 0 && entry.finalResponseHeadersStart === 0 && entry.nextHopProtocol === '', 'TAO filters network details independently of CORS');
    }
  };
  const checkPartialResponse = (entry, timingAllowed) => {
    assert(entry.responseStatus === 200 && entry.contentType === 'image/png', 'body failure retains the received response metadata');
    assert(entry.encodedBodySize === 4 && entry.decodedBodySize === 4, 'partial response counts only received bytes');
    assert(entry.transferSize === (timingAllowed ? 304 : 0), 'partial response follows TAO');
    if (timingAllowed) {
      assert(entry.requestStart >= entry.startTime && entry.requestStart <= entry.responseStart && entry.responseStart > 0 && entry.responseStart <= entry.responseEnd, 'partial body preserves native header timing');
    } else {
      assert(entry.requestStart === 0 && entry.responseStart === 0 && entry.nextHopProtocol === '', 'TAO hides partial-response connection details');
    }
  };
  for (const owner of ['main', 'child']) {
    let frame;
    const realm = owner === 'main' ? window :
      (frame = document.body.appendChild(document.createElement('iframe'))).contentWindow;
    try {
      for (const status of [200, 204, 205, 404]) await test(`${owner} HTTP ${status}`, async () => {
        const url = urlFor(0, `status=${status}`), result = await request(realm, url);
        const entry = await entryFor(realm, url);
        checkEntry(realm, entry, url);
        checkResponse(result, entry, status, status === 204 || status === 205 ? 0 : 68, true);
        await new Promise(resolve => setTimeout(resolve, 0));
        assert(entries(realm, url).length === 1, 'a later transport task does not report again');
      });
      await test(`${owner} HEAD completes timing at headers`, async () => {
        const url = urlFor(), result = await request(realm, url, 'HEAD');
        const entry = await entryFor(realm, url);
        checkEntry(realm, entry, url);
        checkResponse(result, entry, 200, 0, true);
        await new Promise(resolve => setTimeout(resolve, 0));
        assert(entries(realm, url).length === 1, 'HEAD transport completion does not duplicate timing');
      });
      for (const tao of [false, true]) await test(`${owner} CORS allowed TAO=${tao}`, async () => {
        const url = urlFor(1, `cors=*&${tao ? 'tao=*' : ''}`), result = await request(realm, url);
        const entry = await entryFor(realm, url);
        checkEntry(realm, entry, url);
        checkResponse(result, entry, 200, 68, tao);
      });
      for (const origin of [0, 1]) await test(`${owner} network failure origin=${origin}`, async () => {
        const url = urlFor(origin, '', '/fail'), result = await request(realm, url);
        const entry = await entryFor(realm, url);
        checkEntry(realm, entry, url);
        checkFailure(result, entry);
      });
      for (const tao of [false, true]) await test(`${owner} CORS rejected TAO=${tao}`, async () => {
        const url = urlFor(1, tao ? 'tao=*' : ''), result = await request(realm, url);
        const entry = await entryFor(realm, url);
        checkEntry(realm, entry, url);
        checkFailure(result, entry);
      });
      for (const origin of [0, 1]) await test(`${owner} truncated body origin=${origin}`, async () => {
        const url = urlFor(origin, `truncate=1&cors=*`), result = await request(realm, url);
        const entry = await entryFor(realm, url);
        checkEntry(realm, entry, url);
        assert(result.events.join() === 'error' && result.xhr.status === 0 && result.entryCountAtTerminal === 1, 'body failure still dispatches one XHR error');
        checkPartialResponse(entry, origin === 0);
      });
      for (const [origin, tao] of [[0, true], [1, false], [1, true]]) await test(`${owner} Fetch body failure origin=${origin} TAO=${tao}`, async () => {
        const url = urlFor(origin, `truncate=1&cors=*&${tao ? 'tao=*' : ''}`);
        const response = await realm.fetch(url);
        let error;
        try { await response.arrayBuffer(); } catch (caught) { error = caught; }
        assert(response.status === 200 && error?.name === 'TypeError', 'Fetch exposes the head and rejects the interrupted body');
        const entry = await entryFor(realm, url);
        checkEntry(realm, entry, url, 'fetch');
        checkPartialResponse(entry, tao);
      });
      await test(`${owner} error callback reuses XHR`, async () => {
        const first = urlFor(0, '', '/fail'), second = urlFor();
        const xhr = new realm.XMLHttpRequest(), events = [];
        let firstCount = -1;
        await new Promise((resolve, reject) => {
          xhr.onerror = () => {
            events.push('error'); firstCount = entries(realm, first).length;
            if (events.length > 1) { reject(new Error('replacement XHR failed')); return; }
            xhr.open('GET', second); xhr.send();
          };
          xhr.onload = () => { events.push('load'); resolve(); };
          xhr.open('GET', first); xhr.send();
        });
        checkEntry(realm, await entryFor(realm, first), first);
        checkEntry(realm, await entryFor(realm, second), second);
        assert(firstCount === 1 && events.join() === 'error,load' && xhr.status === 200, 'old terminal cannot replace the new request or its timing');
      });
      await test(`${owner} streaming entry waits for body completion`, async () => {
        const key = `xhr-stream-${++sequence}`, url = `${__fetchOrigins[0]}/resource?gate=body&key=${key}`;
        const xhr = new realm.XMLHttpRequest();
        let progress;
        const loading = new Promise(resolve => { progress = resolve; });
        const done = new Promise(resolve => { xhr.onloadend = resolve; });
        xhr.onprogress = progress;
        xhr.open('GET', url); xhr.send();
        try {
          await loading;
          assert(xhr.readyState === 3 && entries(realm, url).length === 0, 'headers and partial data do not complete timing');
        } finally { await fetch(`${__fetchOrigins[0]}/release?key=${key}`); }
        await done;
        const entry = await entryFor(realm, url);
        checkEntry(realm, entry, url);
        assert(xhr.status === 200 && entry.decodedBodySize === 68, 'complete body survives streaming');
      });
      await test(`${owner} observer receives failed entry with zero buffer`, async () => {
        const url = urlFor(0, '', '/fail');
        realm.performance.clearResourceTimings(); realm.performance.setResourceTimingBufferSize(0);
        try {
          const observed = entryFor(realm, url), result = await request(realm, url);
          const entry = await observed;
          assert(entry instanceof realm.PerformanceResourceTiming && entry.name === url && entry.initiatorType === 'xmlhttprequest', 'observer delivers failed XHR');
          assert(result.events.join() === 'error' && entry.responseStatus === 0, 'request still fails');
          assert(entries(realm, url).length === 0, 'zero-sized buffer stays empty');
        } finally { realm.performance.setResourceTimingBufferSize(250); }
      });
    } finally { if (frame) frame.remove(); }
  }
  return JSON.stringify({total: rows.length, failures: rows.filter(row => !row.passed)});
})();
