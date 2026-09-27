(async cases => {
  const parsed = Array.from(document.querySelectorAll('link[data-preload-sri]'));
  const links = parsed.length ? parsed : cases.map(test => {
    const link = document.createElement('link');
    Object.assign(link, {rel: 'preload', as: test.destination, href: test.src});
    for (const attribute of ['integrity', 'crossOrigin']) {
      if (attribute in test) link[attribute] = test[attribute];
    }
    return link;
  });
  const observations = await Promise.all(links.map((link, index) => new Promise(resolve => {
    let timer;
    const finish = event => {
      clearTimeout(timer);
      resolve({
        name: cases[index].name, event, expected: cases[index].expected,
        expectedEncodedBodySize: cases[index].expectedEncodedBodySize,
        timing: performance.getEntriesByName(link.href).map(entry => ({
          initiator: entry.initiatorType,
          encoded: entry.encodedBodySize
        }))
      });
    };
    const previous = parsed.length && globalThis.preloadSriEvents[index];
    if (previous) { finish(previous); return; }
    link.onload = link.onerror = event => finish(event.type);
    timer = setTimeout(() => finish('timeout'), 5000);
    if (!parsed.length) document.head.append(link);
  })));
  for (const link of links) link.remove();
  return {
    observations,
    failures: observations.filter(row => row.event !== row.expected ||
      (row.expectedEncodedBodySize !== undefined &&
       (row.timing.length !== 1 ||
        row.timing[0].encoded !== row.expectedEncodedBodySize))),
    executions: globalThis.sriExecutions || 0
  };
})
