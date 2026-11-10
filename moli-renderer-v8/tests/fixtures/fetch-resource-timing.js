window.fetchTimingDone = (async () => {
  const url = new URL('/asset?context=' + location.pathname, location.href).href;
  const observed = new Promise(resolve => {
    const observer = new PerformanceObserver(list => {
      const entry = list.getEntries().find(entry => entry.name === url);
      if (entry) { observer.disconnect(); resolve(entry); }
    });
    observer.observe({type: 'resource'});
  });
  const response = await fetch(url);
  const body = await response.text();
  const entry = await observed;
  return {
    body, name: entry.name, type: entry.initiatorType,
    source: entry.initiatorUrl, jsonSource: entry.toJSON().initiatorUrl,
    status: entry.responseStatus, decoded: entry.decodedBodySize,
    native: entry instanceof PerformanceResourceTiming,
    entries: performance.getEntriesByType('resource')
      .filter(entry => entry.initiatorType === 'fetch').map(entry => entry.name)
  };
})();
