async function () {
  const good = 'sha384-b9DQbErnj+wcR+dXhGEavTqeJs6OQTWA2lGTnjyUlV+dCmcdacihfgX5gSogUi4Z';
  const weaker = 'sha256-47XGYWUsSDkxStoU/S3LvDnl+iNx3+kDvJGmbIh1r8w=';
  const bad = 'sha384-AAAA';
  const rows = [];
  async function run({cache, phase, cors = null, crossOrigin = false, name = 'basic',
                      preloadIntegrity = '', consumerIntegrity = '',
                      expectedEvents = ['load', 'load'], expectedRequests = 1}) {
    const token = 'style-' + rows.length;
    const url = new URL('/probe-asset', location.href);
    Object.entries({token, as: 'style', cache}).forEach(([key, value]) => url.searchParams.set(key, value));
    if (crossOrigin) url.hostname = location.hostname === 'localhost' ? '127.0.0.1' : 'localhost';
    const preload = document.createElement('link');
    Object.assign(preload, {rel: 'preload', as: 'style', href: url.href, integrity: preloadIntegrity});
    if (cors !== null) preload.crossOrigin = cors;
    const preloaded = new Promise(resolve => { preload.onload = preload.onerror = e => resolve(e.type); });
    document.head.append(preload);
    await fetch('/probe-started?token=' + token);
    if (phase === 'complete') {
      await fetch('/probe-release?token=' + token);
      await preloaded;
    }
    const installedBeforeConsumer = document.styleSheets.length;
    const consumer = document.createElement('link');
    Object.assign(consumer, {rel: 'stylesheet', href: url.href, integrity: consumerIntegrity});
    if (cors !== null) consumer.crossOrigin = cors;
    // These fields are part of the stylesheet cache key, but not the preload key.
    if (name === 'different-consumer-metadata') {
      consumer.referrerPolicy = 'no-referrer';
      consumer.charset = 'utf-8';
    }
    let timingAtConsumer;
    const consumed = new Promise(resolve => {
      consumer.onload = consumer.onerror = e => {
        timingAtConsumer = performance.getEntriesByName(url.href).length;
        resolve(e.type);
      };
    });
    document.head.append(consumer);
    if (phase === 'pending') await fetch('/probe-release?token=' + token);
    const events = await Promise.all([preloaded, consumed]);
    let css;
    try { css = consumer.sheet?.cssRules[0]?.style.getPropertyValue('--preload-value') || ''; }
    catch (e) { css = e.name; }
    const {count} = await (await fetch('/probe-stats?token=' + token)).json();
    const timing = performance.getEntriesByName(url.href).map(e => e.initiatorType);
    const expectedCss = expectedEvents[1] === 'error' ? '' : crossOrigin && cors === null ? 'SecurityError' : 'loaded';
    rows.push({name, cache, phase, cors, crossOrigin, events, expectedEvents, count, expectedRequests,
               timing, timingAtConsumer, installedBeforeConsumer, css, expectedCss});
    consumer.remove();
    preload.remove();
  }
  for (const cache of ['max-age=60', 'no-store'])
    for (const phase of ['pending', 'complete'])
      for (const cors of [null, 'anonymous', 'use-credentials'])
        for (const crossOrigin of [false, true])
          await run({cache, phase, cors, crossOrigin});

  const cases = [
    ['good-empty', good, '', 'load', 'load', 1],
    ['bad-empty', bad, '', 'error', 'error', 1],
    ['bad-same', bad, bad, 'error', 'error', 1],
    ['bad-good', bad, good, 'error', 'load', 2],
    ['good-bad', good, bad, 'load', 'error', 2],
    ['good-options', good + '?ignored=option', good, 'load', 'load', 1],
    ['different-algorithm', good, weaker, 'load', 'load', 2],
    ['empty-good', '', good, 'load', 'load', 2],
    ['same-set-different-order', good + ' ' + weaker, weaker + ' ' + good, 'load', 'load', 1],
    ['different-weaker-hash', good + ' ' + weaker, good + ' sha256-AAAA', 'load', 'load', 2],
    ['different-consumer-metadata', good, '', 'load', 'load', 1],
    ['unknown-metadata', 'unknown-AAAA', '', 'load', 'load', 1],
  ];
  for (const cache of ['max-age=60', 'no-store'])
    for (const phase of ['pending', 'complete'])
      for (const [name, preloadIntegrity, consumerIntegrity, preloadEvent, consumerEvent, expectedRequests] of cases)
        await run({cache, phase, name, preloadIntegrity, consumerIntegrity,
                   expectedEvents: [preloadEvent, consumerEvent], expectedRequests});
  return {rows, failures: rows.filter(r =>
    JSON.stringify(r.events) !== JSON.stringify(r.expectedEvents) ||
    r.timing.length !== r.expectedRequests ||
    (r.expectedRequests === 1 && r.timingAtConsumer !== 1) ||
    (r.cache === 'no-store' && r.count !== r.expectedRequests) ||
    (r.expectedRequests === 1 && r.count !== 1) ||
    r.installedBeforeConsumer !== 0 || r.css !== r.expectedCss)};
}
