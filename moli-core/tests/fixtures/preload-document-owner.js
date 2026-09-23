(async () => {
  const failures = [];
  const observations = [];
  const frames = [];
  for (let i = 0; i < 2; ++i) {
    const frame = document.createElement('iframe');
    frame.srcdoc = '<!doctype html><head></head><body>preload owner';
    await new Promise(resolve => { frame.onload = resolve; document.body.append(frame); });
    frames.push(frame);
  }
  const resource = new URL(globalThis.preloadProbeResourcePath ||
    '/assets/module-synthetic-style.css', location.href);
  const preload = (win, destination, url) => new Promise(resolve => {
    const link = win.document.createElement('link');
    Object.assign(link, {rel: 'preload', as: destination, href: url.href});
    link.onload = link.onerror = event => resolve({
      event: event.type,
      ownRealm: event instanceof win.Event,
      parentRealm: event instanceof Event,
      ownTarget: event.target === link,
      childEntries: win.performance.getEntriesByName(url.href).length,
      parentEntries: performance.getEntriesByName(url.href).length
    });
    win.document.head.append(link);
  });
  for (const destination of ['fetch', 'script', 'style', 'image', 'font', 'track']) {
    const url = new URL(resource);
    url.searchParams.set('preload-owner', destination);
    const results = await Promise.all(frames.map(frame =>
      preload(frame.contentWindow, destination, url)));
    for (const [index, result] of results.entries()) {
      observations.push({destination, index, ...result});
      if (!result.ownRealm || result.parentRealm ||
          !result.ownTarget || result.childEntries !== 1 || result.parentEntries !== 0)
        failures.push({destination, index, ...result});
    }
    if (destination === 'style') {
      const cached = await preload(frames[0].contentWindow, destination, url);
      observations.push({destination: 'cached-style', index: 0, ...cached});
      if (cached.event !== 'load' || !cached.ownRealm || cached.parentRealm ||
          !cached.ownTarget || cached.childEntries !== 1 || cached.parentEntries !== 0)
        failures.push({destination: 'cached-style', ...cached});
    }
  }
  frames.forEach(frame => frame.remove());
  return {observations, failures};
})()
