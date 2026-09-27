(async () => {
  const [home, other] = globalThis.originTestOrigins || globalThis.__fetchOrigins;
  const inspect = (0, eval)(await (await fetch(home + '/storage-report.js')).text());
  const keys = ['localStorage', 'sessionStorage'];
  const getters = keys.map(name => Object.getOwnPropertyDescriptor(self, name).get);
  const errors = [], observations = [];
  const check = (name, report, child) => {
    const expectedAccess = report.origin === 'null' ? 'SecurityError' : 'accessible';
    const expectedGetter = report.origin === 'null' ? 'SecurityError' : 'same';
    const borrowed = getters.map(getter => {
      try { getter.call(child); return 'accessible'; } catch (error) { return error.name; }
    });
    observations.push({name, report, borrowed});
    for (const key of keys) {
      const value = report.storage[key];
      if (JSON.stringify(value.descriptor) !== JSON.stringify(['function', 'undefined', true, true, false]) ||
          value.access !== expectedAccess || value.ownGetter !== expectedGetter || value.forged !== 'TypeError') {
        errors.push({name, key, value, expectedAccess, expectedGetter});
      }
    }
    const expectedBorrowed = report.origin === home ? 'accessible' : 'SecurityError';
    if (borrowed.some(value => value !== expectedBorrowed)) errors.push({name, borrowed, expectedBorrowed});
  };
  const cases = [
    ['popup HTTP', {}, home],
    ['CSP sandbox popup', {csp: 'sandbox allow-scripts'}, 'null'],
    ['CSP same-origin popup', {csp: 'sandbox allow-scripts allow-same-origin'}, home],
    ['cross-origin popup', {remote: true}, other],
    ['inherited sandbox popup', {sandbox: 'allow-scripts allow-popups'}, 'null'],
    ['escaped sandbox popup', {sandbox: 'allow-scripts allow-popups allow-popups-to-escape-sandbox'}, home],
    ['CSP sandbox iframe', {frame: true, csp: 'sandbox allow-scripts'}, 'null'],
    ['same-origin iframe', {frame: true}, home],
  ];
  for (const [name, options, expectedOrigin] of cases) {
    const token = 'storage-' + observations.length;
    const url = new URL('/storage-report', options.remote ? other : home);
    url.searchParams.set('token', token);
    if (options.csp) url.searchParams.set('csp', options.csp);
    if (options.sandbox) url.searchParams.set('launch', '1');
    let popup, frame, reportedWindow, listener, timer;
    try {
      const event = await new Promise((resolve, reject) => {
        listener = event => { if (event.data && event.data.token === token) resolve(event); };
        addEventListener('message', listener);
        timer = setTimeout(() => reject(new Error('child did not report')), 5000);
        if (options.frame || options.sandbox) {
          frame = document.createElement('iframe');
          if (options.sandbox) frame.setAttribute('sandbox', options.sandbox);
          frame.src = url.href; document.body.append(frame);
        } else {
          popup = open(url.href, '_blank');
          // Seed both caches in the initial same-origin Document. A later
          // sandboxed response must still deny access to its Storage getters.
          for (const key of keys) popup[key];
        }
      });
      reportedWindow = event.source;
      if (event.data.origin !== expectedOrigin || event.origin !== expectedOrigin) errors.push({name, origin: event.data.origin, messageOrigin: event.origin, expectedOrigin});
      check(name, event.data, reportedWindow);
    } catch (error) {
      observations.push({name, error: String(error)}); errors.push({name, error: String(error)});
    } finally {
      clearTimeout(timer); removeEventListener('message', listener);
      if (options.sandbox && reportedWindow) reportedWindow.close();
      if (popup) popup.close();
      if (frame) frame.remove();
    }
  }
  const blank = open();
  try { check('initial popup', inspect(blank, 'initial'), blank); } finally { blank.close(); }
  // Closing is queued: the current task must still see the same Storage objects.
  const closing = open();
  const cached = keys.map(key => closing[key]);
  closing.close();
  const retained = keys.map((key, index) => {
    try { return closing[key] === cached[index]; } catch (error) { return error.name; }
  });
  observations.push({name: 'queued popup close', retained});
  if (!closing.closed || retained.some(value => value !== true)) errors.push({name: 'queued popup close', retained});
  return JSON.stringify({errors, observations});
})()
