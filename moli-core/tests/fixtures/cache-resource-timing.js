(async () => {
  const xhr = url => new Promise((resolve, reject) => {
    const request = new XMLHttpRequest();
    request.open('GET', url);
    request.onload = () => resolve(request.responseText);
    request.onerror = () => reject(new Error('xhr ' + url));
    request.send();
  });
  const context = globalThis.cacheTimingContext || 'top';
  const resourceUrl = (path, parameters) => {
    const url = new URL(path, document.baseURI);
    url.search = new URLSearchParams({...parameters, context});
    return url.href;
  };
  const result = [];
  for (const mode of ['fresh', 'revalidate', 'no-store']) {
    const url = resourceUrl('/probe-cache', {mode});
    const observations = [];
    for (let i = 0; i < 3; i++) {
      const body = await xhr(url);
      await new Promise(resolve => setTimeout(resolve, 0));
      const entries = performance.getEntriesByName(url);
      const entry = entries[entries.length - 1];
      observations.push({
        body,
        count: entries.length,
        transfer: entry.transferSize,
        encoded: entry.encodedBodySize,
        decoded: entry.decodedBodySize,
        status: entry.responseStatus
      });
    }
    const stats = await (await fetch(resourceUrl('/probe-cache-stats', {mode}))).json();
    result.push({mode, observations, stats});
  }
  const headers = [];
  for (const padding of [0, 4096]) {
    const url = resourceUrl('/probe-cache', {mode: 'padding', padding});
    await xhr(url);
    await new Promise(resolve => setTimeout(resolve, 0));
    const entry = performance.getEntriesByName(url)[0];
    headers.push({padding, transfer: entry.transferSize, encoded: entry.encodedBodySize});
  }
  return {result, headers};
})()
