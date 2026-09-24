globalThis.windowOpenReferrerResults = new Map();
globalThis.windowOpenReferrerExpected = new Map();
globalThis.windowOpenReferrerReturns = [];
const referrerChannel = new BroadcastChannel('window-open-referrer');
referrerChannel.onmessage = event => {
  windowOpenReferrerResults.set(event.data.label, event.data);
};
let referrerEntry;

globalThis.startWindowOpenReferrerTest = (base, kind) => {
  const entryUrl = base + '/entry/page.html?entry=' + kind + '#fragment';
  globalThis.__entryLoaded = entry => {
    referrerEntry = entry;
    const meta = entry.document.createElement('meta');
    meta.name = 'referrer';
    entry.document.head.append(meta);
    const incumbent = kind === 'popup' ? window : entry.frames[0];
    const relevant = kind === 'popup' ? window : entry.frames[1];
    const invoke = incumbent.Function('relevant', 'url', 'target', 'features',
      'return relevant.open(url, target, features)');
    const run = mode => {
      for (const [policy, features] of [
        ['strict-origin-when-cross-origin', ''],
        ['strict-origin-when-cross-origin', 'noopener'],
        ['unsafe-url', 'noreferrer'],
        ['no-referrer', ''],
        ['origin', ''],
        ['unsafe-url', '']
      ]) {
        meta.content = policy;
        const label = mode + ':' + policy + ':' + features;
        const path = 'popup.html?label=' + encodeURIComponent(label);
        const referrer = features === 'noreferrer' || policy === 'no-referrer' ? '' :
          policy === 'origin' ? base + '/' : entryUrl.split('#')[0];
        windowOpenReferrerExpected.set(label, {
          href: base + '/entry/base/' + path,
          referrer,
          hasOpener: features === ''
        });
        const result = invoke(relevant, path, 'target-' + label, features);
        windowOpenReferrerReturns.push([label, (result === null) === (features !== '')]);
      }
    };
    run('event');
    entry.scheduleReferrerCases(() => run('promise'));
  };
  if (kind === 'popup') {
    referrerEntry = open(entryUrl);
  } else {
    const frame = document.createElement('iframe');
    frame.src = entryUrl;
    document.body.append(frame);
  }
};

globalThis.windowOpenReferrerFailures = () => {
  const failures = windowOpenReferrerReturns.filter(([, ok]) => !ok);
  for (const [label, expected] of windowOpenReferrerExpected) {
    const actual = windowOpenReferrerResults.get(label);
    for (const key of Object.keys(expected)) {
      if (!actual || actual[key] !== expected[key]) failures.push([label, key, expected[key], actual?.[key]]);
    }
  }
  return failures;
};

globalThis.windowOpenReferrerCleanup = () => {
  referrerChannel.close();
  if (referrerEntry && referrerEntry !== top) referrerEntry.close();
};
