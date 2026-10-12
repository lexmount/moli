(owner => {
  const doc = owner.document;
  const checks = [];
  const events = [];
  const check = (name, passed, observed) => checks.push({
    name, passed, observed: observed === undefined ? null : JSON.parse(JSON.stringify(observed)),
  });
  const mutations = [
    ['setAttribute', (node, source) => node.setAttribute('onclick', source)],
    ['Attr.value', (node, source) => { node.getAttributeNode('onclick').value = source; }],
    ['setAttributeNode', (node, source) => {
      const attr = doc.createAttribute('onclick');
      attr.value = source;
      node.setAttributeNode(attr);
    }],
    ['NamedNodeMap', (node, source) => {
      const attr = doc.createAttribute('onclick');
      attr.value = source;
      node.attributes.setNamedItem(attr);
    }],
    ['setAttributeNS', (node, source) => node.setAttributeNS(null, 'onclick', source)],
  ];
  const button = id => {
    const node = doc.createElement('button');
    node.id = id;
    doc.body.appendChild(node);
    return node;
  };
  const nodes = mutations.map(([name, mutate], index) => {
    const node = button('admission-' + index);
    const trace = [];
    node.setAttribute('onclick', 'return 1');
    node.onclick = null;
    node.addEventListener('click', () => trace.push('before'));
    const previous = () => trace.push('previous');
    node.onclick = previous;
    node.addEventListener('click', () => trace.push('after'));
    return {name, mutate, node, trace, previous};
  });
  const raw = button('admission-raw');
  raw.setAttribute('onclick', "return '\ud800'");
  const allowed = button('admission-allowed');
  allowed.setAttribute('onclick', 'return 23');
  const fresh = button('admission-fresh');
  const foreign = doc.createElementNS('urn:admission', 'button');
  doc.body.appendChild(foreign);
  const parserRoot = doc.createElement('div');
  doc.body.appendChild(parserRoot);
  const previousBody = () => 29;
  doc.body.onresize = previousBody;
  doc.addEventListener('securitypolicyviolation', event => events.push({
    directive: event.effectiveDirective,
    disposition: event.disposition,
    sample: event.sample,
    target: event.target === doc.body ? 'body' : event.target.id,
  }));
  const meta = doc.createElement('meta');
  meta.httpEquiv = 'Content-Security-Policy';
  meta.content = "script-src-attr 'none' 'report-sample'";
  doc.head.appendChild(meta);
  for (const entry of nodes) entry.mutate(entry.node, 'return 31');
  raw.setAttribute('onclick', 'return 37');
  fresh.setAttribute('onclick', 'return 41');
  fresh.setAttribute('onclick', 'return 41');
  doc.body.setAttribute('onresize', 'return 43');
  parserRoot.innerHTML = '<button id="admission-parser" onclick="return 47"></button>';
  const nonContentNames = ['onnotanevent', 'onreadystatechange', 'onvisibilitychange', 'onmessage'];
  for (const name of nonContentNames) {
    fresh.setAttribute(name, 'globalThis.admissionUnexpected = true');
  }
  foreign.setAttribute('onclick', 'globalThis.admissionUnexpected = true');
  fresh.setAttributeNS('urn:admission', 'ns:onclick', 'return 59');
  fresh.removeAttribute('onload');
  check('reports-are-asynchronous', events.length === 0, events.length);
  const expectedReports = nodes.map(entry => [entry.node.id, 'return 31']).concat([
    ['admission-raw', 'return 37'],
    ['admission-fresh', 'return 41'],
    ['admission-fresh', 'return 41'],
    ['body', 'return 43'],
    ['admission-parser', 'return 47'],
  ]);
  return {
    beforeGetter() {
      check('reports-before-getters', events.length === expectedReports.length, events);
      check('report-samples-and-targets',
        JSON.stringify(events.map(event => [event.target, event.sample])) === JSON.stringify(expectedReports), events);
      check('report-directive-and-disposition', events.every(event =>
        event.directive === 'script-src-attr' && event.disposition === 'enforce'), events);
      for (const {name, node, previous, trace} of nodes) {
        check(name + ':keeps-idl-handler', node.onclick === previous);
        check(name + ':updates-attribute', node.getAttribute('onclick') === 'return 31');
        node.dispatchEvent(new owner.Event('click'));
        check(name + ':keeps-listener-position', JSON.stringify(trace) === '["before","previous","after"]', trace.slice());
      }
      check('keeps-uncompiled-source', typeof raw.onclick === 'function' && raw.onclick() === '\ud800');
      check('later-policy-does-not-block-admitted-source', typeof allowed.onclick === 'function' && allowed.onclick() === 23);
      check('fresh-blocked-handler-is-null', fresh.onclick === null && fresh.onclick === null);
      check('parser-blocked-handler-is-null', parserRoot.firstChild.onclick === null);
      check('body-keeps-window-handler', doc.body.onresize === previousBody && owner.onresize === previousBody);
      fresh.removeAttribute('onclick');
      fresh.removeAttribute('onclick');
      check('removal-does-not-activate-blocked-source', fresh.onclick === null);
      const {node, trace, previous} = nodes[0];
      node.removeAttribute('onclick');
      check('removal-deactivates-prior-idl-handler', node.onclick === null);
      node.dispatchEvent(new owner.Event('click'));
      node.onclick = previous;
      node.dispatchEvent(new owner.Event('click'));
      check('reactivation-uses-new-listener-position',
        JSON.stringify(trace) === '["before","previous","after","before","after","before","after","previous"]', trace.slice());
      for (const name of nonContentNames) {
        fresh.addEventListener(name.slice(2), () => {});
        fresh.dispatchEvent(new owner.Event(name.slice(2)));
        check(name + ':not-a-content-handler', owner.admissionUnexpected !== true);
      }
      foreign.addEventListener('click', () => {});
      foreign.dispatchEvent(new owner.Event('click'));
      check('foreign-namespace-not-a-content-handler', owner.admissionUnexpected !== true);
      return checks;
    },
    finish() {
      check('getters-and-removals-do-not-report-again', events.length === expectedReports.length, events);
      doc.body.onresize = null;
      const result = {complete: true, total: checks.length, passed: checks.filter(check => check.passed).length, checks};
      return result;
    },
  };
})
