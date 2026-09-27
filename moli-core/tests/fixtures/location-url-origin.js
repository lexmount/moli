(async () => {
  const [home, other] = globalThis.originTestOrigins || globalThis.__fetchOrigins;
  const childSource = await (await fetch(home + '/origin-report.js')).text();
  const read = callback => {
    try { return callback(); } catch (error) { return error.name; }
  };
  const getter = Object.getOwnPropertyDescriptor(location, 'origin').get;
  const cases = [
    ['HTTP', {}, home, home],
    ['sandbox HTTP', {sandbox: 'allow-scripts'}, home, 'null'],
    ['same-origin sandbox HTTP', {sandbox: 'allow-scripts allow-same-origin'}, home, home],
    ['cross-origin HTTP', {remote: true}, other, other],
    ['cross-origin sandbox HTTP', {remote: true, sandbox: 'allow-scripts'}, other, 'null'],
    ['CSP sandbox HTTP', {csp: 'sandbox allow-scripts'}, home, 'null'],
    ['CSP same-origin HTTP', {csp: 'sandbox allow-scripts allow-same-origin'}, home, home],
    ['popup HTTP', {popup: true}, home, home],
    ['CSP sandbox popup', {popup: true, csp: 'sandbox allow-scripts'}, home, 'null'],
    ['Blob', {mode: 'blob'}, home, home],
    ['sandbox Blob', {mode: 'blob', sandbox: 'allow-scripts'}, home, 'null'],
    ['data', {mode: 'data'}, 'null', 'null'],
    ['srcdoc', {mode: 'srcdoc'}, 'null', home],
    ['sandbox srcdoc', {mode: 'srcdoc', sandbox: 'allow-scripts'}, 'null', 'null'],
  ];
  const errors = [], observations = [];
  for (const [name, options, locationOrigin, windowOrigin] of cases) {
    const token = 'origin-' + observations.length;
    const url = new URL('/origin-report', options.remote ? other : home);
    url.searchParams.set('token', token);
    if (options.csp) url.searchParams.set('csp', options.csp);
    const markup = '<!doctype html><script>globalThis.originToken=' + JSON.stringify(token) + ';' + childSource + '</script>';
    let target = url.href, blob, frame, popup, listener, timer;
    if (options.mode === 'blob') target = blob = URL.createObjectURL(new Blob([markup], {type: 'text/html'}));
    if (options.mode === 'data') target = 'data:text/html,' + encodeURIComponent(markup);
    try {
      const message = await new Promise((resolve, reject) => {
        listener = event => {
          if (event.data && event.data.token === token) resolve(event);
        };
        addEventListener('message', listener);
        timer = setTimeout(() => reject(new Error('child did not report')), 5000);
        if (options.popup) popup = open(target, '_blank');
        else {
          frame = document.createElement('iframe');
          if (options.sandbox) frame.setAttribute('sandbox', options.sandbox);
          if (options.mode === 'srcdoc') frame.srcdoc = markup;
          else frame.src = target;
          document.body.append(frame);
        }
      });
      const child = popup || frame.contentWindow;
      const parentRead = read(() => child.location.origin);
      const borrowedRead = read(() => getter.call(child.location));
      const expectedAccess = windowOrigin === home ? locationOrigin : 'SecurityError';
      const observed = {name, data: message.data, eventOrigin: message.origin, parentRead, borrowedRead};
      observations.push(observed);
      if (message.source !== child || message.data.locationOrigin !== locationOrigin ||
          message.data.ownGetter !== locationOrigin || message.data.windowOrigin !== windowOrigin ||
          message.origin !== windowOrigin ||
          parentRead !== expectedAccess || borrowedRead !== expectedAccess) {
        errors.push({observed, expected: {locationOrigin, windowOrigin, expectedAccess}});
      }
    } catch (error) {
      const observed = {name, error: String(error)};
      observations.push(observed); errors.push(observed);
    } finally {
      clearTimeout(timer); removeEventListener('message', listener);
      if (popup) popup.close();
      if (frame) frame.remove();
      if (blob) URL.revokeObjectURL(blob);
    }
  }
  const blank = document.createElement('iframe');
  document.body.append(blank);
  const blankOrigins = [blank.contentWindow.location.origin, blank.contentWindow.origin];
  observations.push({name: 'initial about:blank', origins: blankOrigins});
  if (JSON.stringify(blankOrigins) !== JSON.stringify(['null', home])) errors.push({blankOrigins});
  blank.remove();
  return JSON.stringify({errors, observations});
})()
