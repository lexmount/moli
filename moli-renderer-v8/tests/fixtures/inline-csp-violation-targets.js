(async () => {
  const checks = [];
  const check = (name, passed, details = '') => checks.push({name, passed: !!passed, details});
  const popup = open();
  const owners = [
    ['root', globalThis],
    ['child', document.querySelector('iframe').contentWindow],
    ['popup', popup],
  ];
  const runs = [];
  const cleanups = [];
  let checkpointRan = false;
  Promise.resolve().then(() => { checkpointRan = true; });
  try {
    for (const [label, owner] of owners) {
      const doc = owner.document;
      const intrinsic = owner.SecurityPolicyViolationEvent;
      const entries = new Map();
      const trace = [];
      const meta = doc.createElement('meta');
      meta.httpEquiv = 'Content-Security-Policy';
      meta.content = "script-src-attr 'none' 'report-sample'";
      doc.head.appendChild(meta);
      owner.__inlineCspExecuted = 0;
      owner.SecurityPolicyViolationEvent = function () { throw Error('author constructor'); };
      cleanups.push(() => { owner.SecurityPolicyViolationEvent = intrinsic; });
      const observeDocument = event => {
        const entry = entries.get(event.sample);
        if (!entry) return;
        entry.documentEvents.push(event);
        entry.documentTargets.push(event.target);
        entry.afterCheckpoint.push(checkpointRan);
        trace.push(entry.name);
        queueMicrotask(() => trace.push('micro:' + entry.name));
      };
      const observeWindow = event => {
        const entry = entries.get(event.sample);
        if (entry) {
          entry.windowTargets.push(event.target);
          // Document callbacks have their own microtask checkpoint. Complete
          // only after the same event has reached the Window listener.
          entry.resolve();
        }
      };
      doc.addEventListener('securitypolicyviolation', observeDocument);
      owner.addEventListener('securitypolicyviolation', observeWindow);
      cleanups.push(() => {
        doc.removeEventListener('securitypolicyviolation', observeDocument);
        owner.removeEventListener('securitypolicyviolation', observeWindow);
      });
      const add = (name, mode) => {
        const element = mode === 'body' ? doc.body : doc.createElement('button');
        const source = `/*${label}:${name}*/globalThis.__inlineCspExecuted++`;
        const entry = {
          name, element, sample: source.slice(0, 40), documentEvents: [],
          documentTargets: [], windowTargets: [], elementTargets: [], afterCheckpoint: [],
          target: doc, outsideTarget: doc,
        };
        entry.done = new Promise(resolve => { entry.resolve = resolve; });
        entries.set(entry.sample, entry);
        element.addEventListener('securitypolicyviolation', event => {
          if (event.sample === entry.sample) entry.elementTargets.push(event.target);
        });
        if (mode === 'shadow-open' || mode === 'shadow-closed') {
          const host = doc.createElement('div');
          doc.body.appendChild(host);
          host.attachShadow({mode: mode.slice(7)}).appendChild(element);
          entry.target = element;
          entry.outsideTarget = host;
        } else if (['connected', 'removed', 'adopted', 'body'].includes(mode)) {
          if (mode !== 'body') doc.body.appendChild(element);
          if (mode !== 'body') entry.target = entry.outsideTarget = element;
        } else if (mode === 'fragment') {
          doc.createDocumentFragment().appendChild(element);
        }
        const property = mode === 'body' ? 'onresize' : 'onclick';
        element.setAttribute(property, source);
        if (mode === 'connected' || mode.startsWith('shadow-')) element.click();
        const compiled = mode === 'body' ? owner[property] : element[property];
        check(`${label}:${name}:blocked`, compiled === null);
        // A failed compilation is cached. Reading it again cannot report twice.
        check(`${label}:${name}:cached`, element[property] === null);
        if (mode === 'connected') element.removeAttribute(property);
        if (mode === 'removed') {
          element.remove();
          entry.target = entry.outsideTarget = doc;
        } else if (mode === 'adopted') {
          const destination = document.implementation.createHTMLDocument('destination');
          destination.body.appendChild(element);
          entry.target = entry.outsideTarget = doc;
        } else if (mode === 'connected-later') {
          doc.body.appendChild(element);
          entry.target = entry.outsideTarget = element;
        }
      };
      for (const mode of [
        'connected', 'detached', 'removed', 'adopted', 'fragment',
        'connected-later', 'shadow-open', 'shadow-closed', 'body',
      ]) add(mode, mode);
      check(`${label}:task-boundary`, [...entries.values()].every(entry => entry.documentEvents.length === 0));
      const allowed = doc.createElement('button');
      allowed.onclick = () => { owner.__inlineCspExecuted++; };
      allowed.click();
      check(`${label}:IDL-handler-allowed`, owner.__inlineCspExecuted === 1);
      runs.push({label, owner, intrinsic, entries, trace});
    }
    let timer;
    await Promise.race([
      Promise.all(runs.flatMap(run => [...run.entries.values()].map(entry => entry.done))),
      new Promise(resolve => { timer = setTimeout(resolve, 3000); }),
    ]);
    clearTimeout(timer);
    await Promise.resolve();
    for (const {label, owner, intrinsic, entries, trace} of runs) {
      for (const entry of entries.values()) {
        const prefix = `${label}:${entry.name}:`;
        const events = entry.documentEvents;
        const event = events[0];
        const details = JSON.stringify(events.map(value => ({
          sample: value.sample, disposition: value.disposition,
          target: value.target?.nodeName, sourceFile: value.sourceFile,
          lineNumber: value.lineNumber, columnNumber: value.columnNumber,
        })));
        check(prefix + 'one-event', events.length === 1, details);
        check(prefix + 'document-target', entry.documentTargets.length === 1 && entry.documentTargets[0] === entry.outsideTarget, details);
        check(prefix + 'window-bubbling', entry.windowTargets.length === 1 && entry.windowTargets[0] === entry.outsideTarget);
        check(prefix + 'element-target', entry.target === entry.element
          ? entry.elementTargets.length === 1 && entry.elementTargets[0] === entry.element
          : entry.elementTargets.length === 0);
        check(prefix + 'native-event-realm', event instanceof intrinsic);
        check(prefix + 'trusted-bubbling-composed', event?.isTrusted === true && event.bubbles === true && event.composed === true);
        check(prefix + 'policy-fields', event?.blockedURI === 'inline' && event.effectiveDirective === 'script-src-attr' && event.disposition === 'enforce');
        check(prefix + 'immutable-sample', event?.sample === entry.sample);
        check(prefix + 'after-microtask', entry.afterCheckpoint.length === 1 && entry.afterCheckpoint[0] === true);
      }
      check(`${label}:enforcement`, owner.__inlineCspExecuted === 1);
      const expected = [...entries.values()].flatMap(entry => [entry.name, 'micro:' + entry.name]);
      check(`${label}:task-FIFO-and-checkpoints`, JSON.stringify(trace) === JSON.stringify(expected), JSON.stringify(trace));
    }
  } catch (error) {
    check('fixture-completed-without-exception', false, String(error.stack || error));
  } finally {
    for (const cleanup of cleanups.reverse()) cleanup();
    popup?.close();
  }
  globalThis.__uiEventResults = {
    complete: true, checks, total: checks.length,
    passed: checks.filter(row => row.passed).length,
  };
  return checks.every(row => row.passed);
})()
