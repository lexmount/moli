(owner => {
  const doc = owner.document;
  const checks = [];
  const events = [];
  let checkpoint = false;
  const check = (name, passed, observed = null) => checks.push({
    name, passed, observed: JSON.parse(JSON.stringify(observed)),
  });
  doc.addEventListener('securitypolicyviolation', event => events.push({
    target: event.target === doc ? '#document' : event.target.id,
    directive: event.effectiveDirective,
    disposition: event.disposition,
    sample: event.sample,
    trusted: event.isTrusted,
    native: event instanceof owner.SecurityPolicyViolationEvent,
    checkpoint,
  }));
  const meta = doc.createElement('meta');
  meta.httpEquiv = 'Content-Security-Policy';
  meta.content = "script-src 'unsafe-eval'; script-src-elem 'none' 'report-sample'; " +
    "script-src-attr 'none' 'report-sample'; style-src 'none' 'report-sample'";
  doc.head.appendChild(meta);
  const expected = [];
  const append = (tag, id, source) => {
    const element = doc.createElement(tag);
    element.id = id;
    if (source !== undefined) element.textContent = source;
    doc.body.appendChild(element);
    return element;
  };
  const block = (suffix, retarget) => {
    const script = append('script', 'script-' + suffix, 'globalThis.inlineCspScriptRan = true');
    expected.push([retarget ? '#document' : script.id, 'script-src-elem', script.textContent]);
    const button = append('button', 'handler-' + suffix);
    button.setAttribute('onclick', 'return 83');
    expected.push([retarget ? '#document' : button.id, 'script-src-attr', 'return 83']);
    const style = append('style', 'style-' + suffix, '.inline-csp { margin-left: 13px; }');
    expected.push([retarget ? '#document' : style.id, 'style-src-elem', style.textContent]);
    const div = append('div', 'attribute-' + suffix);
    div.setAttribute('style', 'margin-right: 17px;');
    expected.push([retarget ? '#document' : div.id, 'style-src-attr', 'margin-right: 17px;']);
    check(suffix + ':script-is-blocked', owner.inlineCspScriptRan !== true);
    check(suffix + ':handler-is-blocked', button.onclick === null);
    check(suffix + ':stylesheet-is-blocked', style.sheet === null);
    check(suffix + ':style-attribute-is-blocked', div.style.length === 0, div.style.cssText);
    if (retarget) {
      script.remove();
      button.remove();
      const inert = doc.implementation.createHTMLDocument('');
      style.remove();
      inert.body.appendChild(div);
    }
  };
  block('connected', false);
  block('retargeted', true);
  check('reports-are-asynchronous', events.length === 0, events);
  owner.queueMicrotask(() => { checkpoint = true; });
  return {
    finish() {
      check('one-report-per-inline-check', events.length === 8, events);
      check('inline-checks-share-fifo-order-and-dispatch-time-targeting',
        JSON.stringify(events.map(event => [event.target, event.directive, event.sample])) ===
          JSON.stringify(expected), events);
      check('violations-use-the-owner-native-event-interface', events.every(event => event.native), events);
      check('violations-are-trusted', events.every(event => event.trusted), events);
      check('violations-run-after-the-triggering-task-checkpoint', events.every(event => event.checkpoint), events);
      check('violations-preserve-enforcement-disposition', events.every(event => event.disposition === 'enforce'), events);
      return {complete: true, total: checks.length, passed: checks.filter(check => check.passed).length, checks};
    },
  };
})
