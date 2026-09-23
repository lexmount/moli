async function(crossOrigin) {
  const rows = [], failures = [];
  const check = (name, actual, expected) => {
    rows.push({name, actual});
    if (JSON.stringify(actual) !== JSON.stringify(expected)) failures.push({name, actual, expected});
  };
  const url = (token, query = '', origin = location.origin) =>
    origin + '/probe-asset?as=font&token=' + token + query;
  const control = (action, token, origin = location.origin) =>
    fetch(origin + '/probe-' + action + '?token=' + token).then(r => r.json());
  const outcome = face => face.load().then(value => value === face ? 'loaded' : 'wrong value', e => e.name);
  const event = name => new Promise(resolve => document.fonts.addEventListener(name, resolve, {once: true}));

  const face = new FontFace('AsyncFont', `url("${url('async')}")`);
  document.fonts.add(face);
  const previousReady = document.fonts.ready;
  const done = event('loadingdone');
  const loaded = face.load();
  const setLoad = document.fonts.load('16px AsyncFont');
  const ready = document.fonts.ready;
  let settled = false, setSettled = false, readySettled = false;
  loaded.then(() => settled = true);
  setLoad.then(() => setSettled = true);
  ready.then(() => readySettled = true);
  check('synchronous-state', [face.status, document.fonts.status, face.loaded === loaded,
    face.load() === loaded, ready !== previousReady, document.fonts.check('16px AsyncFont')],
    ['loading', 'loading', true, true, true, false]);
  // Font loading and its events are native tasks, outside the public timer ids.
  for (let id = 1; id < 100; id++) clearTimeout(id);
  await control('started', 'async');
  await Promise.resolve();
  check('waits-for-bytes', [settled, setSettled, readySettled], [false, false, false]);
  await control('release', 'async');
  const values = await setLoad;
  await ready;
  const completed = await done;
  check('completed-state', [face.status, document.fonts.status, values.length, values[0] === face,
    completed.fontfaces.length, completed.fontfaces[0] === face, document.fonts.check('16px AsyncFont'),
    (await control('stats', 'async')).count, performance.getEntriesByName(url('async')).map(e => e.initiatorType)],
    ['loaded', 'loaded', 1, true, 1, true, true, 1, ['css']]);

  const one = new FontFace('GroupOne', `url("${url('one')}")`);
  const two = new FontFace('GroupTwo', `url("${url('two')}")`);
  document.fonts.add(one); document.fonts.add(two);
  const groupDone = event('loadingdone');
  const first = outcome(one), second = outcome(two);
  const groupReady = document.fonts.ready;
  let groupSettled = false; groupReady.then(() => groupSettled = true);
  await control('started', 'one'); await control('started', 'two');
  await control('release', 'one'); await first; await Promise.resolve();
  check('ready-waits-for-all', [groupSettled, document.fonts.status], [false, 'loading']);
  document.fonts.delete(two);
  await groupReady;
  const groupEvent = await groupDone;
  await control('release', 'two'); await second;
  check('removing-pending-face', [document.fonts.status, groupEvent.fontfaces.map(f => f.family)], ['loaded', ['GroupOne']]);

  for (const [token, query] of [['invalid', '&invalid=1'], ['http-error', '&status=404']]) {
    await control('release', token);
    check(token, await outcome(new FontFace(token, `url("${url(token, query)}")`)), 'NetworkError');
  }
  await control('release', 'fallback-bad'); await control('release', 'fallback-good');
  const fallback = new FontFace('Fallback', `url("${url('fallback-bad', '&invalid=1')}"), url("${url('fallback-good')}")`);
  check('fallback-after-decode-failure', [await outcome(fallback), (await control('stats', 'fallback-bad')).count,
    (await control('stats', 'fallback-good')).count], ['loaded', 1, 1]);

  await control('release', 'cross-good', crossOrigin); await control('release', 'cross-bad', crossOrigin);
  check('cors', [await outcome(new FontFace('CrossGood', `url("${url('cross-good', '', crossOrigin)}")`)),
    await outcome(new FontFace('CrossBad', `url("${url('cross-bad', '&cors=none', crossOrigin)}")`))], ['loaded', 'NetworkError']);

  const base = document.createElement('base'); base.href = location.origin + '/before/'; document.head.append(base);
  const relative = new FontFace('Relative', 'url("../probe-asset?as=font&token=relative")');
  base.href = location.origin + '/after/sub/'; await control('release', 'relative');
  check('creation-base-url', await outcome(relative), 'loaded'); base.remove();

  await control('release', 'local');
  const bytes = await (await fetch(url('local'))).arrayBuffer();
  const blob = URL.createObjectURL(new Blob([bytes], {type: 'font/ttf'}));
  const data = 'data:font/ttf;base64,' + btoa(String.fromCharCode(...new Uint8Array(bytes)));
  check('local-font-validation', [await outcome(new FontFace('BlobFont', `url("${blob}")`)),
    await outcome(new FontFace('DataFont', `url("${data}")`)),
    await outcome(new FontFace('Truncated', 'url("data:font/ttf;base64,AAEAAA==")'))], ['loaded', 'loaded', 'NetworkError']);
  URL.revokeObjectURL(blob);

  const iframe = document.createElement('iframe');
  const frameReady = new Promise(resolve => iframe.onload = resolve);
  iframe.srcdoc = '<!doctype html><body>child'; document.body.append(iframe); await frameReady;
  const other = iframe.contentWindow;
  const childFace = new other.FontFace('ChildFont', `url("${url('child')}")`);
  await control('release', 'child');
  const childLoad = FontFace.prototype.load.call(childFace);
  check('cross-realm-load', [childLoad instanceof other.Promise, await childLoad === childFace, childFace.status], [true, true, 'loaded']);
  iframe.remove();

  const failed = new FontFace('RejectedSet', `url("${url('set-error', '&invalid=1')}")`);
  document.fonts.add(failed); await control('release', 'set-error');
  const loadingError = event('loadingerror');
  const result = await document.fonts.load('16px RejectedSet').then(() => 'loaded', e => e.name);
  await document.fonts.ready;
  check('set-rejection-and-ready', [result, document.fonts.status, (await loadingError).fontfaces[0] === failed], ['NetworkError', 'loaded', true]);

  const meta = document.createElement('meta'); meta.httpEquiv = 'Content-Security-Policy'; meta.content = "font-src 'none'"; document.head.append(meta);
  const violation = new Promise(resolve => addEventListener('securitypolicyviolation', e => resolve(e.effectiveDirective), {once: true}));
  await control('release', 'csp');
  check('font-csp', [await outcome(new FontFace('BlockedFont', `url("${url('csp')}")`)), await violation,
    (await control('stats', 'csp')).count], ['NetworkError', 'font-src', 0]);
  return {rows, failures};
}
