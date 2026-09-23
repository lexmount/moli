async function () {
  const controlled = navigator.serviceWorker.controller ? Promise.resolve() :
    new Promise(resolve => navigator.serviceWorker.addEventListener('controllerchange', resolve, {once: true}));
  await navigator.serviceWorker.register('/probe-worker.js');
  await navigator.serviceWorker.ready;
  await controlled;
  const good = 'sha384-b9DQbErnj+wcR+dXhGEavTqeJs6OQTWA2lGTnjyUlV+dCmcdacihfgX5gSogUi4Z';
  const rows = [];
  for (const phase of ['pending', 'complete'])
    for (const source of ['basic', 'cors', 'default', 'opaque'])
      for (const integrity of ['', good, 'sha384-AAAA']) {
        const token = 'style-sw-' + rows.length;
        const url = new URL('/probe-worker-asset', location.href);
        Object.entries({token, source, as: 'style'}).forEach(([key, value]) => url.searchParams.set(key, value));
        const preload = document.createElement('link');
        Object.assign(preload, {rel: 'preload', as: 'style', href: url.href, integrity});
        const preloaded = new Promise(resolve => { preload.onload = preload.onerror = e => resolve(e.type); });
        document.head.append(preload);
        if (source !== 'default') await fetch('/probe-started?token=' + token);
        if (phase === 'complete') {
          await fetch('/probe-release?token=' + token);
          await preloaded;
        }
        const consumer = document.createElement('link');
        Object.assign(consumer, {rel: 'stylesheet', href: url.href});
        const consumed = new Promise(resolve => { consumer.onload = consumer.onerror = e => resolve(e.type); });
        document.head.append(consumer);
        if (phase === 'pending') await fetch('/probe-release?token=' + token);
        const events = await Promise.all([preloaded, consumed]);
        const counts = await (await fetch('/probe-worker-counts')).json();
        let css;
        try { css = consumer.sheet?.cssRules[0]?.style.getPropertyValue('--preload-value') || ''; }
        catch (e) { css = e.name; }
        const rejected = integrity !== '' && (integrity !== good || source === 'opaque');
        const expectedEvent = rejected ? 'error' : 'load';
        const expectedCss = rejected ? '' : source === 'opaque' ? 'SecurityError' : 'loaded';
        rows.push({phase, source, integrity, events, expectedEvent, css, expectedCss,
                   dispatches: counts[url.href], timing: performance.getEntriesByName(url.href).length});
        consumer.remove();
        preload.remove();
      }
  return {rows, failures: rows.filter(r => r.events.some(e => e !== r.expectedEvent) ||
    r.css !== r.expectedCss || r.dispatches !== 1 || r.timing !== 1)};
}
