async function() {
  const control = (action, token) => fetch('/probe-' + action + '?token=' + token).then(r => r.json());
  await control('release', 'font-hash');
  const bytes = await (await fetch('/probe-asset?as=font&token=font-hash')).arrayBuffer();
  const hash = await crypto.subtle.digest('SHA-384', bytes);
  const integrity = 'sha384-' + btoa(String.fromCharCode(...new Uint8Array(hash)));
  const rows = [], failures = [];
  for (const phase of ['pending', 'completed']) {
    for (const crossOrigin of ['anonymous', 'use-credentials', null]) {
      for (const valid of [true, false]) {
        const token = 'font-preload-' + rows.length;
        const url = new URL('/probe-asset?as=font&token=' + token, location.href).href;
        const link = document.createElement('link');
        Object.assign(link, {rel: 'preload', as: 'font', href: url, integrity: valid ? integrity : 'sha384-AAAA==='});
        if (crossOrigin !== null) link.crossOrigin = crossOrigin;
        const preload = new Promise(resolve => link.onload = link.onerror = e => resolve(e.type));
        document.head.append(link);
        await control('started', token);
        if (phase === 'completed') { await control('release', token); await preload; }
        const face = new FontFace('Preloaded' + rows.length, `url("${url}")`);
        const loaded = face.load().then(() => 'loaded', e => e.name);
        if (phase === 'pending') {
          if (face.status !== 'loading') failures.push({phase, crossOrigin, valid, status: face.status});
          await control('release', token);
        }
        const actual = [await preload, await loaded, (await control('stats', token)).count,
          performance.getEntriesByName(url).map(e => e.initiatorType).sort()];
        const shared = crossOrigin === 'anonymous';
        const expected = [valid ? 'load' : 'error', shared && !valid ? 'NetworkError' : 'loaded',
          shared ? 1 : 2, shared ? ['link'] : ['css', 'link']];
        rows.push({phase, crossOrigin, valid, actual});
        if (JSON.stringify(actual) !== JSON.stringify(expected)) failures.push({phase, crossOrigin, valid, actual, expected});
        link.remove();
      }
    }
  }
  return {rows, failures};
}
