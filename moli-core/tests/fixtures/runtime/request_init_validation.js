async function runRequestInitValidationProbe(scenario, url) {
  const errors = [];
  const check = (value, label) => { if (!value) errors.push(label); };
  const controller = new AbortController();
  const reason = {probe: 'already aborted'};
  controller.abort(reason);
  const fetchError = async (input, init, expected, label, fetchFunction = fetch, receiver = self) => {
    let promise;
    try { promise = fetchFunction.call(receiver, input, init); }
    catch (error) { errors.push(label + ': synchronous throw ' + error); return; }
    try { await promise; errors.push(label + ': fulfilled'); }
    catch (error) {
      check(expected === reason ? error === reason : error instanceof expected,
        label + ': wrong rejection ' + error);
    }
  };
  const throwsTypeError = (input, init, label) => {
    try { new Request(input, init); errors.push(label + ': accepted'); }
    catch (error) { check(error instanceof TypeError, label + ': wrong exception ' + error); }
  };

  if (scenario === 'invalid') {
    const invalid = [
      ['navigate', {mode: 'navigate'}],
      ['default mode with only-if-cached', {cache: 'only-if-cached'}],
      ['cors with only-if-cached', {mode: 'cors', cache: 'only-if-cached'}],
      ['no-cors with only-if-cached', {mode: 'no-cors', cache: 'only-if-cached'}],
      ['invalid referrer', {referrer: 'http://:invalid'}],
      ...['', 'IN VALID', ' GET', 'GET\t', 'a/b', 'a:b', 'é', '\u0100', 'TRACE']
        .map(method => ['method ' + JSON.stringify(method), {method}]),
      ...[['cache', 'reload'], ['referrerPolicy', 'origin'], ['duplex', 'half'],
        ['mode', 'cors'], ['credentials', 'omit'], ['redirect', 'follow']]
        .flatMap(([key, valid]) => ['BAD', null, valid.toUpperCase(), ' ' + valid]
          .map(value => [key + ' ' + JSON.stringify(value), {[key]: value}]))
    ];
    for (const [label, init] of invalid) {
      throwsTypeError(url, init, label);
      await fetchError(url, init, TypeError, label + ' live');
      await fetchError(url, {...init, signal: controller.signal}, TypeError, label + ' aborted');
    }
    for (const credentials of ['user:pass', 'user', ':pass']) {
      const credentialURL = new URL(url);
      const [username, password = ''] = credentials.split(':');
      credentialURL.username = username;
      credentialURL.password = password;
      throwsTypeError(credentialURL.href, {}, 'URL credentials ' + credentials);
      await fetchError(credentialURL.href, {signal: controller.signal}, TypeError,
        'URL credentials ' + credentials + ' aborted');
    }
  } else if (scenario === 'valid') {
    const valid = [
      [{window: undefined}, {}], [{window: null}, {}],
      [{method: null}, {method: 'null'}], [{method: 'pOsT'}, {method: 'POST'}],
      [{method: "a!#$%&'*+-.^_`|~"}, {method: "a!#$%&'*+-.^_`|~"}],
      [{referrer: ''}, {referrer: ''}],
      [{referrer: 'about:client'}, {referrer: 'about:client'}],
      [{referrer: new URL('/referrer', url).href}, {referrer: new URL('/referrer', url).href}],
      [{referrer: 'https://other.invalid/'}, {referrer: 'about:client'}],
      [{duplex: 'half'}, {duplex: 'half'}],
      ...['default', 'no-store', 'reload', 'no-cache', 'force-cache', 'only-if-cached']
        .map(cache => [{cache, mode: 'same-origin'}, {cache, mode: 'same-origin'}]),
      ...['', 'no-referrer', 'no-referrer-when-downgrade', 'same-origin', 'origin',
        'strict-origin', 'origin-when-cross-origin', 'strict-origin-when-cross-origin', 'unsafe-url']
        .map(referrerPolicy => [{referrerPolicy}, {referrerPolicy}])
    ];
    for (const [init, expected] of valid) {
      const request = new Request(url, init);
      for (const [key, value] of Object.entries(expected)) {
        check(request[key] === value, JSON.stringify(init) + ': ' + key + '=' + request[key]);
      }
      await fetchError(url, {...init, signal: controller.signal}, reason, JSON.stringify(init));
    }
  } else if (scenario === 'inheritance') {
    const makeSource = () => new Request(url, {
      mode: 'same-origin', cache: 'only-if-cached', method: 'POST', body: 'original body'
    });
    for (const init of [{mode: 'cors'}, {mode: 'no-cors'}, {mode: 'navigate'},
      {referrer: 'http://:invalid'}, {cache: 'BAD'}, {referrerPolicy: null}]) {
      const source = makeSource();
      throwsTypeError(source, init, 'inherited Request ' + JSON.stringify(init));
      check(!source.bodyUsed, 'failed constructor consumed input body');
      if (!source.bodyUsed) check(await source.text() === 'original body', 'constructor input body remains readable');
      for (const aborted of [false, true]) {
        const source = makeSource();
        const promise = fetchError(source, {...init, ...(aborted ? {signal: controller.signal} : {})},
          TypeError, 'inherited fetch ' + JSON.stringify(init) + ' aborted=' + aborted);
        check(!source.bodyUsed, 'failed fetch synchronously consumed input body');
        await promise;
        check(!source.bodyUsed, 'failed fetch consumed input body');
        if (!source.bodyUsed) check(await source.text() === 'original body', 'fetch input body remains readable');
      }
    }
    const cached = new Request(url, {mode: 'same-origin', cache: 'only-if-cached'});
    for (const init of [{}, {mode: undefined, cache: undefined}, {cache: 'reload', mode: 'cors'}]) {
      const copy = new Request(cached, init);
      check(copy.mode === (init.mode || 'same-origin'), 'inherited effective mode');
      check(copy.cache === (init.cache || 'only-if-cached'), 'inherited effective cache');
      await fetchError(cached, {...init, signal: controller.signal}, reason, 'valid inheritance');
    }
  } else if (scenario === 'realm') {
    if (document.readyState === 'loading') {
      await new Promise(resolve => addEventListener('load', resolve, {once: true}));
    }
    const frame = document.createElement('iframe');
    const loaded = new Promise(resolve => frame.onload = resolve);
    frame.src = 'about:blank';
    document.body.append(frame);
    await loaded;
    const child = frame.contentWindow;
    for (const [fn, receiver, ErrorConstructor, PromiseConstructor] of [
      [fetch, child, TypeError, Promise],
      [child.fetch, self, child.TypeError, child.Promise]
    ]) {
      for (const init of [{cache: 'BAD'}, {referrerPolicy: null}, {duplex: 'full'}, {redirect: null}]) {
        await fetchError(url, {...init, signal: controller.signal}, ErrorConstructor,
          'conversion realm ' + JSON.stringify(init), fn, receiver);
      }
      for (const init of [{mode: 'navigate'}, {cache: 'only-if-cached'},
        {referrer: 'http://:invalid'}]) {
        await fetchError(url, {...init, signal: controller.signal}, ErrorConstructor,
          'construction realm ' + JSON.stringify(init), fn, receiver);
        const promise = fn.call(receiver, url, init);
        promise.catch(() => {});
        check(promise instanceof PromiseConstructor, 'construction rejection promise realm');
      }
    }
    frame.remove();
  } else if (scenario === 'getters') {
    for (const init of [{mode: 'navigate'}, {mode: 'cors', cache: 'only-if-cached'},
      {window: 1}, {referrer: 'http://:invalid'}]) {
      for (const member of ['body', 'signal']) {
        const marker = {member};
        const options = {method: 'POST', ...init, get [member]() { throw marker; }};
        try { new Request(url, options); errors.push(member + ' getter accepted'); }
        catch (error) { check(error === marker, member + ' getter exception replaced'); }
      }
    }
    const source = new Request(url, {signal: controller.signal});
    const copy = new Request(source, {signal: undefined});
    check(copy.signal.aborted && copy.signal.reason === reason, 'undefined signal preserves inheritance');
  } else if (scenario === 'window') {
    // Fetch's RequestInit.window is still specified and covered by WPT, even
    // though Chromium 145 ignores the member.
    const invalid = [
      ['window string', {window: 'null'}],
      ['window false', {window: false}],
      ['window zero', {window: 0}],
      ['window object', {window: {toString() { throw new Error('must not coerce window'); }}}],
    ];
    for (const [label, init] of invalid) {
      throwsTypeError(url, init, label);
      await fetchError(url, init, TypeError, label + ' live');
      await fetchError(url, {...init, signal: controller.signal}, TypeError, label + ' aborted');
      const source = new Request(url, {method: 'POST', body: 'original body'});
      throwsTypeError(source, init, label + ' inherited');
      check(!source.bodyUsed, 'window rejection consumed constructor input');
      if (!source.bodyUsed) {
        await fetchError(source, {...init, signal: controller.signal}, TypeError, label + ' inherited aborted');
        check(!source.bodyUsed, 'window rejection consumed fetch input');
        if (!source.bodyUsed) check(await source.text() === 'original body', 'window input remains readable');
      }
    }
    let reads = 0;
    const init = {get window() { ++reads; return null; }};
    new Request(url, init);
    check(reads === 1, 'Request reads window once');
    reads = 0;
    await fetchError(url, {get window() { ++reads; return null; }, signal: controller.signal},
      reason, 'fetch reads window');
    check(reads === 1, 'fetch reads window once');
  } else {
    throw new Error('unknown scenario ' + scenario);
  }
  return {errors};
}
