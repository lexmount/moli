(async cases => {
  const observations = [];
  const eventResult = node => new Promise(resolve => {
    const timer = setTimeout(() => resolve('timeout'), 5000);
    node.onload = node.onerror = event => { clearTimeout(timer); resolve(event.type); };
  });
  for (const test of cases) {
    let objectURL;
    if ('data' in test) {
      test.src = test.blob
        ? (objectURL = URL.createObjectURL(new Blob([test.data], {type: test.mime})))
        : 'data:' + test.mime + ',' + encodeURIComponent(test.data);
    }
    const link = document.createElement('link');
    Object.assign(link, {rel: 'preload', as: test.destination, href: test.src});
    if ('integrity' in test) link.integrity = test.integrity;
    if ('crossOrigin' in test) link.crossOrigin = test.crossOrigin;
    const preloadDone = eventResult(link);
    document.head.append(link);
    if (!test.inFlight) await preloadDone;
    const before = globalThis.sriExecutions || 0;
    let consumer;
    let owner;
    let consumerDone;
    let statusText;
    if (test.destination === 'fetch') {
      consumerDone = fetch(test.src, {
        mode: test.consumerCrossOrigin ? 'cors' : 'no-cors',
        credentials: test.consumerCrossOrigin === 'anonymous' ? 'same-origin' : 'include'
      }).then(response => { statusText = response.statusText; return response.text(); })
        .then(() => 'load', () => 'error');
    } else {
      consumer = document.createElement(test.destination === 'image' ? 'img' : test.destination);
      if ('consumerIntegrity' in test) consumer.integrity = test.consumerIntegrity;
      if ('consumerCrossOrigin' in test) consumer.crossOrigin = test.consumerCrossOrigin;
      consumerDone = eventResult(consumer);
      consumer.src = test.src;
      if (test.destination === 'track') {
        owner = document.createElement('video');
        owner.crossOrigin = 'anonymous';
        consumer.default = true;
        owner.append(consumer);
        document.body.append(owner);
        consumer.track.mode = 'hidden';
      } else {
        document.body.append(consumer);
      }
    }
    observations.push({
      name: test.name,
      preload: await preloadDone,
      consumer: await consumerDone,
      statusText,
      executions: (globalThis.sriExecutions || 0) - before,
      timings: performance.getEntriesByName(test.src).length
    });
    link.remove();
    if (consumer && consumer.remove) consumer.remove();
    if (owner) owner.remove();
    if (objectURL) URL.revokeObjectURL(objectURL);
  }
  return JSON.stringify(observations);
})
