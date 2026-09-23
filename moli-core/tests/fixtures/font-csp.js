async function(crossOrigin) {
  const rows = [], failures = [];
  const control = (action, token, origin = location.origin) =>
    fetch(origin + '/probe-' + action + '?token=' + token).then(r => r.json());
  const url = (token, origin = location.origin, path = '/probe-asset') =>
    origin + path + '?as=font&token=' + token;
  const frame = async (policy, reportOnly = false) => {
    const iframe = document.createElement('iframe');
    const ready = new Promise(resolve => iframe.onload = resolve);
    if (reportOnly) iframe.src = '/?reportPolicy=' + encodeURIComponent(policy);
    else iframe.srcdoc = `<!doctype html><meta http-equiv="Content-Security-Policy" content="${policy}"><body>font CSP`;
    document.body.append(iframe); await ready;
    return iframe;
  };
  const outcome = face => face.load().then(() => 'loaded', e => e.name);
  const check = (name, actual, expected) => {
    rows.push({name, actual});
    if (JSON.stringify(actual) !== JSON.stringify(expected)) failures.push({name, actual, expected});
  };

  const denied = await frame("font-src 'none'");
  const child = denied.contentWindow;
  const violation = new Promise(resolve => child.addEventListener('securitypolicyviolation', resolve, {once: true}));
  await control('release', 'child-block'); await control('release', 'parent-allow');
  const childFace = new child.FontFace('Blocked', `url("${url('child-block')}")`);
  const childResult = await FontFace.prototype.load.call(childFace).then(() => 'loaded', e => e.name);
  const event = await violation;
  check('creation-document-csp', [childResult, event.effectiveDirective, event.target === child.document,
    (await control('stats', 'child-block')).count,
    await outcome(new FontFace('Allowed', `url("${url('parent-allow')}")`))],
    ['NetworkError', 'font-src', true, 0, 'loaded']);
  denied.remove();

  const redirectFrame = await frame('font-src ' + location.origin + '/probe-asset');
  const other = redirectFrame.contentWindow;
  await control('release', 'redirect-cross'); await control('release', 'blocked-final', crossOrigin);
  const initial = url('redirect-cross') + '&status=302&redirect=' + encodeURIComponent(url('blocked-final', crossOrigin));
  const redirectedViolation = new Promise(resolve => other.addEventListener('securitypolicyviolation', resolve, {once: true}));
  const redirected = await outcome(new other.FontFace('RedirectBlocked', `url("${initial}")`));
  const redirectEvent = await redirectedViolation;
  check('redirect-blocks-before-wire', [redirected, redirectEvent.effectiveDirective, redirectEvent.blockedURI,
    (await control('stats', 'redirect-cross')).count, (await control('stats', 'blocked-final', crossOrigin)).count],
    ['NetworkError', 'font-src', initial, 1, 0]);

  await control('release', 'redirect-path'); await control('release', 'allowed-final');
  const pathRedirect = url('redirect-path') + '&status=302&redirect=' + encodeURIComponent(url('allowed-final', location.origin, '/probe-font-final'));
  check('redirect-ignores-source-path', [await outcome(new other.FontFace('RedirectPath', `url("${pathRedirect}")`)),
    (await control('stats', 'redirect-path')).count, (await control('stats', 'allowed-final')).count], ['loaded', 1, 1]);
  redirectFrame.remove();

  const reported = await frame("font-src 'none'", true);
  const reporting = reported.contentWindow;
  const report = new Promise(resolve => reporting.addEventListener('securitypolicyviolation', resolve, {once: true}));
  await control('release', 'report-only');
  const reportedResult = await outcome(new reporting.FontFace('Reported', `url("${url('report-only')}")`));
  const reportEvent = await report;
  check('report-only-does-not-block', [reportedResult, reportEvent.disposition, reportEvent.effectiveDirective,
    (await control('stats', 'report-only')).count], ['loaded', 'report', 'font-src', 1]);
  reported.remove();
  return {rows, failures};
}
