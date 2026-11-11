async function fetchAbortResourceTiming(base, realm) {
  const observed = [];
  const observer = new PerformanceObserver(list => observed.push(...list.getEntries()));
  observer.observe({type: 'resource'});
  const rows = [];
  for (const stage of ['normal', 'pre-aborted', 'headers', 'body']) {
    const token = realm + '-' + stage;
    const url = new URL(stage === 'normal' ? '/normal' : stage === 'pre-aborted' ? '/never' : '/slow', base);
    url.search = new URLSearchParams({token, stage});
    const controller = new AbortController();
    if (stage === 'pre-aborted') controller.abort();
    const response = fetch(url, {signal: controller.signal});
    const completed = response.then(r => r.text()).then(() => 'success', error => error.name);
    if (stage === 'headers' || stage === 'body') {
      await (await fetch(new URL('/seen?token=' + token, base))).text();
      if (stage === 'body') await response;
      controller.abort();
    }
    const outcome = await completed;
    if (stage === 'headers' || stage === 'body') {
      await (await fetch(new URL('/release?token=' + token, base))).text();
    }
    observed.push(...observer.takeRecords());
    const entries = performance.getEntriesByName(url.href);
    rows.push({stage, outcome, count: entries.length,
      observed: observed.filter(entry => entry.name === url.href).length,
      types: entries.map(entry => entry.initiatorType)});
  }
  observer.disconnect();
  return rows;
}
