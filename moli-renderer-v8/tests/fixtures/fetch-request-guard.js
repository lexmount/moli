async function fetchRequestGuardProbe(base, transport = true) {
  const checks = [];
  let requests = 0;
  const check = (label, actual, wanted) => checks.push({label, actual, wanted, pass: actual === wanted});
  const forbidden = [
    ['Accept-Charset', 'author-charset'], ['Accept-Encoding', 'author-encoding'],
    ['Access-Control-Request-Headers', 'author-header'], ['Access-Control-Request-Method', 'AUTHOR'],
    ['Connection', 'author-connection'], ['Content-Length', '42'],
    ['Cookie', 'author_cookie=bad'], ['Cookie2', 'author_cookie2=bad'],
    ['Date', 'author-date'], ['DNT', '4'], ['Host', 'forged.invalid'],
    ['Origin', 'https://forged.invalid'], ['Referer', 'https://forged.invalid/'],
    ['Set-Cookie', 'author_response=bad'], ['TE', 'author-te'],
    ['Trailer', 'author-trailer'], ['Upgrade', 'author-upgrade'], ['Via', 'author-via'],
    ['Proxy-Probe', 'author-proxy'], ['Sec-Probe', 'author-sec']
  ];
  const pairs = [...forbidden,
    ['Accept', 'text/plain'], ['Content-Language', 'en'], ['Content-Type', 'text/plain'],
    ['X-Allowed', 'kept'], ['X-Cookie', 'ordinary'], ['Authorization', 'Bearer ordinary'],
    ['X-HTTP-Method-Override', 'GETTRACE'], ['X-HTTP-Method', '\",TRACE\",'],
    ['X-Method-Override', 'GET'], ['x-method-override', 'TRACE']
  ];
  const snapshot = headers => JSON.stringify(Array.from(new Headers(headers).entries()));
  async function send(label, input, init, mode, expected, initializer) {
    const originalRequest = typeof input === 'string' ? null : snapshot(input.headers);
    const originalInit = initializer === undefined ? null : snapshot(initializer);
    requests++;
    try {
      const response = await fetch(input, init);
      check(label + '/status', response.status, 200);
      const observed = new Headers((await response.json()).headers);
      for (const [name, value] of forbidden) {
        check(label + '/' + name + '/forbidden', observed.get(name) === value, false);
      }
      for (const [name, value] of Object.entries(expected)) check(label + '/' + name, observed.get(name), value);
      if (transport) {
        check(label + '/host', observed.get('Host'), new URL(base).host);
        check(label + '/cookie', (observed.get('Cookie') || '').includes('guard_real=1'), true);
        check(label + '/fetch-mode', observed.get('Sec-Fetch-Mode'), mode);
        check(label + '/referrer', (observed.get('Referer') || '').startsWith(base), true);
      }
    } catch (error) {
      check(label + '/fetch', String(error), 'successful response');
    }
    if (originalRequest !== null) check(label + '/input-unchanged', snapshot(input.headers), originalRequest);
    if (originalInit !== null) check(label + '/initializer-unchanged', snapshot(initializer), originalInit);
  }
  for (const mode of ['cors', 'same-origin', 'no-cors']) {
    for (const inputKind of ['url', 'request-override', 'request-inherit-mode']) {
      for (const form of ['record', 'pairs', 'headers']) {
        const label = [mode, inputKind, form].join('/');
        const headers = form === 'record' ? Object.fromEntries(pairs)
          : form === 'headers' ? new Headers(pairs) : pairs.map(pair => pair.slice());
        const url = base + '/echo?case=' + encodeURIComponent(label);
        const input = inputKind === 'url' ? url : new Request(url, {
          mode: inputKind === 'request-inherit-mode' ? mode : 'cors',
          headers: {'X-Inherited': 'original'}
        });
        const init = {headers};
        if (inputKind === 'request-override' || inputKind === 'url' && mode !== 'cors') init.mode = mode;
        const allowed = value => mode === 'no-cors' ? null : value;
        await send(label, input, init, mode, {
          'Accept': 'text/plain', 'Content-Language': 'en', 'Content-Type': 'text/plain',
          'X-Allowed': allowed('kept'), 'X-Cookie': allowed('ordinary'),
          'Authorization': allowed('Bearer ordinary'), 'X-Inherited': null,
          'X-HTTP-Method-Override': allowed('GETTRACE'), 'X-HTTP-Method': allowed('\",TRACE\",'),
          // A Headers initializer has already combined these two values.
          // Record and sequence initializers validate each append separately.
          'X-Method-Override': allowed(form === 'headers' ? null : 'GET')
        }, headers);
      }
    }
    for (const emptyInit of [false, true]) {
      const label = mode + '/inherited/' + emptyInit;
      const input = new Request(base + '/echo?case=' + encodeURIComponent(label), {
        mode, headers: {'X-Inherited': 'original', 'Accept': 'text/plain', 'Cookie': 'author_cookie=bad'}
      });
      await send(label, input, emptyInit ? {} : undefined, mode, {
        'Accept': 'text/plain', 'X-Inherited': mode === 'no-cors' ? null : 'original'
      });
    }
  }
  for (const inputKind of ['url', 'request']) {
    const url = base + '/must-not-fetch';
    const input = inputKind === 'url' ? url : new Request(url);
    for (const kind of ['getter', 'stringification', 'non-byte', 'invalid-value']) {
      const label = inputKind + '/conversion/' + kind;
      const token = {label};
      const log = [];
      const headers = kind === 'getter' ? {get Cookie() {log.push('cookie'); throw token;}}
        : kind === 'stringification' ? [['Cookie', {toString() {log.push('cookie'); throw token;}}]]
        : {Cookie: kind === 'non-byte' ? '\u0100' : 'invalid\r\nvalue'};
      let synchronous = false;
      let promise;
      try { promise = fetch(input, {headers}); }
      catch (error) { synchronous = true; promise = Promise.reject(error); }
      check(label + '/async', synchronous, false);
      await promise.then(() => check(label + '/rejection', 'fulfilled', 'rejected'), error => {
        check(label + '/rejection', kind === 'getter' || kind === 'stringification' ? error === token : error instanceof TypeError, true);
      });
      if (kind === 'getter' || kind === 'stringification') check(label + '/conversion', log.join(','), 'cookie');
    }
  }
  return {state: checks.every(entry => entry.pass) ? 'pass' : 'fail', requests, checks};
}
