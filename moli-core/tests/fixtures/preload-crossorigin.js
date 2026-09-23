(async () => {
  const cases = [];
  for (const destination of ['script', 'style', 'image', 'font', 'track', 'fetch']) {
    for (const crossorigin of [null, '', 'anonymous', 'use-credentials']) {
      for (const allow of ['none', 'wildcard', 'credentials']) {
        const expected = crossorigin === null || allow === 'credentials' ||
          (allow === 'wildcard' && crossorigin !== 'use-credentials') ? 'load' : 'error';
        cases.push({destination, crossorigin, allow, expected, rel: 'preload'});
      }
    }
  }
  for (const allow of ['none', 'wildcard', 'credentials']) {
    cases.push({destination: 'script', crossorigin: null, allow,
      expected: allow === 'none' ? 'error' : 'load', rel: 'modulepreload'});
  }
  const parsed = Array.from(document.querySelectorAll('link[data-preload-cors]'));
  const context = parsed.length ? 'parser' : 'dynamic';
  const urls = cases.map(({destination, allow}, index) => {
    const url = new URL(globalThis.preloadCrossoriginResource);
    url.searchParams.set('as', destination);
    url.searchParams.set('allow', allow);
    url.searchParams.set('case', context + '-' + index);
    return url.href;
  });
  const links = parsed.length ? parsed : cases.map(({destination, crossorigin, rel}, index) => {
    const link = document.createElement('link');
    link.rel = rel;
    link.as = destination;
    link.href = urls[index];
    if (crossorigin !== null) link.setAttribute('crossorigin', crossorigin);
    return link;
  });
  const observations = await Promise.all(links.map((link, index) => new Promise(resolve => {
    let timer;
    const finish = event => {
      clearTimeout(timer);
      resolve({...cases[index], event});
    };
    const previous = parsed.length ? globalThis.preloadCorsEvents[index] : null;
    if (previous) { finish(previous); return; }
    link.onload = link.onerror = event => finish(event.type);
    timer = setTimeout(() => finish('timeout'), 5000);
    if (!parsed.length) document.head.append(link);
  })));
  const escape = value => value.replaceAll('&', '&amp;').replaceAll('"', '&quot;');
  const parserMarkup = '<!doctype html><head><script>globalThis.preloadCorsEvents=[];' +
    'globalThis.preloadCrossoriginResource=' + JSON.stringify(globalThis.preloadCrossoriginResource) + ';<' + '/script>' +
    cases.map(({destination, crossorigin, rel}, index) =>
      '<link data-preload-cors rel="' + rel + '" as="' + destination + '" href="' + escape(urls[index]) + '" ' +
      (crossorigin === null ? '' : 'crossorigin="' + crossorigin + '" ') +
      'onload="preloadCorsEvents[' + index + ']=event.type" onerror="preloadCorsEvents[' + index + ']=event.type">'
    ).join('') + '</head><body>preload CORS';
  for (const link of links) link.remove();
  return {context, observations, failures: observations.filter(row => row.event !== row.expected), parserMarkup};
})()
