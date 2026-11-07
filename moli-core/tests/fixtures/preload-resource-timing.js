(() => {
  const source = document.currentScript.src;
  const realm = new URL(source).searchParams.get('realm');
  globalThis.preloadResourceTimingResult = (async () => {
    const rows = [];
    for (const kind of ['script', 'style', 'image', 'font', 'fetch']) {
      for (const cache of ['no-store', 'max-age=60']) {
        for (const phase of ['pending', 'complete']) {
          const token = `${realm}-${kind}-${cache}-${phase}`;
          const url = new URL('/probe-asset', location.href);
          url.searchParams.set('token', token);
          url.searchParams.set('as', kind);
          url.searchParams.set('cache', cache);
          const link = document.createElement('link');
          Object.assign(link, {rel: 'preload', as: kind, href: url.href});
          if (kind === 'font' || kind === 'fetch') link.crossOrigin = 'anonymous';
          const loaded = new Promise(resolve => {
            link.onload = link.onerror = event => resolve(event.type);
          });
          document.head.append(link);
          await fetch('/probe-started?token=' + encodeURIComponent(token));
          if (phase === 'complete') {
            await fetch('/probe-release?token=' + encodeURIComponent(token));
            await loaded;
          }
          let consumer;
          let consumed;
          if (kind === 'fetch') {
            consumed = fetch(url).then(response => response.text()).then(() => 'load');
          } else if (kind === 'font') {
            consumer = new FontFace('TimingFont' + rows.length, `url("${url}")`);
            consumed = consumer.load().then(() => 'load', () => 'error');
          } else {
            consumer = document.createElement(kind === 'style' ? 'link' : kind === 'image' ? 'img' : 'script');
            if (kind === 'style') Object.assign(consumer, {rel: 'stylesheet', href: url.href});
            else consumer.src = url.href;
            consumed = new Promise(resolve => {
              consumer.onload = consumer.onerror = event => resolve(event.type);
            });
            document.body.append(consumer);
          }
          if (phase === 'pending') await fetch('/probe-release?token=' + encodeURIComponent(token));
          const events = await Promise.all([loaded, consumed]);
          const stats = await (await fetch('/probe-stats?token=' + encodeURIComponent(token))).json();
          const entries = performance.getEntriesByName(url.href, 'resource').map(entry => ({
            type: entry.initiatorType, source: entry.initiatorUrl,
            encoded: entry.encodedBodySize, end: entry.responseEnd
          }));
          rows.push({kind, cache, phase, events, requests: stats.count, entries});
          link.remove();
          if (consumer && consumer.remove) consumer.remove();
        }
      }
    }
    const controlled = navigator.serviceWorker.controller ? Promise.resolve() :
      new Promise(resolve => navigator.serviceWorker.addEventListener('controllerchange', resolve, {once: true}));
    await navigator.serviceWorker.register('/probe-worker.js');
    await navigator.serviceWorker.ready;
    await controlled;
    const swRows = [];
    for (const response of ['network', 'default', 'basic']) {
      for (const kind of response === 'network' ? ['script'] : ['script', 'style']) {
        const token = `${realm}-sw-${response}-${kind}`;
        const url = new URL(response === 'network' ? '/probe-asset' : '/probe-worker-asset', location.href);
        url.searchParams.set('token', token);
        url.searchParams.set('as', kind);
        url.searchParams.set('source', response);
        await fetch('/probe-release?token=' + encodeURIComponent(token));
        const link = document.createElement('link');
        Object.assign(link, {rel: 'preload', as: kind, href: url.href});
        const loaded = new Promise(resolve => {
          link.onload = link.onerror = event => resolve(event.type);
        });
        document.head.append(link);
        const preloadEvent = await loaded;
        const consumer = document.createElement(kind === 'style' ? 'link' : 'script');
        if (kind === 'style') Object.assign(consumer, {rel: 'stylesheet', href: url.href});
        else consumer.src = url.href;
        const consumed = new Promise(resolve => {
          consumer.onload = consumer.onerror = event => resolve(event.type);
        });
        document.body.append(consumer);
        const events = [preloadEvent, await consumed];
        const stats = await (await fetch('/probe-stats?token=' + encodeURIComponent(token))).json();
        const counts = await (await fetch('/probe-worker-counts')).json();
        const entries = performance.getEntriesByName(url.href, 'resource').map(entry => ({
          type: entry.initiatorType, source: entry.initiatorUrl,
          encoded: entry.encodedBodySize, end: entry.responseEnd
        }));
        swRows.push({response, kind, events, requests: stats.count,
          workerRequests: counts[url.href] || 0, entries});
        link.remove();
        consumer.remove();
      }
    }
    const swFailures = swRows.filter(row => row.requests !== (row.response === 'default' ? 0 : 1) ||
      row.workerRequests !== (row.response === 'network' ? 0 : 1) || row.events.some(event => event !== 'load') ||
      row.entries.length !== 1 || row.entries.some(entry => entry.type !== 'link' ||
        entry.source !== (row.response === 'network' ? source : '') || entry.encoded <= 0 || entry.end <= 0));
    const failures = rows.filter(row => row.requests !== 1 || row.events.some(event => event !== 'load') ||
      row.entries.length !== 1 || row.entries.some(entry => entry.type !== 'link' ||
        entry.source !== source || entry.encoded <= 0 || entry.end <= 0));
    return {source, rows, failures, swRows, swFailures};
  })();
})();
