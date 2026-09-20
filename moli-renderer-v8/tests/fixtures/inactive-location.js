async () => {
  const result = {checks: 0, failures: []};
  const check = (name, action, expected) => {
    let actual;
    try { actual = action(); } catch (error) { actual = error.name; }
    result.checks++;
    if (actual !== expected) result.failures.push({name, actual, expected});
  };
  const load = async path => {
    const frame = document.createElement('iframe');
    const loaded = new Promise(resolve => frame.onload = resolve);
    frame.src = path;
    document.body.append(frame);
    await loaded;
    return frame;
  };
  const properties = {
    href: 'about:blank', origin: 'null', protocol: 'about:', host: '',
    hostname: '', port: '', pathname: 'blank', search: '', hash: ''
  };
  const mutations = [
    ...['href', 'protocol', 'host', 'hostname', 'port', 'pathname', 'search', 'hash']
      .map(name => [name, (loc, value) => { loc[name] = value; }]),
    ...['assign', 'replace'].map(name => [name, (loc, value) => loc[name](value)]),
    ['window-location', (loc, value, win) => { win.location = value; }]
  ];
  const detached = (label, win, loc, doc, oldURL) => {
    check(`${label}:identity`, () => win.location === loc, true);
    check(`${label}:document-URL`, () => doc.URL, oldURL);
    for (const [name, expected] of Object.entries(properties)) {
      check(`${label}:get-${name}`, () => loc[name], expected);
    }
    check(`${label}:stringifier`, () => String(loc), 'about:blank');
    check(`${label}:ancestor-origins`, () => loc.ancestorOrigins.length, 0);
    const emptyOrigins = loc.ancestorOrigins;
    check(`${label}:stable-origins`, () => loc.ancestorOrigins === emptyOrigins, true);
    for (const [name, mutate] of mutations) {
      check(`${label}:set-${name}`, () => { mutate(loc, 'http://test:test/', win); return 'done'; }, 'done');
      check(`${label}:after-${name}`, () => loc.href, 'about:blank');
      let converted = 0;
      const sentinel = {name: 'ConversionSentinel'};
      const poison = {toString() { converted++; throw sentinel; }};
      check(`${label}:conversion-${name}`, () => mutate(loc, poison, win), 'ConversionSentinel');
      check(`${label}:conversion-count-${name}`, () => converted, 1);
    }
    check(`${label}:reload`, () => loc.reload({toString() { throw new Error('not converted'); }}), undefined);
    for (const name of ['assign', 'replace']) {
      check(`${label}:required-${name}`, () => loc[name](), 'TypeError');
    }
    check(`${label}:unchanged-document-URL`, () => doc.URL, oldURL);
  };
  check('live:main-url', () => location.href, document.URL);
  const blankFrame = document.createElement('iframe');
  document.body.append(blankFrame);
  const blankWindow = blankFrame.contentWindow;
  const blankLocation = blankWindow.location;
  const blankDocument = blankWindow.document;
  blankFrame.remove();
  detached('blank', blankWindow, blankLocation, blankDocument, 'about:blank');

  const frame = await load('/removed.html?query=one#fragment');
  const win = frame.contentWindow;
  const loc = win.location;
  const doc = win.document;
  const href = loc.href;
  frame.remove();
  detached('loaded', win, loc, doc, href);
  const loadedAgain = new Promise(resolve => frame.onload = resolve);
  document.body.append(frame);
  await loadedAgain;
  check('reinsert:fresh-location', () => frame.contentWindow.location !== loc, true);
  check('reinsert:old-location', () => loc.href, 'about:blank');
  check('reinsert:old-navigation', () => { loc.href = 'http://test:test/'; return 'done'; }, 'done');
  check('reinsert:fresh-url', () => frame.contentWindow.location.href, href);
  frame.remove();

  for (const [name, mutate] of mutations) {
    const frame = document.createElement('iframe');
    document.body.append(frame);
    const win = frame.contentWindow;
    const loc = win.location;
    let converted = 0;
    const value = {toString() { converted++; frame.remove(); return 'http://test:test/'; }};
    check(`conversion-removes:${name}`, () => { mutate(loc, value, win); return 'done'; }, 'done');
    check(`conversion-removes-count:${name}`, () => converted, 1);
    check(`conversion-removes-url:${name}`, () => loc.href, 'about:blank');
  }

  const srcdocFrame = document.createElement('iframe');
  const srcdocLoaded = new Promise(resolve => srcdocFrame.onload = resolve);
  srcdocFrame.srcdoc = '<!doctype html><p>srcdoc</p>';
  document.body.append(srcdocFrame);
  await srcdocLoaded;
  const srcdocWindow = srcdocFrame.contentWindow;
  const srcdocLocation = srcdocWindow.location;
  const srcdocDocument = srcdocWindow.document;
  const srcdocURL = srcdocDocument.URL;
  srcdocFrame.remove();
  detached('srcdoc', srcdocWindow, srcdocLocation, srcdocDocument, srcdocURL);

  const popup = window.open('about:blank');
  if (!popup) throw new Error('popup should be created');
  const popupLocation = popup.location;
  const popupDocument = popup.document;
  popup.close();
  // close() queues destruction; yield before probing the retired Location.
  await new Promise(resolve => setTimeout(resolve, 10));
  detached('popup', popup, popupLocation, popupDocument, 'about:blank');

  const setter = Object.getOwnPropertyDescriptor(blankLocation, 'href').set;
  let conversions = 0;
  const poison = {toString() { conversions++; return 'http://test:test/'; }};
  for (const receiver of [{}, Object.create(blankLocation), new Proxy(blankLocation, {})]) {
    check('inactive:invalid-receiver', () => setter.call(receiver, poison), 'TypeError');
    check('inactive:brand-before-conversion', () => conversions, 0);
  }

  const navigatedFrame = await load('/before.html');
  const oldLocation = navigatedFrame.contentWindow.location;
  const nextLoad = new Promise(resolve => navigatedFrame.onload = resolve);
  navigatedFrame.src = '/after.html';
  await nextLoad;
  check('retired:fresh-location', () => oldLocation !== navigatedFrame.contentWindow.location, true);
  const activeURL = navigatedFrame.contentWindow.location.href;
  check('retired:invalid-navigation', () => { oldLocation.assign('http://test:test/'); return 'done'; }, 'done');
  oldLocation.hash = '#stale';
  await new Promise(resolve => setTimeout(resolve, 0));
  check('retired:current-document-unchanged', () => navigatedFrame.contentWindow.location.href, activeURL);
  navigatedFrame.remove();
  return result;
}
