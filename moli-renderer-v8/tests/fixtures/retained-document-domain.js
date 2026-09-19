(async (otherPort) => {
  const result = {checks: 0, failures: [], observations: []};
  const check = (name, actual, expected) => {
    result.checks++;
    if (actual !== expected) result.failures.push({name, actual, expected});
  };
  const exercise = async config => {
    const observations = [];
    const documentDiagnostics = [];
    const read = action => {
      try { return action(); } catch (error) { return error.name; }
    };
    const record = (name, action, expected) => observations.push([name, read(action), expected]);
    const observeDocument = (name, action, expected) => documentDiagnostics.push([name, read(action), expected]);
    const peerRead = parent.Function('target', 'try { return target.status; } catch (error) { return error.name; }');
    const frame = document.createElement('iframe');
    if (config.sandbox) frame.sandbox = 'allow-scripts';
    if (config.inherit === 'srcdoc') frame.srcdoc = '<!doctype html><body>inherited</body>';
    else if (!config.inherit) {
      const childURL = new URL('/domain-fixture.html', location.href);
      if (config.childHost) childURL.hostname = config.childHost;
      if (config.childPort) childURL.port = config.childPort;
      if (config.childDomain) childURL.searchParams.set('domain', config.childDomain);
      frame.src = childURL.href;
    }
    const loaded = new Promise(resolve => frame.onload = resolve);
    document.body.append(frame);
    await loaded;
    const view = frame.contentWindow;
    if (config.parentDomain) document.domain = config.parentDomain;
    record('live-access', () => view.status, config.allow ? '' : 'SecurityError');
    const doc = config.allow ? view.document : null;
    const reverse = config.allow ? view.Function('other', 'return () => other.document.domain;')(window) : null;
    frame.remove();
    if (config.afterDomain) document.domain = config.afterDomain;
    const allowedAfter = config.allowAfter ?? config.allow;
    record('removed-access', () => view.status, allowedAfter ? '' : 'SecurityError');
    const childDomainAfter = config.inherit ? (config.afterDomain || config.parentDomain) : config.childDomain;
    const peerAllowed = !config.sandbox && !childDomainAfter && !config.childPort
      && (!config.childHost || config.childHost === location.hostname);
    record('independent-peer-access', () => peerRead(view), peerAllowed ? '' : 'SecurityError');
    if (doc) {
      const expectedDomain = config.inherit
        ? (config.afterDomain || config.parentDomain || location.hostname)
        : (config.childDomain || config.childHost || location.hostname);
      observeDocument('retained-document-domain', () => doc.domain, expectedDomain);
      record('retained-document-view', () => doc.defaultView, null);
      record('retained-document-setter', () => { doc.domain = expectedDomain; return 'accepted'; }, 'SecurityError');
      observeDocument('cloned-document-domain', () => doc.cloneNode(false).domain, expectedDomain);
    }
    if (reverse) record('retired-caller', reverse, allowedAfter ? document.domain : 'SecurityError');
    if (config.reinsert) {
      const newURL = new URL('/domain-fixture.html', location.href);
      newURL.searchParams.set('domain', location.hostname);
      const reloaded = new Promise(resolve => frame.onload = resolve);
      frame.src = newURL.href;
      document.body.append(frame);
      await reloaded;
      record('old-window-after-reinsert', () => view.status, '');
      record('replacement-remains-one-sided', () => frame.contentWindow.status, 'SecurityError');
      observeDocument('old-document-domain-after-reinsert', () => doc.domain, config.childDomain);
    }
    return {observations, documentDiagnostics, late: () => read(() => view.status), allowedAfter};
  };
  const host = location.hostname;
  const relaxed = 'example.test';
  const scenarios = [
    {name: 'tuple', allow: true},
    {name: 'exact-domain', parentDomain: host, childDomain: host, allow: true},
    {name: 'relaxed-domain', parentDomain: relaxed, childDomain: relaxed, allow: true},
    {name: 'subdomain', childHost: 'sub.example.test', parentDomain: relaxed, childDomain: relaxed, allow: true},
    {name: 'port', childPort: otherPort, parentDomain: relaxed, childDomain: relaxed, allow: true},
    {name: 'parent-only', parentDomain: relaxed, allow: false},
    {name: 'child-only', childDomain: relaxed, allow: false},
    {name: 'cross-origin', childHost: 'other.test', allow: false},
    {name: 'opaque', sandbox: true, allow: false},
    {name: 'parent-changes', allow: true, afterDomain: relaxed, allowAfter: false},
    {name: 'parent-catches-up', childDomain: relaxed, allow: false, afterDomain: relaxed, allowAfter: true},
    {name: 'inherited-blank', inherit: 'blank', allow: true, afterDomain: relaxed},
    {name: 'inherited-srcdoc', inherit: 'srcdoc', allow: true, afterDomain: relaxed},
    {name: 'reinsert', parentDomain: relaxed, childDomain: relaxed, allow: true, reinsert: true},
  ];
  for (const config of scenarios) {
    const outer = document.createElement('iframe');
    const loaded = new Promise(resolve => outer.onload = resolve);
    outer.src = '/domain-fixture.html';
    document.body.append(outer);
    await loaded;
    try {
      const observed = await outer.contentWindow.eval('(' + exercise.toString() + ')(' + JSON.stringify(config) + ')');
      result.observations.push({scenario: config.name, document: observed.documentDiagnostics});
      for (const [name, actual, expected] of observed.observations) check(config.name + ':' + name, actual, expected);
      outer.remove();
      await new Promise(resolve => setTimeout(resolve, 0));
      check(config.name + ':both-retired', observed.late(), observed.allowedAfter ? '' : 'SecurityError');
    } catch (error) {
      check(config.name + ':setup', error.name + ': ' + error.message, 'no exception');
    } finally {
      outer.remove();
    }
  }
  return result;
})
