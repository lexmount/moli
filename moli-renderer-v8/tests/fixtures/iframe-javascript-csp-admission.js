(owner, scenario, setter = 'idl') => {
  const doc = owner.document;
  const checks = [];
  const events = [];
  const source = 'javascript:globalThis.__frameJsRan = 1';
  const child = doc.createElement('iframe');
  child.id = 'javascript-destination';
  let targetWindow = null;
  let checkpoint = false;
  const check = (name, passed, observed = null) => checks.push({name, passed, observed});
  const listen = (eventDocument, eventWindow, where) => {
    eventDocument.addEventListener('securitypolicyviolation', event => events.push({
      where, target: event.target === eventDocument ? '#document' : event.target.id,
      sample: event.sample, directive: event.effectiveDirective,
      blockedURI: event.blockedURI, disposition: event.disposition,
      trusted: event.isTrusted, native: event instanceof eventWindow.SecurityPolicyViolationEvent,
      checkpoint,
    }));
  };
  const install = (target, sample = true) => {
    const meta = target.createElement('meta');
    meta.httpEquiv = 'Content-Security-Policy';
    meta.content = "script-src 'unsafe-eval'" + (sample ? " 'report-sample'" : '');
    target.head.appendChild(meta);
  };
  const navigate = () => setter === 'idl' ? child.src = source : child.setAttribute('src', source);
  const connect = () => {
    doc.body.appendChild(child);
    targetWindow = child.contentWindow;
    listen(child.contentDocument, targetWindow, 'target');
  };
  listen(doc, owner, 'source');
  if (scenario === 'inherited' || scenario === 'policy-before-connection') {
    if (scenario === 'inherited') install(doc);
    navigate();
    check('a-detached-src-write-does-not-report', events.length === 0, events.slice());
    if (scenario === 'policy-before-connection') install(doc);
    connect();
  } else if (scenario === 'detached') {
    install(doc);
    navigate();
    check('a-detached-frame-has-no-live-window', child.contentWindow === null);
  } else {
    if (scenario === 'srcdoc-priority') child.srcdoc = '<!doctype html><body>srcdoc</body>';
    connect();
    if (['source-only', 'repeat-same-src', 'retarget-before-report', 'getter-reads', 'srcdoc-priority'].includes(scenario)) install(doc);
    if (scenario === 'no-opt-in') install(doc, false);
    if (scenario === 'target-only') install(child.contentDocument);
    navigate();
    if (scenario === 'source-policy-after-admission') install(doc);
    if (scenario === 'repeat-same-src') navigate();
    if (scenario === 'retarget-before-report') child.remove();
    if (scenario === 'cancel-before-execution') child.src = 'about:blank';
    if (scenario === 'getter-reads') {
      for (let index = 0; index < 4; index++) {
        check('window-getter-' + index, child.contentWindow === targetWindow);
        check('document-getter-' + index, child.contentDocument === targetWindow.document);
      }
    }
  }
  check('reports-are-asynchronous', events.length === 0, events.slice());
  queueMicrotask(() => { checkpoint = true; });
  const reports = ['inherited', 'source-only', 'target-only', 'policy-before-connection',
                   'repeat-same-src', 'retarget-before-report', 'getter-reads', 'no-opt-in'].includes(scenario);
  const expectedCount = !reports ? 0 : scenario === 'repeat-same-src' ? 2 : 1;
  const expectedTarget = scenario === 'retarget-before-report' || scenario === 'target-only' ? '#document' : child.id;
  const expectedOwner = scenario === 'target-only' ? 'target' : 'source';
  const shouldRun = ['permissive', 'source-policy-after-admission'].includes(scenario);
  return {
    finish() {
      check('execution-respects-both-admission-and-target-policy', (targetWindow?.__frameJsRan === 1) === shouldRun,
            targetWindow?.__frameJsRan ?? null);
      check('one-report-per-admitted-navigation-and-policy', events.length === expectedCount, events.slice());
      check('reports-use-the-policy-document', events.every(event => event.where === expectedOwner), events.slice());
      check('reports-retain-the-frame-element-until-dispatch', events.every(event => event.target === expectedTarget), events.slice());
      check('reports-preserve-the-decoded-source-and-sample-opt-in',
            events.every(event => event.sample === (scenario === 'no-opt-in' ? '' : source)), events.slice());
      check('reports-use-the-effective-navigation-directive', events.every(event => event.directive === 'script-src-elem'), events.slice());
      check('blocked-URI-is-inline', events.every(event => event.blockedURI === 'inline'), events.slice());
      check('reports-preserve-enforcement-disposition', events.every(event => event.disposition === 'enforce'), events.slice());
      check('reports-use-the-native-owner-realm-interface', events.every(event => event.native), events.slice());
      check('reports-are-trusted', events.every(event => event.trusted), events.slice());
      check('reporting-follows-the-triggering-task-checkpoint', events.every(event => event.checkpoint), events.slice());
      return {complete: true, total: checks.length, passed: checks.filter(row => row.passed).length, checks};
    },
  };
}
