(async () => {
  const rows = [];
  let sequence = 0;
  const assert = (condition, message) => { if (!condition) throw new Error(message); };
  const test = async (name, run) => {
    try { await run(); rows.push({name, passed: true}); }
    catch (error) { rows.push({name, passed: false, error: String(error)}); }
  };
  const urlFor = (values, origin = 0, extra = '') =>
    `${__fetchOrigins[origin]}/mime?key=mime-${++sequence}&${values.map(value => 'value=' + encodeURIComponent(value)).join('&')}&${extra}`;
  const entryFor = url => new Promise((resolve, reject) => {
    const previous = performance.getEntriesByName(url, 'resource');
    if (previous.length) { resolve(previous[0]); return; }
    const observer = new PerformanceObserver(list => {
      const entry = list.getEntriesByName(url, 'resource')[0];
      if (entry) { clearTimeout(timer); observer.disconnect(); resolve(entry); }
    });
    observer.observe({type: 'resource'});
    const timer = setTimeout(() => { observer.disconnect(); reject(new Error('missing resource entry')); }, 1000);
  });
  const fetchBody = (url, mode = 'cors') => fetch(url, {mode}).then(response => response.arrayBuffer());
  const xhrBody = url => new Promise((resolve, reject) => {
    const xhr = new XMLHttpRequest();
    xhr.open('GET', url);
    xhr.onload = resolve;
    xhr.onerror = () => reject(new Error('XHR failed'));
    xhr.send();
  });
  const check = (entry, expected, initiator) => {
    assert(entry.contentType === expected, `contentType: expected ${JSON.stringify(expected)}, got ${JSON.stringify(entry.contentType)}`);
    assert(entry.toJSON().contentType === expected, 'toJSON preserves the minimized type');
    assert(entry.initiatorType === initiator, 'initiator type');
    assert(performance.getEntriesByName(entry.name, 'resource').length === 1, 'one entry per request');
  };
  const cases = [
    [[], ''],
    [[''], ''],
    [['invalid'], ''],
    [['*/*'], ''],
    [['APPLICATION/X-JAVASCRIPT; charset=UTF-8'], 'text/javascript'],
    [['text/x-ecmascript'], 'text/javascript'],
    [['TEXT/JSON'], 'application/json'],
    [['application/problem+json'], 'application/json'],
    [['application/rss+xml'], 'application/xml'],
    [['image/svg+xml; charset=UTF-8'], 'image/svg+xml'],
    [['text/plain; charset=UTF-8'], 'text/plain'],
    [['image/png'], 'image/png'],
    [['image/jpe'], ''],
    [['application/octet-stream'], ''],
    [['text/html', 'application/json'], 'application/json'],
    [['application/json,text/plain'], 'text/plain'],
    [['text/html', '*/*', 'invalid'], 'text/html'],
    [['text/plain; x="a,application/json"'], 'text/plain'],
    [['application/json', 'application/unknown'], ''],
  ];
  for (const [initiator, load] of [['fetch', fetchBody], ['xmlhttprequest', xhrBody]]) {
    for (const [values, expected] of cases) await test(`${initiator} ${JSON.stringify(values)}`, async () => {
      const url = urlFor(values);
      await load(url);
      check(await entryFor(url), expected, initiator);
    });
    for (const tao of ['', '*']) await test(`${initiator} CORS TAO=${tao}`, async () => {
      const url = urlFor(['application/ld+json'], 1, `cors=*&tao=${tao}`);
      await load(url);
      check(await entryFor(url), 'application/json', initiator);
    });
  }
  for (const tao of ['', '*']) await test(`opaque response TAO=${tao}`, async () => {
    const url = urlFor(['image/png; x=private'], 1, `tao=${tao}`);
    await fetchBody(url, 'no-cors');
    check(await entryFor(url), '', 'fetch');
  });
  return JSON.stringify({total: rows.length, failures: rows.filter(row => !row.passed)});
})();
