(function locationEntrySettings(config) {
  const methods = {
    same: ['href', 'reflect', 'descriptor', 'window', 'document', 'assign', 'replace', 'expando'],
    cross: ['href', 'reflect', 'descriptor', 'window', 'replace']
  };
  const cases = Object.entries(methods).flatMap(([origin, kinds]) =>
    ['event', 'promise'].flatMap(mode => kinds.map(kind =>
      ({origin, mode, kind, label: `${origin}-${mode}-${kind}`}))));

  if (config.role === 'child') {
    if (location.pathname === '/entry/page.html') {
      const base = document.createElement('base');
      base.href = '/entry/base/';
      document.head.append(base);
      window.scheduleLocationCases = callback => Promise.resolve().then(() => callback());
      const incumbent = document.createElement('iframe');
      incumbent.id = 'incumbent';
      incumbent.src = '/incumbent/page.html';
      document.body.append(incumbent);
      for (const item of cases) {
        const frame = document.createElement('iframe');
        frame.id = item.label;
        const url = new URL('/relevant/empty.html', location);
        if (item.origin === 'cross') url.hostname = '127.0.0.1';
        frame.src = url.href;
        document.body.append(frame);
      }
      window.onload = () => top.locationEntryLoaded(window);
    } else if (location.pathname.endsWith('/target.html')) {
      top.postMessage({
        label: new URL(location).searchParams.get('case'),
        href: location.href,
        referrer: document.referrer
      }, '*');
    }
    return;
  }

  return new Promise(resolve => {
    const rows = new Map();
    const failures = [];
    let checks = 0;
    const check = (condition, label) => {
      checks++;
      if (!condition) failures.push(label);
    };
    const frame = document.createElement('iframe');
    const receive = event => {
      if (!cases.some(item => item.label === event.data?.label)) return;
      rows.set(event.data.label, event.data);
      if (rows.size !== cases.length) return;
      for (const [label, row] of rows) {
        const expected = new URL(`/converted/${label}/target.html?case=${label}`, config.entryURL).href;
        check(row.href === expected, `${label}: URL ${row.href}, expected ${expected}`);
        check(row.referrer === new URL('/incumbent/page.html', config.entryURL).href,
          `${label}: referrer ${row.referrer}`);
      }
      window.removeEventListener('message', receive);
      delete window.locationEntryLoaded;
      frame.remove();
      resolve({checks, failures, rows: [...rows.values()]});
    };
    window.addEventListener('message', receive);
    window.locationEntryLoaded = entry => {
      const incumbent = entry.document.getElementById('incumbent').contentWindow;
      const navigate = incumbent.Function('target', 'entry', 'kind', 'label', `
        const input = {toString() {
          entry.document.querySelector('base').href = '/converted/' + label + '/';
          return 'target.html?case=' + label;
        }};
        switch (kind) {
          case 'href': target.location.href = input; break;
          case 'reflect': Reflect.set(target.location, 'href', input); break;
          case 'descriptor': Object.getOwnPropertyDescriptor(target.location, 'href').set.call(target.location, input); break;
          case 'window': target.location = input; break;
          case 'document': target.document.location = input; break;
          case 'assign': target.location.assign(input); break;
          case 'replace': target.location.replace(input); break;
          case 'expando':
            Object.defineProperty(target.location, 'navigateFromSetter', {
              set() { target.location.assign(input); }
            });
            target.location.navigateFromSetter = true;
            break;
        }
      `);
      const exceptions = incumbent.Function('target', 'ExpectedTypeError', 'sameOrigin', `
        const results = [];
        const location = target.location;
        let conversions = 0;
        const value = {toString() { conversions++; return 'target.html'; }};
        if (sameOrigin) {
          const revoked = Proxy.revocable(location, {});
          revoked.revoke();
          for (const receiver of [{}, new Proxy(location, {}), revoked.proxy]) {
            try { Reflect.set(location, 'href', value, receiver); results.push(false); }
            catch (error) { results.push(error instanceof ExpectedTypeError && conversions === 0); }
          }
        } else {
          try { location.hash = value; results.push(false); }
          catch (error) {
            results.push(error instanceof DOMException && error.name === 'SecurityError' && conversions === 0);
          }
        }
        const sentinel = {};
        try { location.href = {toString() { throw sentinel; }}; results.push(false); }
        catch (error) { results.push(error === sentinel); }
        try { location.href = Symbol(); results.push(false); }
        catch (error) { results.push(error instanceof ExpectedTypeError); }
        return results;
      `);
      for (const origin of ['same', 'cross']) {
        const target = entry.document.getElementById(`${origin}-event-href`).contentWindow;
        const expectedTypeError = origin === 'same' ? target.TypeError : incumbent.TypeError;
        exceptions(target, expectedTypeError, origin === 'same').forEach((result, index) =>
          check(result, `${origin}: receiver/conversion exception ${index}`));
      }
      const run = mode => {
        for (const item of cases.filter(item => item.mode === mode)) {
          const target = entry.document.getElementById(item.label).contentWindow;
          navigate(target, entry, item.kind, item.label);
        }
      };
      run('event');
      entry.scheduleLocationCases(() => run('promise'));
    };
    frame.src = config.entryURL;
    document.body.append(frame);
  });
})
