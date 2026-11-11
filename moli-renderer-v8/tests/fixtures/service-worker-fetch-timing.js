async function serviceWorkerFetchTiming(base) {
  const cross = new URL(base); cross.hostname = 'localhost';
  const destination = new URL('/raw?tao=1', base);
  const redirected = new URL('/redirect', cross);
  redirected.searchParams.set('to', destination);
  const allowedRedirect = new URL(redirected); allowedRedirect.searchParams.set('tao', '1');
  const cases = [
    ['synthetic', new URL('/synthetic', base)],
    ['cross-synthetic', new URL('/synthetic', cross)],
    ['stream', new URL('/synthetic?stream=1', base)],
    ['cross-stream', new URL('/synthetic?stream=1', cross)],
    ['forward-same', new URL('/forward?target=' + encodeURIComponent(new URL('/raw', base)), base)],
    ['forward-cors', new URL('/forward?target=' + encodeURIComponent(new URL('/raw', cross)), base)],
    ['forward-tao', new URL('/forward?target=' + encodeURIComponent(new URL('/raw?tao=1', cross)), base)],
    ['cache-same', new URL('/cache?target=' + encodeURIComponent(new URL('/raw', base)), base)],
    ['cache-cors', new URL('/cache?target=' + encodeURIComponent(new URL('/raw', cross)), base)],
    ['cache-tao', new URL('/cache?target=' + encodeURIComponent(new URL('/raw?tao=1', cross)), base)],
    ['cache-again-cors', new URL('/cache?rounds=2&target=' + encodeURIComponent(new URL('/raw', cross)), base)],
    ['cache-again-tao', new URL('/cache?rounds=2&target=' + encodeURIComponent(new URL('/raw?tao=1', cross)), base)],
    ['cache-synthetic', new URL('/cache?synthetic=1', base)],
    ['forward-redirect-tao', new URL('/forward?target=' + encodeURIComponent(allowedRedirect), base)],
    ['forward-redirect-denied', new URL('/forward?target=' + encodeURIComponent(redirected), base)],
    ['cache-redirect-tao', new URL('/cache?target=' + encodeURIComponent(allowedRedirect), base)],
    ['cache-redirect-denied', new URL('/cache?target=' + encodeURIComponent(redirected), base)]
  ];
  const rows = [];
  for (const [name, url] of cases) {
    const timing = new Promise(resolve => {
      const observer = new PerformanceObserver(list => {
        const entry = list.getEntries().find(entry => entry.name === url.href);
        if (entry) { observer.disconnect(); resolve(entry); }
      });
      observer.observe({type:'resource'});
    });
    const response = await fetch(url);
    const body = await response.text();
    const entry = await timing;
    const target = new URL(url.searchParams.get('target') || url.href);
    rows.push({name, body, type:response.type, status:entry.responseStatus,
      encoded:entry.encodedBodySize, decoded:entry.decodedBodySize, transfer:entry.transferSize,
      urlMatchesSource:response.url === (target.searchParams.get('to') || target.href)});
  }
  return rows;
}
